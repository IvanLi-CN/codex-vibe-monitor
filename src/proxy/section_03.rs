fn extract_bearer_token(headers: &HeaderMap) -> Option<String> {
    let authorization = headers.get(header::AUTHORIZATION)?.to_str().ok()?.trim();
    let (scheme, token) = authorization.split_once(' ')?;
    if !scheme.eq_ignore_ascii_case("bearer") {
        return None;
    }
    let normalized = token.trim();
    if normalized.is_empty() {
        None
    } else {
        Some(normalized.to_string())
    }
}

async fn request_matches_pool_route(state: &AppState, headers: &HeaderMap) -> Result<bool> {
    let Some(api_key) = extract_bearer_token(headers) else {
        return Ok(false);
    };
    pool_api_key_matches(state, &api_key).await
}

#[derive(Debug, Default, Deserialize)]
struct StickyKeyProjection {
    #[serde(default)]
    metadata: StickyKeyProjectionMetadata,
    #[serde(default)]
    sticky_key: Option<String>,
    #[serde(default, rename = "stickyKey")]
    sticky_key_alias: Option<String>,
    #[serde(default, rename = "prompt_cache_key")]
    prompt_cache_key: Option<String>,
    #[serde(default, rename = "promptCacheKey")]
    prompt_cache_key_alias: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct StickyKeyProjectionMetadata {
    #[serde(default)]
    sticky_key: Option<String>,
    #[serde(default, rename = "stickyKey")]
    sticky_key_alias: Option<String>,
    #[serde(default, rename = "prompt_cache_key")]
    prompt_cache_key: Option<String>,
    #[serde(default, rename = "promptCacheKey")]
    prompt_cache_key_alias: Option<String>,
}

impl StickyKeyProjection {
    fn into_sticky_key(self) -> Option<String> {
        [
            self.metadata.sticky_key,
            self.metadata.sticky_key_alias,
            self.metadata.prompt_cache_key,
            self.metadata.prompt_cache_key_alias,
            self.sticky_key,
            self.sticky_key_alias,
            self.prompt_cache_key,
            self.prompt_cache_key_alias,
        ]
        .into_iter()
        .flatten()
        .map(|value| value.trim().to_string())
        .find(|value| !value.is_empty())
    }
}

fn extract_sticky_key_from_request_body_projection(bytes: &[u8]) -> Option<String> {
    serde_json::from_slice::<StickyKeyProjection>(bytes)
        .ok()
        .and_then(StickyKeyProjection::into_sticky_key)
}

async fn extract_sticky_key_from_replay_snapshot(
    snapshot: &PoolReplayBodySnapshot,
) -> Option<String> {
    match snapshot {
        PoolReplayBodySnapshot::Empty => None,
        PoolReplayBodySnapshot::Memory(bytes) => {
            extract_sticky_key_from_request_body_projection(bytes.as_ref())
        }
        PoolReplayBodySnapshot::File { temp_file, .. } => {
            let path = temp_file.path.clone();
            tokio::task::spawn_blocking(move || {
                let file = std::fs::File::open(path).ok()?;
                serde_json::from_reader::<_, StickyKeyProjection>(std::io::BufReader::new(file))
                    .ok()
                    .and_then(StickyKeyProjection::into_sticky_key)
            })
            .await
            .ok()
            .flatten()
        }
    }
}

fn pool_account_supports_live_request_body(account: &PoolResolvedAccount, original_uri: &Uri) -> bool {
    match &account.auth {
        PoolResolvedAuth::ApiKey { .. } => true,
        PoolResolvedAuth::Oauth { .. } => original_uri.path() != "/v1/responses",
    }
}

fn should_prebuffer_for_body_sticky_probe(
    has_header_sticky_key: bool,
    content_type: Option<&str>,
    body_size_hint_exact: Option<usize>,
) -> bool {
    !has_header_sticky_key
        && content_type.is_some_and(|value| {
            value
                .to_ascii_lowercase()
                .starts_with("application/json")
        })
        && body_size_hint_exact
            .is_some_and(|value| value <= POOL_REQUEST_REPLAY_MEMORY_THRESHOLD_BYTES)
}

async fn send_pool_request_live_first_attempt(
    state: Arc<AppState>,
    proxy_request_id: u64,
    method: Method,
    original_uri: &Uri,
    headers: &HeaderMap,
    body: reqwest::Body,
    runtime_timeouts: &PoolRoutingTimeoutSettingsResolved,
    handshake_timeout: Duration,
    responses_total_timeout: Option<Duration>,
    responses_total_timeout_started_at: Option<Instant>,
    sticky_key: Option<&str>,
    account: PoolResolvedAccount,
    replay_status_rx: &watch::Receiver<PoolReplayBodyStatus>,
) -> Result<PoolUpstreamResponse, PoolUpstreamError> {
    let pre_first_byte_timeout =
        pool_upstream_first_chunk_timeout(runtime_timeouts, original_uri, &method);
    let Some(attempt_send_timeout) = pool_timeout_budget_with_total_limit(
        pool_upstream_send_timeout(
            original_uri,
            &method,
            handshake_timeout,
            pre_first_byte_timeout,
        ),
        responses_total_timeout,
        responses_total_timeout_started_at,
    ) else {
        return Err(build_pool_total_timeout_exhausted_error(
            responses_total_timeout.expect("live first attempt total timeout should be present"),
            None,
            1,
            1,
        ));
    };
    let Some(attempt_pre_first_byte_timeout) = pool_timeout_budget_with_total_limit(
        pre_first_byte_timeout,
        responses_total_timeout,
        responses_total_timeout_started_at,
    ) else {
        return Err(build_pool_total_timeout_exhausted_error(
            responses_total_timeout.expect("live first attempt total timeout should be present"),
            None,
            1,
            1,
        ));
    };

    let reservation_key = build_pool_routing_reservation_key(proxy_request_id);
    reserve_pool_routing_account(state.as_ref(), &reservation_key, &account);
    let request_connection_scoped = connection_scoped_header_names(headers);
    let connect_started = Instant::now();
    let attempt_started_at_utc = Utc::now();
    let attempted_request_body_for_capture = None;
    let attempted_requested_service_tier = None;
    let (response, mut oauth_responses_debug, forward_proxy_selection) = match &account.auth {
        PoolResolvedAuth::ApiKey { authorization } => {
            let (forward_proxy_scope, selected_proxy, client) =
                match select_pool_account_forward_proxy_client(state.as_ref(), &account).await {
                    Ok(selection) => selection,
                    Err(message) => {
                        release_pool_routing_reservation(state.as_ref(), &reservation_key);
                        return Err(PoolUpstreamError {
                            account: Some(account.clone()),
                            status: StatusCode::BAD_GATEWAY,
                            message,
                            failure_kind: PROXY_FAILURE_FAILED_CONTACT_UPSTREAM,
                            connect_latency_ms: 0.0,
                            upstream_error_code: None,
                            upstream_error_message: None,
                            upstream_request_id: None,
                            oauth_responses_debug: None,
                            attempt_summary: pool_attempt_summary(
                                1,
                                1,
                                Some(PROXY_FAILURE_FAILED_CONTACT_UPSTREAM.to_string()),
                            ),
                            requested_service_tier: attempted_requested_service_tier.clone(),
                            request_body_for_capture: attempted_request_body_for_capture.clone(),
                        });
                    }
                };
            let api_key_target_url =
                match build_proxy_upstream_url(&account.upstream_base_url, original_uri) {
                    Ok(url) => url,
                    Err(err) => {
                        release_pool_routing_reservation(state.as_ref(), &reservation_key);
                        return Err(PoolUpstreamError {
                            account: Some(account.clone()),
                            status: StatusCode::BAD_GATEWAY,
                            message: format!("failed to build pool upstream url: {err}"),
                            failure_kind: PROXY_FAILURE_FAILED_CONTACT_UPSTREAM,
                            connect_latency_ms: 0.0,
                            upstream_error_code: None,
                            upstream_error_message: None,
                            upstream_request_id: None,
                            oauth_responses_debug: None,
                            attempt_summary: pool_attempt_summary(
                                1,
                                1,
                                Some(PROXY_FAILURE_FAILED_CONTACT_UPSTREAM.to_string()),
                            ),
                            requested_service_tier: attempted_requested_service_tier.clone(),
                            request_body_for_capture: attempted_request_body_for_capture.clone(),
                        });
                    }
                };
            let mut request = client.request(method.clone(), api_key_target_url);
            for (name, value) in headers {
                if *name == header::AUTHORIZATION || *name == header::CONTENT_LENGTH {
                    continue;
                }
                if should_forward_proxy_header(name, &request_connection_scoped) {
                    request = request.header(name, value);
                }
            }
            request = request.header(header::AUTHORIZATION, authorization.clone());
            request = request.body(body);
            match timeout(attempt_send_timeout, request.send()).await {
                Ok(Ok(response)) => (
                    ProxyUpstreamResponseBody::Reqwest(response),
                    None,
                    Some((forward_proxy_scope, selected_proxy)),
                ),
                Ok(Err(err)) => {
                    record_pool_account_forward_proxy_result(
                        state.as_ref(),
                        &forward_proxy_scope,
                        &selected_proxy,
                        ForwardProxyRouteResultKind::NetworkFailure,
                    )
                    .await;
                    release_pool_routing_reservation(state.as_ref(), &reservation_key);
                    return Err(PoolUpstreamError {
                        account: Some(account.clone()),
                        status: StatusCode::BAD_GATEWAY,
                        message: format!("failed to contact upstream: {err}"),
                        failure_kind: PROXY_FAILURE_FAILED_CONTACT_UPSTREAM,
                        connect_latency_ms: elapsed_ms(connect_started),
                        upstream_error_code: None,
                        upstream_error_message: None,
                        upstream_request_id: None,
                        oauth_responses_debug: None,
                        attempt_summary: pool_attempt_summary(
                            1,
                            1,
                            Some(PROXY_FAILURE_FAILED_CONTACT_UPSTREAM.to_string()),
                        ),
                        requested_service_tier: attempted_requested_service_tier.clone(),
                        request_body_for_capture: attempted_request_body_for_capture.clone(),
                    });
                }
                Err(_) => {
                    record_pool_account_forward_proxy_result(
                        state.as_ref(),
                        &forward_proxy_scope,
                        &selected_proxy,
                        ForwardProxyRouteResultKind::NetworkFailure,
                    )
                    .await;
                    release_pool_routing_reservation(state.as_ref(), &reservation_key);
                    return Err(PoolUpstreamError {
                        account: Some(account.clone()),
                        status: StatusCode::BAD_GATEWAY,
                        message: proxy_request_send_timeout_message(
                            capture_target_for_request(original_uri.path(), &method),
                            attempt_send_timeout,
                        ),
                        failure_kind: PROXY_FAILURE_UPSTREAM_HANDSHAKE_TIMEOUT,
                        connect_latency_ms: elapsed_ms(connect_started),
                        upstream_error_code: None,
                        upstream_error_message: None,
                        upstream_request_id: None,
                        oauth_responses_debug: None,
                        attempt_summary: pool_attempt_summary(
                            1,
                            1,
                            Some(PROXY_FAILURE_UPSTREAM_HANDSHAKE_TIMEOUT.to_string()),
                        ),
                        requested_service_tier: attempted_requested_service_tier.clone(),
                        request_body_for_capture: attempted_request_body_for_capture.clone(),
                    });
                }
            }
        }
        PoolResolvedAuth::Oauth {
            access_token,
            chatgpt_account_id,
        } => {
            let (forward_proxy_scope, selected_proxy, client) =
                match select_pool_account_forward_proxy_client(state.as_ref(), &account).await {
                    Ok(selection) => selection,
                    Err(message) => {
                        release_pool_routing_reservation(state.as_ref(), &reservation_key);
                        return Err(PoolUpstreamError {
                            account: Some(account.clone()),
                            status: StatusCode::BAD_GATEWAY,
                            message,
                            failure_kind: PROXY_FAILURE_FAILED_CONTACT_UPSTREAM,
                            connect_latency_ms: 0.0,
                            upstream_error_code: None,
                            upstream_error_message: None,
                            upstream_request_id: None,
                            oauth_responses_debug: None,
                            attempt_summary: pool_attempt_summary(
                                1,
                                1,
                                Some(PROXY_FAILURE_FAILED_CONTACT_UPSTREAM.to_string()),
                            ),
                            requested_service_tier: attempted_requested_service_tier.clone(),
                            request_body_for_capture: attempted_request_body_for_capture.clone(),
                        });
                    }
                };
            let oauth_response = oauth_bridge::send_oauth_upstream_request(
                &client,
                method.clone(),
                original_uri,
                headers,
                oauth_bridge::OauthUpstreamRequestBody::Stream {
                    body,
                    debug_body_prefix: None,
                    request_is_stream: None,
                    snapshot_kind: None,
                },
                attempt_send_timeout,
                attempt_pre_first_byte_timeout,
                Some(account.account_id),
                access_token,
                chatgpt_account_id.as_deref(),
                state.upstream_accounts.crypto_key.as_ref(),
            )
            .await;
            (
                ProxyUpstreamResponseBody::Axum(oauth_response.response),
                oauth_response.request_debug,
                Some((forward_proxy_scope, selected_proxy)),
            )
        }
    };

    let connect_latency_ms = elapsed_ms(connect_started);
    let status = response.status();
    if status == StatusCode::TOO_MANY_REQUESTS
        || status.is_server_error()
        || matches!(
            status,
            StatusCode::UNAUTHORIZED | StatusCode::PAYMENT_REQUIRED | StatusCode::FORBIDDEN
        )
    {
        let upstream_request_id_header = response
            .headers()
            .get("x-request-id")
            .and_then(|value| value.to_str().ok())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| value.to_string());
        let oauth_transport_failure_kind =
            oauth_bridge::oauth_transport_failure_kind(response.headers());
        if oauth_transport_failure_kind.is_some()
            && let Some((forward_proxy_scope, selected_proxy)) = forward_proxy_selection.as_ref()
        {
            record_pool_account_forward_proxy_result(
                state.as_ref(),
                forward_proxy_scope,
                selected_proxy,
                ForwardProxyRouteResultKind::NetworkFailure,
            )
            .await;
        }
        let (upstream_error_code, upstream_error_message, upstream_request_id, message) =
            match read_pool_upstream_bytes_with_timeout(
                response,
                attempt_pre_first_byte_timeout,
                connect_started,
                "reading upstream error body",
            )
            .await
            {
                Ok(body_bytes) => summarize_pool_upstream_http_failure(
                    status,
                    upstream_request_id_header.as_deref(),
                    &body_bytes,
                ),
                Err(err) => (
                    None,
                    None,
                    upstream_request_id_header,
                    format!(
                        "pool upstream responded with {} (failed to read error body: {err})",
                        status.as_u16()
                    ),
                ),
            };
        let route_error_message = upstream_error_code
            .as_deref()
            .map_or_else(|| message.clone(), |code| format!("{code}: {message}"));
        let http_failure_classification =
            classify_pool_account_http_failure(&account.kind, status, &route_error_message);
        let failure_kind =
            oauth_transport_failure_kind.unwrap_or(http_failure_classification.failure_kind);
        let compact_support_observation = classify_compact_support_observation(
            original_uri,
            Some(status),
            Some(route_error_message.as_str()),
        );
        if let Some(observation) = compact_support_observation.as_ref()
            && let Err(observation_err) = record_compact_support_observation(
                &state.pool,
                account.account_id,
                observation.status,
                observation.reason.as_deref(),
            )
            .await
        {
            warn!(
                account_id = account.account_id,
                error = %observation_err,
                "failed to record compact support observation"
            );
        }
        let route_failure_result = if oauth_transport_failure_kind.is_some() {
            record_pool_route_transport_failure(
                &state.pool,
                account.account_id,
                sticky_key,
                route_error_message.as_str(),
                None,
            )
            .await
        } else {
            record_pool_route_http_failure(
                &state.pool,
                account.account_id,
                &account.kind,
                sticky_key,
                status,
                &route_error_message,
                None,
            )
            .await
        };
        if let Err(route_err) = route_failure_result {
            warn!(
                account_id = account.account_id,
                error = %route_err,
                "failed to record pool live-attempt failure"
            );
        }
        release_pool_routing_reservation(state.as_ref(), &reservation_key);
        maybe_backfill_oauth_request_debug_from_replay_status(
            &mut oauth_responses_debug,
            original_uri,
            replay_status_rx,
            state.upstream_accounts.crypto_key.as_ref(),
        )
        .await;
        return Err(PoolUpstreamError {
            account: Some(account),
            status,
            message: route_error_message,
            failure_kind,
            connect_latency_ms,
            upstream_error_code,
            upstream_error_message,
            upstream_request_id,
            oauth_responses_debug,
            attempt_summary: pool_attempt_summary(1, 1, Some(failure_kind.to_string())),
            requested_service_tier: attempted_requested_service_tier,
            request_body_for_capture: attempted_request_body_for_capture,
        });
    }

