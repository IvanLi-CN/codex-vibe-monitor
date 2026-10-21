use super::*;

pub(crate) enum PoolAttemptResponseOutcome {
    Retry,
    Failover,
    Response(Box<PoolUpstreamResponse>),
}

pub(crate) struct PoolAttemptResponseContext<'a, 'b> {
    pub(crate) context: &'a mut PoolAttemptExecutionContext<'b>,
    pub(crate) distinct_account_count: usize,
    pub(crate) same_account_attempt: u8,
    pub(crate) same_account_retry_budget: u8,
    pub(crate) attempt_pre_first_byte_timeout: Duration,
    pub(crate) attempt_total_timeout_started_at: Option<Instant>,
    pub(crate) response: ProxyUpstreamResponseBody,
    pub(crate) oauth_responses_debug: Option<oauth_bridge::OauthResponsesDebugInfo>,
    pub(crate) forward_proxy_selection: Option<(ForwardProxyRouteScope, SelectedForwardProxy)>,
    pub(crate) transport_bytes_live_counted: bool,
    pub(crate) codex_imagegen_retest_claimed: bool,
    pub(crate) same_account_only_forward_proxy_candidate: PoolSelectedForwardProxy,
    pub(crate) connect_started: Instant,
    pub(crate) attempt_started_at_utc: DateTime<Utc>,
    pub(crate) attempt_observed_at: String,
    pub(crate) pending_attempt_record: &'a mut Option<PendingPoolAttemptRecord>,
    pub(crate) early_phase_cleanup_guard: &'a mut Option<PoolEarlyPhaseOrphanCleanupGuard>,
    pub(crate) live_attempt_activity_lease: Option<PoolLiveAttemptActivityLease>,
    pub(crate) attempted_requested_hosted_image_intent: ImageIntent,
    pub(crate) attempted_requested_service_tier: Option<String>,
    pub(crate) attempted_request_body_for_capture: Option<Bytes>,
    pub(crate) attempted_codex_imagegen_rewrite: Option<Value>,
}

