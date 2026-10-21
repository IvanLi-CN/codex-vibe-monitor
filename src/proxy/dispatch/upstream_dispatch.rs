use super::*;

pub(crate) struct UpstreamProxyCapture {
    pub(crate) prepared: PreparedProxyCaptureRequest,
    pub(crate) selected_proxy: Option<SelectedForwardProxy>,
    pub(crate) pool_account: Option<PoolResolvedAccount>,
    pub(crate) t_upstream_connect_ms: f64,
    pub(crate) prefetched_first_chunk: Option<Bytes>,
    pub(crate) prefetched_first_chunk_received_at: Option<Instant>,
    pub(crate) prefetched_first_stream_chunk_received_at: Option<Instant>,
    pub(crate) prefetched_stream_timeout: Option<Duration>,
    pub(crate) prefetched_ttfb_ms: f64,
    pub(crate) oauth_responses_debug: Option<oauth_bridge::OauthResponsesDebugInfo>,
    pub(crate) attempt_already_recorded: bool,
    pub(crate) final_attempt_update: Option<ForwardProxyAttemptUpdate>,
    pub(crate) pending_pool_attempt_record: Option<PendingPoolAttemptRecord>,
    pub(crate) deferred_pool_early_phase_cleanup_guard: Option<PoolEarlyPhaseOrphanCleanupGuard>,
    pub(crate) live_pool_attempt_activity_lease: Option<PoolLiveAttemptActivityLease>,
    pub(crate) pending_pool_attempt_summary: PoolAttemptSummary,
    pub(crate) upstream_attempt_started_at: Option<Instant>,
    pub(crate) upstream_attempt_started_at_utc: Option<DateTime<Utc>>,
    pub(crate) direct_http_approx: ForwardProxyHttpApproxObservation,
    pub(crate) final_request_body_for_capture: Option<Bytes>,
    pub(crate) final_requested_service_tier: Option<String>,
    pub(crate) final_codex_imagegen_rewrite: Option<Value>,
    pub(crate) upstream_response: ProxyUpstreamResponseBody,
    pub(crate) transport_bytes_live_counted: bool,
    pub(crate) reservation_guard: Option<PoolRoutingReservationDropGuard>,
    pub(crate) image_tool_rewrite: Option<Value>,
    pub(crate) req_raw_pending: Option<PendingRawPayloadWrite>,
    pub(crate) upstream_status: StatusCode,
    pub(crate) rewritten_location: Option<String>,
    pub(crate) first_byte_timeout: Duration,
    pub(crate) stream_timeout: Option<Duration>,
}