    let first_byte_started = Instant::now();
    let (response, first_chunk) = match read_pool_upstream_first_chunk_with_timeout(
        response,
        attempt_pre_first_byte_timeout,
        connect_started,
    )
    .await
    {
        Ok(value) => value,
        Err(err) => {
            if let Some((forward_proxy_scope, selected_proxy)) = forward_proxy_selection.as_ref() {
                record_pool_account_forward_proxy_result(
                    state.as_ref(),
                    forward_proxy_scope,
                    selected_proxy,
                    ForwardProxyRouteResultKind::NetworkFailure,
                )
                .await;
            }
            let message = format!("upstream stream error before first chunk: {err}");
            if let Err(route_err) = record_pool_route_transport_failure(
                &state.pool,
                account.account_id,
                sticky_key,
                &message,
                None,
            )
            .await
            {
                warn!(
                    account_id = account.account_id,
                    error = %route_err,
                    "failed to record pool live-attempt first-chunk failure"
                );
            }
            release_pool_routing_reservation(state.as_ref(), &reservation_key);
            maybe_backfill_oauth_request_debug_from_replay_status(
                &mut oauth_responses_debug,
                original_uri,
                replay_status_rx,
                state.upstream_accounts.crypto_key.as_ref(),
            )
            .await;
            return Err(PoolUpstreamError {
                account: Some(account),
                status: StatusCode::BAD_GATEWAY,
                message,
                failure_kind: PROXY_FAILURE_UPSTREAM_STREAM_ERROR,
                connect_latency_ms,
                upstream_error_code: None,
                upstream_error_message: None,
                upstream_request_id: None,
                oauth_responses_debug,
                attempt_summary: pool_attempt_summary(
                    1,
                    1,
                    Some(PROXY_FAILURE_UPSTREAM_STREAM_ERROR.to_string()),
                ),
                requested_service_tier: attempted_requested_service_tier,
                request_body_for_capture: attempted_request_body_for_capture,
            });
        }
    };

