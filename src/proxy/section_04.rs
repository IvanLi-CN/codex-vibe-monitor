async fn proxy_openai_v1_inner(
    state: Arc<AppState>,
    proxy_request_id: u64,
    invoke_id: String,
    original_uri: Uri,
    method: Method,
    headers: HeaderMap,
    body: Body,
    peer_ip: Option<IpAddr>,
) -> Result<Response, ProxyErrorResponse> {
    let pool_route_active = request_matches_pool_route(state.as_ref(), &headers)
        .await
        .map_err(|err| ProxyErrorResponse {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message: format!("failed to resolve pool routing settings: {err}"),
            cvm_id: None,
            retry_after_secs: None,
        })?;
    let runtime_timeouts = resolve_proxy_request_timeouts(state.as_ref(), pool_route_active)
        .await
        .map_err(|err| ProxyErrorResponse {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message: format!("failed to resolve pool routing timeouts: {err}"),
            cvm_id: None,
            retry_after_secs: None,
        })?;
    if !pool_route_active {
        // `/v1/*` is pool-only; non-pool traffic must stop here instead of reviving the
        // removed reverse-proxy/direct path.
        return Err(ProxyErrorResponse {
            status: StatusCode::UNAUTHORIZED,
            message: "pool route key missing or invalid".to_string(),
            cvm_id: None,
            retry_after_secs: None,
        });
    }
    let target_url =
        build_proxy_upstream_url(&state.config.openai_upstream_base_url, &original_uri).map_err(
            |err| {
                let status = if err.to_string().contains(PROXY_DOT_SEGMENT_PATH_NOT_ALLOWED)
                    || err.to_string().contains(PROXY_INVALID_REQUEST_TARGET)
                    || err
                        .to_string()
                        .contains("failed to parse proxy upstream url")
                {
                    StatusCode::BAD_REQUEST
                } else {
                    StatusCode::INTERNAL_SERVER_ERROR
                };
                ProxyErrorResponse {
                    status,
                    message: format!("failed to build upstream url: {err}"),
                    cvm_id: None,
                    retry_after_secs: None,
                }
            },
        )?;

    if method == Method::GET && is_models_list_path(original_uri.path()) {
        return proxy_openai_v1_via_pool(
            state,
            proxy_request_id,
            &original_uri,
            method,
            headers,
            body,
            runtime_timeouts,
        )
        .await
        .map_err(|(status, message)| ProxyErrorResponse {
            retry_after_secs: retry_after_secs_for_proxy_error(status, &message),
            status,
            message,
            cvm_id: None,
        });
    }

    if let Some(target) = capture_target_for_request(original_uri.path(), &method) {
        let tracked_invoke_id = invoke_id.clone();
        return proxy_openai_v1_capture_target(
            state,
            proxy_request_id,
            invoke_id,
            &original_uri,
            headers,
            body,
            target,
            target_url,
            peer_ip,
            pool_route_active,
            runtime_timeouts,
        )
        .await
        .map_err(|(status, message)| ProxyErrorResponse {
            retry_after_secs: retry_after_secs_for_proxy_error(status, &message),
            status,
            message,
            cvm_id: Some(tracked_invoke_id),
        });
    }

    return proxy_openai_v1_via_pool(
        state,
        proxy_request_id,
        &original_uri,
        method,
        headers,
        body,
        runtime_timeouts,
    )
    .await
    .map_err(|(status, message)| ProxyErrorResponse {
        retry_after_secs: retry_after_secs_for_proxy_error(status, &message),
        status,
        message,
        cvm_id: None,
    });
}

fn capture_target_for_request(path: &str, method: &Method) -> Option<ProxyCaptureTarget> {
    if *method != Method::POST {
        return None;
    }
    match path {
        "/v1/chat/completions" => Some(ProxyCaptureTarget::ChatCompletions),
        "/v1/responses" => Some(ProxyCaptureTarget::Responses),
        "/v1/responses/compact" => Some(ProxyCaptureTarget::ResponsesCompact),
        _ => None,
    }
}

