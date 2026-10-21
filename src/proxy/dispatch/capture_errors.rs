use super::*;
#[derive(Debug)]
pub(crate) struct ProxyCaptureDispatchError {
    pub(crate) status: StatusCode,
    pub(crate) message: String,
    pub(crate) invoke_id: Option<String>,
}

pub(crate) fn capture_target_for_request(
    path: &str,
    method: &Method,
) -> Option<ProxyCaptureTarget> {
    if *method != Method::POST {
        return None;
    }
    match path {
        "/v1/chat/completions" => Some(ProxyCaptureTarget::ChatCompletions),
        "/v1/responses" => Some(ProxyCaptureTarget::Responses),
        "/v1/responses/compact" => Some(ProxyCaptureTarget::ResponsesCompact),
        "/v1/alpha/search" => Some(ProxyCaptureTarget::StandaloneSearch),
        "/v1/images/generations" => Some(ProxyCaptureTarget::ImageGenerations),
        "/v1/images/edits" => Some(ProxyCaptureTarget::ImageEdits),
        _ => None,
    }
}

pub(crate) async fn emit_admitted_proxy_capture_runtime_snapshot(
    state: &AppState,
    invoke_id: &str,
    occurred_at: &str,
    capture_target: ProxyCaptureTarget,
    requester_ip: Option<&str>,
    sticky_key: Option<&str>,
    prompt_cache_key: Option<&str>,
) {
    let shell_started = Instant::now();
    let admitted_running_record = build_admitted_proxy_capture_runtime_snapshot(
        invoke_id,
        occurred_at,
        capture_target,
        requester_ip,
        sticky_key,
        prompt_cache_key,
    );
    if let Err(err) =
        persist_and_broadcast_proxy_capture_runtime_snapshot(state, admitted_running_record).await
    {
        warn!(
            ?err,
            invoke_id = %invoke_id,
            "failed to broadcast admitted running proxy capture snapshot"
        );
    } else {
        debug!(
            invoke_id = %invoke_id,
            occurred_at = %occurred_at,
            running_shell_emitted = true,
            running_shell_emit_elapsed = shell_started.elapsed().as_millis() as u64,
            "admitted proxy capture handler emitted running shell"
        );
    }
}

pub(crate) fn build_local_capture_error_envelope(
    invoke_id: &str,
    status: StatusCode,
    message: &str,
) -> ProxyErrorResponseEnvelope {
    build_proxy_error_response_envelope(
        &ProxyErrorResponse {
            status,
            message: message.to_string(),
            cvm_id: Some(invoke_id.to_string()),
            retry_after_secs: retry_after_secs_for_proxy_error(status, message),
            code: None,
            blocked_binding: None,
        },
        invoke_id,
    )
}