    let first_byte_latency_ms = elapsed_ms(first_byte_started);
    let compact_support_observation =
        classify_compact_support_observation(original_uri, Some(status), None);
    if let Some(observation) = compact_support_observation.as_ref()
        && let Err(observation_err) = record_compact_support_observation(
            &state.pool,
            account.account_id,
            observation.status,
            observation.reason.as_deref(),
        )
        .await
    {
        warn!(
            account_id = account.account_id,
            error = %observation_err,
            "failed to record compact support observation"
        );
    }
    if let Some((forward_proxy_scope, selected_proxy)) = forward_proxy_selection.as_ref() {
        record_pool_account_forward_proxy_result(
            state.as_ref(),
            forward_proxy_scope,
            selected_proxy,
            ForwardProxyRouteResultKind::CompletedRequest,
        )
        .await;
    }
    maybe_backfill_oauth_request_debug_from_replay_status(
        &mut oauth_responses_debug,
        original_uri,
        replay_status_rx,
        state.upstream_accounts.crypto_key.as_ref(),
    )
    .await;
    Ok(PoolUpstreamResponse {
        account,
        response,
        oauth_responses_debug,
        connect_latency_ms,
        attempt_started_at_utc,
        first_byte_latency_ms,
        first_chunk,
        pending_attempt_record: None,
        deferred_early_phase_cleanup_guard: None,
        live_attempt_activity_lease: None,
        attempt_summary: PoolAttemptSummary::default(),
        requested_service_tier: attempted_requested_service_tier,
        request_body_for_capture: attempted_request_body_for_capture,
    })
}

async fn continue_or_retry_pool_live_request(
    state: Arc<AppState>,
    proxy_request_id: u64,
    method: Method,
    original_uri: &Uri,
    headers: &HeaderMap,
    handshake_timeout: Duration,
    initial_account: PoolResolvedAccount,
    sticky_key: Option<String>,
    responses_total_timeout_started_at: Option<Instant>,
    no_available_wait_deadline: Option<Instant>,
    replay_status_rx: &watch::Receiver<PoolReplayBodyStatus>,
    replay_cancel: &CancellationToken,
    replay_wait_timeout: Duration,
    first_error: PoolUpstreamError,
) -> Result<PoolUpstreamResponse, PoolUpstreamError> {
    let reservation_key = build_pool_routing_reservation_key(proxy_request_id);
    let mut replay_status_rx = replay_status_rx.clone();
    let responses_total_timeout =
        pool_upstream_responses_total_timeout(&state.config, original_uri, &method);
    let wait_for_replay_completion_before_total_timeout =
        pool_uses_responses_timeout_failover_policy(original_uri, &method)
            && first_error.failure_kind == PROXY_FAILURE_UPSTREAM_HANDSHAKE_TIMEOUT
            && pool_error_message_indicates_proxy_timeout(&first_error.message);
    let replay_wait_deadline = Instant::now() + replay_wait_timeout;
    let replay_status = loop {
        let current = { replay_status_rx.borrow().clone() };
        if !matches!(current, PoolReplayBodyStatus::Reading) {
            break current;
        }
        let replay_wait_remaining =
            replay_wait_deadline.saturating_duration_since(Instant::now());
        if replay_wait_remaining.is_zero() {
            replay_cancel.cancel();
            release_pool_routing_reservation(state.as_ref(), &reservation_key);
            return Err(PoolUpstreamError {
                account: Some(initial_account),
                status: StatusCode::REQUEST_TIMEOUT,
                message: format!(
                    "request body read timed out after {}ms",
                    replay_wait_timeout.as_millis()
                ),
                failure_kind: PROXY_FAILURE_REQUEST_BODY_READ_TIMEOUT,
                connect_latency_ms: first_error.connect_latency_ms,
                upstream_error_code: None,
                upstream_error_message: None,
                upstream_request_id: None,
                oauth_responses_debug: first_error.oauth_responses_debug,
                attempt_summary: first_error.attempt_summary,
                requested_service_tier: first_error.requested_service_tier,
                request_body_for_capture: first_error.request_body_for_capture,
            });
        }
        let changed = if let (Some(total_timeout), Some(started_at)) =
            (responses_total_timeout, responses_total_timeout_started_at)
        {
            if wait_for_replay_completion_before_total_timeout {
                timeout(replay_wait_remaining, replay_status_rx.changed()).await
            } else {
                let Some(total_timeout_budget) =
                    remaining_timeout_budget(total_timeout, started_at.elapsed())
                else {
                    replay_cancel.cancel();
                    release_pool_routing_reservation(state.as_ref(), &reservation_key);
                    let attempt_count = first_error.attempt_summary.pool_attempt_count.max(1);
                    let distinct_account_count =
                        first_error.attempt_summary.pool_distinct_account_count.max(1);
                    return Err(build_pool_total_timeout_exhausted_error(
                        total_timeout,
                        Some(first_error),
                        attempt_count,
                        distinct_account_count,
                    ));
                };
                timeout(
                    replay_wait_remaining.min(total_timeout_budget),
                    replay_status_rx.changed(),
                )
                .await
            }
        } else {
            timeout(replay_wait_remaining, replay_status_rx.changed()).await
        };
        match changed {
            Ok(Ok(())) => {}
            Ok(Err(_)) => break PoolReplayBodyStatus::Incomplete,
            Err(_) => {
                replay_cancel.cancel();
                release_pool_routing_reservation(state.as_ref(), &reservation_key);
                if !wait_for_replay_completion_before_total_timeout
                    && let (Some(total_timeout), Some(started_at)) =
                        (responses_total_timeout, responses_total_timeout_started_at)
                    && pool_total_timeout_exhausted(total_timeout, started_at)
                {
                    let attempt_count = first_error.attempt_summary.pool_attempt_count.max(1);
                    let distinct_account_count =
                        first_error.attempt_summary.pool_distinct_account_count.max(1);
                    return Err(build_pool_total_timeout_exhausted_error(
                        total_timeout,
                        Some(first_error),
                        attempt_count,
                        distinct_account_count,
                    ));
                }
                return Err(PoolUpstreamError {
                    account: Some(initial_account),
                    status: StatusCode::REQUEST_TIMEOUT,
                    message: format!(
                        "request body read timed out after {}ms",
                        replay_wait_timeout.as_millis()
                    ),
                    failure_kind: PROXY_FAILURE_REQUEST_BODY_READ_TIMEOUT,
                    connect_latency_ms: first_error.connect_latency_ms,
                    upstream_error_code: None,
                    upstream_error_message: None,
                    upstream_request_id: None,
                    oauth_responses_debug: first_error.oauth_responses_debug,
                    attempt_summary: first_error.attempt_summary,
                    requested_service_tier: first_error.requested_service_tier,
                    request_body_for_capture: first_error.request_body_for_capture,
                });
            }
        }
    };
    match replay_status {
        PoolReplayBodyStatus::Complete(snapshot) => {
            if wait_for_replay_completion_before_total_timeout
                && let (Some(total_timeout), Some(started_at)) =
                    (responses_total_timeout, responses_total_timeout_started_at)
                && pool_total_timeout_exhausted(total_timeout, started_at)
            {
                let attempt_count = first_error.attempt_summary.pool_attempt_count.max(1);
                let distinct_account_count =
                    first_error.attempt_summary.pool_distinct_account_count.max(1);
                replay_cancel.cancel();
                release_pool_routing_reservation(state.as_ref(), &reservation_key);
                return Err(build_pool_total_timeout_exhausted_error(
                    total_timeout,
                    Some(first_error),
                    attempt_count,
                    distinct_account_count,
                ));
            }
            let replay_sticky_key = extract_sticky_key_from_replay_snapshot(&snapshot)
                .await
                .or(sticky_key);
            let uses_timeout_route_failover =
                pool_uses_responses_timeout_failover_policy(original_uri, &method);
            let first_error_is_timeout_shaped = uses_timeout_route_failover
                && pool_failure_is_timeout_shaped(first_error.failure_kind, &first_error.message);
            let preserve_sticky_owner_terminal_error =
                pool_upstream_error_preserves_existing_sticky_owner(Some(&first_error));
            let (preferred_account, failover_progress, same_account_attempts) =
                if first_error_is_timeout_shaped {
                    let mut excluded_upstream_route_keys = HashSet::new();
                    excluded_upstream_route_keys.insert(initial_account.upstream_route_key());
                    (
                        None,
                        PoolFailoverProgress {
                            excluded_account_ids: vec![initial_account.account_id],
                            excluded_upstream_route_keys,
                            attempt_count: 1,
                            last_error: Some(first_error),
                            preserve_sticky_owner_terminal_error,
                            overload_required_upstream_route_key: None,
                            timeout_route_failover_pending: true,
                            responses_total_timeout_started_at,
                            no_available_wait_deadline,
                        },
                        POOL_UPSTREAM_SAME_ACCOUNT_MAX_ATTEMPTS,
                    )
                } else if pool_upstream_error_is_rate_limited(&first_error) {
                    (
                        None,
                        PoolFailoverProgress {
                            excluded_account_ids: vec![initial_account.account_id],
                            attempt_count: 1,
                            last_error: Some(first_error),
                            preserve_sticky_owner_terminal_error,
                            responses_total_timeout_started_at,
                            no_available_wait_deadline,
                            ..PoolFailoverProgress::default()
                        },
                        POOL_UPSTREAM_SAME_ACCOUNT_MAX_ATTEMPTS,
                    )
                } else {
                    (
                        Some(initial_account.clone()),
                        PoolFailoverProgress {
                            attempt_count: 1,
                            responses_total_timeout_started_at,
                            no_available_wait_deadline,
                            ..PoolFailoverProgress::default()
                        },
                        POOL_UPSTREAM_SAME_ACCOUNT_MAX_ATTEMPTS.saturating_sub(1),
                    )
                };
            send_pool_request_with_failover(
                state,
                proxy_request_id,
                method,
                original_uri,
                headers,
                Some(snapshot),
                handshake_timeout,
                None,
                None,
                replay_sticky_key.as_deref(),
                preferred_account,
                failover_progress,
                same_account_attempts,
            )
            .await
        }
        PoolReplayBodyStatus::ReadError(err) => {
            release_pool_routing_reservation(state.as_ref(), &reservation_key);
            Err(PoolUpstreamError {
                account: Some(initial_account),
                status: err.status,
                message: err.message,
                failure_kind: err.failure_kind,
                connect_latency_ms: first_error.connect_latency_ms,
                upstream_error_code: None,
                upstream_error_message: None,
                upstream_request_id: None,
                oauth_responses_debug: None,
                attempt_summary: first_error.attempt_summary.clone(),
                requested_service_tier: first_error.requested_service_tier.clone(),
                request_body_for_capture: first_error.request_body_for_capture.clone(),
            })
        }
        PoolReplayBodyStatus::InternalError(message) => {
            release_pool_routing_reservation(state.as_ref(), &reservation_key);
            Err(PoolUpstreamError {
                account: Some(initial_account),
                status: StatusCode::INTERNAL_SERVER_ERROR,
                message,
                failure_kind: PROXY_FAILURE_FAILED_CONTACT_UPSTREAM,
                connect_latency_ms: first_error.connect_latency_ms,
                upstream_error_code: None,
                upstream_error_message: None,
                upstream_request_id: None,
                oauth_responses_debug: None,
                attempt_summary: first_error.attempt_summary.clone(),
                requested_service_tier: first_error.requested_service_tier.clone(),
                request_body_for_capture: first_error.request_body_for_capture.clone(),
            })
        }
        PoolReplayBodyStatus::Reading | PoolReplayBodyStatus::Incomplete => {
            replay_cancel.cancel();
            release_pool_routing_reservation(state.as_ref(), &reservation_key);
            Err(first_error)
        }
    }
}