#[allow(clippy::too_many_arguments)]
async fn proxy_openai_v1_capture_target(
    state: Arc<AppState>,
    proxy_request_id: u64,
    invoke_id: String,
    original_uri: &Uri,
    headers: HeaderMap,
    body: Body,
    capture_target: ProxyCaptureTarget,
    target_url: Url,
    peer_ip: Option<IpAddr>,
    pool_route_active: bool,
    runtime_timeouts: PoolRoutingTimeoutSettingsResolved,
) -> Result<Response, (StatusCode, String)> {
    if !pool_route_active {
        return Err((
            StatusCode::UNAUTHORIZED,
            "pool route key missing or invalid".to_string(),
        ));
    }
    let capture_started = Instant::now();
    let pool_routing_reservation_key = build_pool_routing_reservation_key(proxy_request_id);
    let occurred_at_utc = Utc::now();
    let occurred_at = format_naive(occurred_at_utc.with_timezone(&Shanghai).naive_local());
    let body_limit = state.config.openai_proxy_max_request_body_bytes;
    let requester_ip = extract_requester_ip(&headers, peer_ip);
    let header_sticky_key = extract_sticky_key_from_headers(&headers);
    let header_prompt_cache_key = extract_prompt_cache_key_from_headers(&headers);

    let req_read_started = Instant::now();
    let request_body_bytes = match read_request_body_with_limit(
        body,
        body_limit,
        runtime_timeouts.request_read_timeout,
        proxy_request_id,
    )
    .await
    {
        Ok(bytes) => bytes,
        Err(read_err) => {
            let t_req_read_ms = elapsed_ms(req_read_started);
            let request_info = RequestCaptureInfo::default();
            let req_raw = store_raw_payload_file(
                &state.config,
                &invoke_id,
                "request",
                &read_err.partial_body,
            )
            .await;
            let usage = ParsedUsage::default();
            let (cost, cost_estimated, price_version) = estimate_proxy_cost_from_shared_catalog(
                &state.pricing_catalog,
                None,
                &usage,
                None,
                ProxyPricingMode::ResponseTier,
            )
            .await;
            let error_message = format!("[{}] {}", read_err.failure_kind, read_err.message);

            warn!(
                proxy_request_id,
                status = %read_err.status,
                failure_kind = read_err.failure_kind,
                error = %read_err.message,
                elapsed_ms = t_req_read_ms,
                "openai proxy request body read failed"
            );

            let record = ProxyCaptureRecord {
                invoke_id,
                occurred_at,
                model: None,
                usage,
                cost,
                cost_estimated,
                price_version,
                status: if read_err.status.is_server_error() {
                    format!("http_{}", read_err.status.as_u16())
                } else {
                    "failed".to_string()
                },
                error_message: Some(error_message),
                failure_kind: Some(read_err.failure_kind.to_string()),
                payload: Some(build_proxy_payload_summary(ProxyPayloadSummary {
                    target: capture_target,
                    status: read_err.status,
                    is_stream: request_info.is_stream,
                    request_model: None,
                    requested_service_tier: request_info.requested_service_tier.as_deref(),
                    billing_service_tier: None,
                    reasoning_effort: request_info.reasoning_effort.as_deref(),
                    response_model: None,
                    usage_missing_reason: None,
                    request_parse_error: request_info.parse_error.as_deref(),
                    failure_kind: Some(read_err.failure_kind),
                    requester_ip: requester_ip.as_deref(),
                    upstream_scope: if pool_route_active {
                        INVOCATION_UPSTREAM_SCOPE_INTERNAL
                    } else {
                        INVOCATION_UPSTREAM_SCOPE_EXTERNAL
                    },
                    route_mode: if pool_route_active {
                        INVOCATION_ROUTE_MODE_POOL
                    } else {
                        INVOCATION_ROUTE_MODE_FORWARD_PROXY
                    },
                    sticky_key: header_sticky_key.as_deref(),
                    prompt_cache_key: header_prompt_cache_key.as_deref(),
                    upstream_account_id: None,
                    upstream_account_name: None,
                    upstream_account_kind: None,
                    upstream_base_url_host: None,
                    oauth_account_header_attached: None,
                    oauth_account_id_shape: None,
                    oauth_forwarded_header_count: None,
                    oauth_forwarded_header_names: None,
                    oauth_fingerprint_version: None,
                    oauth_forwarded_header_fingerprints: None,
                    oauth_prompt_cache_header_forwarded: None,
                    oauth_request_body_prefix_fingerprint: None,
                    oauth_request_body_prefix_bytes: None,
                    oauth_request_body_snapshot_kind: None,
                    oauth_responses_body_mode: None,
                    oauth_responses_rewrite: None,
                    service_tier: None,
                    stream_terminal_event: None,
                    upstream_error_code: None,
                    upstream_error_message: None,
                    downstream_status_code: None,
                    downstream_error_message: None,
                    upstream_request_id: None,
                    response_content_encoding: None,
                    proxy_display_name: None,
                    proxy_weight_delta: None,
                    pool_attempt_count: None,
                    pool_distinct_account_count: None,
                    pool_attempt_terminal_reason: None,
                })),
                raw_response: "{}".to_string(),
                req_raw,
                resp_raw: RawPayloadMeta::default(),
                timings: StageTimings {
                    t_total_ms: 0.0,
                    t_req_read_ms,
                    t_req_parse_ms: 0.0,
                    t_upstream_connect_ms: 0.0,
                    t_upstream_ttfb_ms: 0.0,
                    t_upstream_stream_ms: 0.0,
                    t_resp_parse_ms: 0.0,
                    t_persist_ms: 0.0,
                },
            };
            if let Err(err) =
                persist_and_broadcast_proxy_capture(state.as_ref(), capture_started, record).await
            {
                warn!(proxy_request_id, error = %err, "failed to persist proxy capture record");
            }
            return Err((read_err.status, read_err.message));
        }
    };
    let t_req_read_ms = elapsed_ms(req_read_started);

    let proxy_settings = state.proxy_model_settings.read().await.clone();
    let req_parse_started = Instant::now();
    let (upstream_body, mut request_info, body_rewritten) = prepare_target_request_body(
        capture_target,
        request_body_bytes,
        state.config.proxy_enforce_stream_include_usage,
    );
    let prompt_cache_key = request_info
        .prompt_cache_key
        .clone()
        .or_else(|| header_prompt_cache_key.clone());
    let sticky_key = request_info
        .sticky_key
        .clone()
        .or_else(|| header_sticky_key.clone());
    let pool_attempt_trace_context = pool_route_active.then(|| PoolUpstreamAttemptTraceContext {
        invoke_id: invoke_id.clone(),
        occurred_at: occurred_at.clone(),
        endpoint: capture_target.endpoint().to_string(),
        sticky_key: sticky_key.clone(),
        requester_ip: requester_ip.clone(),
    });
    let t_req_parse_ms = elapsed_ms(req_parse_started);
    let upstream_body_bytes = Bytes::from(upstream_body);
    let base_request_bytes_for_capture = upstream_body_bytes.clone();

    let initial_running_record = build_running_proxy_capture_record(
        &invoke_id,
        &occurred_at,
        capture_target,
        &request_info,
        requester_ip.as_deref(),
        sticky_key.as_deref(),
        prompt_cache_key.as_deref(),
        pool_route_active,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        t_req_read_ms,
        t_req_parse_ms,
        0.0,
        0.0,
    );
    if let Err(err) = persist_and_broadcast_proxy_capture_runtime_snapshot(
        state.as_ref(),
        initial_running_record,
    )
    .await
    {
        warn!(
            ?err,
            invoke_id = %invoke_id,
            "failed to broadcast initial running proxy capture snapshot"
        );
    }

    let mut upstream_headers = headers.clone();
    if body_rewritten {
        upstream_headers.remove(header::CONTENT_LENGTH);
    }
    let pool_attempt_runtime_snapshot =
        pool_route_active.then(|| PoolAttemptRuntimeSnapshotContext {
            capture_target,
            request_info: request_info.clone(),
            prompt_cache_key: prompt_cache_key.clone(),
            t_req_read_ms,
            t_req_parse_ms,
        });
    let handshake_timeout =
        proxy_upstream_send_timeout_for_capture_target(&runtime_timeouts, Some(capture_target));
    let first_byte_timeout =
        pool_upstream_first_chunk_timeout(&runtime_timeouts, &original_uri, &Method::POST);
    let stream_timeout = proxy_capture_target_stream_timeout(&runtime_timeouts, capture_target);
    let mut pool_invocation_cleanup_guard = pool_route_active.then(|| {
        PoolInvocationCleanupGuard::new(
            state.clone(),
            InvocationRecoverySelector::new(invoke_id.clone(), occurred_at.clone()),
            "request_drop_guard",
        )
    });
    let (
        selected_proxy,
        pool_account,
        t_upstream_connect_ms,
        prefetched_first_chunk,
        prefetched_ttfb_ms,
        oauth_responses_debug,
        attempt_already_recorded,
        final_attempt_update,
        pending_pool_attempt_record,
        deferred_pool_early_phase_cleanup_guard,
        live_pool_attempt_activity_lease,
        pending_pool_attempt_summary,
        upstream_attempt_started_at,
        upstream_attempt_started_at_utc,
        final_request_body_for_capture,
        final_requested_service_tier,
        upstream_response,
    ) = if pool_route_active {
        match send_pool_request_with_failover(
            state.clone(),
            proxy_request_id,
            Method::POST,
            &original_uri,
            &upstream_headers,
            Some(PoolReplayBodySnapshot::Memory(upstream_body_bytes.clone())),
            handshake_timeout,
            pool_attempt_trace_context.clone(),
            pool_attempt_runtime_snapshot.clone(),
            sticky_key.as_deref(),
            None,
            PoolFailoverProgress::default(),
            POOL_UPSTREAM_SAME_ACCOUNT_MAX_ATTEMPTS,
        )
        .await
        {
            Ok(response) => (
                None,
                Some(response.account),
                response.connect_latency_ms,
                response.first_chunk,
                response.first_byte_latency_ms,
                response.oauth_responses_debug,
                true,
                None,
                response.pending_attempt_record,
                response.deferred_early_phase_cleanup_guard,
                response.live_attempt_activity_lease,
                response.attempt_summary,
                None,
                Some(response.attempt_started_at_utc),
                response.request_body_for_capture,
                response.requested_service_tier,
                response.response,
            ),
            Err(err) => {
                request_info.requested_service_tier = err
                    .requested_service_tier
                    .clone()
                    .or(request_info.requested_service_tier);
                let request_body_for_capture = err
                    .request_body_for_capture
                    .clone()
                    .unwrap_or_else(|| base_request_bytes_for_capture.clone());
                let req_raw = store_raw_payload_file(
                    &state.config,
                    &invoke_id,
                    "request",
                    request_body_for_capture.as_ref(),
                )
                .await;
                let usage = ParsedUsage::default();
                let (billing_service_tier, pricing_mode) =
                    resolve_proxy_billing_service_tier_and_pricing_mode_for_account(
                        None,
                        request_info.requested_service_tier.as_deref(),
                        None,
                        err.account.as_ref(),
                    );
                let (cost, cost_estimated, price_version) =
                    estimate_proxy_cost_from_shared_catalog(
                        &state.pricing_catalog,
                        request_info.model.as_deref(),
                        &usage,
                        billing_service_tier.as_deref(),
                        pricing_mode,
                    )
                    .await;
                let error_message = format!("[{}] {}", err.failure_kind, err.message);
                let pool_proxy_display_name = resolve_invocation_proxy_display_name(None);
                let record = ProxyCaptureRecord {
                    invoke_id,
                    occurred_at,
                    model: request_info.model,
                    usage,
                    cost,
                    cost_estimated,
                    price_version,
                    status: if err.status.is_server_error() {
                        format!("http_{}", err.status.as_u16())
                    } else {
                        "failed".to_string()
                    },
                    error_message: Some(error_message),
                    failure_kind: Some(err.failure_kind.to_string()),
                    payload: Some(build_proxy_payload_summary(ProxyPayloadSummary {
                        target: capture_target,
                        status: err.status,
                        is_stream: request_info.is_stream,
                        request_model: None,
                        requested_service_tier: request_info.requested_service_tier.as_deref(),
                        billing_service_tier: billing_service_tier.as_deref(),
                        reasoning_effort: request_info.reasoning_effort.as_deref(),
                        response_model: None,
                        usage_missing_reason: None,
                        request_parse_error: request_info.parse_error.as_deref(),
                        failure_kind: Some(err.failure_kind),
                        requester_ip: requester_ip.as_deref(),
                        upstream_scope: INVOCATION_UPSTREAM_SCOPE_INTERNAL,
                        route_mode: INVOCATION_ROUTE_MODE_POOL,
                        sticky_key: sticky_key.as_deref(),
                        prompt_cache_key: prompt_cache_key.as_deref(),
                        upstream_account_id: err.account.as_ref().map(|account| account.account_id),
                        upstream_account_name: err
                            .account
                            .as_ref()
                            .map(|account| account.display_name.as_str()),
                        upstream_account_kind: payload_summary_upstream_account_kind(
                            err.account.as_ref(),
                        ),
                        upstream_base_url_host: payload_summary_upstream_base_url_host(
                            err.account.as_ref(),
                        ),
                        oauth_account_header_attached: oauth_account_header_attached_for_account(
                            err.account.as_ref(),
                        ),
                        oauth_account_id_shape: oauth_account_id_shape_for_account(
                            err.account.as_ref(),
                        ),
                        oauth_forwarded_header_count: err
                            .oauth_responses_debug
                            .as_ref()
                            .map(|debug| debug.forwarded_header_names.len()),
                        oauth_forwarded_header_names: err
                            .oauth_responses_debug
                            .as_ref()
                            .map(|debug| debug.forwarded_header_names.as_slice()),
                        oauth_fingerprint_version: err
                            .oauth_responses_debug
                            .as_ref()
                            .and_then(|debug| debug.fingerprint_version),
                        oauth_forwarded_header_fingerprints: err
                            .oauth_responses_debug
                            .as_ref()
                            .and_then(|debug| debug.forwarded_header_fingerprints.as_ref()),
                        oauth_prompt_cache_header_forwarded: err
                            .oauth_responses_debug
                            .as_ref()
                            .map(|debug| debug.prompt_cache_header_forwarded),
                        oauth_request_body_prefix_fingerprint: err
                            .oauth_responses_debug
                            .as_ref()
                            .and_then(|debug| debug.request_body_prefix_fingerprint.as_deref()),
                        oauth_request_body_prefix_bytes: err
                            .oauth_responses_debug
                            .as_ref()
                            .and_then(|debug| debug.request_body_prefix_bytes),
                        oauth_request_body_snapshot_kind: err
                            .oauth_responses_debug
                            .as_ref()
                            .and_then(|debug| debug.request_body_snapshot_kind),
                        oauth_responses_body_mode: err
                            .oauth_responses_debug
                            .as_ref()
                            .and_then(|debug| debug.responses_body_mode),
                        oauth_responses_rewrite: err
                            .oauth_responses_debug
                            .as_ref()
                            .map(|debug| &debug.rewrite),
                        service_tier: None,
                        stream_terminal_event: None,
                        upstream_error_code: err.upstream_error_code.as_deref(),
                        upstream_error_message: err.upstream_error_message.as_deref(),
                        downstream_status_code: None,
                        downstream_error_message: None,
                        upstream_request_id: err.upstream_request_id.as_deref(),
                        response_content_encoding: None,
                        proxy_display_name: pool_proxy_display_name.as_deref(),
                        proxy_weight_delta: None,
                        pool_attempt_count: Some(err.attempt_summary.pool_attempt_count),
                        pool_distinct_account_count: Some(
                            err.attempt_summary.pool_distinct_account_count,
                        ),
                        pool_attempt_terminal_reason: err
                            .attempt_summary
                            .pool_attempt_terminal_reason
                            .as_deref(),
                    })),
                    raw_response: "{}".to_string(),
                    req_raw,
                    resp_raw: RawPayloadMeta::default(),
                    timings: StageTimings {
                        t_total_ms: 0.0,
                        t_req_read_ms,
                        t_req_parse_ms,
                        t_upstream_connect_ms: err.connect_latency_ms,
                        t_upstream_ttfb_ms: 0.0,
                        t_upstream_stream_ms: 0.0,
                        t_resp_parse_ms: 0.0,
                        t_persist_ms: 0.0,
                    },
                };
                let terminal_invocation_persisted = if let Err(err) =
                    persist_and_broadcast_proxy_capture(state.as_ref(), capture_started, record)
                        .await
                {
                    warn!(proxy_request_id, error = %err, "failed to persist proxy capture record");
                    false
                } else {
                    true
                };
                if terminal_invocation_persisted {
                    disarm_pool_invocation_cleanup_guard(&mut pool_invocation_cleanup_guard);
                }
                return Err((err.status, err.message));
            }
        }
    } else {
        match send_forward_proxy_request_with_429_retry(
            state.clone(),
            Method::POST,
            target_url,
            &upstream_headers,
            Some(upstream_body_bytes.clone()),
            handshake_timeout,
            Some(capture_target),
            proxy_settings.upstream_429_max_retries,
        )
        .await
        {
            Ok(response) => (
                Some(response.selected_proxy),
                None,
                response.connect_latency_ms,
                None,
                0.0,
                None,
                response.attempt_recorded,
                response.attempt_update,
                None,
                None,
                None,
                PoolAttemptSummary::default(),
                Some(response.attempt_started_at),
                None,
                Some(base_request_bytes_for_capture.clone()),
                request_info.requested_service_tier.clone(),
                response.response,
            ),
            Err(err) => {
                let req_raw = store_raw_payload_file(
                    &state.config,
                    &invoke_id,
                    "request",
                    base_request_bytes_for_capture.as_ref(),
                )
                .await;
                let proxy_attempt_update = record_forward_proxy_attempt(
                    state.clone(),
                    err.selected_proxy.clone(),
                    false,
                    Some(err.connect_latency_ms),
                    Some(err.attempt_failure_kind),
                    false,
                )
                .await;
                let usage = ParsedUsage::default();
                let (billing_service_tier, pricing_mode) =
                    resolve_proxy_billing_service_tier_and_pricing_mode(
                        None,
                        request_info.requested_service_tier.as_deref(),
                        None,
                        None,
                    );
                let (cost, cost_estimated, price_version) =
                    estimate_proxy_cost_from_shared_catalog(
                        &state.pricing_catalog,
                        request_info.model.as_deref(),
                        &usage,
                        billing_service_tier.as_deref(),
                        pricing_mode,
                    )
                    .await;
                let error_message = format!("[{}] {}", err.failure_kind, err.message);
                let record = ProxyCaptureRecord {
                    invoke_id,
                    occurred_at,
                    model: request_info.model,
                    usage,
                    cost,
                    cost_estimated,
                    price_version,
                    status: if err.status.is_server_error() {
                        format!("http_{}", err.status.as_u16())
                    } else {
                        "failed".to_string()
                    },
                    error_message: Some(error_message),
                    failure_kind: Some(err.failure_kind.to_string()),
                    payload: Some(build_proxy_payload_summary(ProxyPayloadSummary {
                        target: capture_target,
                        status: err.status,
                        is_stream: request_info.is_stream,
                        request_model: None,
                        requested_service_tier: request_info.requested_service_tier.as_deref(),
                        billing_service_tier: billing_service_tier.as_deref(),
                        reasoning_effort: request_info.reasoning_effort.as_deref(),
                        response_model: None,
                        usage_missing_reason: None,
                        request_parse_error: request_info.parse_error.as_deref(),
                        failure_kind: Some(err.failure_kind),
                        requester_ip: requester_ip.as_deref(),
                        upstream_scope: INVOCATION_UPSTREAM_SCOPE_EXTERNAL,
                        route_mode: INVOCATION_ROUTE_MODE_FORWARD_PROXY,
                        sticky_key: sticky_key.as_deref(),
                        prompt_cache_key: prompt_cache_key.as_deref(),
                        upstream_account_id: None,
                        upstream_account_name: None,
                        upstream_account_kind: None,
                        upstream_base_url_host: None,
                        oauth_account_header_attached: None,
                        oauth_account_id_shape: None,
                        oauth_forwarded_header_count: None,
                        oauth_forwarded_header_names: None,
                        oauth_fingerprint_version: None,
                        oauth_forwarded_header_fingerprints: None,
                        oauth_prompt_cache_header_forwarded: None,
                        oauth_request_body_prefix_fingerprint: None,
                        oauth_request_body_prefix_bytes: None,
                        oauth_request_body_snapshot_kind: None,
                        oauth_responses_body_mode: None,
                        oauth_responses_rewrite: None,
                        service_tier: None,
                        stream_terminal_event: None,
                        upstream_error_code: None,
                        upstream_error_message: None,
                        downstream_status_code: None,
                        downstream_error_message: None,
                        upstream_request_id: None,
                        response_content_encoding: None,
                        proxy_display_name: Some(err.selected_proxy.display_name.as_str()),
                        proxy_weight_delta: proxy_attempt_update.delta(),
                        pool_attempt_count: None,
                        pool_distinct_account_count: None,
                        pool_attempt_terminal_reason: None,
                    })),
                    raw_response: "{}".to_string(),
                    req_raw,
                    resp_raw: RawPayloadMeta::default(),
                    timings: StageTimings {
                        t_total_ms: 0.0,
                        t_req_read_ms,
                        t_req_parse_ms,
                        t_upstream_connect_ms: err.connect_latency_ms,
                        t_upstream_ttfb_ms: 0.0,
                        t_upstream_stream_ms: 0.0,
                        t_resp_parse_ms: 0.0,
                        t_persist_ms: 0.0,
                    },
                };
                if let Err(err) =
                    persist_and_broadcast_proxy_capture(state.as_ref(), capture_started, record)
                        .await
                {
                    warn!(proxy_request_id, error = %err, "failed to persist proxy capture record");
                }
                return Err((err.status, err.message));
            }
        }
    };
    request_info.requested_service_tier = final_requested_service_tier
        .clone()
        .or(request_info.requested_service_tier);
    let req_raw = store_raw_payload_file(
        &state.config,
        &invoke_id,
        "request",
        final_request_body_for_capture
            .as_ref()
            .unwrap_or(&base_request_bytes_for_capture)
            .as_ref(),
    )
    .await;

    let upstream_status = upstream_response.status();
    let location_base_url = location_rewrite_upstream_base(
        pool_account.as_ref(),
        &state.config.openai_upstream_base_url,
    );
    let rewritten_location = match normalize_proxy_location_header(
        upstream_status,
        upstream_response.headers(),
        location_base_url,
    ) {
        Ok(location) => location,
        Err(err) => {
            let message = format!("failed to process upstream redirect: {err}");
            let proxy_attempt_update = if let Some(selected_proxy) = selected_proxy.as_ref() {
                record_forward_proxy_attempt(
                    state.clone(),
                    selected_proxy.clone(),
                    false,
                    Some(t_upstream_connect_ms),
                    Some(FORWARD_PROXY_FAILURE_SEND_ERROR),
                    false,
                )
                .await
            } else {
                ForwardProxyAttemptUpdate::default()
            };
            let usage = ParsedUsage::default();
            let (billing_service_tier, pricing_mode) =
                resolve_proxy_billing_service_tier_and_pricing_mode_for_account(
                    None,
                    request_info.requested_service_tier.as_deref(),
                    None,
                    pool_account.as_ref(),
                );
            let (cost, cost_estimated, price_version) = estimate_proxy_cost_from_shared_catalog(
                &state.pricing_catalog,
                request_info.model.as_deref(),
                &usage,
                billing_service_tier.as_deref(),
                pricing_mode,
            )
            .await;
            let proxy_display_name = resolve_invocation_proxy_display_name(selected_proxy.as_ref());
            let record = ProxyCaptureRecord {
                invoke_id,
                occurred_at,
                model: request_info.model,
                usage,
                cost,
                cost_estimated,
                price_version,
                status: "http_502".to_string(),
                error_message: Some(message.clone()),
                failure_kind: None,
                payload: Some(build_proxy_payload_summary(ProxyPayloadSummary {
                    target: capture_target,
                    status: StatusCode::BAD_GATEWAY,
                    is_stream: request_info.is_stream,
                    request_model: None,
                    requested_service_tier: request_info.requested_service_tier.as_deref(),
                    billing_service_tier: billing_service_tier.as_deref(),
                    reasoning_effort: request_info.reasoning_effort.as_deref(),
                    response_model: None,
                    usage_missing_reason: None,
                    request_parse_error: request_info.parse_error.as_deref(),
                    failure_kind: None,
                    requester_ip: requester_ip.as_deref(),
                    upstream_scope: if pool_route_active {
                        INVOCATION_UPSTREAM_SCOPE_INTERNAL
                    } else {
                        INVOCATION_UPSTREAM_SCOPE_EXTERNAL
                    },
                    route_mode: if pool_route_active {
                        INVOCATION_ROUTE_MODE_POOL
                    } else {
                        INVOCATION_ROUTE_MODE_FORWARD_PROXY
                    },
                    sticky_key: sticky_key.as_deref(),
                    prompt_cache_key: prompt_cache_key.as_deref(),
                    upstream_account_id: pool_account.as_ref().map(|account| account.account_id),
                    upstream_account_name: pool_account
                        .as_ref()
                        .map(|account| account.display_name.as_str()),
                    upstream_account_kind: payload_summary_upstream_account_kind(
                        pool_account.as_ref(),
                    ),
                    upstream_base_url_host: payload_summary_upstream_base_url_host(
                        pool_account.as_ref(),
                    ),
                    oauth_account_header_attached: oauth_account_header_attached_for_account(
                        pool_account.as_ref(),
                    ),
                    oauth_account_id_shape: oauth_account_id_shape_for_account(
                        pool_account.as_ref(),
                    ),
                    oauth_forwarded_header_count: None,
                    oauth_forwarded_header_names: None,
                    oauth_fingerprint_version: None,
                    oauth_forwarded_header_fingerprints: None,
                    oauth_prompt_cache_header_forwarded: None,
                    oauth_request_body_prefix_fingerprint: None,
                    oauth_request_body_prefix_bytes: None,
                    oauth_request_body_snapshot_kind: None,
                    oauth_responses_body_mode: None,
                    oauth_responses_rewrite: None,
                    service_tier: None,
                    stream_terminal_event: None,
                    upstream_error_code: None,
                    upstream_error_message: None,
                    downstream_status_code: None,
                    downstream_error_message: None,
                    upstream_request_id: None,
                    response_content_encoding: Some(
                        summarize_response_content_encoding(
                            upstream_response
                                .headers()
                                .get(header::CONTENT_ENCODING)
                                .and_then(|value| value.to_str().ok()),
                        )
                        .as_str(),
                    ),
                    proxy_display_name: proxy_display_name.as_deref(),
                    proxy_weight_delta: if selected_proxy.is_some() {
                        proxy_attempt_update.delta()
                    } else {
                        None
                    },
                    pool_attempt_count: None,
                    pool_distinct_account_count: None,
                    pool_attempt_terminal_reason: None,
                })),
                raw_response: "{}".to_string(),
                req_raw,
                resp_raw: RawPayloadMeta::default(),
                timings: StageTimings {
                    t_total_ms: 0.0,
                    t_req_read_ms,
                    t_req_parse_ms,
                    t_upstream_connect_ms,
                    t_upstream_ttfb_ms: 0.0,
                    t_upstream_stream_ms: 0.0,
                    t_resp_parse_ms: 0.0,
                    t_persist_ms: 0.0,
                },
            };
            if let Err(err) =
                persist_and_broadcast_proxy_capture(state.as_ref(), capture_started, record).await
            {
                warn!(proxy_request_id, error = %err, "failed to persist proxy capture record");
            }
            return Err((StatusCode::BAD_GATEWAY, message));
        }
    };

    let upstream_connection_scoped = connection_scoped_header_names(upstream_response.headers());
    let response_is_event_stream = upstream_response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.starts_with("text/event-stream"));
    let upstream_content_encoding = upstream_response
        .headers()
        .get(header::CONTENT_ENCODING)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    let response_content_encoding =
        summarize_response_content_encoding(upstream_content_encoding.as_deref());
    let selected_proxy_display_name =
        resolve_invocation_proxy_display_name(selected_proxy.as_ref());
    let response_running_record = build_running_proxy_capture_record(
        &invoke_id,
        &occurred_at,
        capture_target,
        &request_info,
        requester_ip.as_deref(),
        sticky_key.as_deref(),
        prompt_cache_key.as_deref(),
        pool_route_active,
        pool_account.as_ref().map(|account| account.account_id),
        pool_account
            .as_ref()
            .map(|account| account.display_name.as_str()),
        payload_summary_upstream_account_kind(pool_account.as_ref()),
        payload_summary_upstream_base_url_host(pool_account.as_ref()),
        selected_proxy_display_name.as_deref(),
        pool_account
            .as_ref()
            .map(|_| pending_pool_attempt_summary.pool_attempt_count),
        pool_account
            .as_ref()
            .map(|_| pending_pool_attempt_summary.pool_distinct_account_count),
        None,
        Some(response_content_encoding.as_str()),
        t_req_read_ms,
        t_req_parse_ms,
        t_upstream_connect_ms,
        prefetched_ttfb_ms,
    );
    if let Err(err) = persist_and_broadcast_proxy_capture_runtime_snapshot(
        state.as_ref(),
        response_running_record,
    )
    .await
    {
        warn!(
            ?err,
            invoke_id = %invoke_id,
            "failed to broadcast response-ready proxy capture snapshot"
        );
    }
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

    let state_for_task = state.clone();
    let request_info_for_task = request_info.clone();
    let req_raw_for_task = req_raw.clone();
    let invoke_id_for_task = invoke_id.clone();
    let occurred_at_for_task = occurred_at.clone();
    let upstream_content_encoding_for_task = upstream_content_encoding.clone();
    let requester_ip_for_task = requester_ip.clone();
    let sticky_key_for_task = sticky_key.clone();
    let reservation_key_for_task = pool_routing_reservation_key.clone();
    let prompt_cache_key_for_task = prompt_cache_key.clone();
    let selected_proxy_for_task = selected_proxy.clone();
    let selected_proxy_display_name_for_task = selected_proxy_display_name.clone();
    let pool_account_for_task = pool_account.clone();
    let oauth_responses_debug_for_task = oauth_responses_debug.clone();
    let attempt_already_recorded_for_task = attempt_already_recorded;
    let final_attempt_update_for_task = final_attempt_update;
    let pending_pool_attempt_record_for_task = pending_pool_attempt_record.clone();
    let mut deferred_pool_early_phase_cleanup_guard_for_task =
        deferred_pool_early_phase_cleanup_guard;
    let live_pool_attempt_activity_lease_for_task = live_pool_attempt_activity_lease;
    let pending_pool_attempt_summary_for_task = pending_pool_attempt_summary.clone();
    let prefetched_first_chunk_for_task = prefetched_first_chunk;
    let prefetched_ttfb_ms_for_task = prefetched_ttfb_ms;
    let upstream_attempt_started_at_for_task = upstream_attempt_started_at;
    let upstream_attempt_started_at_utc_for_task = upstream_attempt_started_at_utc;
    let first_byte_timeout_for_task = first_byte_timeout;
    let stream_timeout_for_task = stream_timeout;
    let response_is_event_stream_for_task = response_is_event_stream;
    let (tx, rx) = mpsc::channel::<Result<Bytes, io::Error>>(16);

    tokio::spawn(async move {
        let _live_pool_attempt_activity_lease_for_task = live_pool_attempt_activity_lease_for_task;
        let mut stream_invocation_cleanup_guard = pool_account_for_task.as_ref().map(|_| {
            PoolInvocationCleanupGuard::new(
                state_for_task.clone(),
                InvocationRecoverySelector::new(
                    invoke_id_for_task.clone(),
                    occurred_at_for_task.clone(),
                ),
                "stream_invocation_drop_guard",
            )
        });
        let mut stream = upstream_response.into_bytes_stream();
        let ttfb_started = Instant::now();
        let stream_started = Instant::now();
        let mut t_upstream_ttfb_ms = prefetched_ttfb_ms_for_task;
        let mut stream_started_at: Option<Instant> = None;
        let mut response_preview = RawResponsePreviewBuffer::default();
        let mut response_raw_writer =
            AsyncStreamingRawPayloadWriter::new(state_for_task.as_ref(), &invoke_id_for_task, "response");
        let mut stream_response_parser = StreamResponsePayloadChunkParser::default();
        let mut nonstream_parse_buffer = (!response_is_event_stream_for_task).then(|| {
            BoundedResponseParseBuffer::new(BOUNDED_NON_STREAM_RESPONSE_PARSE_LIMIT_BYTES)
        });
        let mut stream_error: Option<String> = None;
        let mut downstream_closed = false;
        let mut forwarded_chunks = 0usize;
        let mut forwarded_bytes = 0usize;

        if let Some(chunk) = prefetched_first_chunk_for_task {
            response_preview.append(&chunk);
            response_raw_writer.append(&chunk);
            stream_response_parser.ingest_bytes(&chunk);
            if let Some(buffer) = nonstream_parse_buffer.as_mut() {
                buffer.append(&chunk);
            }
            forwarded_chunks = forwarded_chunks.saturating_add(1);
            forwarded_bytes = forwarded_bytes.saturating_add(chunk.len());
            stream_started_at = Some(Instant::now());
            if !downstream_closed && tx.send(Ok(chunk)).await.is_err() {
                downstream_closed = true;
            }
        }

        loop {
            let next_chunk = if let Some(stream_started_at) = stream_started_at {
                if let Some(stream_timeout) = stream_timeout_for_task {
                    let Some(timeout_budget) =
                        remaining_timeout_budget(stream_timeout, stream_started_at.elapsed())
                    else {
                        let message = pool_upstream_timeout_message(
                            stream_timeout,
                            "waiting for upstream stream completion",
                        );
                        stream_error = Some(message.clone());
                        if !downstream_closed
                            && tx.send(Err(io::Error::other(message))).await.is_err()
                        {
                            downstream_closed = true;
                        }
                        break;
                    };
                    match timeout(timeout_budget, stream.next()).await {
                        Ok(next_chunk) => next_chunk,
                        Err(_) => {
                            let message = pool_upstream_timeout_message(
                                stream_timeout,
                                "waiting for upstream stream completion",
                            );
                            stream_error = Some(message.clone());
                            if !downstream_closed
                                && tx.send(Err(io::Error::other(message))).await.is_err()
                            {
                                downstream_closed = true;
                            }
                            break;
                        }
                    }
                } else {
                    stream.next().await
                }
            } else if let Some(attempt_started_at) = upstream_attempt_started_at_for_task {
                let Some(timeout_budget) = remaining_timeout_budget(
                    first_byte_timeout_for_task,
                    attempt_started_at.elapsed(),
                ) else {
                    let message = pool_upstream_timeout_message(
                        first_byte_timeout_for_task,
                        "waiting for first upstream chunk",
                    );
                    stream_error = Some(message.clone());
                    if !downstream_closed && tx.send(Err(io::Error::other(message))).await.is_err()
                    {
                        downstream_closed = true;
                    }
                    break;
                };
                match timeout(timeout_budget, stream.next()).await {
                    Ok(next_chunk) => next_chunk,
                    Err(_) => {
                        let message = pool_upstream_timeout_message(
                            first_byte_timeout_for_task,
                            "waiting for first upstream chunk",
                        );
                        stream_error = Some(message.clone());
                        if !downstream_closed
                            && tx.send(Err(io::Error::other(message))).await.is_err()
                        {
                            downstream_closed = true;
                        }
                        break;
                    }
                }
            } else {
                stream.next().await
            };
            let Some(next_chunk) = next_chunk else {
                break;
            };
            match next_chunk {
                Ok(chunk) => {
                    if stream_started_at.is_none() {
                        t_upstream_ttfb_ms = upstream_attempt_started_at_for_task
                            .map(elapsed_ms)
                            .unwrap_or_else(|| elapsed_ms(ttfb_started));
                        stream_started_at = Some(Instant::now());
                        let running_record = build_running_proxy_capture_record(
                            &invoke_id_for_task,
                            &occurred_at_for_task,
                            capture_target,
                            &request_info_for_task,
                            requester_ip_for_task.as_deref(),
                            sticky_key_for_task.as_deref(),
                            prompt_cache_key_for_task.as_deref(),
                            pool_account_for_task.is_some(),
                            pool_account_for_task
                                .as_ref()
                                .map(|account| account.account_id),
                            pool_account_for_task
                                .as_ref()
                                .map(|account| account.display_name.as_str()),
                            payload_summary_upstream_account_kind(
                                pool_account_for_task.as_ref(),
                            ),
                            payload_summary_upstream_base_url_host(
                                pool_account_for_task.as_ref(),
                            ),
                            selected_proxy_display_name_for_task.as_deref(),
                            pool_account_for_task
                                .as_ref()
                                .map(|_| pending_pool_attempt_summary_for_task.pool_attempt_count),
                            pool_account_for_task.as_ref().map(|_| {
                                pending_pool_attempt_summary_for_task.pool_distinct_account_count
                            }),
                            None,
                            Some(response_content_encoding.as_str()),
                            t_req_read_ms,
                            t_req_parse_ms,
                            t_upstream_connect_ms,
                            t_upstream_ttfb_ms,
                        );
                        if let Err(err) = persist_and_broadcast_proxy_capture_runtime_snapshot(
                            state_for_task.as_ref(),
                            running_record,
                        )
                        .await
                        {
                            warn!(
                                ?err,
                                invoke_id = %invoke_id_for_task,
                                "failed to broadcast first-byte proxy capture snapshot"
                            );
                        }
                    }
                    response_preview.append(&chunk);
                    response_raw_writer.append(&chunk);
                    stream_response_parser.ingest_bytes(&chunk);
                    if let Some(buffer) = nonstream_parse_buffer.as_mut() {
                        buffer.append(&chunk);
                    }
                    forwarded_chunks = forwarded_chunks.saturating_add(1);
                    forwarded_bytes = forwarded_bytes.saturating_add(chunk.len());
                    if !downstream_closed && tx.send(Ok(chunk)).await.is_err() {
                        downstream_closed = true;
                    }
                }
                Err(err) => {
                    let msg = format!("upstream stream error: {err}");
                    stream_error = Some(msg.clone());
                    if !downstream_closed {
                        let _ = tx.send(Err(io::Error::other(msg))).await;
                    }
                    break;
                }
            }
        }
        drop(tx);

        let terminal_state = if stream_error.is_some() {
            PROXY_STREAM_TERMINAL_ERROR
        } else if downstream_closed {
            PROXY_STREAM_TERMINAL_DOWNSTREAM_CLOSED
        } else {
            PROXY_STREAM_TERMINAL_COMPLETED
        };
        if let Some(err) = stream_error.as_deref() {
            warn!(
                proxy_request_id,
                terminal_state,
                error = err,
                forwarded_chunks,
                forwarded_bytes,
                elapsed_ms = stream_started.elapsed().as_millis(),
                "openai proxy capture stream finished with upstream error"
            );
        } else {
            info!(
                proxy_request_id,
                terminal_state,
                forwarded_chunks,
                forwarded_bytes,
                elapsed_ms = stream_started.elapsed().as_millis(),
                "openai proxy capture stream finished"
            );
        }

        let t_upstream_stream_ms = stream_started_at.map(elapsed_ms).unwrap_or(0.0);
        let resp_raw = response_raw_writer.finish().await;
        let preview_bytes = response_preview.as_slice().to_vec();
        let raw_response_preview = response_preview.into_preview();
        let streamed_response_outcome = stream_response_parser.finish();
        let preview_looks_like_sse = response_payload_looks_like_sse_after_decode(
            &preview_bytes,
            upstream_content_encoding_for_task.as_deref(),
        );
        let response_is_stream_hint = response_is_event_stream_for_task
            || streamed_response_outcome.saw_stream_fields
            || preview_looks_like_sse;
        let resp_parse_started = Instant::now();
        let mut response_info = if response_is_stream_hint {
            if streamed_response_outcome.saw_stream_fields {
                streamed_response_outcome.response_info
            } else {
                parse_target_response_preview_payload(
                    capture_target,
                    &preview_bytes,
                    true,
                    upstream_content_encoding_for_task.as_deref(),
                )
            }
        } else {
            nonstream_parse_buffer
                .take()
                .map(|buffer| {
                    buffer.into_response_info(
                        capture_target,
                        upstream_content_encoding_for_task.as_deref(),
                    )
                })
                .unwrap_or_else(|| {
                    parse_target_response_payload(
                        capture_target,
                        &preview_bytes,
                        false,
                        upstream_content_encoding_for_task.as_deref(),
                    )
                })
        };
        let t_resp_parse_ms = elapsed_ms(resp_parse_started);

        if response_info.model.is_none() {
            response_info.model = request_info_for_task.model.clone();
        }
        if response_info.usage_missing_reason.is_none() && stream_error.is_some() {
            response_info.usage_missing_reason = Some("upstream_stream_error".to_string());
        }

        let had_stream_error = stream_error.is_some();
        let had_logical_stream_failure = response_info.stream_terminal_event.is_some();
        let pure_downstream_closed = proxy_capture_is_pure_downstream_close(
            upstream_status,
            had_stream_error,
            had_logical_stream_failure,
            downstream_closed,
        );
        let failure_kind = proxy_capture_invocation_failure_kind(
            upstream_status,
            had_stream_error,
            had_logical_stream_failure,
            pure_downstream_closed,
        );

        let error_message = if let Some(err) = stream_error {
            Some(format!("[{}] {err}", PROXY_FAILURE_UPSTREAM_STREAM_ERROR))
        } else if had_logical_stream_failure {
            Some(format_upstream_response_failed_message(&response_info))
        } else if !upstream_status.is_success() {
            response_info
                .upstream_error_message
                .clone()
                .or_else(|| extract_error_message_from_response_preview(&preview_bytes))
        } else {
            None
        };
        let downstream_error_message = if downstream_closed {
            Some(format!(
                "[{}] downstream closed while streaming upstream response",
                PROXY_STREAM_TERMINAL_DOWNSTREAM_CLOSED
            ))
        } else {
            None
        };
        let status =
            proxy_capture_invocation_status(upstream_status, error_message.is_some(), pure_downstream_closed);
        let pending_pool_attempt_terminal_reason = if pool_account_for_task.is_none() {
            None
        } else if had_stream_error {
            Some(PROXY_FAILURE_UPSTREAM_STREAM_ERROR.to_string())
        } else if had_logical_stream_failure {
            Some(PROXY_FAILURE_UPSTREAM_RESPONSE_FAILED.to_string())
        } else if pure_downstream_closed {
            Some(PROXY_STREAM_TERMINAL_DOWNSTREAM_CLOSED.to_string())
        } else if !upstream_status.is_success() {
            Some(
                failure_kind
                    .unwrap_or(PROXY_FAILURE_UPSTREAM_RESPONSE_FAILED)
                    .to_string(),
            )
        } else {
            None
        };
        let mut pending_pool_attempt_summary = pending_pool_attempt_summary_for_task.clone();
        pending_pool_attempt_summary.pool_attempt_terminal_reason =
            pending_pool_attempt_terminal_reason.clone();
        let selected_proxy_display_name =
            resolve_invocation_proxy_display_name(selected_proxy_for_task.as_ref());
        let proxy_attempt_update = if let Some(selected_proxy) = selected_proxy_for_task.as_ref() {
            let forward_proxy_success = proxy_capture_response_status_is_success(
                upstream_status,
                had_stream_error,
                had_logical_stream_failure,
            );
            if attempt_already_recorded_for_task {
                final_attempt_update_for_task.unwrap_or_default()
            } else {
                record_forward_proxy_attempt(
                    state_for_task.clone(),
                    selected_proxy.clone(),
                    forward_proxy_success,
                    Some(t_upstream_connect_ms + t_upstream_ttfb_ms + t_upstream_stream_ms),
                    proxy_capture_response_failure_kind(
                        upstream_status,
                        had_stream_error,
                        had_logical_stream_failure,
                    ),
                    false,
                )
                .await
            }
        } else {
            if let Some(account) = pool_account_for_task.as_ref() {
                let pool_route_success = proxy_capture_response_status_is_success(
                    upstream_status,
                    had_stream_error,
                    had_logical_stream_failure,
                );
                let route_result = if pool_route_success || pure_downstream_closed {
                    consume_pool_routing_reservation(
                        state_for_task.as_ref(),
                        &reservation_key_for_task,
                    );
                    record_pool_route_success(
                        &state_for_task.pool,
                        account.account_id,
                        upstream_attempt_started_at_utc_for_task.unwrap_or_else(Utc::now),
                        sticky_key_for_task.as_deref(),
                        None,
                    )
                    .await
                } else if had_stream_error {
                    let route_message = error_message
                        .as_deref()
                        .unwrap_or("upstream stream error")
                        .to_string();
                    release_pool_routing_reservation(
                        state_for_task.as_ref(),
                        &reservation_key_for_task,
                    );
                    record_pool_route_transport_failure(
                        &state_for_task.pool,
                        account.account_id,
                        sticky_key_for_task.as_deref(),
                        &route_message,
                        None,
                    )
                    .await
                } else {
                    let route_message = error_message
                        .as_deref()
                        .unwrap_or("upstream request failed")
                        .to_string();
                    release_pool_routing_reservation(
                        state_for_task.as_ref(),
                        &reservation_key_for_task,
                    );
                    if response_info_is_retryable_server_overloaded(upstream_status, &response_info)
                    {
                        record_pool_route_retryable_overload_failure(
                            &state_for_task.pool,
                            account.account_id,
                            sticky_key_for_task.as_deref(),
                            &route_message,
                            None,
                        )
                        .await
                    } else {
                        record_pool_route_http_failure(
                            &state_for_task.pool,
                            account.account_id,
                            &account.kind,
                            sticky_key_for_task.as_deref(),
                            upstream_status,
                            &route_message,
                            None,
                        )
                        .await
                    }
                };
                if let Err(err) = route_result {
                    warn!(account_id = account.account_id, error = %err, "failed to record pool capture route state");
                }
            }
            ForwardProxyAttemptUpdate::default()
        };
        let mut final_attempt_persisted = false;
        if let Some(pending_attempt_record) = pending_pool_attempt_record_for_task.as_ref() {
            let finished_at = shanghai_now_string();
            let attempt_status = pool_capture_attempt_status(
                upstream_status,
                had_stream_error,
                had_logical_stream_failure,
                pure_downstream_closed,
            );
            let attempt_failure_kind = if pure_downstream_closed {
                None
            } else {
                failure_kind
            };
            final_attempt_persisted = match finalize_pool_upstream_request_attempt(
                &state_for_task.pool,
                pending_attempt_record,
                finished_at.as_str(),
                attempt_status,
                Some(upstream_status),
                attempt_failure_kind,
                error_message.as_deref(),
                Some(t_upstream_connect_ms),
                Some(t_upstream_ttfb_ms),
                Some(t_upstream_stream_ms),
                response_info.upstream_request_id.as_deref(),
                None,
                None,
            )
            .await
            {
                Ok(()) => true,
                Err(err) => {
                    warn!(
                    invoke_id = %pending_attempt_record.invoke_id,
                    error = %err,
                    "failed to persist final pool attempt"
                );
                    false
                }
            };
            if let Err(err) = broadcast_pool_upstream_attempts_snapshot(
                state_for_task.as_ref(),
                &pending_attempt_record.invoke_id,
            )
            .await
            {
                warn!(
                    invoke_id = %pending_attempt_record.invoke_id,
                    error = %err,
                    "failed to broadcast final pool attempt snapshot"
                );
            }
        }
        let (billing_service_tier, pricing_mode) =
            resolve_proxy_billing_service_tier_and_pricing_mode_for_account(
                None,
                request_info_for_task.requested_service_tier.as_deref(),
                response_info.service_tier.as_deref(),
                pool_account_for_task.as_ref(),
            );
        let (cost, cost_estimated, price_version) = estimate_proxy_cost_from_shared_catalog(
            &state_for_task.pricing_catalog,
            response_info.model.as_deref(),
            &response_info.usage,
            billing_service_tier.as_deref(),
            pricing_mode,
        )
        .await;
        let payload = build_proxy_payload_summary(ProxyPayloadSummary {
            target: capture_target,
            status: upstream_status,
            is_stream: request_info_for_task.is_stream,
            request_model: request_info_for_task.model.as_deref(),
            requested_service_tier: request_info_for_task.requested_service_tier.as_deref(),
            billing_service_tier: billing_service_tier.as_deref(),
            reasoning_effort: request_info_for_task.reasoning_effort.as_deref(),
            response_model: response_info.model.as_deref(),
            usage_missing_reason: response_info.usage_missing_reason.as_deref(),
            request_parse_error: request_info_for_task.parse_error.as_deref(),
            failure_kind,
            requester_ip: requester_ip_for_task.as_deref(),
            upstream_scope: if pool_account_for_task.is_some() {
                INVOCATION_UPSTREAM_SCOPE_INTERNAL
            } else {
                INVOCATION_UPSTREAM_SCOPE_EXTERNAL
            },
            route_mode: if pool_account_for_task.is_some() {
                INVOCATION_ROUTE_MODE_POOL
            } else {
                INVOCATION_ROUTE_MODE_FORWARD_PROXY
            },
            sticky_key: sticky_key_for_task.as_deref(),
            prompt_cache_key: prompt_cache_key_for_task.as_deref(),
            upstream_account_id: pool_account_for_task
                .as_ref()
                .map(|account| account.account_id),
            upstream_account_name: pool_account_for_task
                .as_ref()
                .map(|account| account.display_name.as_str()),
            upstream_account_kind: payload_summary_upstream_account_kind(
                pool_account_for_task.as_ref(),
            ),
            upstream_base_url_host: payload_summary_upstream_base_url_host(
                pool_account_for_task.as_ref(),
            ),
            oauth_account_header_attached: oauth_account_header_attached_for_account(
                pool_account_for_task.as_ref(),
            ),
            oauth_account_id_shape: oauth_account_id_shape_for_account(
                pool_account_for_task.as_ref(),
            ),
            oauth_forwarded_header_count: oauth_responses_debug_for_task
                .as_ref()
                .map(|debug| debug.forwarded_header_names.len()),
            oauth_forwarded_header_names: oauth_responses_debug_for_task
                .as_ref()
                .map(|debug| debug.forwarded_header_names.as_slice()),
            oauth_fingerprint_version: oauth_responses_debug_for_task
                .as_ref()
                .and_then(|debug| debug.fingerprint_version),
            oauth_forwarded_header_fingerprints: oauth_responses_debug_for_task
                .as_ref()
                .and_then(|debug| debug.forwarded_header_fingerprints.as_ref()),
            oauth_prompt_cache_header_forwarded: oauth_responses_debug_for_task
                .as_ref()
                .map(|debug| debug.prompt_cache_header_forwarded),
            oauth_request_body_prefix_fingerprint: oauth_responses_debug_for_task
                .as_ref()
                .and_then(|debug| debug.request_body_prefix_fingerprint.as_deref()),
            oauth_request_body_prefix_bytes: oauth_responses_debug_for_task
                .as_ref()
                .and_then(|debug| debug.request_body_prefix_bytes),
            oauth_request_body_snapshot_kind: oauth_responses_debug_for_task
                .as_ref()
                .and_then(|debug| debug.request_body_snapshot_kind),
            oauth_responses_body_mode: oauth_responses_debug_for_task
                .as_ref()
                .and_then(|debug| debug.responses_body_mode),
            oauth_responses_rewrite: oauth_responses_debug_for_task
                .as_ref()
                .map(|debug| &debug.rewrite),
            service_tier: response_info.service_tier.as_deref(),
            stream_terminal_event: response_info.stream_terminal_event.as_deref(),
            upstream_error_code: response_info.upstream_error_code.as_deref(),
            upstream_error_message: response_info.upstream_error_message.as_deref(),
            downstream_status_code: if downstream_closed {
                Some(upstream_status)
            } else {
                None
            },
            downstream_error_message: downstream_error_message.as_deref(),
            upstream_request_id: response_info.upstream_request_id.as_deref(),
            response_content_encoding: Some(response_content_encoding.as_str()),
            proxy_display_name: selected_proxy_display_name.as_deref(),
            proxy_weight_delta: if selected_proxy_for_task.is_some() {
                proxy_attempt_update.delta()
            } else {
                None
            },
            pool_attempt_count: pool_account_for_task
                .as_ref()
                .map(|_| pending_pool_attempt_summary.pool_attempt_count),
            pool_distinct_account_count: pool_account_for_task
                .as_ref()
                .map(|_| pending_pool_attempt_summary.pool_distinct_account_count),
            pool_attempt_terminal_reason: pool_account_for_task
                .as_ref()
                .and_then(|_| pending_pool_attempt_terminal_reason.as_deref()),
        });

        let record = ProxyCaptureRecord {
            invoke_id: invoke_id_for_task,
            occurred_at: occurred_at_for_task,
            model: response_info.model,
            usage: response_info.usage,
            cost,
            cost_estimated,
            price_version,
            status,
            error_message,
            failure_kind: failure_kind.map(|kind| kind.to_string()),
            payload: Some(payload),
            raw_response: raw_response_preview,
            req_raw: req_raw_for_task,
            resp_raw,
            timings: StageTimings {
                t_total_ms: 0.0,
                t_req_read_ms,
                t_req_parse_ms,
                t_upstream_connect_ms,
                t_upstream_ttfb_ms,
                t_upstream_stream_ms,
                t_resp_parse_ms,
                t_persist_ms: 0.0,
            },
        };

        let terminal_invocation_persisted = if let Err(err) =
            persist_and_broadcast_proxy_capture(state_for_task.as_ref(), capture_started, record)
                .await
        {
            warn!(proxy_request_id, error = %err, "failed to persist proxy capture record");
            false
        } else {
            true
        };
        if terminal_invocation_persisted {
            disarm_pool_invocation_cleanup_guard(&mut stream_invocation_cleanup_guard);
        }
        if terminal_invocation_persisted
            && deferred_pool_early_phase_cleanup_guard_for_task.is_some()
        {
            if !final_attempt_persisted {
                info!(
                    proxy_request_id,
                    "terminal invocation persisted before final pool attempt state; suppressing deferred orphan recovery to avoid rewriting a completed request as interrupted"
                );
            }
            finalize_deferred_pool_early_phase_cleanup_guard_after_terminal_invocation(
                &mut deferred_pool_early_phase_cleanup_guard_for_task,
                terminal_invocation_persisted,
            );
        }
    });
    disarm_pool_invocation_cleanup_guard(&mut pool_invocation_cleanup_guard);

    response_builder
        .body(Body::from_stream(ReceiverStream::new(rx)))
        .map_err(|err| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("failed to build proxy response: {err}"),
            )
        })
}