pub(crate) fn build_local_capture_error_resp_raw(
    envelope: &ProxyErrorResponseEnvelope,
) -> RawPayloadMeta {
    RawPayloadMeta {
        size_bytes: envelope.body_text.len() as i64,
        ..RawPayloadMeta::default()
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn persist_pre_attempt_proxy_capture_error(
    state: Arc<AppState>,
    proxy_request_id: u64,
    capture_started: Instant,
    invoke_id: &str,
    occurred_at: &str,
    capture_target: ProxyCaptureTarget,
    request_info: &RequestCaptureInfo,
    requester_ip: Option<&str>,
    request_chain_metadata: &RequestChainMetadata,
    sticky_key: Option<&str>,
    prompt_cache_key: Option<&str>,
    client_attribution_context: &ClientPromptCacheAttributionContext,
    request_body_for_capture: Option<Bytes>,
    request_body_snapshot: PoolReplayBodySnapshot,
    request_body_logging_enabled: bool,
    t_req_read_ms: f64,
    t_req_parse_ms: f64,
    status: StatusCode,
    failure_kind: &'static str,
    error_message: &str,
    terminal_attempt_summary: Option<&PoolAttemptSummary>,
    terminal_account: Option<&PoolResolvedAccount>,
    terminal_connect_latency_ms: Option<f64>,
    terminal_error: Option<&PoolUpstreamError>,
    terminal_request_compression_algorithm: Option<&str>,
    response_envelope_override: Option<ProxyErrorResponseEnvelope>,
) -> bool {
    let response_envelope = response_envelope_override
        .unwrap_or_else(|| build_local_capture_error_envelope(invoke_id, status, error_message));
    let req_raw = match request_body_for_capture {
        Some(bytes) => spawn_raw_payload_file_write(
            state.as_ref(),
            invoke_id,
            "request",
            bytes,
            request_body_logging_enabled,
        ),
        None => spawn_raw_payload_snapshot_write(
            state.clone(),
            invoke_id,
            "request",
            request_body_snapshot,
            request_body_logging_enabled,
        ),
    }
    .finish()
    .await;
    let usage = ParsedUsage::default();
    let (cost, cost_estimated, price_version) = estimate_proxy_cost_from_shared_catalog(
        &state.pricing_catalog,
        request_info.model.as_deref(),
        &usage,
        None,
        ProxyPricingMode::ResponseTier,
    )
    .await;
    let mut record = ProxyCaptureRecord {
        invoke_id: invoke_id.to_string(),
        occurred_at: occurred_at.to_string(),
        model: request_info.model.clone(),
        usage,
        cost,
        cost_breakdown: None,
        cost_estimated,
        price_version,
        status: if status.is_server_error() {
            format!("http_{}", status.as_u16())
        } else {
            "failed".to_string()
        },
        error_message: Some(format!("[{failure_kind}] {error_message}")),
        failure_kind: Some(failure_kind.to_string()),
        payload: Some(build_proxy_payload_summary(ProxyPayloadSummary {
            target: capture_target,
            status,
            is_stream: request_info.is_stream,
            request_contains_encrypted_content: request_info.contains_encrypted_content,
            response_contains_encrypted_content: false,
            compaction_request_kind: request_info.compaction_request_kind,
            compaction_response_kind: None,
            image_intent: request_info.image_intent.as_deref(),
            request_model: request_info.model.as_deref(),
            requested_service_tier: request_info.requested_service_tier.as_deref(),
            billing_service_tier: None,
            reasoning_effort: request_info.reasoning_effort.as_deref(),
            response_model: None,
            usage_missing_reason: None,
            request_parse_error: request_info.parse_error.as_deref(),
            request_compression_algorithm: terminal_request_compression_algorithm,
            request_compression_mode: None,
            request_compression_logical_body_bytes: None,
            request_compression_transmitted_body_bytes: None,
            request_compression_transmission_complete: None,
            failure_kind: Some(failure_kind),
            requester_ip,
            request_user_agent: request_chain_metadata.user_agent.as_deref(),
            request_x_forwarded_for: request_chain_metadata.x_forwarded_for.as_deref(),
            request_forwarded: request_chain_metadata.forwarded.as_deref(),
            request_x_real_ip: request_chain_metadata.x_real_ip.as_deref(),
            upstream_scope: INVOCATION_UPSTREAM_SCOPE_INTERNAL,
            route_mode: INVOCATION_ROUTE_MODE_POOL,
            sticky_key,
            prompt_cache_key,
            prompt_cache_key_attribution_source: request_info
                .prompt_cache_key_attribution_source
                .as_deref(),
            client_fingerprint: client_attribution_context.fingerprint.as_deref(),
            client_header_fingerprints: Some(&client_attribution_context.header_fingerprints)
                .filter(|fingerprints| !fingerprints.is_empty()),
            upstream_account_id: terminal_account.map(|account| account.account_id),
            upstream_account_name: terminal_account.map(|account| account.display_name.as_str()),
            upstream_account_kind: terminal_account.map(|account| account.kind.as_str()),
            upstream_base_url_host: terminal_account
                .and_then(|account| account.upstream_base_url.host_str()),
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
            upstream_error_code: terminal_error
                .and_then(|error| error.upstream_error_code.as_deref()),
            upstream_error_message: terminal_error
                .and_then(|error| error.upstream_error_message.as_deref()),
            downstream_status_code: Some(status),
            downstream_error_message: Some(error_message),
            upstream_request_id: terminal_error
                .and_then(|error| error.upstream_request_id.as_deref()),
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
            proxy_display_name: None,
            proxy_weight_delta: None,
            pool_attempt_count: terminal_attempt_summary.map(|summary| summary.pool_attempt_count),
            pool_distinct_account_count: terminal_attempt_summary
                .map(|summary| summary.pool_distinct_account_count),
            pool_attempt_terminal_reason: terminal_attempt_summary
                .and_then(|summary| summary.pool_attempt_terminal_reason.as_deref())
                .or(Some(failure_kind)),
            blocked_binding: terminal_error.and_then(|error| error.blocked_binding.as_ref()),
        })),
        raw_response: response_envelope.body_text.clone(),
        response_body_preview_enabled: true,
        req_raw,
        resp_raw: build_local_capture_error_resp_raw(&response_envelope),
        timings: StageTimings {
            t_total_ms: capture_started.elapsed().as_secs_f64() * 1_000.0,
            t_req_read_ms,
            t_req_parse_ms,
            t_upstream_connect_ms: terminal_connect_latency_ms.unwrap_or_default(),
            t_upstream_ttfb_ms: 0.0,
            first_token_ms: None,
            t_upstream_stream_ms: 0.0,
            t_resp_parse_ms: 0.0,
            t_persist_ms: 0.0,
        },
    };
    set_proxy_capture_record_pool_routing_no_candidate_audit(
        &mut record,
        terminal_attempt_summary
            .and_then(|summary| summary.pool_routing_no_candidate_audit.as_ref()),
    );
    if let Err(err) =
        persist_and_broadcast_proxy_capture(state.as_ref(), capture_started, record).await
    {
        warn!(
            proxy_request_id,
            error = %err,
            failure_kind,
            "failed to persist pre-attempt proxy capture terminal record"
        );
        false
    } else {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn standalone_search_capture_and_capability_require_post() {
        assert_eq!(
            capture_target_for_request("/v1/alpha/search", &Method::GET),
            None
        );
        assert_eq!(
            capture_target_for_request("/v1/alpha/search", &Method::POST),
            Some(ProxyCaptureTarget::StandaloneSearch)
        );
        assert!(
            !RequestCapabilityRequirements::from_endpoint_and_image_intent_for_method(
                "/v1/alpha/search",
                ImageIntent::No,
                false,
            )
            .standalone_search
        );
        assert!(
            RequestCapabilityRequirements::from_endpoint_and_image_intent_for_method(
                "/v1/alpha/search",
                ImageIntent::No,
                true,
            )
            .standalone_search
        );
    }
}