pub(crate) async fn dispatch_proxy_upstream(
    PreparedProxyCaptureRequest {
        state,
        proxy_request_id,
        original_uri: original_uri_owned,
        headers,
        capture_target,
        target_url,
        pool_route_active,
        runtime_timeouts,
        proxy_request_permit,
        downstream_request_observer,
        proxy_request_started_at,
        capture_started,
        pool_routing_reservation_key,
        requester_ip,
        request_chain_metadata,
        sticky_key,
        prompt_cache_key,
        client_attribution_context,
        proxy_settings,
        mut request_info,
        semantic_projection,
        prompt_cache_binding_constraint,
        encrypted_owner_auto_guard_active,
        prompt_cache_conversation_override,
        pool_attempt_trace_context,
        t_req_read_ms,
        t_req_parse_ms,
        invoke_id,
        occurred_at,
        mut pool_invocation_cleanup_guard,
    }: PreparedProxyCaptureRequest,
) -> Result<UpstreamProxyCapture, ProxyCaptureDispatchError> {
    let original_uri = &original_uri_owned;
    let body_rewritten = semantic_projection.body_rewritten;
    let upstream_body_snapshot = semantic_projection.upstream_snapshot.clone();
    let mut upstream_headers = headers.clone();
    if body_rewritten {
        upstream_headers.remove(header::CONTENT_LENGTH);
    }
    // Preserve the client's response-encoding negotiation. TTFT inspection decodes a copy of
    // the upstream bytes below; forwarded bytes must retain the negotiated representation.
    let pool_attempt_runtime_snapshot =
        pool_route_active.then(|| PoolAttemptRuntimeSnapshotContext {
            capture_target,
            request_info: request_info.clone(),
            hosted_image_intent: Some(semantic_projection.hosted_image_intent),
            prompt_cache_key: prompt_cache_key.clone(),
            owner_auto_guard_active: encrypted_owner_auto_guard_active,
            t_req_read_ms,
            t_req_parse_ms,
        });
    let handshake_timeout =
        proxy_upstream_send_timeout_for_capture_target(&runtime_timeouts, Some(capture_target));
    let first_byte_timeout =
        pool_upstream_first_chunk_timeout(&runtime_timeouts, original_uri, &Method::POST);
    let stream_timeout = proxy_capture_target_stream_timeout(&runtime_timeouts, capture_target);
    let (
        selected_proxy,
        pool_account,
        t_upstream_connect_ms,
        prefetched_first_chunk,
        prefetched_first_chunk_received_at,
        prefetched_first_stream_chunk_received_at,
        prefetched_stream_timeout,
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
        direct_http_approx,
        final_request_body_for_capture,
        final_requested_service_tier,
        final_codex_imagegen_rewrite,
        upstream_response,
        transport_bytes_live_counted,
        reservation_guard,
    ) = if pool_route_active {
        match send_pool_request_with_failover_and_binding_constraint(
            state.clone(),
            proxy_request_id,
            Method::POST,
            original_uri,
            &upstream_headers,
            Some(upstream_body_snapshot),
            handshake_timeout,
            pool_attempt_trace_context.clone(),
            pool_attempt_runtime_snapshot.clone(),
            sticky_key.as_deref(),
            request_info.sticky_key.as_deref(),
            prompt_cache_binding_constraint.clone(),
            prompt_cache_conversation_override.clone(),
            None,
            PoolFailoverProgress::default(),
            POOL_UPSTREAM_SAME_ACCOUNT_MAX_ATTEMPTS,
            false,
        )
        .await
        {
            Ok(response) => (
                None,
                Some(response.account),
                response.connect_latency_ms,
                response.first_chunk,
                response.first_chunk_received_at,
                response.first_stream_chunk_received_at,
                response.stream_timeout,
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
                ForwardProxyHttpApproxObservation::default(),
                response.request_body_for_capture,
                response.requested_service_tier,
                response.codex_imagegen_rewrite,
                response.response,
                response.transport_bytes_live_counted,
                response.reservation_guard,
            ),
            Err(err) => {
                let response_envelope =
                    build_local_capture_error_envelope(&invoke_id, err.status, &err.message);
                drop(proxy_request_permit);
                request_info.requested_service_tier = err
                    .requested_service_tier
                    .clone()
                    .or(request_info.requested_service_tier);
                let req_raw = match err.request_body_for_capture.clone() {
                    Some(bytes) => spawn_raw_payload_file_write(
                        state.as_ref(),
                        &invoke_id,
                        "request",
                        bytes,
                        proxy_settings.request_body_logging_enabled,
                    ),
                    None => spawn_raw_payload_snapshot_write(
                        state.clone(),
                        &invoke_id,
                        "request",
                        semantic_projection.upstream_snapshot.clone(),
                        proxy_settings.request_body_logging_enabled,
                    ),
                }
                .finish()
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
                let image_tool_rewrite = err.account.as_ref().and_then(|account| {
                    image_tool_rewrite_audit(
                        capture_target,
                        is_openai_responses_lite_request(&headers),
                        account.image_tool_rewrite_mode,
                    )
                });
                let codex_imagegen_rewrite = err.codex_imagegen_rewrite.clone();
                let record = ProxyCaptureRecord {
                    invoke_id: invoke_id.clone(),
                    occurred_at,
                    model: request_info.model.clone(),
                    usage,
                    cost,
                    cost_breakdown: None,
                    cost_estimated,
                    price_version,
                    status: if err.status.is_server_error() {
                        format!("http_{}", err.status.as_u16())
                    } else {
                        "failed".to_string()
                    },
                    error_message: Some(error_message),
                    failure_kind: Some(err.failure_kind.to_string()),
                    payload: Some(with_codex_imagegen_rewrite_payload_summary(
                        with_image_tool_rewrite_payload_summary(
                            build_proxy_payload_summary(ProxyPayloadSummary {
                                target: capture_target,
                                status: err.status,
                                is_stream: request_info.is_stream,
                                request_contains_encrypted_content: request_info
                                    .contains_encrypted_content,
                                response_contains_encrypted_content: false,
                                compaction_request_kind: request_info.compaction_request_kind,
                                compaction_response_kind: None,
                                image_intent: request_info.image_intent.as_deref(),
                                request_model: request_info.model.as_deref(),
                                requested_service_tier: request_info
                                    .requested_service_tier
                                    .as_deref(),
                                billing_service_tier: billing_service_tier.as_deref(),
                                reasoning_effort: request_info.reasoning_effort.as_deref(),
                                response_model: None,
                                usage_missing_reason: None,
                                request_parse_error: request_info.parse_error.as_deref(),
                                request_compression_algorithm: None,
                                request_compression_mode: None,
                                request_compression_logical_body_bytes: None,
                                request_compression_transmitted_body_bytes: None,
                                request_compression_transmission_complete: None,
                                failure_kind: Some(err.failure_kind),
                                requester_ip: requester_ip.as_deref(),
                                request_user_agent: request_chain_metadata.user_agent.as_deref(),
                                request_x_forwarded_for: request_chain_metadata
                                    .x_forwarded_for
                                    .as_deref(),
                                request_forwarded: request_chain_metadata.forwarded.as_deref(),
                                request_x_real_ip: request_chain_metadata.x_real_ip.as_deref(),
                                upstream_scope: INVOCATION_UPSTREAM_SCOPE_INTERNAL,
                                route_mode: INVOCATION_ROUTE_MODE_POOL,
                                sticky_key: sticky_key.as_deref(),
                                prompt_cache_key: prompt_cache_key.as_deref(),
                                prompt_cache_key_attribution_source: request_info
                                    .prompt_cache_key_attribution_source
                                    .as_deref(),
                                client_fingerprint: client_attribution_context
                                    .fingerprint
                                    .as_deref(),
                                client_header_fingerprints: Some(
                                    &client_attribution_context.header_fingerprints,
                                )
                                .filter(|fingerprints| !fingerprints.is_empty()),
                                upstream_account_id: err
                                    .account
                                    .as_ref()
                                    .map(|account| account.account_id),
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
                                oauth_account_header_attached:
                                    oauth_account_header_attached_for_account(err.account.as_ref()),
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
                                    .and_then(|debug| {
                                        debug.request_body_prefix_fingerprint.as_deref()
                                    }),
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
                                downstream_status_code: Some(err.status),
                                downstream_error_message: Some(err.message.as_str()),
                                upstream_request_id: err.upstream_request_id.as_deref(),
                                response_content_encoding: None,
                                stream_failure_origin: None,
                                upstream_read_error_kind: None,
                                content_encoding_chain: None,
                                forwarded_chunk_count: None,
                                forwarded_bytes: None,
                                usage_observed: None,
                                downstream_close_phase: None,
                                downstream_write_error_kind: None,
                                last_upstream_chunk_gap_ms: None,
                                upstream_approx_upload_bytes: None,
                                upstream_approx_download_bytes: None,
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
                                blocked_binding: err.blocked_binding.as_ref(),
                            }),
                            image_tool_rewrite.as_ref(),
                        ),
                        codex_imagegen_rewrite.as_ref(),
                    )),
                    raw_response: response_envelope.body_text.clone(),
                    response_body_preview_enabled: true,
                    req_raw,
                    resp_raw: build_local_capture_error_resp_raw(&response_envelope),
                    timings: StageTimings {
                        t_total_ms: 0.0,
                        t_req_read_ms,
                        t_req_parse_ms,
                        t_upstream_connect_ms: err.connect_latency_ms,
                        t_upstream_ttfb_ms: 0.0,
                        first_token_ms: None,
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
                return Err(ProxyCaptureDispatchError {
                    status: err.status,
                    message: err.message,
                    invoke_id: Some(invoke_id.clone()),
                });
            }
        }
    } else {
        match send_forward_proxy_request_with_429_retry(
            state.clone(),
            Method::POST,
            target_url.clone(),
            &upstream_headers,
            Some(upstream_body_snapshot.clone()),
            Some(UpstreamTrafficReporter::new(
                state.clone(),
                invoke_id.as_str(),
                occurred_at.as_str(),
                None,
                state.config.openai_upstream_base_url.host_str(),
                capture_target.endpoint(),
            )),
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
                None,
                None,
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
                response.http_approx,
                semantic_projection.request_body_for_capture.clone(),
                request_info.requested_service_tier.clone(),
                None,
                response.response,
                true,
                None,
            ),
            Err(err) => {
                let response_envelope =
                    build_local_capture_error_envelope(&invoke_id, err.status, &err.message);
                drop(proxy_request_permit);
                let req_raw = spawn_raw_payload_snapshot_write(
                    state.clone(),
                    &invoke_id,
                    "request",
                    semantic_projection.upstream_snapshot.clone(),
                    proxy_settings.request_body_logging_enabled,
                )
                .finish()
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
                    invoke_id: invoke_id.clone(),
                    occurred_at,
                    model: request_info.model.clone(),
                    usage,
                    cost,
                    cost_breakdown: None,
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
                        request_contains_encrypted_content: request_info.contains_encrypted_content,
                        response_contains_encrypted_content: false,
                        compaction_request_kind: request_info.compaction_request_kind,
                        compaction_response_kind: None,
                        image_intent: request_info.image_intent.as_deref(),
                        request_model: request_info.model.as_deref(),
                        requested_service_tier: request_info.requested_service_tier.as_deref(),
                        billing_service_tier: billing_service_tier.as_deref(),
                        reasoning_effort: request_info.reasoning_effort.as_deref(),
                        response_model: None,
                        usage_missing_reason: None,
                        request_parse_error: request_info.parse_error.as_deref(),
                        request_compression_algorithm: err
                            .http_approx
                            .request_compression
                            .as_ref()
                            .map(|value| value.algorithm.as_str()),
                        request_compression_mode: err
                            .http_approx
                            .request_compression
                            .as_ref()
                            .map(|value| value.mode.as_str()),
                        request_compression_logical_body_bytes: err
                            .http_approx
                            .request_compression
                            .as_ref()
                            .map(|value| value.logical_body_bytes),
                        request_compression_transmitted_body_bytes: err
                            .http_approx
                            .request_compression
                            .as_ref()
                            .map(|value| value.transmitted_body_bytes),
                        request_compression_transmission_complete: Some(
                            err.http_approx.request_transmission_complete,
                        ),
                        failure_kind: Some(err.failure_kind),
                        requester_ip: requester_ip.as_deref(),
                        request_user_agent: request_chain_metadata.user_agent.as_deref(),
                        request_x_forwarded_for: request_chain_metadata.x_forwarded_for.as_deref(),
                        request_forwarded: request_chain_metadata.forwarded.as_deref(),
                        request_x_real_ip: request_chain_metadata.x_real_ip.as_deref(),
                        upstream_scope: INVOCATION_UPSTREAM_SCOPE_EXTERNAL,
                        route_mode: INVOCATION_ROUTE_MODE_FORWARD_PROXY,
                        sticky_key: sticky_key.as_deref(),
                        prompt_cache_key: prompt_cache_key.as_deref(),
                        prompt_cache_key_attribution_source: request_info
                            .prompt_cache_key_attribution_source
                            .as_deref(),
                        client_fingerprint: client_attribution_context.fingerprint.as_deref(),
                        client_header_fingerprints: Some(
                            &client_attribution_context.header_fingerprints,
                        )
                        .filter(|fingerprints| !fingerprints.is_empty()),
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
                        downstream_status_code: Some(err.status),
                        downstream_error_message: Some(err.message.as_str()),
                        upstream_request_id: None,
                        response_content_encoding: None,
                        stream_failure_origin: None,
                        upstream_read_error_kind: None,
                        content_encoding_chain: None,
                        forwarded_chunk_count: None,
                        forwarded_bytes: None,
                        usage_observed: None,
                        downstream_close_phase: None,
                        downstream_write_error_kind: None,
                        last_upstream_chunk_gap_ms: None,
                        upstream_approx_upload_bytes: Some(err.http_approx.approx_upload_bytes),
                        upstream_approx_download_bytes: Some(
                            err.http_approx.approx_download_bytes_before_response_body,
                        ),
                        proxy_display_name: Some(err.selected_proxy.display_name.as_str()),
                        proxy_weight_delta: proxy_attempt_update.delta(),
                        pool_attempt_count: None,
                        pool_distinct_account_count: None,
                        pool_attempt_terminal_reason: None,
                        blocked_binding: None,
                    })),
                    raw_response: response_envelope.body_text.clone(),
                    response_body_preview_enabled: true,
                    req_raw,
                    resp_raw: build_local_capture_error_resp_raw(&response_envelope),
                    timings: StageTimings {
                        t_total_ms: 0.0,
                        t_req_read_ms,
                        t_req_parse_ms,
                        t_upstream_connect_ms: err.connect_latency_ms,
                        t_upstream_ttfb_ms: 0.0,
                        first_token_ms: None,
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
                return Err(ProxyCaptureDispatchError {
                    status: err.status,
                    message: err.message,
                    invoke_id: Some(invoke_id.clone()),
                });
            }
        }
    };

    request_info.requested_service_tier = final_requested_service_tier
        .clone()
        .or(request_info.requested_service_tier);
    let image_tool_rewrite = pool_account.as_ref().and_then(|account| {
        image_tool_rewrite_audit(
            capture_target,
            is_openai_responses_lite_request(&headers),
            account.image_tool_rewrite_mode,
        )
    });
    let mut req_raw_pending = Some(match final_request_body_for_capture.clone() {
        Some(bytes) => spawn_raw_payload_file_write(
            state.as_ref(),
            &invoke_id,
            "request",
            bytes,
            proxy_settings.request_body_logging_enabled,
        ),
        None => spawn_raw_payload_snapshot_write(
            state.clone(),
            &invoke_id,
            "request",
            semantic_projection.upstream_snapshot.clone(),
            proxy_settings.request_body_logging_enabled,
        ),
    });

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
            let response_envelope =
                build_local_capture_error_envelope(&invoke_id, StatusCode::BAD_GATEWAY, &message);
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
            drop(proxy_request_permit);
            let req_raw = req_raw_pending
                .take()
                .expect("request raw capture should be pending before redirect normalization")
                .finish()
                .await;
            let record = ProxyCaptureRecord {
                invoke_id: invoke_id.clone(),
                occurred_at,
                model: request_info.model.clone(),
                usage,
                cost,
                cost_breakdown: None,
                cost_estimated,
                price_version,
                status: "http_502".to_string(),
                error_message: Some(message.clone()),
                failure_kind: None,
                payload: Some(build_proxy_payload_summary(ProxyPayloadSummary {
                    target: capture_target,
                    status: StatusCode::BAD_GATEWAY,
                    is_stream: request_info.is_stream,
                    request_contains_encrypted_content: request_info.contains_encrypted_content,
                    response_contains_encrypted_content: false,
                    compaction_request_kind: request_info.compaction_request_kind,
                    compaction_response_kind: None,
                    image_intent: request_info.image_intent.as_deref(),
                    request_model: request_info.model.as_deref(),
                    requested_service_tier: request_info.requested_service_tier.as_deref(),
                    billing_service_tier: billing_service_tier.as_deref(),
                    reasoning_effort: request_info.reasoning_effort.as_deref(),
                    response_model: None,
                    usage_missing_reason: None,
                    request_parse_error: request_info.parse_error.as_deref(),
                    request_compression_algorithm: None,
                    request_compression_mode: None,
                    request_compression_logical_body_bytes: None,
                    request_compression_transmitted_body_bytes: None,
                    request_compression_transmission_complete: None,
                    failure_kind: None,
                    requester_ip: requester_ip.as_deref(),
                    request_user_agent: request_chain_metadata.user_agent.as_deref(),
                    request_x_forwarded_for: request_chain_metadata.x_forwarded_for.as_deref(),
                    request_forwarded: request_chain_metadata.forwarded.as_deref(),
                    request_x_real_ip: request_chain_metadata.x_real_ip.as_deref(),
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
                    prompt_cache_key_attribution_source: request_info
                        .prompt_cache_key_attribution_source
                        .as_deref(),
                    client_fingerprint: client_attribution_context.fingerprint.as_deref(),
                    client_header_fingerprints: Some(
                        &client_attribution_context.header_fingerprints,
                    )
                    .filter(|fingerprints| !fingerprints.is_empty()),
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
                    downstream_status_code: Some(StatusCode::BAD_GATEWAY),
                    downstream_error_message: Some(message.as_str()),
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
                    stream_failure_origin: None,
                    upstream_read_error_kind: None,
                    content_encoding_chain: None,
                    forwarded_chunk_count: None,
                    forwarded_bytes: None,
                    usage_observed: None,
                    downstream_close_phase: None,
                    downstream_write_error_kind: None,
                    last_upstream_chunk_gap_ms: None,
                    upstream_approx_upload_bytes: None,
                    upstream_approx_download_bytes: None,
                    proxy_display_name: proxy_display_name.as_deref(),
                    proxy_weight_delta: if selected_proxy.is_some() {
                        proxy_attempt_update.delta()
                    } else {
                        None
                    },
                    pool_attempt_count: None,
                    pool_distinct_account_count: None,
                    pool_attempt_terminal_reason: None,
                    blocked_binding: None,
                })),
                raw_response: response_envelope.body_text.clone(),
                response_body_preview_enabled: true,
                req_raw,
                resp_raw: build_local_capture_error_resp_raw(&response_envelope),
                timings: StageTimings {
                    t_total_ms: 0.0,
                    t_req_read_ms,
                    t_req_parse_ms,
                    t_upstream_connect_ms,
                    t_upstream_ttfb_ms: 0.0,
                    first_token_ms: None,
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
            return Err(ProxyCaptureDispatchError {
                status: StatusCode::BAD_GATEWAY,
                message,
                invoke_id: Some(invoke_id.clone()),
            });
        }
    };

    let prepared = PreparedProxyCaptureRequest {
        state,
        proxy_request_id,
        original_uri: original_uri_owned,
        headers,
        capture_target,
        target_url,
        pool_route_active,
        runtime_timeouts,
        proxy_request_permit,
        downstream_request_observer,
        proxy_request_started_at,
        capture_started,
        pool_routing_reservation_key,
        requester_ip,
        request_chain_metadata,
        sticky_key,
        prompt_cache_key,
        client_attribution_context,
        proxy_settings,
        request_info,
        semantic_projection,
        prompt_cache_binding_constraint,
        encrypted_owner_auto_guard_active,
        prompt_cache_conversation_override,
        pool_attempt_trace_context,
        t_req_read_ms,
        t_req_parse_ms,
        invoke_id,
        occurred_at,
        pool_invocation_cleanup_guard,
    };
    Ok(UpstreamProxyCapture {
        prepared,
        selected_proxy,
        pool_account,
        t_upstream_connect_ms,
        prefetched_first_chunk,
        prefetched_first_chunk_received_at,
        prefetched_first_stream_chunk_received_at,
        prefetched_stream_timeout,
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
        direct_http_approx,
        final_request_body_for_capture,
        final_requested_service_tier,
        final_codex_imagegen_rewrite,
        upstream_response,
        transport_bytes_live_counted,
        reservation_guard,
        image_tool_rewrite,
        req_raw_pending,
        upstream_status,
        rewritten_location,
        first_byte_timeout,
        stream_timeout,
    })
}