async fn read_request_body_with_limit(
    body: Body,
    body_limit: usize,
    request_read_timeout: Duration,
    proxy_request_id: u64,
) -> Result<Vec<u8>, RequestBodyReadError> {
    let mut data = Vec::new();
    let mut stream = body.into_data_stream();
    let read_deadline = Instant::now() + request_read_timeout;

    loop {
        let remaining = read_deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            warn!(
                proxy_request_id,
                timeout_ms = request_read_timeout.as_millis(),
                read_bytes = data.len(),
                "openai proxy request body read timed out"
            );
            return Err(RequestBodyReadError {
                status: StatusCode::REQUEST_TIMEOUT,
                message: format!(
                    "request body read timed out after {}ms",
                    request_read_timeout.as_millis()
                ),
                failure_kind: PROXY_FAILURE_REQUEST_BODY_READ_TIMEOUT,
                partial_body: data,
            });
        }

        let next_chunk = match timeout(remaining, stream.next()).await {
            Ok(chunk) => chunk,
            Err(_) => {
                warn!(
                    proxy_request_id,
                    timeout_ms = request_read_timeout.as_millis(),
                    read_bytes = data.len(),
                    "openai proxy request body read timed out"
                );
                return Err(RequestBodyReadError {
                    status: StatusCode::REQUEST_TIMEOUT,
                    message: format!(
                        "request body read timed out after {}ms",
                        request_read_timeout.as_millis()
                    ),
                    failure_kind: PROXY_FAILURE_REQUEST_BODY_READ_TIMEOUT,
                    partial_body: data,
                });
            }
        };

        let Some(chunk) = next_chunk else {
            break;
        };
        let chunk = match chunk {
            Ok(chunk) => chunk,
            Err(err) => {
                warn!(
                    proxy_request_id,
                    error = %err,
                    read_bytes = data.len(),
                    "openai proxy request body stream error"
                );
                return Err(RequestBodyReadError {
                    status: StatusCode::BAD_REQUEST,
                    message: format!("failed to read request body stream: {err}"),
                    failure_kind: PROXY_FAILURE_REQUEST_BODY_STREAM_ERROR_CLIENT_CLOSED,
                    partial_body: data,
                });
            }
        };

        if data.len().saturating_add(chunk.len()) > body_limit {
            let allowed = body_limit.saturating_sub(data.len());
            if allowed > 0 {
                data.extend_from_slice(&chunk[..allowed.min(chunk.len())]);
            }
            return Err(RequestBodyReadError {
                status: StatusCode::PAYLOAD_TOO_LARGE,
                message: format!("request body exceeds {body_limit} bytes"),
                failure_kind: PROXY_FAILURE_BODY_TOO_LARGE,
                partial_body: data,
            });
        }

        data.extend_from_slice(&chunk);
    }

    Ok(data)
}