async fn maybe_backfill_oauth_request_debug_from_replay_status(
    debug: &mut Option<oauth_bridge::OauthResponsesDebugInfo>,
    original_uri: &Uri,
    replay_status_rx: &watch::Receiver<PoolReplayBodyStatus>,
    crypto_key: Option<&[u8; 32]>,
) {
    let Some(debug) = debug.as_mut() else {
        return;
    };
    if debug.request_body_prefix_fingerprint.is_some() || crypto_key.is_none() {
        return;
    }

    let replay_status = { replay_status_rx.borrow().clone() };
    let PoolReplayBodyStatus::Complete(snapshot) = replay_status else {
        return;
    };
    let Ok(prefix) = snapshot
        .to_prefix_bytes(oauth_bridge::OAUTH_REQUEST_BODY_PREFIX_FINGERPRINT_MAX_BYTES)
        .await
    else {
        return;
    };
    oauth_bridge::backfill_oauth_request_debug_body_prefix(
        debug,
        original_uri.path(),
        prefix.as_ref(),
        crypto_key,
    );
}

async fn proxy_openai_v1_via_pool(
    state: Arc<AppState>,
    proxy_request_id: u64,
    original_uri: &Uri,
    method: Method,
    headers: HeaderMap,
    body: Body,
    runtime_timeouts: PoolRoutingTimeoutSettingsResolved,
    mut proxy_request_permit: Option<ProxyRequestConcurrencyPermit>,
) -> Result<Response, (StatusCode, String)> {
    let request_started_at = Instant::now();
    let body_limit = state.config.openai_proxy_max_request_body_bytes;
    let pool_routing_reservation_key = build_pool_routing_reservation_key(proxy_request_id);
    let capture_target = capture_target_for_request(original_uri.path(), &method);
    let handshake_timeout =
        proxy_upstream_send_timeout_for_capture_target(&runtime_timeouts, capture_target);
    let _pre_first_byte_timeout =
        pool_upstream_first_chunk_timeout(&runtime_timeouts, original_uri, &method);
    let responses_total_timeout =
        pool_upstream_responses_total_timeout(&state.config, original_uri, &method);
    let pre_attempt_total_timeout_deadline =
        responses_total_timeout.map(|total_timeout| request_started_at + total_timeout);
    let responses_total_timeout_started_at_from_request =
        responses_total_timeout.map(|_| request_started_at);
    let responses_total_timeout_started_at = None;
    let pre_attempt_total_timeout_exceeded =
        || pre_attempt_total_timeout_deadline.is_some_and(|deadline| Instant::now() >= deadline);
    let pre_attempt_total_timeout_error = || {
        pool_pre_attempt_total_timeout_error(
            responses_total_timeout.expect("pre-attempt total-timeout requires responses timeout"),
        )
    };
    let header_sticky_key = extract_sticky_key_from_headers(&headers);
    let proxy_request_permit = take_or_acquire_proxy_request_concurrency_permit(
        &mut proxy_request_permit,
        state.as_ref(),
        proxy_request_id,
        &method,
        original_uri,
    )
    .await?;
    let body_size_hint_exact = body
        .size_hint()
        .exact()
        .and_then(|value| usize::try_from(value).ok());
    let (upstream, sticky_key) = if request_may_have_body(&method, &headers) {
        let should_prebuffer_for_body_sticky = should_prebuffer_for_body_sticky_probe(
            header_sticky_key.is_some(),
            headers
                .get(header::CONTENT_TYPE)
                .and_then(|value| value.to_str().ok()),
            headers
                .get(header::CONTENT_LENGTH)
                .and_then(|value| value.to_str().ok())
                .and_then(|value| value.parse::<usize>().ok())
                .or(body_size_hint_exact),
        );

        if should_prebuffer_for_body_sticky {
            let request_body_bytes = read_request_body_with_limit(
                body,
                body_limit,
                runtime_timeouts.request_read_timeout,
                proxy_request_id,
            )
            .await
            .map_err(|err| (err.status, err.message))?;
            if pre_attempt_total_timeout_exceeded() {
                return Err(pre_attempt_total_timeout_error());
            }
            let request_body_bytes = Bytes::from(request_body_bytes);
            let body_sticky_key = serde_json::from_slice::<Value>(&request_body_bytes)
                .ok()
                .and_then(|value| extract_sticky_key_from_request_body(&value));
            (
                send_pool_request_with_failover(
                    state.clone(),
                    proxy_request_id,
                    method,
                    original_uri,
                    &headers,
                    Some(PoolReplayBodySnapshot::Memory(request_body_bytes)),
                    handshake_timeout,
                    None,
                    None,
                    body_sticky_key.as_deref(),
                    None,
                    PoolFailoverProgress {
                        responses_total_timeout_started_at:
                            responses_total_timeout_started_at_from_request,
                        ..PoolFailoverProgress::default()
                    },
                    POOL_UPSTREAM_SAME_ACCOUNT_MAX_ATTEMPTS,
                )
                .await
                .map_err(|err| (err.status, err.message))?,
                body_sticky_key,
            )
        } else {
            let unwrap_initial_pool_account = |resolution,
                                               no_available_wait_deadline|
             -> Result<
                (PoolResolvedAccount, Option<Instant>),
                (StatusCode, String),
            > {
                let initial_account = match resolution {
                    Ok(PoolAccountResolutionWithWait::Resolution(
                        PoolAccountResolution::Resolved(account),
                    )) => account,
                    Ok(PoolAccountResolutionWithWait::TotalTimeoutExpired) => {
                        let total_timeout = responses_total_timeout
                            .expect("pre-attempt total-timeout expiry requires responses timeout");
                        return Err((
                            StatusCode::GATEWAY_TIMEOUT,
                            pool_total_timeout_exhausted_message(total_timeout),
                        ));
                    }
                    Ok(PoolAccountResolutionWithWait::Resolution(
                        PoolAccountResolution::Unavailable,
                    ))
                    | Ok(PoolAccountResolutionWithWait::Resolution(
                        PoolAccountResolution::NoCandidate,
                    )) => {
                        return Err((
                            StatusCode::SERVICE_UNAVAILABLE,
                            POOL_NO_AVAILABLE_ACCOUNT_MESSAGE.to_string(),
                        ));
                    }
                    Ok(PoolAccountResolutionWithWait::Resolution(
                        PoolAccountResolution::RateLimited,
                    )) => {
                        return Err((
                            StatusCode::TOO_MANY_REQUESTS,
                            POOL_ALL_ACCOUNTS_RATE_LIMITED_MESSAGE.to_string(),
                        ));
                    }
                    Ok(PoolAccountResolutionWithWait::Resolution(
                        PoolAccountResolution::DegradedOnly,
                    )) => {
                        return Err((
                            StatusCode::SERVICE_UNAVAILABLE,
                            POOL_ALL_ACCOUNTS_DEGRADED_MESSAGE.to_string(),
                        ));
                    }
                    Ok(PoolAccountResolutionWithWait::Resolution(
                        PoolAccountResolution::BlockedByPolicy(message),
                    )) => {
                        return Err((StatusCode::SERVICE_UNAVAILABLE, message));
                    }
                    Err(err) => {
                        return Err((
                            StatusCode::BAD_GATEWAY,
                            format!("failed to resolve pool account: {err}"),
                        ));
                    }
                };
                Ok((initial_account, no_available_wait_deadline))
            };
            let (
                request_body_snapshot,
                body_sticky_key,
                initial_account,
                no_available_wait_deadline,
            ) = if let Some(sticky_key) = header_sticky_key.clone() {
                let state_for_wait = state.clone();
                let wait_task_sticky_key = sticky_key.clone();
                let shared_wait_deadline = Arc::new(std::sync::Mutex::new(None));
                let shared_wait_deadline_for_task = shared_wait_deadline.clone();
                let header_sticky_resolution = async move {
                    let excluded_ids = Vec::new();
                    let excluded_upstream_route_keys = HashSet::new();
                    let mut no_available_wait_deadline = None;
                    let poll_interval = state_for_wait
                        .pool_no_available_wait
                        .normalized_poll_interval();
                    loop {
                        if pre_attempt_total_timeout_deadline
                            .is_some_and(|deadline| Instant::now() >= deadline)
                        {
                            break (
                                Ok(PoolAccountResolutionWithWait::TotalTimeoutExpired),
                                no_available_wait_deadline,
                            );
                        }
                        let resolution = resolve_pool_account_for_request(
                            state_for_wait.as_ref(),
                            Some(wait_task_sticky_key.as_str()),
                            &excluded_ids,
                            &excluded_upstream_route_keys,
                        )
                        .await;
                        if pre_attempt_total_timeout_deadline
                            .is_some_and(|deadline| Instant::now() >= deadline)
                        {
                            break (
                                Ok(PoolAccountResolutionWithWait::TotalTimeoutExpired),
                                no_available_wait_deadline,
                            );
                        }
                        match resolution {
                            Ok(
                                resolution @ (PoolAccountResolution::Unavailable
                                | PoolAccountResolution::NoCandidate),
                            ) => {
                                let wait_deadline =
                                    if let Some(deadline) = no_available_wait_deadline {
                                        deadline
                                    } else {
                                        let deadline = Instant::now()
                                            + state_for_wait.pool_no_available_wait.timeout;
                                        no_available_wait_deadline = Some(deadline);
                                        *shared_wait_deadline_for_task
                                            .lock()
                                            .expect("lock shared header wait deadline") =
                                            Some(deadline);
                                        deadline
                                    };
                                let effective_deadline = pre_attempt_total_timeout_deadline
                                    .map(|deadline| std::cmp::min(wait_deadline, deadline))
                                    .unwrap_or(wait_deadline);
                                let now = Instant::now();
                                if now >= effective_deadline {
                                    if pre_attempt_total_timeout_deadline
                                        .is_some_and(|deadline| deadline <= wait_deadline)
                                    {
                                        break (
                                            Ok(PoolAccountResolutionWithWait::TotalTimeoutExpired),
                                            no_available_wait_deadline,
                                        );
                                    }
                                    break (
                                        Ok(PoolAccountResolutionWithWait::Resolution(resolution)),
                                        no_available_wait_deadline,
                                    );
                                }
                                tokio::time::sleep(
                                    poll_interval
                                        .min(effective_deadline.saturating_duration_since(now)),
                                )
                                .await;
                            }
                            Ok(resolution) => {
                                break (
                                    Ok(PoolAccountResolutionWithWait::Resolution(resolution)),
                                    no_available_wait_deadline,
                                );
                            }
                            Err(err) => break (Err(err), no_available_wait_deadline),
                        }
                    }
                };
                tokio::pin!(header_sticky_resolution);
                let mut header_sticky_resolution_finished = false;
                let mut pending_header_sticky_terminal_error: Option<(StatusCode, String)> = None;
                let mut resolved_header_sticky_account: Option<PoolResolvedAccount> = None;
                let mut header_sticky_wait_deadline = None;
                let mut request_body_buffer = PoolReplayBodyBuffer::new(proxy_request_id);
                let mut request_body_stream = body.into_data_stream();
                let request_body_deadline = Instant::now() + runtime_timeouts.request_read_timeout;
                let mut request_body_len = 0usize;
                let mut observed_body_sticky_key = None;
                let mut sticky_key_probe = Vec::new();
                let mut sticky_key_probe_exhausted = false;
                let request_body_timeout_error = || RequestBodyReadError {
                    status: StatusCode::REQUEST_TIMEOUT,
                    message: format!(
                        "request body read timed out after {}ms",
                        runtime_timeouts.request_read_timeout.as_millis()
                    ),
                    failure_kind: PROXY_FAILURE_REQUEST_BODY_READ_TIMEOUT,
                    partial_body: Vec::new(),
                };
                let request_body_snapshot = loop {
                    let remaining = request_body_deadline.saturating_duration_since(Instant::now());
                    if remaining.is_zero() {
                        return Err((
                            request_body_timeout_error().status,
                            request_body_timeout_error().message,
                        ));
                    }
                    tokio::select! {
                        resolution_result = &mut header_sticky_resolution, if !header_sticky_resolution_finished => {
                            header_sticky_resolution_finished = true;
                            let (resolution, no_available_wait_deadline) = resolution_result;
                            header_sticky_wait_deadline = no_available_wait_deadline;
                            match resolution {
                                Ok(PoolAccountResolutionWithWait::Resolution(
                                    PoolAccountResolution::Resolved(account),
                                )) => {
                                    resolved_header_sticky_account = Some(account);
                                }
                                Ok(PoolAccountResolutionWithWait::TotalTimeoutExpired) => {
                                    let total_timeout =
                                        responses_total_timeout.expect(
                                            "pre-attempt total-timeout expiry requires responses timeout",
                                        );
                                    return Err((
                                        StatusCode::GATEWAY_TIMEOUT,
                                        pool_total_timeout_exhausted_message(total_timeout),
                                    ));
                                }
                                Ok(PoolAccountResolutionWithWait::Resolution(
                                    PoolAccountResolution::RateLimited,
                                )) => {
                                    pending_header_sticky_terminal_error = Some((
                                        StatusCode::TOO_MANY_REQUESTS,
                                        POOL_ALL_ACCOUNTS_RATE_LIMITED_MESSAGE.to_string(),
                                    ));
                                }
                                Ok(PoolAccountResolutionWithWait::Resolution(
                                    PoolAccountResolution::DegradedOnly,
                                )) => {
                                    pending_header_sticky_terminal_error = Some((
                                        StatusCode::SERVICE_UNAVAILABLE,
                                        POOL_ALL_ACCOUNTS_DEGRADED_MESSAGE.to_string(),
                                    ));
                                }
                                Ok(PoolAccountResolutionWithWait::Resolution(
                                    PoolAccountResolution::BlockedByPolicy(message),
                                )) => {
                                    pending_header_sticky_terminal_error =
                                        Some((StatusCode::SERVICE_UNAVAILABLE, message));
                                }
                                Err(err) => {
                                    pending_header_sticky_terminal_error = Some((
                                        StatusCode::BAD_GATEWAY,
                                        format!("failed to resolve pool account: {err}"),
                                    ));
                                }
                                Ok(PoolAccountResolutionWithWait::Resolution(
                                    PoolAccountResolution::Unavailable,
                                ))
                                | Ok(PoolAccountResolutionWithWait::Resolution(
                                    PoolAccountResolution::NoCandidate,
                                )) => {}
                            }
                            if observed_body_sticky_key.as_deref() == Some(sticky_key.as_str())
                                && let Some((status, message)) =
                                    pending_header_sticky_terminal_error.as_ref()
                            {
                                return Err((*status, message.clone()));
                            }
                        }
                        next_chunk = timeout(remaining, request_body_stream.next()) => {
                            let next_chunk = match next_chunk {
                                Ok(chunk) => chunk,
                                Err(_) => {
                                    let err = request_body_timeout_error();
                                    return Err((err.status, err.message));
                                }
                            };
                            let Some(chunk) = next_chunk else {
                                break request_body_buffer.finish().await.map_err(|err| {
                                    (
                                        StatusCode::BAD_GATEWAY,
                                        format!("failed to cache request body for oauth replay: {err}"),
                                    )
                                })?;
                            };
                            let chunk = match chunk {
                                Ok(chunk) => chunk,
                                Err(err) => {
                                    return Err((
                                        StatusCode::BAD_REQUEST,
                                        format!("failed to read request body stream: {err}"),
                                    ));
                                }
                            };
                            if request_body_len.saturating_add(chunk.len()) > body_limit {
                                return Err((
                                    StatusCode::PAYLOAD_TOO_LARGE,
                                    format!("request body exceeds {body_limit} bytes"),
                                ));
                            }
                            request_body_len = request_body_len.saturating_add(chunk.len());
                            request_body_buffer.append(&chunk).await.map_err(|err| {
                                (
                                    StatusCode::BAD_GATEWAY,
                                    format!("failed to cache request body for oauth replay: {err}"),
                                )
                            })?;
                            if !sticky_key_probe_exhausted
                                && observed_body_sticky_key.is_none()
                                && sticky_key_probe.len() < HEADER_STICKY_EARLY_STICKY_SCAN_BYTES
                            {
                                let probe_remaining = HEADER_STICKY_EARLY_STICKY_SCAN_BYTES
                                    .saturating_sub(sticky_key_probe.len());
                                sticky_key_probe.extend_from_slice(&chunk[..chunk.len().min(probe_remaining)]);
                                observed_body_sticky_key =
                                    best_effort_extract_sticky_key_from_request_body_prefix(
                                        &sticky_key_probe,
                                    );
                                sticky_key_probe_exhausted =
                                    observed_body_sticky_key.is_some()
                                        || sticky_key_probe.len()
                                            >= HEADER_STICKY_EARLY_STICKY_SCAN_BYTES;
                            }
                            if header_sticky_resolution_finished
                                && observed_body_sticky_key.as_deref() == Some(sticky_key.as_str())
                                && let Some((status, message)) =
                                    pending_header_sticky_terminal_error.as_ref()
                            {
                                return Err((*status, message.clone()));
                            }
                        }
                    }
                };
                let body_sticky_key =
                    if let Some(observed_body_sticky_key) = observed_body_sticky_key {
                        Some(observed_body_sticky_key)
                    } else {
                        extract_sticky_key_from_replay_snapshot(&request_body_snapshot)
                            .await
                            .or(Some(sticky_key.clone()))
                    };
                if !header_sticky_resolution_finished {
                    if body_sticky_key.as_deref() == Some(sticky_key.as_str()) {
                        let (resolution, no_available_wait_deadline) =
                            header_sticky_resolution.await;
                        header_sticky_wait_deadline = no_available_wait_deadline;
                        match resolution {
                            Ok(PoolAccountResolutionWithWait::Resolution(
                                PoolAccountResolution::Resolved(account),
                            )) => {
                                resolved_header_sticky_account = Some(account);
                            }
                            Ok(PoolAccountResolutionWithWait::TotalTimeoutExpired) => {
                                let total_timeout = responses_total_timeout.expect(
                                    "pre-attempt total-timeout expiry requires responses timeout",
                                );
                                return Err((
                                    StatusCode::GATEWAY_TIMEOUT,
                                    pool_total_timeout_exhausted_message(total_timeout),
                                ));
                            }
                            Ok(PoolAccountResolutionWithWait::Resolution(
                                PoolAccountResolution::RateLimited,
                            )) => {
                                pending_header_sticky_terminal_error = Some((
                                    StatusCode::TOO_MANY_REQUESTS,
                                    POOL_ALL_ACCOUNTS_RATE_LIMITED_MESSAGE.to_string(),
                                ));
                            }
                            Ok(PoolAccountResolutionWithWait::Resolution(
                                PoolAccountResolution::DegradedOnly,
                            )) => {
                                pending_header_sticky_terminal_error = Some((
                                    StatusCode::SERVICE_UNAVAILABLE,
                                    POOL_ALL_ACCOUNTS_DEGRADED_MESSAGE.to_string(),
                                ));
                            }
                            Ok(PoolAccountResolutionWithWait::Resolution(
                                PoolAccountResolution::BlockedByPolicy(message),
                            )) => {
                                pending_header_sticky_terminal_error =
                                    Some((StatusCode::SERVICE_UNAVAILABLE, message));
                            }
                            Err(err) => {
                                pending_header_sticky_terminal_error = Some((
                                    StatusCode::BAD_GATEWAY,
                                    format!("failed to resolve pool account: {err}"),
                                ));
                            }
                            Ok(PoolAccountResolutionWithWait::Resolution(
                                PoolAccountResolution::Unavailable,
                            ))
                            | Ok(PoolAccountResolutionWithWait::Resolution(
                                PoolAccountResolution::NoCandidate,
                            )) => {}
                        }
                    }
                }
                if body_sticky_key.as_deref() == Some(sticky_key.as_str())
                    && let Some((status, message)) = pending_header_sticky_terminal_error
                {
                    return Err((status, message));
                }
                let mut no_available_wait_deadline =
                    header_sticky_wait_deadline.or(*shared_wait_deadline
                        .lock()
                        .expect("lock shared header wait deadline"));
                let initial_account = if body_sticky_key.as_deref() == Some(sticky_key.as_str()) {
                    if let Some(account) = resolved_header_sticky_account {
                        account
                    } else {
                        let resolution = resolve_pool_account_for_request_with_wait(
                            state.as_ref(),
                            body_sticky_key.as_deref(),
                            &[],
                            &HashSet::new(),
                            None,
                            true,
                            &mut no_available_wait_deadline,
                            pre_attempt_total_timeout_deadline,
                        )
                        .await;
                        let (initial_account, updated_no_available_wait_deadline) =
                            unwrap_initial_pool_account(resolution, no_available_wait_deadline)?;
                        no_available_wait_deadline = updated_no_available_wait_deadline;
                        initial_account
                    }
                } else {
                    let resolution = resolve_pool_account_for_request_with_wait(
                        state.as_ref(),
                        body_sticky_key.as_deref(),
                        &[],
                        &HashSet::new(),
                        None,
                        true,
                        &mut no_available_wait_deadline,
                        pre_attempt_total_timeout_deadline,
                    )
                    .await;
                    let (initial_account, updated_no_available_wait_deadline) =
                        unwrap_initial_pool_account(resolution, no_available_wait_deadline)?;
                    no_available_wait_deadline = updated_no_available_wait_deadline;
                    initial_account
                };
                if pre_attempt_total_timeout_exceeded() {
                    return Err(pre_attempt_total_timeout_error());
                }
                (
                    request_body_snapshot,
                    body_sticky_key,
                    initial_account,
                    no_available_wait_deadline,
                )
            } else {
                let mut no_available_wait_deadline = None;
                let is_json_body = headers
                    .get(header::CONTENT_TYPE)
                    .and_then(|value| value.to_str().ok())
                    .is_some_and(|value| value.to_ascii_lowercase().starts_with("application/json"));
                let replayable_body = spawn_pool_replayable_request_body(
                    body,
                    body_limit,
                    runtime_timeouts.request_read_timeout,
                    proxy_request_id,
                );
                let replay_status_rx = replayable_body.status_rx.clone();
                let replay_cancel = replayable_body.cancel.clone();
                let live_body_sticky_key = if is_json_body {
                    let sticky_key_probe_wait_timeout = live_body_sticky_key_probe_wait_timeout(
                        runtime_timeouts.request_read_timeout,
                        pre_attempt_total_timeout_deadline,
                    );
                    wait_for_replay_body_sticky_key_probe(
                        &replayable_body.sticky_key_probe_rx,
                        sticky_key_probe_wait_timeout,
                    )
                    .await
                } else {
                    None
                };

                let resolution = resolve_pool_account_for_request_with_wait(
                    state.as_ref(),
                    live_body_sticky_key.as_deref(),
                    &[],
                    &HashSet::new(),
                    None,
                    true,
                    &mut no_available_wait_deadline,
                    pre_attempt_total_timeout_deadline,
                )
                .await;
                let (initial_account, no_available_wait_deadline) =
                    unwrap_initial_pool_account(resolution, no_available_wait_deadline)?;
                let live_responses_total_timeout_started_at =
                    responses_total_timeout_started_at_from_request;

                if pool_account_supports_live_request_body(&initial_account, original_uri) {
                    let upstream = match send_pool_request_live_first_attempt(
                        state.clone(),
                        proxy_request_id,
                        method.clone(),
                        original_uri,
                        &headers,
                        replayable_body.body,
                        &runtime_timeouts,
                        handshake_timeout,
                        responses_total_timeout,
                        live_responses_total_timeout_started_at,
                        live_body_sticky_key.as_deref(),
                        initial_account.clone(),
                        &replay_status_rx,
                    )
                    .await
                    {
                        Ok(upstream) => upstream,
                        Err(first_error) => continue_or_retry_pool_live_request(
                            state.clone(),
                            proxy_request_id,
                            method.clone(),
                            original_uri,
                            &headers,
                            handshake_timeout,
                            initial_account,
                            live_body_sticky_key.clone(),
                            live_responses_total_timeout_started_at,
                            no_available_wait_deadline,
                            &replay_status_rx,
                            &replay_cancel,
                            runtime_timeouts.request_read_timeout,
                            first_error,
                        )
                        .await
                        .map_err(|err| (err.status, err.message))?,
                    };
                    let account = upstream.account;
                    let upstream_attempt_started_at_utc = upstream.attempt_started_at_utc;
                    let upstream_invoke_id = upstream
                        .pending_attempt_record
                        .as_ref()
                        .map(|record| record.invoke_id.clone());
                    let t_upstream_connect_ms = upstream.connect_latency_ms;
                    let t_upstream_ttfb_ms = upstream.first_byte_latency_ms;
                    let upstream_response = upstream.response;
                    let rewritten_location = normalize_proxy_location_header(
                        upstream_response.status(),
                        upstream_response.headers(),
                        &account.upstream_base_url,
                    )
                    .map_err(|err| {
                        (
                            StatusCode::BAD_GATEWAY,
                            format!("failed to process upstream redirect: {err}"),
                        )
                    })?;

                    let upstream_status = upstream_response.status();
                    let upstream_connection_scoped =
                        connection_scoped_header_names(upstream_response.headers());
                    let mut response_builder = Response::builder().status(upstream_status);
                    for (name, value) in upstream_response.headers() {
                        if should_forward_proxy_header(name, &upstream_connection_scoped) {
                            if name == header::LOCATION {
                                if let Some(rewritten) = rewritten_location.as_deref() {
                                    response_builder = response_builder.header(name, rewritten);
                                }
                            } else {
                                response_builder = response_builder.header(name, value);
                            }
                        }
                    }

                    let mut upstream_stream = upstream_response.into_bytes_stream();
                    let first_chunk = upstream.first_chunk;
                    if let Some(chunk) = first_chunk.as_ref() {
                        info!(
                            proxy_request_id,
                            account_id = account.account_id,
                            ttfb_ms = t_upstream_ttfb_ms,
                            first_chunk_bytes = chunk.len(),
                            "pool upstream response first chunk ready"
                        );
                    } else {
                        consume_pool_routing_reservation(
                            state.as_ref(),
                            &pool_routing_reservation_key,
                        );
                        if let Err(route_err) = record_pool_route_success(
                            &state.pool,
                            account.account_id,
                            upstream_attempt_started_at_utc,
                            live_body_sticky_key.as_deref(),
                            upstream_invoke_id.as_deref(),
                        )
                        .await
                        {
                            warn!(account_id = account.account_id, error = %route_err, "failed to record pool route success");
                        }
                        return response_builder.body(Body::empty()).map_err(|err| {
                            (
                                StatusCode::INTERNAL_SERVER_ERROR,
                                format!("failed to build proxy response: {err}"),
                            )
                        });
                    }
                    let (tx, rx) = mpsc::channel::<Result<Bytes, io::Error>>(16);
                    let state_for_record = state.clone();
                    let reservation_key_for_record = pool_routing_reservation_key.clone();
                    let sticky_key_for_record = live_body_sticky_key.clone();
                    let invoke_id_for_record = upstream_invoke_id.clone();
                    let upstream_attempt_started_at_utc_for_record = upstream_attempt_started_at_utc;
                    let proxy_request_permit_for_task = proxy_request_permit;
                    tokio::spawn(async move {
                        let mut forwarded_chunks = 0usize;
                        let mut forwarded_bytes = 0usize;
                        let stream_started_at = Instant::now();
                        let mut stream_error_message: Option<String> = None;
                        let mut downstream_closed = false;
                        let mut proxy_request_permit_for_task = Some(proxy_request_permit_for_task);

                        if let Some(chunk) = first_chunk {
                            forwarded_chunks = forwarded_chunks.saturating_add(1);
                            forwarded_bytes = forwarded_bytes.saturating_add(chunk.len());
                            if tx.send(Ok(chunk)).await.is_err() {
                                downstream_closed = true;
                                let _ = proxy_request_permit_for_task.take();
                            }
                        }

                        loop {
                            if downstream_closed {
                                break;
                            }
                            let Some(next_chunk) = upstream_stream.next().await else {
                                break;
                            };
                            match next_chunk {
                                Ok(chunk) => {
                                    forwarded_chunks = forwarded_chunks.saturating_add(1);
                                    forwarded_bytes = forwarded_bytes.saturating_add(chunk.len());
                                    if tx.send(Ok(chunk)).await.is_err() {
                                        let _ = proxy_request_permit_for_task.take();
                                        break;
                                    }
                                }
                                Err(err) => {
                                    let message = format!("upstream stream error: {err}");
                                    stream_error_message = Some(message.clone());
                                    let _ = tx.send(Err(io::Error::other(message))).await;
                                    break;
                                }
                            }
                        }
                        drop(proxy_request_permit_for_task.take());

                        if let Some(message) = stream_error_message.as_deref() {
                            release_pool_routing_reservation(
                                state_for_record.as_ref(),
                                &reservation_key_for_record,
                            );
                            if let Err(route_err) = record_pool_route_transport_failure(
                                &state_for_record.pool,
                                account.account_id,
                                sticky_key_for_record.as_deref(),
                                message,
                                invoke_id_for_record.as_deref(),
                            )
                            .await
                            {
                                warn!(account_id = account.account_id, error = %route_err, "failed to record pool stream error");
                            }
                        } else {
                            consume_pool_routing_reservation(
                                state_for_record.as_ref(),
                                &reservation_key_for_record,
                            );
                            if let Err(route_err) = record_pool_route_success(
                                &state_for_record.pool,
                                account.account_id,
                                upstream_attempt_started_at_utc_for_record,
                                sticky_key_for_record.as_deref(),
                                invoke_id_for_record.as_deref(),
                            )
                            .await
                            {
                                warn!(account_id = account.account_id, error = %route_err, "failed to record pool route success");
                            }
                        }

                        info!(
                            proxy_request_id,
                            account_id = account.account_id,
                            t_upstream_connect_ms,
                            forwarded_chunks,
                            forwarded_bytes,
                            elapsed_ms = stream_started_at.elapsed().as_millis(),
                            "pool upstream response stream completed"
                        );
                    });

                    return response_builder
                        .body(Body::from_stream(ReceiverStream::new(rx)))
                        .map_err(|err| {
                            (
                                StatusCode::INTERNAL_SERVER_ERROR,
                                format!("failed to build proxy response: {err}"),
                            )
                        });
                }

                let request_body_snapshot = wait_for_replay_body_snapshot(
                    state.as_ref(),
                    original_uri,
                    &method,
                    &replay_status_rx,
                    &replay_cancel,
                    runtime_timeouts.request_read_timeout,
                    live_responses_total_timeout_started_at,
                )
                .await?;
                let body_sticky_key = extract_sticky_key_from_replay_snapshot(&request_body_snapshot)
                    .await
                    .or(live_body_sticky_key);
                let mut no_available_wait_deadline = None;
                let resolution = resolve_pool_account_for_request_with_wait(
                    state.as_ref(),
                    body_sticky_key.as_deref(),
                    &[],
                    &HashSet::new(),
                    None,
                    true,
                    &mut no_available_wait_deadline,
                    pre_attempt_total_timeout_deadline,
                )
                .await;
                let (initial_account, no_available_wait_deadline) =
                    unwrap_initial_pool_account(resolution, no_available_wait_deadline)?;
                (
                    request_body_snapshot,
                    body_sticky_key,
                    initial_account,
                    no_available_wait_deadline,
                )
            };
            (
                send_pool_request_with_failover(
                    state.clone(),
                    proxy_request_id,
                    method,
                    original_uri,
                    &headers,
                    Some(request_body_snapshot),
                    handshake_timeout,
                    None,
                    None,
                    body_sticky_key.as_deref(),
                    Some(initial_account),
                    PoolFailoverProgress {
                        responses_total_timeout_started_at:
                            responses_total_timeout_started_at_from_request,
                        no_available_wait_deadline,
                        ..PoolFailoverProgress::default()
                    },
                    POOL_UPSTREAM_SAME_ACCOUNT_MAX_ATTEMPTS,
                )
                .await
                .map_err(|err| (err.status, err.message))?,
                body_sticky_key,
            )
        }
    } else {
        let same_account_attempts = if method == Method::GET {
            1
        } else {
            POOL_UPSTREAM_SAME_ACCOUNT_MAX_ATTEMPTS
        };
        (
            send_pool_request_with_failover(
                state.clone(),
                proxy_request_id,
                method,
                original_uri,
                &headers,
                None,
                handshake_timeout,
                None,
                None,
                header_sticky_key.as_deref(),
                None,
                PoolFailoverProgress {
                    responses_total_timeout_started_at,
                    ..PoolFailoverProgress::default()
                },
                same_account_attempts,
            )
            .await
            .map_err(|err| (err.status, err.message))?,
            header_sticky_key,
        )
    };

    let account = upstream.account;
    let upstream_attempt_started_at_utc = upstream.attempt_started_at_utc;
    let upstream_invoke_id = upstream
        .pending_attempt_record
        .as_ref()
        .map(|record| record.invoke_id.clone());
    let t_upstream_connect_ms = upstream.connect_latency_ms;
    let t_upstream_ttfb_ms = upstream.first_byte_latency_ms;
    let upstream_response = upstream.response;
    let rewritten_location = normalize_proxy_location_header(
        upstream_response.status(),
        upstream_response.headers(),
        &account.upstream_base_url,
    )
    .map_err(|err| {
        (
            StatusCode::BAD_GATEWAY,
            format!("failed to process upstream redirect: {err}"),
        )
    })?;

    let upstream_status = upstream_response.status();
    let upstream_connection_scoped = connection_scoped_header_names(upstream_response.headers());
    let mut response_builder = Response::builder().status(upstream_status);
    for (name, value) in upstream_response.headers() {
        if should_forward_proxy_header(name, &upstream_connection_scoped) {
            if name == header::LOCATION {
                if let Some(rewritten) = rewritten_location.as_deref() {
                    response_builder = response_builder.header(name, rewritten);
                }
            } else {
                response_builder = response_builder.header(name, value);
            }
        }
    }

    let mut upstream_stream = upstream_response.into_bytes_stream();
    let first_chunk = upstream.first_chunk;
    if let Some(chunk) = first_chunk.as_ref() {
        info!(
            proxy_request_id,
            account_id = account.account_id,
            ttfb_ms = t_upstream_ttfb_ms,
            first_chunk_bytes = chunk.len(),
            "pool upstream response first chunk ready"
        );
    } else {
        consume_pool_routing_reservation(state.as_ref(), &pool_routing_reservation_key);
        if let Err(route_err) = record_pool_route_success(
            &state.pool,
            account.account_id,
            upstream_attempt_started_at_utc,
            sticky_key.as_deref(),
            upstream_invoke_id.as_deref(),
        )
        .await
        {
            warn!(account_id = account.account_id, error = %route_err, "failed to record pool route success");
        }
        return response_builder.body(Body::empty()).map_err(|err| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("failed to build proxy response: {err}"),
            )
        });
    }

    let (tx, rx) = mpsc::channel::<Result<Bytes, io::Error>>(16);
    let state_for_record = state.clone();
    let reservation_key_for_record = pool_routing_reservation_key.clone();
    let sticky_key_for_record = sticky_key.clone();
    let invoke_id_for_record = upstream_invoke_id.clone();
    let upstream_attempt_started_at_utc_for_record = upstream_attempt_started_at_utc;
    let proxy_request_permit_for_task = proxy_request_permit;
    tokio::spawn(async move {
        let mut forwarded_chunks = 0usize;
        let mut forwarded_bytes = 0usize;
        let stream_started_at = Instant::now();
        let mut stream_error_message: Option<String> = None;
        let mut downstream_closed = false;
        let mut proxy_request_permit_for_task = Some(proxy_request_permit_for_task);

        if let Some(chunk) = first_chunk {
            forwarded_chunks = forwarded_chunks.saturating_add(1);
            forwarded_bytes = forwarded_bytes.saturating_add(chunk.len());
            if tx.send(Ok(chunk)).await.is_err() {
                downstream_closed = true;
                let _ = proxy_request_permit_for_task.take();
            }
        }

        loop {
            if downstream_closed {
                break;
            }
            let Some(next_chunk) = upstream_stream.next().await else {
                break;
            };
            match next_chunk {
                Ok(chunk) => {
                    forwarded_chunks = forwarded_chunks.saturating_add(1);
                    forwarded_bytes = forwarded_bytes.saturating_add(chunk.len());
                    if tx.send(Ok(chunk)).await.is_err() {
                        let _ = proxy_request_permit_for_task.take();
                        break;
                    }
                }
                Err(err) => {
                    let message = format!("upstream stream error: {err}");
                    stream_error_message = Some(message.clone());
                    let _ = tx.send(Err(io::Error::other(message))).await;
                    break;
                }
            }
        }
        drop(proxy_request_permit_for_task.take());

        if let Some(message) = stream_error_message.as_deref() {
            release_pool_routing_reservation(
                state_for_record.as_ref(),
                &reservation_key_for_record,
            );
            if let Err(route_err) = record_pool_route_transport_failure(
                &state_for_record.pool,
                account.account_id,
                sticky_key_for_record.as_deref(),
                message,
                invoke_id_for_record.as_deref(),
            )
            .await
            {
                warn!(account_id = account.account_id, error = %route_err, "failed to record pool stream error");
            }
        } else {
            consume_pool_routing_reservation(
                state_for_record.as_ref(),
                &reservation_key_for_record,
            );
            if let Err(route_err) = record_pool_route_success(
                &state_for_record.pool,
                account.account_id,
                upstream_attempt_started_at_utc_for_record,
                sticky_key_for_record.as_deref(),
                invoke_id_for_record.as_deref(),
            )
            .await
            {
                warn!(account_id = account.account_id, error = %route_err, "failed to record pool route success");
            }
        }

        info!(
            proxy_request_id,
            account_id = account.account_id,
            t_upstream_connect_ms,
            forwarded_chunks,
            forwarded_bytes,
            elapsed_ms = stream_started_at.elapsed().as_millis(),
            "pool upstream response stream completed"
        );
    });

    response_builder
        .body(Body::from_stream(ReceiverStream::new(rx)))
        .map_err(|err| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("failed to build proxy response: {err}"),
            )
        })
}