pub(crate) async fn handle_pool_attempt_response<'a, 'b>(
    response_context: PoolAttemptResponseContext<'a, 'b>,
) -> Result<PoolAttemptResponseOutcome, PoolUpstreamError> {
    let PoolAttemptResponseContext {
        context,
        distinct_account_count,
        same_account_attempt,
        same_account_retry_budget,
        attempt_pre_first_byte_timeout,
        attempt_total_timeout_started_at,
        response,
        oauth_responses_debug,
        forward_proxy_selection,
        transport_bytes_live_counted,
        codex_imagegen_retest_claimed,
        same_account_only_forward_proxy_candidate,
        connect_started,
        attempt_started_at_utc,
        attempt_observed_at,
        pending_attempt_record,
        early_phase_cleanup_guard,
        live_attempt_activity_lease,
        attempted_requested_hosted_image_intent,
        attempted_requested_service_tier,
        attempted_request_body_for_capture,
        attempted_codex_imagegen_rewrite,
    } = response_context;
    let apply_retry_delay_budget = |retry_delay| {
        pool_retry_delay_with_total_timeout(
            retry_delay,
            *context.same_account_only_retry_active,
            context.responses_total_timeout,
            attempt_total_timeout_started_at,
        )
    };
    let connect_latency_ms = elapsed_ms(connect_started);
    let status = response.status();
    if matches!(status, StatusCode::BAD_REQUEST | StatusCode::NOT_FOUND)
        || status == StatusCode::TOO_MANY_REQUESTS
        || status == StatusCode::PAYLOAD_TOO_LARGE
        || status.is_server_error()
        || matches!(
            status,
            StatusCode::UNAUTHORIZED | StatusCode::PAYMENT_REQUIRED | StatusCode::FORBIDDEN
        )
    {
        let has_retry_budget = same_account_attempt + 1 < same_account_retry_budget;
        let same_account_only_retry_has_budget = !*context.same_account_only_retry_active
            || same_account_attempt + 1 < context.overload_same_account_attempt_budget;
        let has_upstream_413_retry_budget = pool_upstream_413_retry_allowed(
            status,
            context.priority_handoff_admitted,
            *context.retried_upstream_413_for_account,
        ) && same_account_only_retry_has_budget;
        let has_group_upstream_429_retry_budget = status == StatusCode::TOO_MANY_REQUESTS
            && *context.group_upstream_429_retry_count < context.group_upstream_429_max_retries
            && same_account_only_retry_has_budget;
        let upstream_request_id_header = response
            .headers()
            .get("x-request-id")
            .and_then(|value| value.to_str().ok())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| value.to_string());
        let retry_after_header = response.headers().get(header::RETRY_AFTER).cloned();
        let oauth_transport_failure_kind =
            oauth_bridge::oauth_transport_failure_kind(response.headers());
        if oauth_transport_failure_kind.is_some()
            && let Some((forward_proxy_scope, selected_proxy)) = forward_proxy_selection.as_ref()
        {
            record_pool_account_forward_proxy_result(
                context.state.as_ref(),
                forward_proxy_scope,
                selected_proxy,
                ForwardProxyRouteResultKind::NetworkFailure,
            )
            .await;
        }
        let response_headers = response.headers().clone();
        let (
            error_body_bytes,
            upstream_error_code,
            upstream_error_message,
            upstream_request_id,
            message,
        ) = match read_pool_upstream_bytes_with_timeout(
            response,
            attempt_pre_first_byte_timeout,
            connect_started,
            "reading upstream error body",
        )
        .await
        {
            Ok(body_bytes) => {
                let (upstream_error_code, upstream_error_message, upstream_request_id, message) =
                    summarize_pool_upstream_http_failure(
                        status,
                        upstream_request_id_header.as_deref(),
                        &body_bytes,
                    );
                (
                    Some(body_bytes),
                    upstream_error_code,
                    upstream_error_message,
                    upstream_request_id,
                    message,
                )
            }
            Err(err) => (
                None,
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
        // Upstream compatibility errors describe the model that was actually sent.
        // Keep route health keyed by the original request model elsewhere, but classify
        // an upstream "model not found" response against the mapped target here.
        let requested_model_for_failure = context.upstream_request_model.or_else(|| {
            context.requested_model.as_deref().or_else(|| {
                context
                    .trace_context
                    .as_ref()
                    .and_then(|trace| trace.request_model.as_deref())
            })
        });
        let explicit_model_failure = requested_model_for_failure.map_or_else(
            || is_explicit_model_failure(status, Some(&route_error_message)),
            |model| {
                is_explicit_model_failure_for_model(status, Some(&route_error_message), Some(model))
            },
        );
        if matches!(status, StatusCode::BAD_REQUEST | StatusCode::NOT_FOUND)
            && !explicit_model_failure
        {
            let first_byte_latency_ms = connect_latency_ms;
            if let Some(guard) = early_phase_cleanup_guard.as_mut() {
                guard.mark_first_byte_observed(first_byte_latency_ms);
            }
            let proxy_binding_key_snapshot =
                if let Some((_, selected_proxy)) = forward_proxy_selection.as_ref() {
                    canonical_pool_attempt_proxy_binding_key(
                        context.state.as_ref(),
                        selected_proxy.key.as_str(),
                    )
                    .await
                } else {
                    None
                };

            let mut response_builder = Response::builder().status(status);
            let connection_scoped = connection_scoped_header_names(&response_headers);
            for (name, value) in &response_headers {
                if should_forward_proxy_header(name, &connection_scoped) {
                    response_builder = response_builder.header(name, value);
                }
            }
            let response = response_builder
                // The caller prepends `first_chunk` to this response body. Keep the body empty
                // here so buffered 4xx payloads are not emitted twice.
                .body(Body::empty())
                .map_err(|err| PoolUpstreamError {
                    codex_imagegen_rewrite: attempted_codex_imagegen_rewrite.clone(),
                    account: Some(context.account.clone()),
                    status: StatusCode::INTERNAL_SERVER_ERROR,
                    message: format!("failed to build proxy response: {err}"),
                    canonical_error_message: None,
                    failure_kind: PROXY_FAILURE_UPSTREAM_RESPONSE_FAILED,
                    blocked_binding: None,
                    connect_latency_ms,
                    upstream_error_code: None,
                    upstream_error_message: None,
                    downstream_error_message: None,
                    upstream_request_id: upstream_request_id.clone(),
                    proxy_binding_key_snapshot: proxy_binding_key_snapshot.clone(),
                    oauth_responses_debug: oauth_responses_debug.clone(),
                    attempt_summary: PoolAttemptSummary::default(),
                    requested_service_tier: attempted_requested_service_tier.clone(),
                    request_body_for_capture: attempted_request_body_for_capture.clone(),
                })?;
            let first_chunk = error_body_bytes.filter(|bytes| !bytes.is_empty());

            let deferred_early_phase_cleanup_guard =
                if let Some(pending_attempt_record) = pending_attempt_record.as_ref() {
                    enqueue_pool_streaming_phase_and_defer_guard(
                        context.state.as_ref(),
                        pending_attempt_record,
                        early_phase_cleanup_guard,
                        connect_latency_ms,
                        first_byte_latency_ms,
                    )
                    .await
                } else {
                    disarm_pool_early_phase_cleanup_guard(early_phase_cleanup_guard);
                    None
                };

            let compact_support_observation = classify_compact_support_observation(
                context.original_uri,
                Some(status),
                Some(route_error_message.as_str()),
            );
            if let Some(observation) = compact_support_observation.as_ref()
                && let Err(observation_err) = record_compact_support_observation_with_observed_at(
                    &context.state.pool,
                    context.account.account_id,
                    observation.status,
                    observation.reason.as_deref(),
                    Some(&attempt_observed_at),
                )
                .await
            {
                warn!(
                    account_id = context.account.account_id,
                    error = %observation_err,
                    "failed to record compact support observation"
                );
            }
            let capability_requirements =
                RequestCapabilityRequirements::from_endpoint_and_image_intent_for_method(
                    context.original_uri.path(),
                    attempted_requested_hosted_image_intent,
                    context.method == Method::POST,
                );
            if capability_requirements.response_endpoint
                && classify_response_endpoint_capability_observation(
                    status,
                    Some(route_error_message.as_str()),
                ) == CapabilitySupport::Unsupported
                && let Err(observation_err) = record_capability_observation_with_observed_at(
                    &context.state.pool,
                    context.account.account_id,
                    UpstreamCapabilityAxis::ResponseEndpoint,
                    CapabilitySupport::Unsupported,
                    Some(route_error_message.as_str()),
                    Some(&attempt_observed_at),
                )
                .await
            {
                warn!(
                    account_id = context.account.account_id,
                    error = %observation_err,
                    "failed to record response endpoint capability observation"
                );
            }
            if capability_requirements.chat_completions_endpoint
                && classify_chat_completions_capability_observation(
                    status,
                    Some(route_error_message.as_str()),
                ) == CapabilitySupport::Unsupported
                && let Err(observation_err) = record_capability_observation_with_observed_at(
                    &context.state.pool,
                    context.account.account_id,
                    UpstreamCapabilityAxis::ChatCompletionsEndpoint,
                    CapabilitySupport::Unsupported,
                    Some(route_error_message.as_str()),
                    Some(&attempt_observed_at),
                )
                .await
            {
                warn!(
                    account_id = context.account.account_id,
                    error = %observation_err,
                    "failed to record chat completions capability observation"
                );
            }
            if capability_requirements.image_endpoint
                && classify_image_endpoint_capability_observation(
                    status,
                    Some(route_error_message.as_str()),
                ) == CapabilitySupport::Unsupported
                && let Err(observation_err) = record_capability_observation_with_observed_at(
                    &context.state.pool,
                    context.account.account_id,
                    UpstreamCapabilityAxis::ImageEndpoint,
                    CapabilitySupport::Unsupported,
                    Some(route_error_message.as_str()),
                    Some(&attempt_observed_at),
                )
                .await
            {
                warn!(
                    account_id = context.account.account_id,
                    error = %observation_err,
                    "failed to record image endpoint capability observation"
                );
            }
            if capability_requirements.response_image_tool
                && classify_response_image_tool_capability_observation(
                    status,
                    Some(route_error_message.as_str()),
                ) == CapabilitySupport::Unsupported
                && let Err(observation_err) = record_capability_observation_with_observed_at(
                    &context.state.pool,
                    context.account.account_id,
                    UpstreamCapabilityAxis::ResponseImageTool,
                    CapabilitySupport::Unsupported,
                    Some(route_error_message.as_str()),
                    Some(&attempt_observed_at),
                )
                .await
            {
                warn!(
                    account_id = context.account.account_id,
                    error = %observation_err,
                    "failed to record response image-tool capability observation"
                );
            }
            if capability_requirements.standalone_search
                && classify_standalone_search_capability_observation(
                    status,
                    Some(route_error_message.as_str()),
                ) == CapabilitySupport::Unsupported
                && let Err(observation_err) = record_capability_observation_with_observed_at(
                    &context.state.pool,
                    context.account.account_id,
                    UpstreamCapabilityAxis::StandaloneSearch,
                    CapabilitySupport::Unsupported,
                    Some(route_error_message.as_str()),
                    Some(&attempt_observed_at),
                )
                .await
            {
                warn!(
                    account_id = context.account.account_id,
                    error = %observation_err,
                    "failed to record standalone search capability observation"
                );
            }

            if let Some((forward_proxy_scope, selected_proxy)) = forward_proxy_selection.as_ref() {
                record_pool_account_forward_proxy_result(
                    context.state.as_ref(),
                    forward_proxy_scope,
                    selected_proxy,
                    ForwardProxyRouteResultKind::CompletedRequest,
                )
                .await;
            }
            return Ok(PoolAttemptResponseOutcome::Response(Box::new(
                PoolUpstreamResponse {
                    account: context.account.clone(),
                    response: ProxyUpstreamResponseBody::Axum(response),
                    transport_bytes_live_counted,
                    stream_timeout: context.stream_timeout,
                    oauth_responses_debug,
                    connect_latency_ms,
                    attempt_started_at_utc,
                    first_byte_latency_ms,
                    first_chunk,
                    first_chunk_received_at: None,
                    first_stream_chunk_received_at: None,
                    pending_attempt_record: pending_attempt_record.take().map(|mut pending| {
                        pending.connect_latency_ms = connect_latency_ms;
                        pending.first_byte_latency_ms = first_byte_latency_ms;
                        pending.compact_support_status = compact_support_observation
                            .as_ref()
                            .map(|value| value.status.to_string());
                        pending.compact_support_reason = compact_support_observation
                            .as_ref()
                            .and_then(|value| value.reason.clone());
                        enqueue_pool_upstream_request_attempt_snapshot(
                            context.state.as_ref(),
                            &pending,
                        );
                        pending
                    }),
                    deferred_early_phase_cleanup_guard,
                    live_attempt_activity_lease,
                    attempt_summary: pool_attempt_summary(
                        *context.attempt_count,
                        distinct_account_count,
                        None,
                    ),
                    requested_service_tier: attempted_requested_service_tier,
                    request_body_for_capture: attempted_request_body_for_capture,
                    codex_imagegen_rewrite: attempted_codex_imagegen_rewrite.clone(),
                    reservation_guard: context.reservation_guard.take(),
                },
            )));
        }
        let http_failure_classification =
            classify_pool_account_http_failure(&context.account.kind, status, &route_error_message);
        let failure_kind =
            oauth_transport_failure_kind.unwrap_or(http_failure_classification.failure_kind);
        let normalized_failure = normalize_pool_upstream_failure_record(
            status,
            oauth_transport_failure_kind,
            &message,
            upstream_error_message.as_deref(),
        );
        let canonical_override = (normalized_failure.canonical_error_message != message)
            .then(|| normalized_failure.canonical_error_message.clone());
        let compact_support_observation = classify_compact_support_observation(
            context.original_uri,
            Some(status),
            Some(route_error_message.as_str()),
        );
        let compact_support_is_unsupported = compact_support_observation
            .as_ref()
            .is_some_and(|value| value.status == COMPACT_SUPPORT_STATUS_UNSUPPORTED);
        let timeout_shaped_failure =
            status.is_server_error() && pool_failure_is_timeout_shaped(failure_kind, &message);
        let should_timeout_route_failover =
            context.uses_timeout_route_failover && timeout_shaped_failure;
        let direct_image_handshake_timeout = context.direct_image_request
            && failure_kind == PROXY_FAILURE_UPSTREAM_HANDSHAKE_TIMEOUT;
        let codex_imagegen_upstream_incompatible = codex_imagegen_upstream_incompatibility(
            status,
            &route_error_message,
            attempted_codex_imagegen_rewrite.as_ref(),
        );
        let should_schedule_retry = has_retry_budget
            && !compact_support_is_unsupported
            && !should_timeout_route_failover
            && !direct_image_handshake_timeout
            && !codex_imagegen_upstream_incompatible
            && !codex_imagegen_retest_claimed
            && status.is_server_error()
            && status != StatusCode::TOO_MANY_REQUESTS;
        let retry_delay = should_schedule_retry.then(|| {
            retry_after_header
                .as_ref()
                .and_then(parse_retry_after_delay)
                .unwrap_or_else(|| {
                    fallback_proxy_429_retry_delay_for_state(
                        context.state.as_ref(),
                        u32::from(same_account_attempt) + 1,
                    )
                })
        });
        let finished_at = shanghai_now_string();
        if let Some(response_body) = error_body_bytes.as_ref()
            && let Some(pending_attempt_record) = pending_attempt_record.as_mut()
        {
            let response_body_logging_enabled = context
                .state
                .proxy_model_settings
                .read()
                .await
                .response_body_logging_enabled;
            let response_content_encoding = response_headers
                .get(header::CONTENT_ENCODING)
                .and_then(|value| value.to_str().ok());
            persist_pool_attempt_response_capture(
                context.state.clone(),
                pending_attempt_record,
                response_body.clone(),
                response_body_logging_enabled,
                response_content_encoding.map(str::to_string),
            )
            .await;
        }
        if let Some(pending_attempt_record) = pending_attempt_record.as_ref()
            && let Err(record_err) = finalize_pool_upstream_request_attempt_in_memory(
                context.state.as_ref(),
                pending_attempt_record,
                finished_at.as_str(),
                normalized_failure.attempt_status,
                normalized_failure.upstream_http_status,
                normalized_failure.downstream_http_status,
                Some(failure_kind),
                Some(normalized_failure.canonical_error_message.as_str()),
                normalized_failure.downstream_error_message.as_deref(),
                Some(connect_latency_ms),
                None,
                None,
                upstream_request_id.as_deref(),
                compact_support_observation
                    .as_ref()
                    .map(|value| value.status),
                compact_support_observation
                    .as_ref()
                    .and_then(|value| value.reason.as_deref()),
            )
            .await
        {
            warn!(
                invoke_id = pending_attempt_record.invoke_id,
                error = %record_err,
                "failed to persist pool http failure attempt"
            );
        }
        if codex_imagegen_upstream_incompatible
            && let Err(observation_err) = record_capability_observation_with_observed_at(
                &context.state.pool,
                context.account.account_id,
                UpstreamCapabilityAxis::CodexImagegen,
                CapabilitySupport::Unsupported,
                Some(route_error_message.as_str()),
                Some(&attempt_observed_at),
            )
            .await
        {
            warn!(
                account_id = context.account.account_id,
                error = %observation_err,
                "failed to record Codex imagegen capability observation"
            );
        }
        if let Some(pending_attempt_record) = pending_attempt_record.as_ref()
            && let Err(err) = broadcast_pool_upstream_attempts_snapshot(
                context.state.as_ref(),
                &pending_attempt_record.invoke_id,
            )
            .await
        {
            warn!(
                invoke_id = pending_attempt_record.invoke_id,
                error = %err,
                "failed to broadcast pool http failure snapshot"
            );
        }
        if has_group_upstream_429_retry_budget && !codex_imagegen_retest_claimed {
            let retry_delay = pool_group_upstream_429_retry_delay(context.state.as_ref());
            let retry_delay = apply_retry_delay_budget(retry_delay);
            let group_retry_index = *context.group_upstream_429_retry_count + 1;
            info!(
                account_id = context.account.account_id,
                status = status.as_u16(),
                retry_index = same_account_attempt + 1,
                group_retry_index,
                max_same_account_attempts = context.same_account_attempt_loop_budget,
                context.group_upstream_429_max_retries,
                retry_after_ms = retry_delay.as_millis(),
                "pool upstream responded with group retryable 429; retrying same account"
            );
            *context.group_upstream_429_retry_count += 1;
            disarm_pool_early_phase_cleanup_guard(early_phase_cleanup_guard);
            crate::observability::diagnostics::wait(
                crate::observability::diagnostics::Resource::RetryBackoff,
                sleep(retry_delay),
            )
            .await;
            return Ok(PoolAttemptResponseOutcome::Retry);
        }
        if has_upstream_413_retry_budget && !codex_imagegen_retest_claimed {
            let retry_delay = fallback_proxy_429_retry_delay_for_state(
                context.state.as_ref(),
                u32::from(same_account_attempt) + 1,
            );
            let retry_delay = apply_retry_delay_budget(retry_delay);
            info!(
                account_id = context.account.account_id,
                status = status.as_u16(),
                retry_index = same_account_attempt + 1,
                max_same_account_attempts = 2,
                retry_after_ms = retry_delay.as_millis(),
                "pool upstream responded with 413; retrying same account once"
            );
            *context.retried_upstream_413_for_account = true;
            disarm_pool_early_phase_cleanup_guard(early_phase_cleanup_guard);
            crate::observability::diagnostics::wait(
                crate::observability::diagnostics::Resource::RetryBackoff,
                sleep(retry_delay),
            )
            .await;
            return Ok(PoolAttemptResponseOutcome::Retry);
        }
        if let Some(retry_delay) = retry_delay
            && !codex_imagegen_retest_claimed
        {
            let retry_delay = apply_retry_delay_budget(retry_delay);
            info!(
                account_id = context.account.account_id,
                status = status.as_u16(),
                retry_index = same_account_attempt + 1,
                max_same_account_attempts = context.same_account_attempt_budget,
                retry_after_ms = retry_delay.as_millis(),
                "pool upstream responded with retryable status; retrying same account"
            );
            disarm_pool_early_phase_cleanup_guard(early_phase_cleanup_guard);
            crate::observability::diagnostics::wait(
                crate::observability::diagnostics::Resource::RetryBackoff,
                sleep(retry_delay),
            )
            .await;
            return Ok(PoolAttemptResponseOutcome::Retry);
        }
        let route_failure_result = if codex_imagegen_upstream_incompatible {
            Ok(())
        } else if oauth_transport_failure_kind.is_some() {
            context
                .reservation_guard
                .as_mut()
                .expect("pool routing reservation should be active")
                .fence_failure(record_runtime_pool_route_transport_failure(
                    context.state.as_ref(),
                    context.account.account_id,
                    context.sticky_key,
                    normalized_failure.canonical_error_message.as_str(),
                    oauth_transport_failure_kind.unwrap_or(PROXY_FAILURE_FAILED_CONTACT_UPSTREAM),
                    context.trace_context.map(|trace| trace.invoke_id.as_str()),
                    pending_attempt_record
                        .as_ref()
                        .and_then(|pending| pending.attempt_id),
                    context.requested_model.as_deref(),
                ))
                .await
        } else {
            context
                .reservation_guard
                .as_mut()
                .expect("pool routing reservation should be active")
                .fence_failure(record_runtime_pool_route_http_failure(
                    context.state.as_ref(),
                    context.account.account_id,
                    &context.account.kind,
                    context.account.single_account_rotation_enabled,
                    context.sticky_key,
                    status,
                    &route_error_message,
                    context.trace_context.map(|trace| trace.invoke_id.as_str()),
                    context.capability_endpoint,
                    attempted_requested_hosted_image_intent,
                    pending_attempt_record
                        .as_ref()
                        .and_then(|pending| pending.attempt_id),
                    pending_attempt_record
                        .as_ref()
                        .and_then(|pending| pending.sticky_affinity_generation),
                    context
                        .prompt_cache_key
                        .or(context.sticky_event_prompt_cache_key),
                    requested_model_for_failure,
                ))
                .await
        };
        if let Err(route_err) = route_failure_result {
            warn!(
                account_id = context.account.account_id,
                error = %route_err,
                "failed to record pool live-attempt failure"
            );
        }
        if let Some(observation) = compact_support_observation.as_ref()
            && let Err(observation_err) = record_compact_support_observation_with_observed_at(
                &context.state.pool,
                context.account.account_id,
                observation.status,
                observation.reason.as_deref(),
                Some(&attempt_observed_at),
            )
            .await
        {
            warn!(
                account_id = context.account.account_id,
                error = %observation_err,
                "failed to record compact support observation"
            );
        }
        store_pool_failover_error(
            context.last_error,
            &mut *context.preserve_sticky_owner_terminal_error,
            PoolUpstreamError {
                codex_imagegen_rewrite: attempted_codex_imagegen_rewrite.clone(),
                account: Some(context.account.clone()),
                status,
                message: message.clone(),
                canonical_error_message: canonical_override,
                failure_kind,
                blocked_binding: None,
                connect_latency_ms,
                upstream_error_code,
                upstream_error_message,
                downstream_error_message: normalized_failure.downstream_error_message,
                upstream_request_id,
                proxy_binding_key_snapshot: if let Some((_, selected_proxy)) =
                    forward_proxy_selection.as_ref()
                {
                    canonical_pool_attempt_proxy_binding_key(
                        context.state.as_ref(),
                        selected_proxy.key.as_str(),
                    )
                    .await
                } else {
                    None
                },
                oauth_responses_debug: oauth_responses_debug.clone(),
                attempt_summary: PoolAttemptSummary::default(),
                requested_service_tier: attempted_requested_service_tier.clone(),
                request_body_for_capture: attempted_request_body_for_capture.clone(),
            },
            attempted_codex_imagegen_rewrite.as_ref(),
        );
        if context.direct_image_request && failure_kind == PROXY_FAILURE_UPSTREAM_HANDSHAKE_TIMEOUT
        {
            let mut final_error = context
                .last_error
                .take()
                .expect("direct-image timeout stores a terminal upstream error");
            final_error.status = StatusCode::GATEWAY_TIMEOUT;
            final_error.message = format!(
                "[{PROXY_FAILURE_UPSTREAM_HANDSHAKE_TIMEOUT}] {}",
                final_error.message
            );
            final_error.upstream_error_code =
                Some(PROXY_FAILURE_UPSTREAM_HANDSHAKE_TIMEOUT.to_string());
            final_error.attempt_summary = pool_attempt_summary(
                *context.attempt_count,
                distinct_account_count,
                Some(PROXY_FAILURE_UPSTREAM_HANDSHAKE_TIMEOUT.to_string()),
            );
            disarm_pool_early_phase_cleanup_guard(early_phase_cleanup_guard);
            return Err(final_error);
        }
        *context.exhausted_accounts_all_rate_limited &= status == StatusCode::TOO_MANY_REQUESTS;
        if should_timeout_route_failover {
            context
                .excluded_upstream_route_keys
                .insert(context.upstream_route_key.clone());
            *context.overload_required_upstream_route_key = None;
            *context.timeout_route_failover_pending = true;
        }
        disarm_pool_early_phase_cleanup_guard(early_phase_cleanup_guard);
        if let Some(final_error) = take_priority_handoff_terminal_error(
            &context.account,
            context.last_error,
            *context.attempt_count,
            distinct_account_count,
        ) {
            return Err(final_error);
        }
        if *context.same_account_only_retry_active {
            return Err(take_same_account_only_terminal_error(
                context.last_error,
                *context.attempt_count,
                distinct_account_count,
            ));
        }
        return Ok(PoolAttemptResponseOutcome::Failover);
    }

    if let Some(pending_attempt_record) = pending_attempt_record.as_ref()
        && let Err(err) = advance_pool_upstream_request_attempt_phase(
            context.state.as_ref(),
            pending_attempt_record,
            POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_WAITING_FIRST_BYTE,
        )
        .await
    {
        warn!(
            invoke_id = %pending_attempt_record.invoke_id,
            error = %err,
            "failed to advance pool attempt into wait-first-byte phase"
        );
    }
    let first_byte_started = Instant::now();
    let (response, first_chunk, first_chunk_received_at) =
        match read_pool_upstream_first_chunk_with_timeout(
            response,
            attempt_pre_first_byte_timeout,
            connect_started,
        )
        .await
        {
            Ok(value) => value,
            Err(err) => {
                if let Some((forward_proxy_scope, selected_proxy)) =
                    forward_proxy_selection.as_ref()
                {
                    record_pool_account_forward_proxy_result(
                        context.state.as_ref(),
                        forward_proxy_scope,
                        selected_proxy,
                        ForwardProxyRouteResultKind::NetworkFailure,
                    )
                    .await;
                }
                let message = format!("upstream stream error before first chunk: {err}");
                let compact_support_observation = classify_compact_support_observation(
                    context.original_uri,
                    None,
                    Some(message.as_str()),
                );
                let timeout_shaped_failure =
                    pool_failure_is_timeout_shaped(PROXY_FAILURE_UPSTREAM_STREAM_ERROR, &message);
                let should_timeout_route_failover =
                    context.uses_timeout_route_failover && timeout_shaped_failure;
                let finished_at = shanghai_now_string();
                if let Some(pending_attempt_record) = pending_attempt_record.as_ref()
                    && let Err(record_err) = finalize_pool_upstream_request_attempt_in_memory(
                        context.state.as_ref(),
                        pending_attempt_record,
                        finished_at.as_str(),
                        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_TRANSPORT_FAILURE,
                        None,
                        None,
                        Some(PROXY_FAILURE_UPSTREAM_STREAM_ERROR),
                        Some(message.as_str()),
                        None,
                        Some(connect_latency_ms),
                        None,
                        None,
                        None,
                        compact_support_observation
                            .as_ref()
                            .map(|value| value.status),
                        compact_support_observation
                            .as_ref()
                            .and_then(|value| value.reason.as_deref()),
                    )
                    .await
                {
                    warn!(
                        invoke_id = pending_attempt_record.invoke_id,
                        error = %record_err,
                        "failed to persist pool first-chunk transport attempt"
                    );
                }
                if let Some(pending_attempt_record) = pending_attempt_record.as_ref()
                    && let Err(err) = broadcast_pool_upstream_attempts_snapshot(
                        context.state.as_ref(),
                        &pending_attempt_record.invoke_id,
                    )
                    .await
                {
                    warn!(
                        invoke_id = pending_attempt_record.invoke_id,
                        error = %err,
                        "failed to broadcast pool first-chunk failure snapshot"
                    );
                }
                if let (Some(total_timeout), Some(started_at)) = (
                    context.responses_total_timeout,
                    attempt_total_timeout_started_at,
                ) && pool_total_timeout_exhausted(total_timeout, started_at)
                {
                    let final_error = build_pool_total_timeout_exhausted_error(
                        total_timeout,
                        Some(PoolUpstreamError {
                            codex_imagegen_rewrite: attempted_codex_imagegen_rewrite.clone(),
                            account: Some(context.account.clone()),
                            status: StatusCode::BAD_GATEWAY,
                            message: message.clone(),
                            canonical_error_message: None,
                            failure_kind: PROXY_FAILURE_UPSTREAM_STREAM_ERROR,
                            blocked_binding: None,
                            connect_latency_ms,
                            upstream_error_code: None,
                            upstream_error_message: None,
                            downstream_error_message: None,
                            upstream_request_id: None,
                            proxy_binding_key_snapshot: if let Some((_, selected_proxy)) =
                                forward_proxy_selection.as_ref()
                            {
                                canonical_pool_attempt_proxy_binding_key(
                                    context.state.as_ref(),
                                    selected_proxy.key.as_str(),
                                )
                                .await
                            } else {
                                None
                            },
                            oauth_responses_debug: oauth_responses_debug.clone(),
                            attempt_summary: PoolAttemptSummary::default(),
                            requested_service_tier: attempted_requested_service_tier.clone(),
                            request_body_for_capture: attempted_request_body_for_capture.clone(),
                        }),
                        *context.attempt_count,
                        distinct_account_count,
                    );
                    if let Some(trace) = context.trace_context
                        && let Err(err) = insert_pool_upstream_terminal_attempt(
                            &context.state.pool,
                            trace,
                            &final_error,
                            (*context.attempt_count + 1) as i64,
                            distinct_account_count as i64,
                            PROXY_FAILURE_POOL_TOTAL_TIMEOUT_EXHAUSTED,
                        )
                        .await
                    {
                        warn!(
                            invoke_id = trace.invoke_id,
                            error = %err,
                            "failed to persist pool total-timeout exhaustion attempt"
                        );
                    }
                    disarm_pool_early_phase_cleanup_guard(early_phase_cleanup_guard);
                    return Err(final_error);
                }
                let has_retry_budget = same_account_attempt + 1 < same_account_retry_budget;
                if has_retry_budget && !should_timeout_route_failover {
                    let retry_delay = fallback_proxy_429_retry_delay_for_state(
                        context.state.as_ref(),
                        u32::from(same_account_attempt) + 1,
                    );
                    let retry_delay = apply_retry_delay_budget(retry_delay);
                    info!(
                        account_id = context.account.account_id,
                        retry_index = same_account_attempt + 1,
                        max_same_account_attempts = same_account_retry_budget,
                        retry_after_ms = retry_delay.as_millis(),
                        "pool upstream first chunk failed; retrying same account"
                    );
                    disarm_pool_early_phase_cleanup_guard(early_phase_cleanup_guard);
                    crate::observability::diagnostics::wait(
                        crate::observability::diagnostics::Resource::RetryBackoff,
                        sleep(retry_delay),
                    )
                    .await;
                    return Ok(PoolAttemptResponseOutcome::Retry);
                }
                if let Err(route_err) = context
                    .reservation_guard
                    .as_mut()
                    .expect("pool routing reservation should be active")
                    .fence_failure(record_runtime_pool_route_transport_failure(
                        context.state.as_ref(),
                        context.account.account_id,
                        context.sticky_key,
                        &message,
                        PROXY_FAILURE_UPSTREAM_STREAM_ERROR,
                        context.trace_context.map(|trace| trace.invoke_id.as_str()),
                        pending_attempt_record
                            .as_ref()
                            .and_then(|pending| pending.attempt_id),
                        pending_attempt_record.as_ref().and_then(|pending| {
                            pending
                                .request_model
                                .as_deref()
                                .or(pending.upstream_request_model.as_deref())
                        }),
                    ))
                    .await
                {
                    warn!(account_id = context.account.account_id, error = %route_err, "failed to record pool first chunk failure");
                }
                store_pool_failover_error(
                    context.last_error,
                    &mut *context.preserve_sticky_owner_terminal_error,
                    PoolUpstreamError {
                        codex_imagegen_rewrite: attempted_codex_imagegen_rewrite.clone(),
                        account: Some(context.account.clone()),
                        status: StatusCode::BAD_GATEWAY,
                        message: message.clone(),
                        canonical_error_message: None,
                        failure_kind: PROXY_FAILURE_UPSTREAM_STREAM_ERROR,
                        blocked_binding: None,
                        connect_latency_ms,
                        upstream_error_code: None,
                        upstream_error_message: None,
                        downstream_error_message: None,
                        upstream_request_id: None,
                        proxy_binding_key_snapshot: if let Some((_, selected_proxy)) =
                            forward_proxy_selection.as_ref()
                        {
                            canonical_pool_attempt_proxy_binding_key(
                                context.state.as_ref(),
                                selected_proxy.key.as_str(),
                            )
                            .await
                        } else {
                            None
                        },
                        oauth_responses_debug: oauth_responses_debug.clone(),
                        attempt_summary: PoolAttemptSummary::default(),
                        requested_service_tier: attempted_requested_service_tier.clone(),
                        request_body_for_capture: attempted_request_body_for_capture.clone(),
                    },
                    attempted_codex_imagegen_rewrite.as_ref(),
                );
                *context.exhausted_accounts_all_rate_limited = false;
                if should_timeout_route_failover {
                    context
                        .excluded_upstream_route_keys
                        .insert(context.upstream_route_key.clone());
                    *context.overload_required_upstream_route_key = None;
                    *context.timeout_route_failover_pending = true;
                }
                disarm_pool_early_phase_cleanup_guard(early_phase_cleanup_guard);
                if let Some(final_error) = take_priority_handoff_terminal_error(
                    &context.account,
                    context.last_error,
                    *context.attempt_count,
                    distinct_account_count,
                ) {
                    return Err(final_error);
                }
                if *context.same_account_only_retry_active {
                    return Err(take_same_account_only_terminal_error(
                        context.last_error,
                        *context.attempt_count,
                        distinct_account_count,
                    ));
                }
                return Ok(PoolAttemptResponseOutcome::Failover);
            }
        };

    let first_byte_latency_ms = elapsed_ms(first_byte_started);
    if let Some(guard) = early_phase_cleanup_guard.as_mut() {
        guard.mark_first_byte_observed(first_byte_latency_ms);
    }
    let response_is_event_stream = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.starts_with("text/event-stream"));
    let response_content_encoding_for_attempt = response
        .headers()
        .get(header::CONTENT_ENCODING)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    let initial_gate_outcome = if context.original_uri.path() == "/v1/responses"
        && status == StatusCode::OK
        && response_is_event_stream
    {
        gate_pool_initial_response_stream_with_timestamp(
            response,
            first_chunk,
            first_chunk_received_at,
            attempt_pre_first_byte_timeout,
            connect_started,
        )
        .await
    } else if context.original_uri.path() == "/v1/responses/compact" && status == StatusCode::OK {
        Ok(
            gate_pool_initial_compact_response(status, response.headers(), first_chunk.as_ref())
                .unwrap_or(PoolInitialResponseGateOutcome::Forward {
                    response,
                    prefetched_bytes: first_chunk,
                    prefetched_bytes_received_at: first_chunk_received_at,
                    replayed_bytes_received_at: None,
                }),
        )
    } else {
        Ok(PoolInitialResponseGateOutcome::Forward {
            response,
            prefetched_bytes: first_chunk,
            prefetched_bytes_received_at: first_chunk_received_at,
            replayed_bytes_received_at: None,
        })
    };
    let (response, first_chunk, first_chunk_received_at, first_stream_chunk_received_at) =
        match initial_gate_outcome {
            Ok(PoolInitialResponseGateOutcome::Forward {
                response,
                prefetched_bytes,
                prefetched_bytes_received_at,
                replayed_bytes_received_at,
            }) => (
                response,
                prefetched_bytes,
                prefetched_bytes_received_at,
                replayed_bytes_received_at,
            ),
            Ok(PoolInitialResponseGateOutcome::RetrySameAccount {
                retry_policy,
                message,
                upstream_error_code,
                upstream_error_message,
                upstream_request_id,
                raw_body,
            }) => {
                if retry_policy == PoolResponsesOverloadRetryPolicy::SameAccountOnly {
                    *context.same_account_only_retry_active = true;
                    *context.same_account_only_forward_proxy =
                        Some(same_account_only_forward_proxy_candidate);
                }
                let finished_at = shanghai_now_string();
                if let Some(pending_attempt_record) = pending_attempt_record.as_mut() {
                    let response_body_logging_enabled = context
                        .state
                        .proxy_model_settings
                        .read()
                        .await
                        .response_body_logging_enabled;
                    persist_pool_attempt_response_capture(
                        context.state.clone(),
                        pending_attempt_record,
                        raw_body.clone(),
                        response_body_logging_enabled,
                        response_content_encoding_for_attempt.clone(),
                    )
                    .await;
                }
                if let Some(pending_attempt_record) = pending_attempt_record.as_ref()
                    && let Err(record_err) = finalize_pool_upstream_request_attempt_in_memory(
                        context.state.as_ref(),
                        pending_attempt_record,
                        finished_at.as_str(),
                        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_HTTP_FAILURE,
                        Some(status),
                        None,
                        Some(PROXY_FAILURE_UPSTREAM_RESPONSE_FAILED),
                        Some(message.as_str()),
                        None,
                        Some(connect_latency_ms),
                        Some(first_byte_latency_ms),
                        None,
                        upstream_request_id.as_deref(),
                        None,
                        None,
                    )
                    .await
                {
                    warn!(
                        invoke_id = pending_attempt_record.invoke_id,
                        error = %record_err,
                        "failed to persist pool retryable response.failed attempt"
                    );
                }
                if let Some(pending_attempt_record) = pending_attempt_record.as_ref()
                    && let Err(err) = broadcast_pool_upstream_attempts_snapshot(
                        context.state.as_ref(),
                        &pending_attempt_record.invoke_id,
                    )
                    .await
                {
                    warn!(
                        invoke_id = pending_attempt_record.invoke_id,
                        error = %err,
                        "failed to broadcast retryable response.failed snapshot"
                    );
                }

                let retryable_response_failed_proxy_binding_key =
                    if let Some((_, selected_proxy)) = forward_proxy_selection.as_ref() {
                        canonical_pool_attempt_proxy_binding_key(
                            context.state.as_ref(),
                            selected_proxy.key.as_str(),
                        )
                        .await
                    } else {
                        None
                    };
                store_pool_failover_error(
                    context.last_error,
                    &mut *context.preserve_sticky_owner_terminal_error,
                    PoolUpstreamError {
                        codex_imagegen_rewrite: attempted_codex_imagegen_rewrite.clone(),
                        account: Some(context.account.clone()),
                        status,
                        message: message.clone(),
                        canonical_error_message: None,
                        failure_kind: PROXY_FAILURE_UPSTREAM_RESPONSE_FAILED,
                        blocked_binding: None,
                        connect_latency_ms,
                        upstream_error_code: upstream_error_code.clone(),
                        upstream_error_message: upstream_error_message.clone(),
                        downstream_error_message: None,
                        upstream_request_id: upstream_request_id.clone(),
                        proxy_binding_key_snapshot: retryable_response_failed_proxy_binding_key,
                        oauth_responses_debug: oauth_responses_debug.clone(),
                        attempt_summary: PoolAttemptSummary::default(),
                        requested_service_tier: attempted_requested_service_tier.clone(),
                        request_body_for_capture: attempted_request_body_for_capture.clone(),
                    },
                    attempted_codex_imagegen_rewrite.as_ref(),
                );

                let has_retry_budget =
                    same_account_attempt + 1 < context.overload_same_account_attempt_budget;
                if has_retry_budget {
                    let retry_delay = fallback_proxy_429_retry_delay_for_state(
                        context.state.as_ref(),
                        u32::from(same_account_attempt) + 1,
                    );
                    let retry_delay = pool_retry_delay_with_total_timeout(
                        retry_delay,
                        *context.same_account_only_retry_active,
                        context.responses_total_timeout,
                        attempt_total_timeout_started_at
                            .or(*context.first_response_attempt_started_at),
                    );
                    info!(
                        account_id = context.account.account_id,
                        retry_index = same_account_attempt + 1,
                        max_same_account_attempts = context.overload_same_account_attempt_budget,
                        retry_after_ms = retry_delay.as_millis(),
                        "pool upstream reported retryable response.failed before forwarding; retrying same account"
                    );
                    disarm_pool_early_phase_cleanup_guard(early_phase_cleanup_guard);
                    crate::observability::diagnostics::wait(
                        crate::observability::diagnostics::Resource::RetryBackoff,
                        sleep(retry_delay),
                    )
                    .await;
                    return Ok(PoolAttemptResponseOutcome::Retry);
                }

                if let Err(route_err) = context
                    .reservation_guard
                    .as_mut()
                    .expect("pool routing reservation should be active")
                    .fence_failure(record_runtime_pool_route_overload_failure(
                        context.state.as_ref(),
                        context.account.account_id,
                        context.sticky_key,
                        &message,
                        context.trace_context.map(|trace| trace.invoke_id.as_str()),
                        pending_attempt_record
                            .as_ref()
                            .and_then(|pending| pending.attempt_id),
                        context.requested_model.as_deref().or_else(|| {
                            pending_attempt_record.as_ref().and_then(|pending| {
                                pending
                                    .request_model
                                    .as_deref()
                                    .or(pending.upstream_request_model.as_deref())
                            })
                        }),
                    ))
                    .await
                {
                    warn!(account_id = context.account.account_id, error = %route_err, "failed to record retryable response.failed route state");
                }
                store_pool_failover_error(
                    context.last_error,
                    &mut *context.preserve_sticky_owner_terminal_error,
                    PoolUpstreamError {
                        codex_imagegen_rewrite: attempted_codex_imagegen_rewrite.clone(),
                        account: Some(context.account.clone()),
                        status,
                        message: message.clone(),
                        canonical_error_message: None,
                        failure_kind: PROXY_FAILURE_UPSTREAM_RESPONSE_FAILED,
                        blocked_binding: None,
                        connect_latency_ms,
                        upstream_error_code,
                        upstream_error_message,
                        downstream_error_message: None,
                        upstream_request_id,
                        proxy_binding_key_snapshot: if let Some((_, selected_proxy)) =
                            forward_proxy_selection.as_ref()
                        {
                            canonical_pool_attempt_proxy_binding_key(
                                context.state.as_ref(),
                                selected_proxy.key.as_str(),
                            )
                            .await
                        } else {
                            None
                        },
                        oauth_responses_debug: oauth_responses_debug.clone(),
                        attempt_summary: PoolAttemptSummary::default(),
                        requested_service_tier: attempted_requested_service_tier.clone(),
                        request_body_for_capture: attempted_request_body_for_capture.clone(),
                    },
                    attempted_codex_imagegen_rewrite.as_ref(),
                );
                *context.exhausted_accounts_all_rate_limited = false;
                if *context.same_account_only_retry_active {
                    disarm_pool_early_phase_cleanup_guard(early_phase_cleanup_guard);
                    return Err(take_same_account_only_terminal_error(
                        context.last_error,
                        *context.attempt_count,
                        distinct_account_count,
                    ));
                }
                *context.overload_required_upstream_route_key =
                    Some(context.upstream_route_key.clone());
                disarm_pool_early_phase_cleanup_guard(early_phase_cleanup_guard);
                if let Some(final_error) = take_priority_handoff_terminal_error(
                    &context.account,
                    context.last_error,
                    *context.attempt_count,
                    distinct_account_count,
                ) {
                    return Err(final_error);
                }
                return Ok(PoolAttemptResponseOutcome::Failover);
            }
            Err(err) => {
                let message = format!("failed to gate initial upstream response: {err}");
                let finished_at = shanghai_now_string();
                if let Some(pending_attempt_record) = pending_attempt_record.as_ref()
                    && let Err(record_err) = finalize_pool_upstream_request_attempt_in_memory(
                        context.state.as_ref(),
                        pending_attempt_record,
                        finished_at.as_str(),
                        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_TRANSPORT_FAILURE,
                        None,
                        None,
                        Some(PROXY_FAILURE_UPSTREAM_STREAM_ERROR),
                        Some(message.as_str()),
                        None,
                        Some(connect_latency_ms),
                        Some(first_byte_latency_ms),
                        None,
                        None,
                        None,
                        None,
                    )
                    .await
                {
                    warn!(
                        invoke_id = pending_attempt_record.invoke_id,
                        error = %record_err,
                        "failed to persist first-event gate failure attempt"
                    );
                }
                if let Some(pending_attempt_record) = pending_attempt_record.as_ref()
                    && let Err(err) = broadcast_pool_upstream_attempts_snapshot(
                        context.state.as_ref(),
                        &pending_attempt_record.invoke_id,
                    )
                    .await
                {
                    warn!(
                        invoke_id = pending_attempt_record.invoke_id,
                        error = %err,
                        "failed to broadcast first-event gate failure snapshot"
                    );
                }
                if let Err(route_err) = context
                    .reservation_guard
                    .as_mut()
                    .expect("pool routing reservation should be active")
                    .fence_failure(record_runtime_pool_route_transport_failure(
                        context.state.as_ref(),
                        context.account.account_id,
                        context.sticky_key,
                        &message,
                        PROXY_FAILURE_UPSTREAM_STREAM_ERROR,
                        context.trace_context.map(|trace| trace.invoke_id.as_str()),
                        pending_attempt_record
                            .as_ref()
                            .and_then(|pending| pending.attempt_id),
                        pending_attempt_record.as_ref().and_then(|pending| {
                            pending
                                .request_model
                                .as_deref()
                                .or(pending.upstream_request_model.as_deref())
                        }),
                    ))
                    .await
                {
                    warn!(account_id = context.account.account_id, error = %route_err, "failed to record first-event gate transport failure");
                }
                store_pool_failover_error(
                    context.last_error,
                    &mut *context.preserve_sticky_owner_terminal_error,
                    PoolUpstreamError {
                        codex_imagegen_rewrite: attempted_codex_imagegen_rewrite.clone(),
                        account: Some(context.account.clone()),
                        status: StatusCode::BAD_GATEWAY,
                        message: message.clone(),
                        canonical_error_message: None,
                        failure_kind: PROXY_FAILURE_UPSTREAM_STREAM_ERROR,
                        blocked_binding: None,
                        connect_latency_ms,
                        upstream_error_code: None,
                        upstream_error_message: None,
                        downstream_error_message: None,
                        upstream_request_id: None,
                        proxy_binding_key_snapshot: if let Some((_, selected_proxy)) =
                            forward_proxy_selection.as_ref()
                        {
                            canonical_pool_attempt_proxy_binding_key(
                                context.state.as_ref(),
                                selected_proxy.key.as_str(),
                            )
                            .await
                        } else {
                            None
                        },
                        oauth_responses_debug: oauth_responses_debug.clone(),
                        attempt_summary: PoolAttemptSummary::default(),
                        requested_service_tier: attempted_requested_service_tier.clone(),
                        request_body_for_capture: attempted_request_body_for_capture.clone(),
                    },
                    attempted_codex_imagegen_rewrite.as_ref(),
                );
                *context.exhausted_accounts_all_rate_limited = false;
                disarm_pool_early_phase_cleanup_guard(early_phase_cleanup_guard);
                if let Some(final_error) = take_priority_handoff_terminal_error(
                    &context.account,
                    context.last_error,
                    *context.attempt_count,
                    distinct_account_count,
                ) {
                    return Err(final_error);
                }
                if *context.same_account_only_retry_active {
                    return Err(take_same_account_only_terminal_error(
                        context.last_error,
                        *context.attempt_count,
                        distinct_account_count,
                    ));
                }
                return Ok(PoolAttemptResponseOutcome::Failover);
            }
        };

    let deferred_early_phase_cleanup_guard =
        if let Some(pending_attempt_record) = pending_attempt_record.as_ref() {
            enqueue_pool_streaming_phase_and_defer_guard(
                context.state.as_ref(),
                pending_attempt_record,
                early_phase_cleanup_guard,
                connect_latency_ms,
                first_byte_latency_ms,
            )
            .await
        } else {
            disarm_pool_early_phase_cleanup_guard(early_phase_cleanup_guard);
            None
        };

    let compact_support_observation =
        classify_compact_support_observation(context.original_uri, Some(status), None);
    if let Some(observation) = compact_support_observation.as_ref()
        && let Err(observation_err) = record_compact_support_observation_with_observed_at(
            &context.state.pool,
            context.account.account_id,
            observation.status,
            observation.reason.as_deref(),
            Some(&attempt_observed_at),
        )
        .await
    {
        warn!(
            account_id = context.account.account_id,
            error = %observation_err,
            "failed to record compact support observation"
        );
    }

    if let Some((forward_proxy_scope, selected_proxy)) = forward_proxy_selection.as_ref() {
        record_pool_account_forward_proxy_result(
            context.state.as_ref(),
            forward_proxy_scope,
            selected_proxy,
            ForwardProxyRouteResultKind::CompletedRequest,
        )
        .await;
    }
    Ok(PoolAttemptResponseOutcome::Response(Box::new(
        PoolUpstreamResponse {
            account: context.account.clone(),
            response,
            transport_bytes_live_counted,
            stream_timeout: context.stream_timeout,
            oauth_responses_debug,
            connect_latency_ms,
            attempt_started_at_utc,
            first_byte_latency_ms,
            first_chunk,
            first_chunk_received_at,
            first_stream_chunk_received_at,
            pending_attempt_record: pending_attempt_record.take().map(|mut pending| {
                pending.connect_latency_ms = connect_latency_ms;
                pending.first_byte_latency_ms = first_byte_latency_ms;
                pending.compact_support_status = compact_support_observation
                    .as_ref()
                    .map(|value| value.status.to_string());
                pending.compact_support_reason = compact_support_observation
                    .as_ref()
                    .and_then(|value| value.reason.clone());
                enqueue_pool_upstream_request_attempt_snapshot(context.state.as_ref(), &pending);
                pending
            }),
            deferred_early_phase_cleanup_guard,
            live_attempt_activity_lease,
            attempt_summary: pool_attempt_summary(
                *context.attempt_count,
                distinct_account_count,
                None,
            ),
            requested_service_tier: attempted_requested_service_tier,
            request_body_for_capture: attempted_request_body_for_capture,
            codex_imagegen_rewrite: attempted_codex_imagegen_rewrite,
            reservation_guard: context.reservation_guard.take(),
        },
    )))
}