async fn read_request_body_snapshot_with_limit(
    body: Body,
    body_limit: usize,
    request_read_timeout: Duration,
    proxy_request_id: u64,
) -> Result<PoolReplayBodySnapshot, RequestBodyReadError> {
    let mut buffer = PoolReplayBodyBuffer::new(proxy_request_id);
    let mut stream = body.into_data_stream();
    let read_deadline = Instant::now() + request_read_timeout;
    let mut data_len = 0usize;

    loop {
        let remaining = read_deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            warn!(
                proxy_request_id,
                timeout_ms = request_read_timeout.as_millis(),
                read_bytes = data_len,
                "openai proxy request body read timed out"
            );
            return Err(RequestBodyReadError {
                status: StatusCode::REQUEST_TIMEOUT,
                message: format!(
                    "request body read timed out after {}ms",
                    request_read_timeout.as_millis()
                ),
                failure_kind: PROXY_FAILURE_REQUEST_BODY_READ_TIMEOUT,
                partial_body: Vec::new(),
            });
        }

        let next_chunk = match timeout(remaining, stream.next()).await {
            Ok(chunk) => chunk,
            Err(_) => {
                warn!(
                    proxy_request_id,
                    timeout_ms = request_read_timeout.as_millis(),
                    read_bytes = data_len,
                    "openai proxy request body read timed out"
                );
                return Err(RequestBodyReadError {
                    status: StatusCode::REQUEST_TIMEOUT,
                    message: format!(
                        "request body read timed out after {}ms",
                        request_read_timeout.as_millis()
                    ),
                    failure_kind: PROXY_FAILURE_REQUEST_BODY_READ_TIMEOUT,
                    partial_body: Vec::new(),
                });
            }
        };

        let Some(chunk) = next_chunk else {
            return buffer.finish().await.map_err(|err| RequestBodyReadError {
                status: StatusCode::BAD_GATEWAY,
                message: format!("failed to cache request body for oauth replay: {err}"),
                failure_kind: PROXY_FAILURE_FAILED_CONTACT_UPSTREAM,
                partial_body: Vec::new(),
            });
        };

        let chunk = match chunk {
            Ok(chunk) => chunk,
            Err(err) => {
                warn!(
                    proxy_request_id,
                    error = %err,
                    read_bytes = data_len,
                    "openai proxy request body stream error"
                );
                return Err(RequestBodyReadError {
                    status: StatusCode::BAD_REQUEST,
                    message: format!("failed to read request body stream: {err}"),
                    failure_kind: PROXY_FAILURE_REQUEST_BODY_STREAM_ERROR_CLIENT_CLOSED,
                    partial_body: Vec::new(),
                });
            }
        };

        if data_len.saturating_add(chunk.len()) > body_limit {
            return Err(RequestBodyReadError {
                status: StatusCode::PAYLOAD_TOO_LARGE,
                message: format!("request body exceeds {body_limit} bytes"),
                failure_kind: PROXY_FAILURE_BODY_TOO_LARGE,
                partial_body: Vec::new(),
            });
        }
        data_len = data_len.saturating_add(chunk.len());

        buffer
            .append(&chunk)
            .await
            .map_err(|err| RequestBodyReadError {
                status: StatusCode::BAD_GATEWAY,
                message: format!("failed to cache request body for oauth replay: {err}"),
                failure_kind: PROXY_FAILURE_FAILED_CONTACT_UPSTREAM,
                partial_body: Vec::new(),
            })?;
    }
}