pub(crate) async fn send_forward_proxy_request_with_429_retry(
    state: Arc<AppState>,
    method: Method,
    target_url: Url,
    headers: &HeaderMap,
    body: Option<Bytes>,
    handshake_timeout: Duration,
    capture_target: Option<ProxyCaptureTarget>,
    upstream_429_max_retries: u8,
) -> Result<ForwardProxyUpstreamResponse, ForwardProxyUpstreamError> {
    let request_connection_scoped = connection_scoped_header_names(headers);

    for attempt in 0..=upstream_429_max_retries {
        let selected_proxy = match select_forward_proxy_for_request(state.as_ref()).await {
            Ok(selected_proxy) => selected_proxy,
            Err(err) => {
                return Err(ForwardProxyUpstreamError {
                    selected_proxy: SelectedForwardProxy::from_endpoint(
                        &ForwardProxyEndpoint::direct(),
                    ),
                    status: StatusCode::BAD_GATEWAY,
                    message: format!("failed to select forward proxy node: {err}"),
                    failure_kind: PROXY_FAILURE_FAILED_CONTACT_UPSTREAM,
                    attempt_failure_kind: FORWARD_PROXY_FAILURE_SEND_ERROR,
                    connect_latency_ms: 0.0,
                });
            }
        };
        let client = match state
            .http_clients
            .client_for_forward_proxy(selected_proxy.endpoint_url.as_ref())
        {
            Ok(client) => client,
            Err(err) => {
                return Err(ForwardProxyUpstreamError {
                    selected_proxy,
                    status: StatusCode::BAD_GATEWAY,
                    message: format!("failed to initialize forward proxy client: {err}"),
                    failure_kind: PROXY_FAILURE_FAILED_CONTACT_UPSTREAM,
                    attempt_failure_kind: FORWARD_PROXY_FAILURE_SEND_ERROR,
                    connect_latency_ms: 0.0,
                });
            }
        };

        let mut request = client.request(method.clone(), target_url.clone());
        for (name, value) in headers {
            if should_forward_proxy_header(name, &request_connection_scoped) {
                request = request.header(name, value);
            }
        }
        if let Some(body_bytes) = body.clone() {
            request = request.body(body_bytes);
        }

        let connect_started = Instant::now();
        let response = match timeout(handshake_timeout, request.send()).await {
            Ok(Ok(response)) => response,
            Ok(Err(err)) => {
                return Err(ForwardProxyUpstreamError {
                    selected_proxy,
                    status: StatusCode::BAD_GATEWAY,
                    message: format!("failed to contact upstream: {err}"),
                    failure_kind: PROXY_FAILURE_FAILED_CONTACT_UPSTREAM,
                    attempt_failure_kind: FORWARD_PROXY_FAILURE_SEND_ERROR,
                    connect_latency_ms: elapsed_ms(connect_started),
                });
            }
            Err(_) => {
                return Err(ForwardProxyUpstreamError {
                    selected_proxy,
                    status: StatusCode::BAD_GATEWAY,
                    message: proxy_request_send_timeout_message(capture_target, handshake_timeout),
                    failure_kind: PROXY_FAILURE_UPSTREAM_HANDSHAKE_TIMEOUT,
                    attempt_failure_kind: FORWARD_PROXY_FAILURE_HANDSHAKE_TIMEOUT,
                    connect_latency_ms: elapsed_ms(connect_started),
                });
            }
        };

        let connect_latency_ms = elapsed_ms(connect_started);
        if response.status() != StatusCode::TOO_MANY_REQUESTS {
            return Ok(ForwardProxyUpstreamResponse {
                selected_proxy,
                response: ProxyUpstreamResponseBody::Reqwest(response),
                connect_latency_ms,
                attempt_started_at: connect_started,
                attempt_recorded: false,
                attempt_update: None,
            });
        }

        if attempt < upstream_429_max_retries {
            record_forward_proxy_attempt(
                state.clone(),
                selected_proxy.clone(),
                false,
                Some(connect_latency_ms),
                Some(FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429),
                false,
            )
            .await;

            let retry_delay = response
                .headers()
                .get(header::RETRY_AFTER)
                .and_then(parse_retry_after_delay)
                .unwrap_or_else(|| fallback_proxy_429_retry_delay(u32::from(attempt) + 1));
            info!(
                proxy_key_ref = %forward_proxy_log_ref(&selected_proxy.key),
                proxy_source = selected_proxy.source,
                proxy_label = selected_proxy.display_name,
                proxy_url_ref = %forward_proxy_log_ref_option(selected_proxy.endpoint_url_raw.as_deref()),
                retry_index = attempt + 1,
                max_429_retries = upstream_429_max_retries,
                retry_after_ms = retry_delay.as_millis(),
                "upstream responded 429; retrying forward proxy request"
            );
            sleep(retry_delay).await;
            continue;
        }

        // Final 429: defer attempt recording until the caller finishes consuming / forwarding
        // the response body, so a later stream error can override this classification.
        return Ok(ForwardProxyUpstreamResponse {
            selected_proxy,
            response: ProxyUpstreamResponseBody::Reqwest(response),
            connect_latency_ms,
            attempt_started_at: connect_started,
            attempt_recorded: false,
            attempt_update: None,
        });
    }

    unreachable!("429 retry loop should always return a response or error")
}
