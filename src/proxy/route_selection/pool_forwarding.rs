use super::*;

pub(crate) fn proxy_openai_v1_via_pool(
    state: Arc<AppState>,
    proxy_request_id: u64,
    original_uri: &Uri,
    method: Method,
    headers: HeaderMap,
    body: Body,
    runtime_timeouts: PoolRoutingTimeoutSettingsResolved,
    mut proxy_request_permit: Option<ProxyRequestConcurrencyPermit>,
) -> ViaPoolResponseFuture<'_> {
    Box::pin(async move {
        let request_started_at = Instant::now();
        if prompt_cache_key_headers_are_oversized(&headers) {
            return Err(plain_proxy_error(
                StatusCode::PAYLOAD_TOO_LARGE,
                "prompt-cache key exceeds the supported 512-byte limit",
            ));
        }
        let header_prompt_cache_key = extract_prompt_cache_key_from_headers(&headers);
        let mut via_pool_invoke_id = allocate_via_pool_invoke_id(
            &state,
            proxy_request_id,
            header_prompt_cache_key.as_deref(),
        )
        .await?;
        let mut runtime_snapshot_cleanup_guard = Some(PoolViaRuntimeSnapshotCleanupGuard::new(
            state.clone(),
            via_pool_invoke_id.clone(),
        ));
        runtime_snapshot_cleanup_guard
            .as_mut()
            .expect("via-pool runtime cleanup guard should exist")
            .adopt_prompt_cache_key(header_prompt_cache_key.as_deref())
            .await;
        let body_limit = state.config.openai_proxy_max_request_body_bytes;
        let pool_routing_reservation_key = build_pool_routing_reservation_key(proxy_request_id);
        let capture_target = capture_target_for_request(original_uri.path(), &method);
        let capability_endpoint = if method == Method::POST {
            original_uri.path()
        } else {
            ""
        };
        let codex_imagegen_request = codex_imagegen_protocol_from_headers(&headers).is_some();
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
        let pre_attempt_total_timeout_exceeded = || {
            pre_attempt_total_timeout_deadline.is_some_and(|deadline| Instant::now() >= deadline)
        };
        let pre_attempt_total_timeout_error = || {
            plain_proxy_error(
                StatusCode::GATEWAY_TIMEOUT,
                responses_total_timeout
                    .map(pool_total_timeout_exhausted_message)
                    .expect("pre-attempt total-timeout requires responses timeout"),
            )
        };
        let mut request_image_intent = infer_request_image_intent(capture_target, None);
        let mut request_compaction_kind = infer_request_compaction_kind(capture_target, None);
        let header_sticky_key = extract_sticky_key_from_headers(&headers);
        let proxy_request_permit = take_or_acquire_proxy_request_concurrency_permit(
            &mut proxy_request_permit,
            state.as_ref(),
            proxy_request_id,
            &method,
            original_uri,
        )
        .await;
        let body_size_hint_exact = body
            .size_hint()
            .exact()
            .and_then(|value| usize::try_from(value).ok());
        let (upstream, sticky_key, prompt_cache_key, request_contains_encrypted_content) =
            if request_may_have_body(&method, &headers) {
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
                    .map_err(|err| plain_proxy_error(err.status, err.message))?;
                    if pre_attempt_total_timeout_exceeded() {
                        return Err(pre_attempt_total_timeout_error());
                    }
                    let request_body_bytes = Bytes::from(request_body_bytes);
                    let parsed_request_body =
                        serde_json::from_slice::<Value>(&request_body_bytes).ok();
                    if parsed_request_body
                        .as_ref()
                        .is_some_and(request_body_has_invalid_prompt_cache_key)
                    {
                        return Err(plain_proxy_error(
                            StatusCode::PAYLOAD_TOO_LARGE,
                            "prompt-cache key exceeds the supported 512-byte limit",
                        ));
                    }
                    let requested_model = parsed_request_body
                        .as_ref()
                        .and_then(extract_model_from_payload);
                    request_image_intent =
                        infer_request_image_intent(capture_target, parsed_request_body.as_ref());
                    request_compaction_kind =
                        infer_request_compaction_kind(capture_target, parsed_request_body.as_ref());
                    let body_sticky_key = parsed_request_body
                        .as_ref()
                        .and_then(extract_sticky_key_from_request_body);
                    let body_prompt_cache_key = parsed_request_body
                        .as_ref()
                        .and_then(extract_prompt_cache_key_from_request_body);
                    let request_contains_encrypted_content = parsed_request_body
                        .as_ref()
                        .is_some_and(value_contains_encrypted_content);
                    let effective_prompt_cache_key = body_prompt_cache_key
                        .clone()
                        .or(header_prompt_cache_key.clone());
                    if effective_prompt_cache_key.as_deref() != header_prompt_cache_key.as_deref() {
                        via_pool_invoke_id = allocate_via_pool_invoke_id(
                            &state,
                            proxy_request_id,
                            effective_prompt_cache_key.as_deref(),
                        )
                        .await?;
                        runtime_snapshot_cleanup_guard
                            .as_mut()
                            .expect("via-pool runtime cleanup guard should exist")
                            .set_invoke_id(via_pool_invoke_id.clone());
                        runtime_snapshot_cleanup_guard
                            .as_mut()
                            .expect("via-pool runtime cleanup guard should exist")
                            .adopt_prompt_cache_key(effective_prompt_cache_key.as_deref())
                            .await;
                    }
                    let (
                        prompt_cache_binding_constraint,
                        owner_auto_guard_active,
                        conversation_override,
                    ) = load_via_pool_effective_routing(
                        state.as_ref(),
                        effective_prompt_cache_key.as_deref(),
                        request_contains_encrypted_content,
                    )
                    .await
                    .map_err(|(status, message)| plain_proxy_error(status, message))?;
                    let pool_attempt_trace_context = build_via_pool_attempt_trace_context(
                        &via_pool_invoke_id,
                        original_uri.path(),
                        body_sticky_key.clone(),
                    );
                    let request_body_snapshot = pool_replay_snapshot_from_bytes(
                        proxy_request_id,
                        request_body_bytes.clone(),
                    )
                    .await
                    .map_err(|err| {
                        plain_proxy_error(
                            StatusCode::BAD_GATEWAY,
                            format!("failed to persist pool replay request body: {err}"),
                        )
                    })?;
                    (
                        send_pool_request_with_failover_and_binding_constraint(
                            state.clone(),
                            proxy_request_id,
                            method,
                            original_uri,
                            &headers,
                            Some(request_body_snapshot),
                            handshake_timeout,
                            Some(pool_attempt_trace_context),
                            Some(PoolAttemptRuntimeSnapshotContext {
                                capture_target: capture_target
                                    .unwrap_or(ProxyCaptureTarget::Responses),
                                request_info: RequestCaptureInfo {
                                    model: requested_model,
                                    contains_encrypted_content: request_contains_encrypted_content,
                                    image_intent: Some(request_image_intent.as_str().to_string()),
                                    compaction_request_kind: request_compaction_kind,
                                    ..RequestCaptureInfo::default()
                                },
                                hosted_image_intent: None,
                                prompt_cache_key: effective_prompt_cache_key.clone(),
                                owner_auto_guard_active,
                                t_req_read_ms: 0.0,
                                t_req_parse_ms: 0.0,
                            }),
                            body_sticky_key.as_deref(),
                            body_sticky_key.as_deref(),
                            prompt_cache_binding_constraint,
                            conversation_override,
                            None,
                            PoolFailoverProgress {
                                responses_total_timeout_started_at:
                                    responses_total_timeout_started_at_from_request,
                                ..PoolFailoverProgress::default()
                            },
                            POOL_UPSTREAM_SAME_ACCOUNT_MAX_ATTEMPTS,
                            capture_target.is_some(),
                        )
                        .await
                        .map_err(|err| proxy_error_response_from_pool_upstream_error(err, None))?,
                        body_sticky_key,
                        effective_prompt_cache_key,
                        request_contains_encrypted_content,
                    )
                } else {
                    let (
                        request_body_snapshot,
                        body_sticky_key,
                        initial_account,
                        no_available_wait_deadline,
                        prompt_cache_binding_constraint,
                        body_prompt_cache_key,
                        conversation_override,
                        request_body_model,
                        request_contains_encrypted_content,
                        owner_auto_guard_active,
                    ) = if let Some(sticky_key) = header_sticky_key.clone() {
                        let initial_header_sticky_resolution =
                            resolve_pool_account_for_request_with_image_intent_and_codex_imagegen_request(
                                state.as_ref(),
                                Some(sticky_key.as_str()),
                                None,
                                &[],
                                &HashSet::new(),
                                capability_endpoint,
                                request_image_intent,
                                codex_imagegen_request,
                            )
                            .await;
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
                                let now = Instant::now();
                                if pre_attempt_total_timeout_deadline
                                    .is_some_and(|deadline| now >= deadline)
                                {
                                    break (
                                        Ok(PoolAccountResolutionWithWait::TotalTimeoutExpired),
                                        no_available_wait_deadline,
                                    );
                                }
                                let resolution =
                                    resolve_pool_account_for_request_with_image_intent_and_codex_imagegen_request(
                                        state_for_wait.as_ref(),
                                        Some(wait_task_sticky_key.as_str()),
                                        None,
                                        &excluded_ids,
                                        &excluded_upstream_route_keys,
                                        capability_endpoint,
                                        request_image_intent,
                                        codex_imagegen_request,
                                    )
                                    .await;
                                if matches!(
                                    resolution,
                                    Ok(PoolAccountResolution::Unavailable
                                        | PoolAccountResolution::NoCandidate(_))
                                ) && no_available_wait_deadline.is_none()
                                {
                                    let deadline = Instant::now()
                                        + state_for_wait.pool_no_available_wait.timeout;
                                    no_available_wait_deadline = Some(deadline);
                                    *shared_wait_deadline_for_task
                                        .lock()
                                        .expect("lock shared header wait deadline") =
                                        Some(deadline);
                                }
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
                                        | PoolAccountResolution::NoCandidate(_)),
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
                                                Ok(PoolAccountResolutionWithWait::Resolution(
                                                    resolution,
                                                )),
                                                no_available_wait_deadline,
                                            );
                                        }
                                        notify_pool_no_available_wait_hook(state_for_wait.as_ref());
                                        crate::observability::diagnostics::wait(
                                            crate::observability::diagnostics::Resource::AccountCapacity,
                                            tokio::time::sleep(poll_interval.min(
                                            effective_deadline.saturating_duration_since(now),
                                        )))
                                        .await;
                                    }
                                    Ok(resolution) => {
                                        break (
                                            Ok(PoolAccountResolutionWithWait::Resolution(
                                                resolution,
                                            )),
                                            no_available_wait_deadline,
                                        );
                                    }
                                    Err(err) => break (Err(err), no_available_wait_deadline),
                                }
                            }
                        };
                        tokio::pin!(header_sticky_resolution);
                        let mut header_sticky_resolution_finished = false;
                        let mut pending_header_sticky_terminal_error: Option<
                            ViaPoolResolutionTerminalError,
                        > = None;
                        let mut header_sticky_wait_deadline = None;
                        match initial_header_sticky_resolution {
                            Ok(PoolAccountResolution::Resolved(_)) => {
                                header_sticky_resolution_finished = true;
                            }
                            Ok(PoolAccountResolution::RateLimited) => {
                                pending_header_sticky_terminal_error =
                                    Some(ViaPoolResolutionTerminalError::plain(
                                        StatusCode::TOO_MANY_REQUESTS,
                                        POOL_ALL_ACCOUNTS_RATE_LIMITED_MESSAGE.to_string(),
                                    ));
                                header_sticky_resolution_finished = true;
                            }
                            Ok(PoolAccountResolution::DegradedOnly) => {
                                pending_header_sticky_terminal_error =
                                    Some(ViaPoolResolutionTerminalError::plain(
                                        StatusCode::SERVICE_UNAVAILABLE,
                                        POOL_ALL_ACCOUNTS_DEGRADED_MESSAGE.to_string(),
                                    ));
                                header_sticky_resolution_finished = true;
                            }
                            Ok(PoolAccountResolution::BlockedByPolicy(message)) => {
                                pending_header_sticky_terminal_error =
                                    Some(ViaPoolResolutionTerminalError::plain(
                                        StatusCode::SERVICE_UNAVAILABLE,
                                        message,
                                    ));
                                header_sticky_resolution_finished = true;
                            }
                            Ok(PoolAccountResolution::AssignedBlocked(blocked)) => {
                                pending_header_sticky_terminal_error =
                                    Some(ViaPoolResolutionTerminalError::assigned_blocked(blocked));
                                header_sticky_resolution_finished = true;
                            }
                            Err(err) => {
                                pending_header_sticky_terminal_error =
                                    Some(ViaPoolResolutionTerminalError::plain(
                                        StatusCode::BAD_GATEWAY,
                                        format!("failed to resolve pool account: {err}"),
                                    ));
                                header_sticky_resolution_finished = true;
                            }
                            Ok(
                                PoolAccountResolution::Unavailable
                                | PoolAccountResolution::NoCandidate(_),
                            ) => {}
                        }
                        let mut request_body_buffer = PoolReplayBodyBuffer::new(proxy_request_id);
                        let mut request_body_stream = body.into_data_stream();
                        let request_body_deadline =
                            Instant::now() + runtime_timeouts.request_read_timeout;
                        let mut request_body_len = 0usize;
                        let mut observed_body_sticky_key = None;
                        let mut observed_body_prompt_cache_key = None;
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
                            let now = Instant::now();
                            let remaining = request_body_deadline.saturating_duration_since(now);
                            if remaining.is_zero() {
                                let err = request_body_timeout_error();
                                return Err(plain_proxy_error(err.status, err.message));
                            }
                            let request_body_wait_budget = if let Some(total_timeout_deadline) =
                                pre_attempt_total_timeout_deadline
                            {
                                let total_timeout_remaining =
                                    total_timeout_deadline.saturating_duration_since(now);
                                if total_timeout_remaining.is_zero() {
                                    let total_timeout = responses_total_timeout.expect(
                                    "pre-attempt total-timeout expiry requires responses timeout",
                                );
                                    return Err(plain_proxy_error(
                                        StatusCode::GATEWAY_TIMEOUT,
                                        pool_total_timeout_exhausted_message(total_timeout),
                                    ));
                                }
                                remaining.min(total_timeout_remaining)
                            } else {
                                remaining
                            };
                            tokio::select! {
                                resolution_result = &mut header_sticky_resolution, if !header_sticky_resolution_finished => {
                                    header_sticky_resolution_finished = true;
                                    let (resolution, no_available_wait_deadline) = resolution_result;
                                    header_sticky_wait_deadline = no_available_wait_deadline;
                                    match resolution {
                                        Ok(PoolAccountResolutionWithWait::Resolution(
                                            PoolAccountResolution::Resolved(_),
                                        )) => {}
                                        Ok(PoolAccountResolutionWithWait::TotalTimeoutExpired) => {
                                            let total_timeout =
                                                responses_total_timeout.expect(
                                                    "pre-attempt total-timeout expiry requires responses timeout",
                                                );
                                            return Err(plain_proxy_error(
                                                StatusCode::GATEWAY_TIMEOUT,
                                                pool_total_timeout_exhausted_message(total_timeout),
                                            ));
                                        }
                                        Ok(PoolAccountResolutionWithWait::Resolution(
                                            PoolAccountResolution::RateLimited,
                                        )) => {
                                            pending_header_sticky_terminal_error = Some(
                                                ViaPoolResolutionTerminalError::plain(
                                                    StatusCode::TOO_MANY_REQUESTS,
                                                    POOL_ALL_ACCOUNTS_RATE_LIMITED_MESSAGE.to_string(),
                                                ),
                                            );
                                        }
                                        Ok(PoolAccountResolutionWithWait::Resolution(
                                            PoolAccountResolution::DegradedOnly,
                                        )) => {
                                            pending_header_sticky_terminal_error = Some(
                                                ViaPoolResolutionTerminalError::plain(
                                                    StatusCode::SERVICE_UNAVAILABLE,
                                                    POOL_ALL_ACCOUNTS_DEGRADED_MESSAGE.to_string(),
                                                ),
                                            );
                                        }
                                        Ok(PoolAccountResolutionWithWait::Resolution(
                                            PoolAccountResolution::AssignedBlocked(blocked),
                                        )) => {
                                            pending_header_sticky_terminal_error = Some(
                                                ViaPoolResolutionTerminalError::assigned_blocked(blocked),
                                            );
                                        }
                                        Ok(PoolAccountResolutionWithWait::Resolution(
                                            PoolAccountResolution::BlockedByPolicy(message),
                                        )) => {
                                            pending_header_sticky_terminal_error = Some(
                                                ViaPoolResolutionTerminalError::plain(
                                                    StatusCode::SERVICE_UNAVAILABLE,
                                                    message,
                                                ),
                                            );
                                        }
                                        Err(err) => {
                                            pending_header_sticky_terminal_error = Some(
                                                ViaPoolResolutionTerminalError::plain(
                                                    StatusCode::BAD_GATEWAY,
                                                    format!("failed to resolve pool account: {err}"),
                                                ),
                                            );
                                        }
                                        Ok(PoolAccountResolutionWithWait::Resolution(
                                            PoolAccountResolution::Unavailable,
                                        ))
                                        | Ok(PoolAccountResolutionWithWait::Resolution(
                                            PoolAccountResolution::NoCandidate(_),
                                        )) => {}
                                    }
                                    if observed_body_sticky_key.as_deref() == Some(sticky_key.as_str())
                                        && let Some(terminal_error) =
                                            pending_header_sticky_terminal_error.clone()
                                    {
                                        let (prompt_cache_binding_constraint, _, conversation_override) =
                                            load_via_pool_effective_routing(
                                                state.as_ref(),
                                                observed_body_prompt_cache_key
                                                    .as_deref()
                                                    .or(header_prompt_cache_key.as_deref()),
                                                false,
                                            )
                                            .await
                                            .map_err(|(status, message)| {
                                                plain_proxy_error(status, message)
                                            })?;
                                        if prompt_cache_binding_constraint.is_none()
                                            && conversation_override.is_none()
                                        {
                                            let trace_context = build_via_pool_attempt_trace_context(
                                                &via_pool_invoke_id,
                                                original_uri.path(),
                                                Some(sticky_key.clone()),
                                            );
                                            terminal_error
                                                .persist_if_needed(state.as_ref(), Some(&trace_context))
                                                .await;
                                            return Err(plain_proxy_error(
                                                terminal_error.status,
                                                terminal_error.message,
                                            ));
                                        }
                                    }
                                }
                                next_chunk = timeout(request_body_wait_budget, request_body_stream.next()) => {
                                    let next_chunk = match next_chunk {
                                        Ok(chunk) => chunk,
                                        Err(_) => {
                                            if pre_attempt_total_timeout_deadline
                                                .is_some_and(|deadline| Instant::now() >= deadline)
                                            {
                                                let total_timeout = responses_total_timeout.expect(
                                                    "pre-attempt total-timeout expiry requires responses timeout",
                                                );
                                                return Err(plain_proxy_error(
                                                    StatusCode::GATEWAY_TIMEOUT,
                                                    pool_total_timeout_exhausted_message(total_timeout),
                                                ));
                                            }
                                            let err = request_body_timeout_error();
                                            return Err(plain_proxy_error(err.status, err.message));
                                        }
                                    };
                                    let Some(chunk) = next_chunk else {
                                        break request_body_buffer.finish().await.map_err(|err| {
                                            plain_proxy_error(
                                                StatusCode::BAD_GATEWAY,
                                                format!("failed to cache request body for oauth replay: {err}"),
                                            )
                                        })?;
                                    };
                                    let chunk = match chunk {
                                        Ok(chunk) => chunk,
                                        Err(err) => {
                                            return Err(plain_proxy_error(
                                                StatusCode::BAD_REQUEST,
                                                format!("failed to read request body stream: {err}"),
                                            ));
                                        }
                                    };
                                    if request_body_len.saturating_add(chunk.len()) > body_limit {
                                        return Err(plain_proxy_error(
                                            StatusCode::PAYLOAD_TOO_LARGE,
                                            format!("request body exceeds {body_limit} bytes"),
                                        ));
                                    }
                                    request_body_len = request_body_len.saturating_add(chunk.len());
                                    request_body_buffer.append(&chunk).await.map_err(|err| {
                                        plain_proxy_error(
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
                                        observed_body_prompt_cache_key =
                                            best_effort_extract_prompt_cache_key_from_request_body_prefix(
                                                &sticky_key_probe,
                                            );
                                        sticky_key_probe_exhausted =
                                            observed_body_sticky_key.is_some()
                                                || sticky_key_probe.len()
                                                    >= HEADER_STICKY_EARLY_STICKY_SCAN_BYTES;
                                    }
                                    if header_sticky_resolution_finished
                                        && observed_body_sticky_key.as_deref() == Some(sticky_key.as_str())
                                        && let Some(terminal_error) =
                                            pending_header_sticky_terminal_error.clone()
                                    {
                                        let (prompt_cache_binding_constraint, _, conversation_override) =
                                            load_via_pool_effective_routing(
                                                state.as_ref(),
                                                observed_body_prompt_cache_key
                                                    .as_deref()
                                                    .or(header_prompt_cache_key.as_deref()),
                                                false,
                                            )
                                            .await
                                            .map_err(|(status, message)| {
                                                plain_proxy_error(status, message)
                                            })?;
                                        if prompt_cache_binding_constraint.is_none()
                                            && conversation_override.is_none()
                                        {
                                            let trace_context = build_via_pool_attempt_trace_context(
                                                &via_pool_invoke_id,
                                                original_uri.path(),
                                                Some(sticky_key.clone()),
                                            );
                                            terminal_error
                                                .persist_if_needed(state.as_ref(), Some(&trace_context))
                                                .await;
                                            return Err(plain_proxy_error(
                                                terminal_error.status,
                                                terminal_error.message,
                                            ));
                                        }
                                    }
                                }
                            }
                        };
                        let request_analysis = analyze_replay_snapshot_for_pool_routing(
                            &request_body_snapshot,
                            capture_target,
                            proxy_request_id,
                            "initial_via_pool",
                        )
                        .await;
                        if request_analysis.invalid_prompt_cache_key {
                            return Err(plain_proxy_error(
                                StatusCode::PAYLOAD_TOO_LARGE,
                                "prompt-cache key exceeds the supported 512-byte limit",
                            ));
                        }
                        let body_sticky_key = observed_body_sticky_key
                            .or(request_analysis.sticky_key)
                            .or(Some(sticky_key.clone()));
                        if !header_sticky_resolution_finished
                            && body_sticky_key.as_deref() == Some(sticky_key.as_str())
                        {
                            let (resolution, no_available_wait_deadline) =
                                header_sticky_resolution.await;
                            header_sticky_wait_deadline = no_available_wait_deadline;
                            match resolution {
                                Ok(PoolAccountResolutionWithWait::Resolution(
                                    PoolAccountResolution::Resolved(_),
                                )) => {}
                                Ok(PoolAccountResolutionWithWait::TotalTimeoutExpired) => {
                                    let total_timeout = responses_total_timeout.expect(
                                    "pre-attempt total-timeout expiry requires responses timeout",
                                );
                                    return Err(plain_proxy_error(
                                        StatusCode::GATEWAY_TIMEOUT,
                                        pool_total_timeout_exhausted_message(total_timeout),
                                    ));
                                }
                                Ok(PoolAccountResolutionWithWait::Resolution(
                                    PoolAccountResolution::RateLimited,
                                )) => {
                                    pending_header_sticky_terminal_error =
                                        Some(ViaPoolResolutionTerminalError::plain(
                                            StatusCode::TOO_MANY_REQUESTS,
                                            POOL_ALL_ACCOUNTS_RATE_LIMITED_MESSAGE.to_string(),
                                        ));
                                }
                                Ok(PoolAccountResolutionWithWait::Resolution(
                                    PoolAccountResolution::DegradedOnly,
                                )) => {
                                    pending_header_sticky_terminal_error =
                                        Some(ViaPoolResolutionTerminalError::plain(
                                            StatusCode::SERVICE_UNAVAILABLE,
                                            POOL_ALL_ACCOUNTS_DEGRADED_MESSAGE.to_string(),
                                        ));
                                }
                                Ok(PoolAccountResolutionWithWait::Resolution(
                                    PoolAccountResolution::AssignedBlocked(blocked),
                                )) => {
                                    pending_header_sticky_terminal_error = Some(
                                        ViaPoolResolutionTerminalError::assigned_blocked(blocked),
                                    );
                                }
                                Ok(PoolAccountResolutionWithWait::Resolution(
                                    PoolAccountResolution::BlockedByPolicy(message),
                                )) => {
                                    pending_header_sticky_terminal_error =
                                        Some(ViaPoolResolutionTerminalError::plain(
                                            StatusCode::SERVICE_UNAVAILABLE,
                                            message,
                                        ));
                                }
                                Err(err) => {
                                    pending_header_sticky_terminal_error =
                                        Some(ViaPoolResolutionTerminalError::plain(
                                            StatusCode::BAD_GATEWAY,
                                            format!("failed to resolve pool account: {err}"),
                                        ));
                                }
                                Ok(PoolAccountResolutionWithWait::Resolution(
                                    PoolAccountResolution::Unavailable,
                                ))
                                | Ok(PoolAccountResolutionWithWait::Resolution(
                                    PoolAccountResolution::NoCandidate(_),
                                )) => {}
                            }
                        }
                        let body_prompt_cache_key = request_analysis.prompt_cache_key;
                        let effective_prompt_cache_key = body_prompt_cache_key
                            .as_deref()
                            .or(header_prompt_cache_key.as_deref());
                        if effective_prompt_cache_key != header_prompt_cache_key.as_deref() {
                            via_pool_invoke_id = allocate_via_pool_invoke_id(
                                &state,
                                proxy_request_id,
                                effective_prompt_cache_key,
                            )
                            .await?;
                            runtime_snapshot_cleanup_guard
                                .as_mut()
                                .expect("via-pool runtime cleanup guard should exist")
                                .set_invoke_id(via_pool_invoke_id.clone());
                            runtime_snapshot_cleanup_guard
                                .as_mut()
                                .expect("via-pool runtime cleanup guard should exist")
                                .adopt_prompt_cache_key(effective_prompt_cache_key)
                                .await;
                        }
                        let request_contains_encrypted_content =
                            request_analysis.contains_encrypted_content;
                        let request_image_intent = request_analysis.image_intent;
                        let _request_compaction_kind = request_analysis.compaction_kind;
                        let (
                            prompt_cache_binding_constraint,
                            owner_auto_guard_active,
                            conversation_override,
                        ) = load_via_pool_effective_routing(
                            state.as_ref(),
                            body_prompt_cache_key
                                .as_deref()
                                .or(header_prompt_cache_key.as_deref()),
                            request_contains_encrypted_content,
                        )
                        .await
                        .map_err(|(status, message)| plain_proxy_error(status, message))?;
                        let requested_model = request_analysis.requested_model;
                        if prompt_cache_binding_constraint.is_none()
                            && conversation_override.is_none()
                            && body_sticky_key.as_deref() == Some(sticky_key.as_str())
                            && let Some(terminal_error) = pending_header_sticky_terminal_error
                        {
                            let trace_context = build_via_pool_attempt_trace_context(
                                &via_pool_invoke_id,
                                original_uri.path(),
                                Some(sticky_key.clone()),
                            );
                            terminal_error
                                .persist_if_needed(state.as_ref(), Some(&trace_context))
                                .await;
                            return Err(plain_proxy_error(
                                terminal_error.status,
                                terminal_error.message,
                            ));
                        }
                        let mut no_available_wait_deadline =
                            header_sticky_wait_deadline.or(*shared_wait_deadline
                                .lock()
                                .expect("lock shared header wait deadline"));
                        let initial_account = if prompt_cache_binding_constraint.is_some()
                            || conversation_override.is_some()
                        {
                            let resolution = resolve_pool_account_for_request_with_wait_and_binding_constraint_with_image_intent_and_override_and_codex_imagegen_request_and_reservation(
                        state.as_ref(),
                        body_sticky_key.as_deref(),
                        requested_model.as_deref(),
                        &[],
                        &HashSet::new(),
                        None,
	                        prompt_cache_binding_constraint.as_ref(),
	                        conversation_override.as_ref(),
	                        true,
	                        &mut no_available_wait_deadline,
	                        pre_attempt_total_timeout_deadline,
	                        capability_endpoint,
	                        request_image_intent,
	                        codex_imagegen_request,
                            Some(&pool_routing_reservation_key),
	                    )
                    .await;
                            let (initial_account, updated_no_available_wait_deadline) =
                                unwrap_via_pool_initial_account(
                                    state.as_ref(),
                                    Some(&build_via_pool_attempt_trace_context(
                                        &via_pool_invoke_id,
                                        original_uri.path(),
                                        body_sticky_key.clone(),
                                    )),
                                    resolution,
                                    no_available_wait_deadline,
                                    responses_total_timeout,
                                    prompt_cache_binding_constraint.as_ref(),
                                    owner_auto_guard_active,
                                    body_sticky_key.as_deref(),
                                )
                                .await?;
                            no_available_wait_deadline = updated_no_available_wait_deadline;
                            initial_account
                        } else if body_sticky_key.as_deref() == Some(sticky_key.as_str()) {
                            // Header inspection happens before the body model is known. Resolve
                            // again once it is available so this fast path takes the same atomic
                            // account/model reservation as all other pool routes.
                            let resolution =
                                resolve_pool_account_for_request_with_wait_and_image_intent_and_codex_imagegen_request_and_reservation(
                                    state.as_ref(),
                                    body_sticky_key.as_deref(),
                                    requested_model.as_deref(),
                                    &[],
                                    &HashSet::new(),
                                    None,
                                    true,
                                    &mut no_available_wait_deadline,
                                    pre_attempt_total_timeout_deadline,
                                    capability_endpoint,
                                    request_image_intent,
                                    codex_imagegen_request,
                                    Some(&pool_routing_reservation_key),
                                )
                                .await;
                            let (initial_account, updated_no_available_wait_deadline) =
                                unwrap_via_pool_initial_account(
                                    state.as_ref(),
                                    Some(&build_via_pool_attempt_trace_context(
                                        &via_pool_invoke_id,
                                        original_uri.path(),
                                        body_sticky_key.clone(),
                                    )),
                                    resolution,
                                    no_available_wait_deadline,
                                    responses_total_timeout,
                                    prompt_cache_binding_constraint.as_ref(),
                                    owner_auto_guard_active,
                                    body_sticky_key.as_deref(),
                                )
                                .await?;
                            no_available_wait_deadline = updated_no_available_wait_deadline;
                            initial_account
                        } else {
                            let resolution =
                                resolve_pool_account_for_request_with_wait_and_image_intent_and_codex_imagegen_request_and_reservation(
                                    state.as_ref(),
                                    body_sticky_key.as_deref(),
                                    requested_model.as_deref(),
                                    &[],
                                    &HashSet::new(),
                                    None,
                                    true,
                                    &mut no_available_wait_deadline,
                                    pre_attempt_total_timeout_deadline,
                                    capability_endpoint,
                                    request_image_intent,
                                    codex_imagegen_request,
                                    Some(&pool_routing_reservation_key),
                                )
                                .await;
                            let (initial_account, updated_no_available_wait_deadline) =
                                unwrap_via_pool_initial_account(
                                    state.as_ref(),
                                    Some(&build_via_pool_attempt_trace_context(
                                        &via_pool_invoke_id,
                                        original_uri.path(),
                                        body_sticky_key.clone(),
                                    )),
                                    resolution,
                                    no_available_wait_deadline,
                                    responses_total_timeout,
                                    prompt_cache_binding_constraint.as_ref(),
                                    owner_auto_guard_active,
                                    body_sticky_key.as_deref(),
                                )
                                .await?;
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
                            prompt_cache_binding_constraint,
                            body_prompt_cache_key,
                            conversation_override,
                            requested_model,
                            request_contains_encrypted_content,
                            owner_auto_guard_active,
                        )
                    } else {
                        let request_body_snapshot = read_request_body_snapshot_with_limit(
                            body,
                            body_limit,
                            runtime_timeouts.request_read_timeout,
                            proxy_request_id,
                        )
                        .await
                        .map_err(|err| plain_proxy_error(err.status, err.message))?;
                        if pre_attempt_total_timeout_exceeded() {
                            return Err(pre_attempt_total_timeout_error());
                        }
                        let request_analysis = analyze_replay_snapshot_for_pool_routing(
                            &request_body_snapshot,
                            capture_target,
                            proxy_request_id,
                            "buffered_body_route",
                        )
                        .await;
                        if request_analysis.invalid_prompt_cache_key {
                            return Err(plain_proxy_error(
                                StatusCode::PAYLOAD_TOO_LARGE,
                                "prompt-cache key exceeds the supported 512-byte limit",
                            ));
                        }
                        let body_sticky_key = request_analysis.sticky_key;
                        let body_prompt_cache_key = request_analysis.prompt_cache_key;
                        let effective_prompt_cache_key = body_prompt_cache_key
                            .as_deref()
                            .or(header_prompt_cache_key.as_deref());
                        if effective_prompt_cache_key != header_prompt_cache_key.as_deref() {
                            via_pool_invoke_id = allocate_via_pool_invoke_id(
                                &state,
                                proxy_request_id,
                                effective_prompt_cache_key,
                            )
                            .await?;
                            runtime_snapshot_cleanup_guard
                                .as_mut()
                                .expect("via-pool runtime cleanup guard should exist")
                                .set_invoke_id(via_pool_invoke_id.clone());
                            runtime_snapshot_cleanup_guard
                                .as_mut()
                                .expect("via-pool runtime cleanup guard should exist")
                                .adopt_prompt_cache_key(effective_prompt_cache_key)
                                .await;
                        }
                        let requested_model = request_analysis.requested_model.clone();
                        let request_contains_encrypted_content =
                            request_analysis.contains_encrypted_content;
                        let request_image_intent = request_analysis.image_intent;
                        let (
                            prompt_cache_binding_constraint,
                            owner_auto_guard_active,
                            conversation_override,
                        ) = load_via_pool_effective_routing(
                            state.as_ref(),
                            body_prompt_cache_key
                                .as_deref()
                                .or(header_prompt_cache_key.as_deref()),
                            request_contains_encrypted_content,
                        )
                        .await
                        .map_err(|(status, message)| plain_proxy_error(status, message))?;
                        let mut no_available_wait_deadline = None;
                        let resolution = resolve_pool_account_for_request_with_wait_and_binding_constraint_with_image_intent_and_override_and_codex_imagegen_request_and_reservation(
                    state.as_ref(),
                    body_sticky_key.as_deref(),
                    requested_model.as_deref(),
                    &[],
                    &HashSet::new(),
                    None,
                    prompt_cache_binding_constraint.as_ref(),
                    conversation_override.as_ref(),
                    true,
                    &mut no_available_wait_deadline,
                    pre_attempt_total_timeout_deadline,
                        capability_endpoint,
                        request_image_intent,
                        codex_imagegen_request,
                        Some(&pool_routing_reservation_key),
                )
                .await;
                        let (initial_account, no_available_wait_deadline) =
                            unwrap_via_pool_initial_account(
                                state.as_ref(),
                                Some(&build_via_pool_attempt_trace_context(
                                    &via_pool_invoke_id,
                                    original_uri.path(),
                                    body_sticky_key.clone(),
                                )),
                                resolution,
                                no_available_wait_deadline,
                                responses_total_timeout,
                                prompt_cache_binding_constraint.as_ref(),
                                owner_auto_guard_active,
                                body_prompt_cache_key
                                    .as_deref()
                                    .or(header_prompt_cache_key.as_deref()),
                            )
                            .await?;
                        let request_body_model = request_analysis.requested_model;
                        let effective_prompt_cache_key = body_prompt_cache_key
                            .clone()
                            .or(header_prompt_cache_key.clone());
                        (
                            request_body_snapshot,
                            body_sticky_key,
                            initial_account,
                            no_available_wait_deadline,
                            prompt_cache_binding_constraint,
                            effective_prompt_cache_key,
                            conversation_override,
                            request_body_model,
                            request_contains_encrypted_content,
                            owner_auto_guard_active,
                        )
                    };
                    (
                        send_pool_request_with_failover_and_binding_constraint(
                            state.clone(),
                            proxy_request_id,
                            method,
                            original_uri,
                            &headers,
                            Some(request_body_snapshot),
                            handshake_timeout,
                            Some(build_via_pool_attempt_trace_context(
                                &via_pool_invoke_id,
                                original_uri.path(),
                                body_sticky_key.clone(),
                            )),
                            Some(PoolAttemptRuntimeSnapshotContext {
                                capture_target: capture_target
                                    .unwrap_or(ProxyCaptureTarget::Responses),
                                request_info: RequestCaptureInfo {
                                    model: request_body_model,
                                    contains_encrypted_content: request_contains_encrypted_content,
                                    image_intent: Some(request_image_intent.as_str().to_string()),
                                    compaction_request_kind: request_compaction_kind,
                                    ..RequestCaptureInfo::default()
                                },
                                hosted_image_intent: None,
                                prompt_cache_key: body_prompt_cache_key.clone(),
                                owner_auto_guard_active,
                                t_req_read_ms: 0.0,
                                t_req_parse_ms: 0.0,
                            }),
                            body_sticky_key.as_deref(),
                            body_sticky_key.as_deref(),
                            prompt_cache_binding_constraint,
                            conversation_override,
                            Some(initial_account),
                            PoolFailoverProgress {
                                responses_total_timeout_started_at:
                                    responses_total_timeout_started_at_from_request,
                                no_available_wait_deadline,
                                ..PoolFailoverProgress::default()
                            },
                            POOL_UPSTREAM_SAME_ACCOUNT_MAX_ATTEMPTS,
                            capture_target.is_some(),
                        )
                        .await
                        .map_err(|err| proxy_error_response_from_pool_upstream_error(err, None))?,
                        body_sticky_key,
                        body_prompt_cache_key,
                        request_contains_encrypted_content,
                    )
                }
            } else {
                let same_account_attempts = if method == Method::GET {
                    1
                } else {
                    POOL_UPSTREAM_SAME_ACCOUNT_MAX_ATTEMPTS
                };
                let (
                    prompt_cache_binding_constraint,
                    owner_auto_guard_active,
                    conversation_override,
                ) = load_via_pool_effective_routing(
                    state.as_ref(),
                    header_prompt_cache_key.as_deref(),
                    false,
                )
                .await
                .map_err(|(status, message)| plain_proxy_error(status, message))?;
                let header_capture_target =
                    capture_target_for_request(original_uri.path(), &method)
                        .unwrap_or(ProxyCaptureTarget::Responses);
                (
                    send_pool_request_with_failover_and_binding_constraint(
                        state.clone(),
                        proxy_request_id,
                        method,
                        original_uri,
                        &headers,
                        None,
                        handshake_timeout,
                        Some(build_via_pool_attempt_trace_context(
                            &via_pool_invoke_id,
                            original_uri.path(),
                            header_sticky_key.clone(),
                        )),
                        Some(PoolAttemptRuntimeSnapshotContext {
                            capture_target: header_capture_target,
                            request_info: RequestCaptureInfo::default(),
                            hosted_image_intent: None,
                            prompt_cache_key: header_prompt_cache_key.clone(),
                            owner_auto_guard_active,
                            t_req_read_ms: 0.0,
                            t_req_parse_ms: 0.0,
                        }),
                        header_sticky_key.as_deref(),
                        None,
                        prompt_cache_binding_constraint,
                        conversation_override,
                        None,
                        PoolFailoverProgress {
                            responses_total_timeout_started_at,
                            ..PoolFailoverProgress::default()
                        },
                        same_account_attempts,
                        capture_target.is_some(),
                    )
                    .await
                    .map_err(|err| proxy_error_response_from_pool_upstream_error(err, None))?,
                    header_sticky_key,
                    header_prompt_cache_key.clone(),
                    false,
                )
            };

        let account = upstream.account;
        let upstream_attempt_started_at_utc = upstream.attempt_started_at_utc;
        let codex_imagegen_rewrite = upstream.codex_imagegen_rewrite;
        let pending_pool_attempt_record = upstream.pending_attempt_record;
        let upstream_invoke_id = pending_pool_attempt_record
            .as_ref()
            .map(|record| record.invoke_id.clone());
        let t_upstream_connect_ms = upstream.connect_latency_ms;
        let t_upstream_ttfb_ms = upstream.first_byte_latency_ms;
        let mut deferred_pool_early_phase_cleanup_guard =
            upstream.deferred_early_phase_cleanup_guard;
        let live_pool_attempt_activity_lease = upstream.live_attempt_activity_lease;
        let upstream_response = upstream.response;
        let transport_bytes_live_counted = upstream.transport_bytes_live_counted;
        let rewritten_location = normalize_proxy_location_header(
            upstream_response.status(),
            upstream_response.headers(),
            &account.upstream_base_url,
        )
        .map_err(|err| {
            plain_proxy_error(
                StatusCode::BAD_GATEWAY,
                format!("failed to process upstream redirect: {err}"),
            )
        })?;

        let upstream_status = upstream_response.status();
        let upstream_request_id = upstream_response
            .headers()
            .get("x-request-id")
            .and_then(|value| value.to_str().ok())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| value.to_string());
        let upstream_content_encoding = upstream_response
            .headers()
            .get(header::CONTENT_ENCODING)
            .and_then(|value| value.to_str().ok())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
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
        let stream_timeout = upstream.stream_timeout;
        if let Some(chunk) = first_chunk.as_ref() {
            info!(
                proxy_request_id,
                account_id = account.account_id,
                ttfb_ms = t_upstream_ttfb_ms,
                first_chunk_bytes = chunk.len(),
                "pool upstream response first chunk ready"
            );
        } else {
            let pool_route_success = pool_route_response_status_is_success(upstream_status);
            let route_http_failure_message = (!pool_route_success)
                .then(|| format!("pool upstream responded with {}", upstream_status.as_u16()));
            finalize_tracked_pool_attempt(
                state.as_ref(),
                pending_pool_attempt_record.as_ref(),
                if pool_route_success {
                    POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS
                } else {
                    POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_HTTP_FAILURE
                },
                Some(upstream_status),
                None,
                None,
                None,
                route_http_failure_message.as_deref(),
                Some(t_upstream_connect_ms),
                Some(t_upstream_ttfb_ms),
                Some(0.0),
                upstream_request_id.as_deref(),
                "via-pool failover empty response",
            )
            .await;
            complete_deferred_pool_early_phase_cleanup_guard(
                &mut deferred_pool_early_phase_cleanup_guard,
            );
            if pool_route_success {
                consume_pool_routing_reservation(state.as_ref(), &pool_routing_reservation_key);
                if let Err(route_err) = record_runtime_pool_route_success(
                    state.as_ref(),
                    account.account_id,
                    upstream_attempt_started_at_utc,
                    sticky_key.as_deref(),
                    prompt_cache_key.as_deref(),
                    upstream_invoke_id.as_deref(),
                    capability_endpoint,
                    request_image_intent,
                    codex_imagegen_rewrite.as_ref(),
                    pending_pool_attempt_record
                        .as_ref()
                        .and_then(|pending| pending.attempt_id),
                    pending_pool_attempt_record.as_ref().and_then(|pending| {
                        pending
                            .sticky_affinity_generation
                            .or_else(|| pending.attempt_id.and(account.sticky_affinity_generation))
                    }),
                    pending_pool_attempt_record.as_ref().and_then(|pending| {
                        pending
                            .request_model
                            .as_deref()
                            .or(pending.upstream_request_model.as_deref())
                    }),
                )
                .await
                {
                    warn!(account_id = account.account_id, error = %route_err, "failed to record pool route success");
                }
                if let Some(prompt_cache_key) = prompt_cache_key.as_deref()
                    && request_contains_encrypted_content
                {
                    match confirm_prompt_cache_encrypted_session_owner_success_if_enabled(
                        state.as_ref(),
                        prompt_cache_key,
                        account.account_id,
                    )
                    .await
                    {
                        Ok(true) => {
                            if let Err(err) =
                                promote_prompt_cache_group_binding_to_upstream_account_and_broadcast(
                                    state.as_ref(),
                                    prompt_cache_key,
                                    account.account_id,
                                )
                                .await
                            {
                                warn!(
                                    prompt_cache_key,
                                    account_id = account.account_id,
                                    error = %err,
                                    "failed to promote prompt cache group binding after via-pool encrypted session success"
                                );
                            }
                        }
                        Ok(false) => {}
                        Err(err) => {
                            warn!(
                                prompt_cache_key,
                                account_id = account.account_id,
                                error = %err,
                                "failed to persist via-pool encrypted session owner"
                            );
                        }
                    }
                }
            } else {
                release_pool_routing_reservation(state.as_ref(), &pool_routing_reservation_key);
                if let Err(route_err) = record_runtime_pool_route_http_failure(
                    state.as_ref(),
                    account.account_id,
                    &account.kind,
                    account.single_account_rotation_enabled
                        && account.effective_upstream_429_max_retries() == 0,
                    sticky_key.as_deref(),
                    upstream_status,
                    route_http_failure_message
                        .as_deref()
                        .unwrap_or("upstream request failed"),
                    upstream_invoke_id.as_deref(),
                    capability_endpoint,
                    request_image_intent,
                    pending_pool_attempt_record
                        .as_ref()
                        .and_then(|pending| pending.attempt_id),
                    pending_pool_attempt_record
                        .as_ref()
                        .and_then(|pending| pending.sticky_affinity_generation),
                    prompt_cache_key.as_deref(),
                    pending_pool_attempt_record.as_ref().and_then(|pending| {
                        pending
                            .request_model
                            .as_deref()
                            .or(pending.upstream_request_model.as_deref())
                    }),
                )
                .await
                {
                    warn!(account_id = account.account_id, error = %route_err, "failed to record pool route HTTP failure");
                }
            }
            return response_builder.body(Body::empty()).map_err(|err| {
                plain_proxy_error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("failed to build proxy response: {err}"),
                )
            });
        }

        let (tx, rx) = mpsc::channel::<Result<Bytes, io::Error>>(16);
        let state_for_record = state.clone();
        let reservation_key_for_record = pool_routing_reservation_key.clone();
        let sticky_key_for_record = sticky_key.clone();
        let prompt_cache_key_for_record = prompt_cache_key.clone();
        let invoke_id_for_record = upstream_invoke_id.clone();
        let occurred_at_for_record = format_naive(
            upstream_attempt_started_at_utc
                .with_timezone(&Shanghai)
                .naive_local(),
        );
        let pending_pool_attempt_record_for_task = pending_pool_attempt_record;
        let upstream_request_id_for_task = upstream_request_id;
        let upstream_attempt_started_at_utc_for_record = upstream_attempt_started_at_utc;
        let request_contains_encrypted_content_for_task = request_contains_encrypted_content;
        let upstream_content_encoding_for_task = upstream_content_encoding.clone();
        let request_path_for_record = capability_endpoint.to_string();
        let codex_imagegen_rewrite_for_record = codex_imagegen_rewrite.clone();
        let proxy_request_permit_for_task = proxy_request_permit;
        let runtime_snapshot_cleanup_guard_for_task = runtime_snapshot_cleanup_guard.take();
        crate::observability::diagnostics::spawn(async move {
            let _runtime_snapshot_cleanup_guard_for_task = runtime_snapshot_cleanup_guard_for_task;
            let mut deferred_pool_early_phase_cleanup_guard_for_task =
                deferred_pool_early_phase_cleanup_guard;
            let _live_pool_attempt_activity_lease_for_task = live_pool_attempt_activity_lease;
            let mut forwarded_chunks = 0usize;
            let mut forwarded_bytes = 0usize;
            let stream_started_at = Instant::now();
            let mut stream_error_message: Option<String> = None;
            let mut downstream_closed = false;
            let mut response_parse_buffer =
                BoundedResponseParseBuffer::new(RAW_RESPONSE_PREVIEW_LIMIT);
            let mut proxy_request_permit_for_task = Some(proxy_request_permit_for_task);

            if let Some(chunk) = first_chunk {
                response_parse_buffer.append(&chunk);
                forwarded_chunks = forwarded_chunks.saturating_add(1);
                forwarded_bytes = forwarded_bytes.saturating_add(chunk.len());
                if !transport_bytes_live_counted
                    && let Some(invoke_id) = invoke_id_for_record.as_deref()
                {
                    state_for_record
                        .dashboard_network_speed_cache
                        .record_response_chunk_bytes(
                            invoke_id,
                            &occurred_at_for_record,
                            Some(account.account_id),
                            account.upstream_base_url.host_str(),
                            chunk.len(),
                            Utc::now(),
                        );
                    schedule_dashboard_network_projection(state_for_record.as_ref());
                }
                if tx.send(Ok(chunk)).await.is_err() {
                    downstream_closed = true;
                    let _ = proxy_request_permit_for_task.take();
                }
            }

            loop {
                if downstream_closed {
                    break;
                }
                let next_chunk = if let Some(stream_timeout) = stream_timeout {
                    match timeout(stream_timeout, upstream_stream.next()).await {
                        Ok(next_chunk) => next_chunk,
                        Err(_) => {
                            let message = pool_upstream_timeout_message(
                                stream_timeout,
                                "waiting for upstream stream completion",
                            );
                            stream_error_message = Some(message.clone());
                            let _ = tx.send(Err(io::Error::other(message))).await;
                            break;
                        }
                    }
                } else {
                    upstream_stream.next().await
                };
                let Some(next_chunk) = next_chunk else {
                    break;
                };
                match next_chunk {
                    Ok(chunk) => {
                        response_parse_buffer.append(&chunk);
                        forwarded_chunks = forwarded_chunks.saturating_add(1);
                        forwarded_bytes = forwarded_bytes.saturating_add(chunk.len());
                        if !transport_bytes_live_counted
                            && let Some(invoke_id) = invoke_id_for_record.as_deref()
                        {
                            state_for_record
                                .dashboard_network_speed_cache
                                .record_response_chunk_bytes(
                                    invoke_id,
                                    &occurred_at_for_record,
                                    Some(account.account_id),
                                    account.upstream_base_url.host_str(),
                                    chunk.len(),
                                    Utc::now(),
                                );
                            schedule_dashboard_network_projection(state_for_record.as_ref());
                        }
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
            let stream_latency_ms = elapsed_ms(stream_started_at);
            let response_info = response_parse_buffer.into_response_info(
                ProxyCaptureTarget::Responses,
                upstream_content_encoding_for_task.as_deref(),
            );
            let had_logical_stream_failure = response_info.stream_terminal_event.is_some();
            let logical_stream_failure_message = had_logical_stream_failure
                .then(|| format_upstream_response_failed_message(&response_info));
            let response_contains_encrypted_content = response_info.contains_encrypted_content;

            if let Some(message) = stream_error_message.as_deref() {
                release_pool_routing_reservation(
                    state_for_record.as_ref(),
                    &reservation_key_for_record,
                );
                if let Err(route_err) = record_runtime_pool_route_transport_failure(
                    state_for_record.as_ref(),
                    account.account_id,
                    sticky_key_for_record.as_deref(),
                    message,
                    PROXY_FAILURE_UPSTREAM_STREAM_ERROR,
                    invoke_id_for_record.as_deref(),
                    pending_pool_attempt_record_for_task
                        .as_ref()
                        .and_then(|pending| pending.attempt_id),
                    pending_pool_attempt_record_for_task
                        .as_ref()
                        .and_then(|pending| {
                            pending
                                .request_model
                                .as_deref()
                                .or(pending.upstream_request_model.as_deref())
                        }),
                )
                .await
                {
                    warn!(account_id = account.account_id, error = %route_err, "failed to record pool stream error");
                }
            } else if pool_route_response_status_is_success(upstream_status)
                && !had_logical_stream_failure
            {
                consume_pool_routing_reservation(
                    state_for_record.as_ref(),
                    &reservation_key_for_record,
                );
                if let Err(route_err) = record_runtime_pool_route_success(
                    state_for_record.as_ref(),
                    account.account_id,
                    upstream_attempt_started_at_utc_for_record,
                    sticky_key_for_record.as_deref(),
                    prompt_cache_key_for_record.as_deref(),
                    invoke_id_for_record.as_deref(),
                    request_path_for_record.as_str(),
                    request_image_intent,
                    codex_imagegen_rewrite_for_record.as_ref(),
                    pending_pool_attempt_record_for_task
                        .as_ref()
                        .and_then(|pending| pending.attempt_id),
                    pending_pool_attempt_record_for_task
                        .as_ref()
                        .and_then(|pending| {
                            pending.sticky_affinity_generation.or_else(|| {
                                pending.attempt_id.and(account.sticky_affinity_generation)
                            })
                        }),
                    pending_pool_attempt_record_for_task
                        .as_ref()
                        .and_then(|pending| {
                            pending
                                .request_model
                                .as_deref()
                                .or(pending.upstream_request_model.as_deref())
                        }),
                )
                .await
                {
                    warn!(account_id = account.account_id, error = %route_err, "failed to record pool route success");
                }
                if let Some(prompt_cache_key) = prompt_cache_key_for_record.as_deref()
                    && (request_contains_encrypted_content_for_task
                        || response_contains_encrypted_content)
                {
                    match confirm_prompt_cache_encrypted_session_owner_success_if_enabled(
                        state_for_record.as_ref(),
                        prompt_cache_key,
                        account.account_id,
                    )
                    .await
                    {
                        Ok(true) => {
                            if let Err(err) =
                                promote_prompt_cache_group_binding_to_upstream_account_and_broadcast(
                                    state_for_record.as_ref(),
                                    prompt_cache_key,
                                    account.account_id,
                                )
                                .await
                            {
                                warn!(
                                    prompt_cache_key,
                                    account_id = account.account_id,
                                    error = %err,
                                    "failed to promote prompt cache group binding after via-pool encrypted session success"
                                );
                            }
                        }
                        Ok(false) => {}
                        Err(err) => {
                            warn!(
                                prompt_cache_key,
                                account_id = account.account_id,
                                error = %err,
                                "failed to persist via-pool encrypted session owner"
                            );
                        }
                    }
                }
            } else {
                let route_http_failure_message =
                    logical_stream_failure_message.clone().unwrap_or_else(|| {
                        format!("pool upstream responded with {}", upstream_status.as_u16())
                    });
                release_pool_routing_reservation(
                    state_for_record.as_ref(),
                    &reservation_key_for_record,
                );
                if let Err(route_err) = record_runtime_pool_route_http_failure(
                    state_for_record.as_ref(),
                    account.account_id,
                    &account.kind,
                    account.single_account_rotation_enabled
                        && account.effective_upstream_429_max_retries() == 0,
                    sticky_key_for_record.as_deref(),
                    upstream_status,
                    &route_http_failure_message,
                    invoke_id_for_record.as_deref(),
                    request_path_for_record.as_str(),
                    request_image_intent,
                    pending_pool_attempt_record_for_task
                        .as_ref()
                        .and_then(|pending| pending.attempt_id),
                    pending_pool_attempt_record_for_task
                        .as_ref()
                        .and_then(|pending| pending.sticky_affinity_generation),
                    prompt_cache_key_for_record.as_deref(),
                    pending_pool_attempt_record_for_task
                        .as_ref()
                        .and_then(|pending| {
                            pending
                                .request_model
                                .as_deref()
                                .or(pending.upstream_request_model.as_deref())
                        }),
                )
                .await
                {
                    warn!(account_id = account.account_id, error = %route_err, "failed to record pool route HTTP failure");
                }
            }
            finalize_tracked_pool_attempt(
                state_for_record.as_ref(),
                pending_pool_attempt_record_for_task.as_ref(),
                if stream_error_message.is_some() {
                    POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_TRANSPORT_FAILURE
                } else if had_logical_stream_failure
                    || !pool_route_response_status_is_success(upstream_status)
                {
                    POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_HTTP_FAILURE
                } else {
                    POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS
                },
                Some(upstream_status),
                if downstream_closed {
                    Some(upstream_status)
                } else {
                    None
                },
                stream_error_message
                    .as_ref()
                    .map(|_| PROXY_FAILURE_UPSTREAM_STREAM_ERROR)
                    .or_else(|| {
                        had_logical_stream_failure.then_some(PROXY_FAILURE_UPSTREAM_RESPONSE_FAILED)
                    }),
                stream_error_message
                    .as_deref()
                    .or(logical_stream_failure_message.as_deref())
                    .or_else(|| {
                        (!pool_route_response_status_is_success(upstream_status))
                            .then_some("upstream HTTP failure")
                    }),
                None,
                Some(t_upstream_connect_ms),
                Some(t_upstream_ttfb_ms),
                Some(stream_latency_ms),
                upstream_request_id_for_task.as_deref(),
                "via-pool failover streamed response",
            )
            .await;
            complete_deferred_pool_early_phase_cleanup_guard(
                &mut deferred_pool_early_phase_cleanup_guard_for_task,
            );

            info!(
                proxy_request_id,
                account_id = account.account_id,
                t_upstream_connect_ms,
                forwarded_chunks,
                forwarded_bytes,
                elapsed_ms = stream_latency_ms,
                "pool upstream response stream completed"
            );
        });

        response_builder
            .body(Body::from_stream(ReceiverStream::new(rx)))
            .map_err(|err| {
                plain_proxy_error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("failed to build proxy response: {err}"),
                )
            })
    })
}
