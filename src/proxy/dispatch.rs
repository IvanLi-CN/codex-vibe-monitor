use super::*;

pub(crate) fn proxy_stream_usage_observed(response_info: &ResponseCaptureInfo) -> bool {
    response_info.usage.total_tokens.is_some()
        || response_info.usage.input_tokens.is_some()
        || response_info.usage.output_tokens.is_some()
}

pub(crate) fn proxy_stream_failure_origin_from_usage_reason(
    usage_missing_reason: Option<&str>,
) -> Option<&'static str> {
    let reason = usage_missing_reason?;
    if reason.contains("response_decode_failed:") {
        Some("content_decode")
    } else if reason
        .split(';')
        .any(|part| part.trim().eq_ignore_ascii_case("stream_event_parse_error"))
    {
        Some("stream_parse")
    } else {
        None
    }
}

pub(crate) fn proxy_stream_upstream_read_error_kind(err: &io::Error) -> &'static str {
    if let Some(source) = err.get_ref()
        && let Some(reqwest_err) = source.downcast_ref::<reqwest::Error>()
    {
        if reqwest_err.is_timeout() {
            return "timeout";
        }
        if reqwest_err.is_decode() {
            return "decode";
        }
        if reqwest_err.is_body() {
            return "body";
        }
        if reqwest_err.is_request() {
            return "request";
        }
    }

    match err.kind() {
        io::ErrorKind::TimedOut => "timeout",
        io::ErrorKind::UnexpectedEof | io::ErrorKind::InvalidData => "decode",
        io::ErrorKind::ConnectionReset
        | io::ErrorKind::ConnectionAborted
        | io::ErrorKind::BrokenPipe => "connection",
        _ => "other",
    }
}

pub(crate) const PROXY_DOWNSTREAM_WRITE_ERROR_GRACE_PERIOD: Duration = Duration::from_secs(2);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum DownstreamBodyTerminalState {
    Open,
    SuccessfulTerminalForwarded,
    SuccessfulTerminalCompleted,
    Completed,
    Dropped,
}

pub(crate) struct DownstreamResponseChunk {
    bytes: Bytes,
    successful_terminal: bool,
}

pub(crate) struct TrackedDownstreamReceiverStream {
    inner: ReceiverStream<Result<DownstreamResponseChunk, io::Error>>,
    terminal_tx: watch::Sender<DownstreamBodyTerminalState>,
    terminal_state: DownstreamBodyTerminalState,
}

impl TrackedDownstreamReceiverStream {
    fn new(
        inner: ReceiverStream<Result<DownstreamResponseChunk, io::Error>>,
        terminal_tx: watch::Sender<DownstreamBodyTerminalState>,
    ) -> Self {
        Self {
            inner,
            terminal_tx,
            terminal_state: DownstreamBodyTerminalState::Open,
        }
    }

    fn mark_terminal(&mut self, state: DownstreamBodyTerminalState) {
        let state = if self.terminal_state
            == DownstreamBodyTerminalState::SuccessfulTerminalForwarded
            && state == DownstreamBodyTerminalState::Completed
        {
            DownstreamBodyTerminalState::SuccessfulTerminalCompleted
        } else {
            state
        };
        let should_update = matches!(
            (self.terminal_state, state),
            (
                DownstreamBodyTerminalState::Open,
                DownstreamBodyTerminalState::SuccessfulTerminalForwarded
                    | DownstreamBodyTerminalState::Completed
                    | DownstreamBodyTerminalState::Dropped
            ) | (
                DownstreamBodyTerminalState::SuccessfulTerminalForwarded,
                DownstreamBodyTerminalState::SuccessfulTerminalCompleted
                    | DownstreamBodyTerminalState::Dropped
            )
        );
        if should_update {
            self.terminal_state = state;
            let _ = self.terminal_tx.send(state);
        }
    }
}

impl futures_util::Stream for TrackedDownstreamReceiverStream {
    type Item = Result<Bytes, io::Error>;

    fn poll_next(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        match Pin::new(&mut self.inner).poll_next(cx) {
            std::task::Poll::Ready(Some(Ok(chunk))) => {
                if chunk.successful_terminal {
                    self.mark_terminal(DownstreamBodyTerminalState::SuccessfulTerminalForwarded);
                }
                std::task::Poll::Ready(Some(Ok(chunk.bytes)))
            }
            std::task::Poll::Ready(Some(Err(err))) => std::task::Poll::Ready(Some(Err(err))),
            std::task::Poll::Ready(None) => {
                self.mark_terminal(DownstreamBodyTerminalState::Completed);
                std::task::Poll::Ready(None)
            }
            std::task::Poll::Pending => std::task::Poll::Pending,
        }
    }
}

impl Drop for TrackedDownstreamReceiverStream {
    fn drop(&mut self) {
        if self.terminal_state == DownstreamBodyTerminalState::SuccessfulTerminalForwarded {
            self.mark_terminal(DownstreamBodyTerminalState::Completed);
        } else {
            self.mark_terminal(DownstreamBodyTerminalState::Dropped);
        }
    }
}

pub(crate) fn proxy_stream_observe_downstream_body_terminal(
    state: DownstreamBodyTerminalState,
    successful_terminal_forwarded: &mut bool,
    downstream_body_available: &mut bool,
    downstream_closed: &mut bool,
    downstream_write_error_kind: &mut Option<&'static str>,
    last_upstream_chunk_received_at: Option<Instant>,
    last_upstream_chunk_gap_ms: &mut Option<u64>,
) {
    match state {
        DownstreamBodyTerminalState::Open
        | DownstreamBodyTerminalState::SuccessfulTerminalCompleted
        | DownstreamBodyTerminalState::Completed => {}
        DownstreamBodyTerminalState::SuccessfulTerminalForwarded => {
            *successful_terminal_forwarded = true;
        }
        DownstreamBodyTerminalState::Dropped => {
            *downstream_body_available = false;
            *downstream_closed = true;
            downstream_write_error_kind.get_or_insert("body_dropped");
            if last_upstream_chunk_gap_ms.is_none() {
                *last_upstream_chunk_gap_ms = last_upstream_chunk_received_at
                    .map(|instant| instant.elapsed().as_millis() as u64);
            }
        }
    }
}

fn proxy_stream_write_error_is_post_terminal(successful_terminal_forwarded: bool) -> bool {
    successful_terminal_forwarded
}

pub(crate) async fn wait_for_downstream_body_terminal_until(
    downstream_body_terminal_rx: &mut watch::Receiver<DownstreamBodyTerminalState>,
    deadline: Instant,
    successful_terminal_forwarded: &mut bool,
    downstream_body_available: &mut bool,
    downstream_closed: &mut bool,
    downstream_write_error_kind: &mut Option<&'static str>,
    last_upstream_chunk_received_at: Option<Instant>,
    last_upstream_chunk_gap_ms: &mut Option<u64>,
) {
    loop {
        let state = *downstream_body_terminal_rx.borrow_and_update();
        match state {
            DownstreamBodyTerminalState::Open => {
                let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
                    break;
                };
                if tokio::time::timeout(remaining, downstream_body_terminal_rx.changed())
                    .await
                    .is_err()
                {
                    break;
                }
            }
            DownstreamBodyTerminalState::SuccessfulTerminalForwarded => {
                *successful_terminal_forwarded = true;
                let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
                    break;
                };
                if tokio::time::timeout(remaining, downstream_body_terminal_rx.changed())
                    .await
                    .is_err()
                {
                    break;
                }
            }
            DownstreamBodyTerminalState::SuccessfulTerminalCompleted => {
                *successful_terminal_forwarded = true;
                break;
            }
            DownstreamBodyTerminalState::Completed => break,
            DownstreamBodyTerminalState::Dropped => {
                proxy_stream_observe_downstream_body_terminal(
                    state,
                    successful_terminal_forwarded,
                    downstream_body_available,
                    downstream_closed,
                    downstream_write_error_kind,
                    last_upstream_chunk_received_at,
                    last_upstream_chunk_gap_ms,
                );
                break;
            }
        }
    }
}

pub(crate) async fn proxy_openai_v1_inner(
    state: Arc<AppState>,
    proxy_request_id: u64,
    invoke_id: String,
    original_uri: Uri,
    method: Method,
    headers: HeaderMap,
    body: Body,
    target_url: Url,
    peer_ip: Option<IpAddr>,
    pool_route_active: bool,
    runtime_timeouts: PoolRoutingTimeoutSettingsResolved,
    mut proxy_request_permit: Option<ProxyRequestConcurrencyPermit>,
    admitted_runtime_snapshot: Option<AdmittedProxyRuntimeSnapshot>,
    downstream_request_observer: Option<DownstreamRequestObserver>,
    proxy_request_started_at: Instant,
) -> Result<Response, ProxyErrorResponse> {
    if !pool_route_active {
        // `/v1/*` is pool-only; non-pool traffic must stop here instead of reviving the
        // removed reverse-proxy/direct path.
        return Err(ProxyErrorResponse {
            status: StatusCode::UNAUTHORIZED,
            message: PROXY_POOL_ROUTE_KEY_MISSING_OR_INVALID_MESSAGE.to_string(),
            cvm_id: None,
            retry_after_secs: None,
            code: None,
            blocked_binding: None,
        });
    }

    if method == Method::GET && is_models_list_path(original_uri.path()) {
        let proxy_settings = state.proxy_model_settings.read().await.clone();
        if proxy_settings.hijack_enabled {
            let mut payload = build_preset_models_payload(&proxy_settings.enabled_preset_models);
            let mut merge_status: Option<&'static str> = None;
            if proxy_settings.merge_upstream_enabled {
                match fetch_upstream_models_payload(
                    state.clone(),
                    target_url.clone(),
                    &headers,
                    proxy_settings.upstream_429_max_retries,
                )
                .await
                {
                    Ok(upstream_payload) => match merge_models_payload_with_upstream(
                        &upstream_payload,
                        &proxy_settings.enabled_preset_models,
                    ) {
                        Ok(merged_payload) => {
                            payload = merged_payload;
                            merge_status = Some(PROXY_MODEL_MERGE_STATUS_SUCCESS);
                        }
                        Err(err) => {
                            warn!(
                                proxy_request_id,
                                error = %err,
                                "failed to merge upstream model list; falling back to preset models"
                            );
                            merge_status = Some(PROXY_MODEL_MERGE_STATUS_FAILED);
                        }
                    },
                    Err(err) => {
                        warn!(
                            proxy_request_id,
                            error = %err,
                            "failed to fetch upstream model list for merge; falling back to preset models"
                        );
                        merge_status = Some(PROXY_MODEL_MERGE_STATUS_FAILED);
                    }
                }
            }

            let mut response = Json(payload).into_response();
            if let Some(status) = merge_status {
                response.headers_mut().insert(
                    HeaderName::from_static(PROXY_MODEL_MERGE_STATUS_HEADER),
                    HeaderValue::from_static(status),
                );
            }
            return Ok(response);
        }

        return proxy_openai_v1_via_pool(
            state,
            proxy_request_id,
            &original_uri,
            method,
            headers,
            body,
            runtime_timeouts,
            proxy_request_permit.take(),
        )
        .await;
    }

    if let Some(target) = capture_target_for_request(original_uri.path(), &method) {
        let tracked_invoke_id = invoke_id.clone();
        return Box::pin(proxy_openai_v1_capture_target(
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
            proxy_request_permit.take(),
            admitted_runtime_snapshot,
            downstream_request_observer,
            proxy_request_started_at,
        ))
        .await
        .map_err(|(status, message)| ProxyErrorResponse {
            retry_after_secs: retry_after_secs_for_proxy_error(status, &message),
            status,
            message,
            cvm_id: Some(tracked_invoke_id),
            code: None,
            blocked_binding: None,
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
        proxy_request_permit.take(),
    )
    .await;
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

fn build_local_capture_error_envelope(
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

fn build_local_capture_error_resp_raw(envelope: &ProxyErrorResponseEnvelope) -> RawPayloadMeta {
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
    live_request_streaming_decision: Option<&LiveRequestStreamingDecision>,
    live_request_streaming_measurement: Option<&LiveRequestStreamingMeasurement>,
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
        payload: Some(
            if let (Some(decision), Some(measurement)) = (
                live_request_streaming_decision,
                live_request_streaming_measurement,
            ) {
                with_live_request_streaming_payload_summary(
                    build_proxy_payload_summary(ProxyPayloadSummary {
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
                        client_header_fingerprints: Some(
                            &client_attribution_context.header_fingerprints,
                        )
                        .filter(|fingerprints| !fingerprints.is_empty()),
                        upstream_account_id: terminal_account.map(|account| account.account_id),
                        upstream_account_name: terminal_account
                            .map(|account| account.display_name.as_str()),
                        upstream_account_kind: terminal_account
                            .map(|account| account.kind.as_str()),
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
                        pool_attempt_count: terminal_attempt_summary
                            .map(|summary| summary.pool_attempt_count),
                        pool_distinct_account_count: terminal_attempt_summary
                            .map(|summary| summary.pool_distinct_account_count),
                        pool_attempt_terminal_reason: terminal_attempt_summary
                            .and_then(|summary| summary.pool_attempt_terminal_reason.as_deref())
                            .or(Some(failure_kind)),
                        blocked_binding: terminal_error
                            .and_then(|error| error.blocked_binding.as_ref()),
                    }),
                    decision,
                    measurement,
                )
            } else {
                build_proxy_payload_summary(ProxyPayloadSummary {
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
                    client_header_fingerprints: Some(
                        &client_attribution_context.header_fingerprints,
                    )
                    .filter(|fingerprints| !fingerprints.is_empty()),
                    upstream_account_id: terminal_account.map(|account| account.account_id),
                    upstream_account_name: terminal_account
                        .map(|account| account.display_name.as_str()),
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
                    pool_attempt_count: terminal_attempt_summary
                        .map(|summary| summary.pool_attempt_count),
                    pool_distinct_account_count: terminal_attempt_summary
                        .map(|summary| summary.pool_distinct_account_count),
                    pool_attempt_terminal_reason: terminal_attempt_summary
                        .and_then(|summary| summary.pool_attempt_terminal_reason.as_deref())
                        .or(Some(failure_kind)),
                    blocked_binding: terminal_error
                        .and_then(|error| error.blocked_binding.as_ref()),
                })
            },
        ),
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

struct PreparedCaptureRequestBody {
    request_body_snapshot_result: Result<PoolReplayBodySnapshot, RequestBodyReadError>,
    live_first_pool_response: Option<PoolUpstreamResponse>,
    prepared_live_request_streaming_decision: Option<LiveRequestStreamingDecision>,
    live_first_attempt_failed: bool,
    live_first_failed_account_id: Option<i64>,
    live_first_request_body_first_byte_at: Option<Instant>,
    live_oauth_rewrite_rx:
        Option<watch::Receiver<Option<oauth_bridge::OauthResponsesRewriteSummary>>>,
    live_first_experiment_group: Option<String>,
    live_route_finalization_measurement: Option<LiveRequestStreamingMeasurement>,
}

async fn wait_for_live_request_body_finalization(finalization_rx: &mut watch::Receiver<bool>) {
    if *finalization_rx.borrow() {
        return;
    }
    let _ = finalization_rx.changed().await;
}

async fn reject_live_first_provisional_response_after_body_failure(
    state: &AppState,
    response: &mut PoolUpstreamResponse,
    request_body_error: &RequestBodyReadError,
) {
    let upstream_status = response.response.status();
    let upstream_request_id = response
        .response
        .headers()
        .get("x-request-id")
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty());
    finalize_tracked_pool_attempt(
        state,
        response.pending_attempt_record.as_ref(),
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_TRANSPORT_FAILURE,
        Some(upstream_status),
        Some(request_body_error.status),
        Some(request_body_error.failure_kind),
        None,
        Some(request_body_error.message.as_str()),
        Some(response.connect_latency_ms),
        Some(response.first_byte_latency_ms),
        None,
        upstream_request_id,
        "live-first request body failed after provisional response",
    )
    .await;
    complete_deferred_pool_early_phase_cleanup_guard(
        &mut response.deferred_early_phase_cleanup_guard,
    );
}

#[allow(clippy::too_many_arguments)]
async fn prepare_capture_request_body(
    state: Arc<AppState>,
    proxy_request_id: u64,
    invoke_id: &str,
    occurred_at: &str,
    original_uri: &Uri,
    headers: &HeaderMap,
    body: Body,
    capture_target: ProxyCaptureTarget,
    body_limit: usize,
    runtime_timeouts: &PoolRoutingTimeoutSettingsResolved,
    requester_ip: Option<&str>,
    header_sticky_key: Option<&str>,
    header_prompt_cache_key: Option<&str>,
    req_read_started: Instant,
) -> PreparedCaptureRequestBody {
    let live_eligible = capture_target == ProxyCaptureTarget::Responses;

    if !live_eligible {
        return PreparedCaptureRequestBody {
            request_body_snapshot_result: read_request_body_snapshot_with_partial_limit(
                body,
                body_limit,
                runtime_timeouts.request_read_timeout,
                proxy_request_id,
            )
            .await,
            live_first_pool_response: None,
            prepared_live_request_streaming_decision: None,
            live_first_attempt_failed: false,
            live_first_failed_account_id: None,
            live_first_request_body_first_byte_at: None,
            live_oauth_rewrite_rx: None,
            live_first_experiment_group: None,
            live_route_finalization_measurement: None,
        };
    }

    let live_routing_snapshot = load_pool_routing_runtime_cache_with_status(state.as_ref())
        .await
        .ok();
    let live_routing_hot_cache_hit = live_routing_snapshot.as_ref().map(|(_, hit)| *hit);
    let live_routing_reservation_key = build_pool_routing_reservation_key(proxy_request_id);
    let live_settings = live_routing_snapshot
        .as_ref()
        .map(|(snapshot, _)| snapshot.live_request_streaming.clone());
    let Some(live_settings) = live_settings.filter(|settings| settings.enabled) else {
        return PreparedCaptureRequestBody {
            request_body_snapshot_result: read_request_body_snapshot_with_partial_limit(
                body,
                body_limit,
                runtime_timeouts.request_read_timeout,
                proxy_request_id,
            )
            .await,
            live_first_pool_response: None,
            prepared_live_request_streaming_decision: None,
            live_first_attempt_failed: false,
            live_first_failed_account_id: None,
            live_first_request_body_first_byte_at: None,
            live_oauth_rewrite_rx: None,
            live_first_experiment_group: None,
            live_route_finalization_measurement: None,
        };
    };

    // Cohort assignment is independent of request contents. Decide it before
    // attaching the new parser so the buffered control remains the existing
    // request path; only treatment pays for route-finalization analysis.
    let sampled_live_decision =
        decide_live_request_streaming(&live_settings, invoke_id, capture_target, true, true);
    if sampled_live_decision.variant != Some(LiveRequestStreamingExperimentVariant::Treatment) {
        return PreparedCaptureRequestBody {
            request_body_snapshot_result: read_request_body_snapshot_with_partial_limit(
                body,
                body_limit,
                runtime_timeouts.request_read_timeout,
                proxy_request_id,
            )
            .await,
            live_first_pool_response: None,
            prepared_live_request_streaming_decision: Some(sampled_live_decision),
            live_first_attempt_failed: false,
            live_first_request_body_first_byte_at: None,
            live_oauth_rewrite_rx: None,
            live_first_experiment_group: None,
            live_route_finalization_measurement: None,
        };
    }

    let encrypted_owner_routing_enabled = state
        .proxy_model_settings
        .read()
        .await
        .encrypted_session_owner_routing_enabled;

    let replayable_body = spawn_pool_replayable_request_body(
        body,
        body_limit,
        runtime_timeouts.request_read_timeout,
        proxy_request_id,
    );
    let downstream_content_encoding = headers
        .get(header::CONTENT_ENCODING)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    let mut live_pipeline = Some(spawn_live_responses_request_body_pipeline(
        replayable_body.body,
        downstream_content_encoding.clone(),
    ));
    let live_request_body_error_rx = live_pipeline
        .as_ref()
        .expect("live pipeline is present before the first attempt")
        .request_body_error_rx
        .clone();
    let mut live_request_body_finalization_rx = live_pipeline
        .as_ref()
        .expect("live pipeline is present before the first attempt")
        .finalization_rx
        .clone();
    let live_oauth_rewrite_rx = live_pipeline
        .as_ref()
        .expect("live pipeline is present before routing")
        .oauth_rewrite_rx
        .clone();
    let replay_status_rx = replayable_body.status_rx.clone();
    let replay_cancel = replayable_body.cancel.clone();
    let live_body_key_probe = wait_for_replay_body_sticky_key_probe(
        &live_pipeline
            .as_ref()
            .expect("live pipeline is present before routing")
            .routing_probe_rx,
        runtime_timeouts.request_read_timeout,
    )
    .await;
    let live_resolved_content_encoding = live_pipeline
        .as_ref()
        .and_then(|pipeline| *pipeline.resolved_request_content_encoding_rx.borrow());
    let live_route_finalization_ms = elapsed_ms(req_read_started);
    let response_timeout =
        pool_upstream_responses_total_timeout(&state.config, original_uri, &Method::POST);
    let live_body_sticky_key = live_body_key_probe
        .sticky_key
        .clone()
        .or_else(|| header_sticky_key.map(str::to_string));
    let live_prompt_cache_key = live_body_key_probe
        .prompt_cache_key
        .clone()
        .or_else(|| header_prompt_cache_key.map(str::to_string));
    let live_candidate = live_body_key_probe.model.is_some();
    let mut live_first_pool_response = None;
    let mut prepared_live_request_streaming_decision = None;
    let mut live_first_attempt_failed = false;
    let mut live_first_failed_account_id = None;
    let mut live_first_request_body_first_byte_at = None;
    let mut live_first_experiment_group = None;
    let mut live_route_lookup_cache_hit = live_routing_hot_cache_hit;

    if !live_route_probe_can_start_before_eof(&live_body_key_probe) {
        // The generic JSON contract accepts arbitrary field order and duplicate
        // keys. This probe is published only after the root object's EOF check,
        // so forwarding it as a chunked body would be buffered in practice and
        // would not provide a real overlap benefit. Let the established replay
        // path make the final request instead.
        prepared_live_request_streaming_decision = Some(LiveRequestStreamingDecision {
            transport_mode: RequestBodyTransportMode::Buffered,
            eligible: false,
            reason: "route_finalized_at_eof",
            ..decide_live_request_streaming(&live_settings, invoke_id, capture_target, true, true)
        });
        drop(live_pipeline.take());
    } else if live_candidate {
        let mut no_available_wait_deadline = None;
        let resolution = match load_via_pool_effective_routing_with_cache(
            state.as_ref(),
            live_prompt_cache_key.as_deref(),
            live_body_key_probe.contains_encrypted_content,
        )
        .await
        {
            Ok((
                prompt_cache_binding_constraint,
                _owner_auto_guard_active,
                conversation_override,
                cache_hit,
            )) => {
                live_route_lookup_cache_hit = Some(cache_hit);
                if prompt_cache_binding_constraint.is_some() || conversation_override.is_some() {
                    resolve_pool_account_for_request_with_wait_and_binding_constraint_with_image_intent_and_override_and_codex_imagegen_request_and_reservation(
                        state.as_ref(),
                        live_body_sticky_key.as_deref(),
                        live_body_key_probe.model.as_deref(),
                        &[],
                        &HashSet::new(),
                        None,
                        prompt_cache_binding_constraint.as_ref(),
                        conversation_override.as_ref(),
                        true,
                        &mut no_available_wait_deadline,
                        None,
                        capture_target.endpoint(),
                        live_body_key_probe.image_intent,
                        codex_imagegen_protocol_from_headers(headers).is_some(),
                        Some(live_routing_reservation_key.as_str()),
                    )
                    .await
                } else {
                    resolve_pool_account_for_request_with_wait_and_image_intent_and_codex_imagegen_request_and_reservation(
                        state.as_ref(),
                        live_body_sticky_key.as_deref(),
                        live_body_key_probe.model.as_deref(),
                        &[],
                        &HashSet::new(),
                        None,
                        None,
                        None,
                        true,
                        &mut no_available_wait_deadline,
                        None,
                        capture_target.endpoint(),
                        live_body_key_probe.image_intent,
                        codex_imagegen_protocol_from_headers(headers).is_some(),
                        Some(live_routing_reservation_key.as_str()),
                    )
                    .await
                }
            }
            Err((status, message)) => {
                warn!(
                    proxy_request_id,
                    ?status,
                    %message,
                    "live-first routing metadata could not be resolved; replaying buffered request"
                );
                drop(live_pipeline.take());
                prepared_live_request_streaming_decision = Some(decide_live_request_streaming(
                    &live_settings,
                    invoke_id,
                    capture_target,
                    false,
                    false,
                ));
                Err(anyhow::anyhow!(message))
            }
        };
        if let Ok(PoolAccountResolutionWithWait::Resolution(PoolAccountResolution::Resolved(
            initial_account,
        ))) = resolution
        {
            live_first_experiment_group = initial_account.group_name.clone();
            let mut live_route_reservation_guard = Some(PoolRoutingReservationDropGuard::new(
                state.clone(),
                live_routing_reservation_key.clone(),
            ));
            let decision = decide_live_request_streaming(
                &live_settings,
                invoke_id,
                capture_target,
                true,
                pool_account_supports_live_request_body(
                    &initial_account,
                    original_uri,
                    &Method::POST,
                    headers,
                ),
            );
            prepared_live_request_streaming_decision = Some(decision.clone());
            if decision.transport_mode == RequestBodyTransportMode::LiveFirst {
                let trace_context = PoolUpstreamAttemptTraceContext {
                    invoke_id: invoke_id.to_string(),
                    occurred_at: occurred_at.to_string(),
                    endpoint: capture_target.endpoint().to_string(),
                    sticky_key: live_body_sticky_key.clone(),
                    requester_ip: requester_ip.map(str::to_string),
                    upstream_base_url_host: None,
                    request_model: live_body_key_probe.model.clone(),
                };
                let target_encoding = live_resolved_content_encoding
                    .map(|resolved| {
                        live_responses_target_request_content_encoding_with_resolved(
                            resolved,
                            initial_account.request_compression_algorithm,
                        )
                    })
                    .unwrap_or_else(|| {
                        live_responses_target_request_content_encoding(
                            downstream_content_encoding.as_deref(),
                            initial_account.request_compression_algorithm,
                        )
                    });
                let Ok(target_encoding) = target_encoding else {
                    warn!(
                        proxy_request_id,
                        "live-first request body uses an unsupported Content-Encoding; replaying buffered request"
                    );
                    drop(live_pipeline);
                    return PreparedCaptureRequestBody {
                        request_body_snapshot_result: wait_for_replay_body_snapshot(
                            state.as_ref(),
                            original_uri,
                            &Method::POST,
                            &replay_status_rx,
                            &replay_cancel,
                            runtime_timeouts.request_read_timeout,
                            response_timeout.map(|_| req_read_started),
                        )
                        .await
                        .map_err(|(status, message)| RequestBodyReadError {
                            status,
                            message,
                            failure_kind: if status == StatusCode::REQUEST_TIMEOUT {
                                PROXY_FAILURE_REQUEST_BODY_READ_TIMEOUT
                            } else {
                                PROXY_FAILURE_REQUEST_BODY_STREAM_ERROR_CLIENT_CLOSED
                            },
                            partial_body: Vec::new(),
                        }),
                        live_first_pool_response: None,
                        prepared_live_request_streaming_decision: Some(
                            LiveRequestStreamingDecision {
                                transport_mode: RequestBodyTransportMode::Buffered,
                                eligible: false,
                                reason: "downstream_content_encoding_not_supported",
                                ..decision
                            },
                        ),
                        live_first_attempt_failed: false,
                        live_first_failed_account_id: None,
                        live_first_request_body_first_byte_at: None,
                        live_oauth_rewrite_rx: None,
                        live_first_experiment_group,
                        live_route_finalization_measurement: Some(
                            LiveRequestStreamingMeasurement {
                                route_finalization_ms: Some(live_route_finalization_ms),
                                route_finalization_outcome: Some("buffered_encoding"),
                                route_dependency_factors: live_route_dependency_factors(
                                    &live_body_key_probe,
                                    encrypted_owner_routing_enabled,
                                ),
                                routing_hot_cache_hit: live_route_lookup_cache_hit,
                                routing_hot_cache_cold_load: live_route_lookup_cache_hit
                                    .map(|hit| !hit),
                                ..LiveRequestStreamingMeasurement::default()
                            },
                        ),
                    };
                };
                let oauth = match &initial_account.auth {
                    PoolResolvedAuth::Oauth { .. } => Some(LiveOauthResponsesTransform {
                        account_id: Some(initial_account.account_id),
                        installation_seed: Some(state.oauth_installation_seed),
                    }),
                    PoolResolvedAuth::ApiKey { .. } => None,
                };
                let compression_level = live_routing_snapshot
                    .as_ref()
                    .map(|(snapshot, _)| snapshot.request_compression.level_preset)
                    .unwrap_or_default();
                let initial_account_id = initial_account.account_id;
                let model_mapping = match load_model_mapping_for_account(
                    state.as_ref(),
                    initial_account.account_id,
                    live_body_key_probe.model.as_deref(),
                )
                .await
                {
                    Ok(mapping) => mapping,
                    Err(err) => {
                        warn!(
                            proxy_request_id,
                            account_id = initial_account.account_id,
                            error = %err,
                            "failed to resolve live-first model mapping; replaying buffered request"
                        );
                        live_first_attempt_failed = true;
                        drop(live_pipeline.take());
                        None
                    }
                };
                if live_pipeline.is_some()
                    && !live_pipeline
                        .as_mut()
                        .expect("live pipeline is present before configuration")
                        .configure(LiveResponsesBodyTransformConfig {
                            target_encoding,
                            compression_level,
                            enforce_include_usage: state.config.proxy_enforce_stream_include_usage,
                            oauth,
                            fast_mode_rewrite_mode: initial_account.fast_mode_rewrite_mode,
                            image_tool_rewrite_mode: initial_account.image_tool_rewrite_mode,
                            codex_imagegen_rewrite_mode: initial_account
                                .codex_imagegen_rewrite_mode,
                            codex_imagegen_protocol: codex_imagegen_protocol_from_headers(headers),
                            model_mapping_target: model_mapping
                                .as_ref()
                                .map(|mapping| mapping.target_model.clone()),
                        })
                {
                    live_first_attempt_failed = true;
                } else if live_pipeline.is_some() {
                    let mut live_headers = headers.clone();
                    live_headers.remove(header::CONTENT_LENGTH);
                    match target_encoding.header_value() {
                        Some(value) => {
                            live_headers
                                .insert(header::CONTENT_ENCODING, HeaderValue::from_static(value));
                        }
                        None => {
                            live_headers.remove(header::CONTENT_ENCODING);
                        }
                    }
                    let pipeline = live_pipeline
                        .take()
                        .expect("live pipeline is consumed by the live-first attempt");
                    let first_upstream_body_poll_at_rx =
                        pipeline.first_upstream_body_poll_at_rx.clone();
                    let original_request_stream_rx = pipeline.original_request_stream_rx.clone();
                    match send_pool_request_live_first_attempt(
                        state.clone(),
                        proxy_request_id,
                        Method::POST,
                        original_uri,
                        &live_headers,
                        pipeline.body,
                        live_prompt_cache_key.as_deref(),
                        live_body_sticky_key.as_deref(),
                        live_body_key_probe.image_intent,
                        proxy_upstream_send_timeout_for_capture_target(
                            runtime_timeouts,
                            Some(capture_target),
                        ),
                        response_timeout,
                        response_timeout.map(|_| req_read_started),
                        live_body_sticky_key.as_deref(),
                        initial_account.clone(),
                        model_mapping,
                        Some(&trace_context),
                        live_route_reservation_guard.take(),
                        &replay_status_rx,
                        &first_upstream_body_poll_at_rx,
                        Some(original_request_stream_rx),
                    )
                    .await
                    {
                        Ok(mut response) => {
                            // An upstream is allowed to respond before it has
                            // consumed the complete HTTP request body. Do not
                            // accept that provisional response until the local
                            // parser confirms that no later JSON field changed
                            // the selected route.
                            wait_for_live_request_body_finalization(
                                &mut live_request_body_finalization_rx,
                            )
                            .await;
                            let request_body_error =
                                { live_request_body_error_rx.borrow().clone() };
                            if let Some(request_body_error) = request_body_error {
                                live_first_attempt_failed = true;
                                live_first_request_body_first_byte_at =
                                    live_first_request_body_first_byte_at
                                        .or_else(|| *first_upstream_body_poll_at_rx.borrow());
                                warn!(
                                    proxy_request_id,
                                    status = %request_body_error.status,
                                    failure_kind = request_body_error.failure_kind,
                                    "live-first request body validation cancelled a provisional upstream response"
                                );
                                reject_live_first_provisional_response_after_body_failure(
                                    state.as_ref(),
                                    &mut response,
                                    &request_body_error,
                                )
                                .await;
                            } else {
                                live_first_request_body_first_byte_at =
                                    response.live_request_body_first_byte_at;
                                live_first_pool_response = Some(response);
                            }
                        }
                        Err(error) => {
                            live_first_attempt_failed = true;
                            live_first_request_body_first_byte_at =
                                *first_upstream_body_poll_at_rx.borrow();
                            // A selected account is not proof that a live-first request
                            // reached its upstream. Local timeout/configuration failures carry
                            // that account too; only a completed non-timeout attempt excludes
                            // it from replay.
                            live_first_failed_account_id = live_first_replay_exclusion(
                                error.attempt_summary.pool_attempt_count,
                                error.failure_kind,
                                &error.message,
                                initial_account_id,
                            );
                            warn!(
                                proxy_request_id,
                                error = %error.message,
                                "live-first pool attempt failed; replaying the captured request"
                            );
                        }
                    }
                }
            } else {
                drop(live_pipeline.take());
            }
        } else {
            drop(live_pipeline.take());
        }
    } else {
        prepared_live_request_streaming_decision = Some(decide_live_request_streaming(
            &live_settings,
            invoke_id,
            capture_target,
            false,
            false,
        ));
        drop(live_pipeline.take());
    }

    drop(live_pipeline);

    let request_body_snapshot_result = {
        let snapshot_result = wait_for_replay_body_snapshot(
            state.as_ref(),
            original_uri,
            &Method::POST,
            &replay_status_rx,
            &replay_cancel,
            runtime_timeouts.request_read_timeout,
            response_timeout.map(|_| req_read_started),
        )
        .await
        .map_err(|(status, message)| RequestBodyReadError {
            status,
            message,
            failure_kind: if status == StatusCode::REQUEST_TIMEOUT {
                PROXY_FAILURE_REQUEST_BODY_READ_TIMEOUT
            } else {
                PROXY_FAILURE_REQUEST_BODY_STREAM_ERROR_CLIENT_CLOSED
            },
            partial_body: Vec::new(),
        });
        if snapshot_result.is_ok() || live_first_pool_response.is_some() {
            wait_for_live_request_body_finalization(&mut live_request_body_finalization_rx).await;
        }
        match snapshot_result {
            Err(error) => Err(error),
            Ok(snapshot) => live_request_body_error_rx
                .borrow()
                .clone()
                .map_or(Ok(snapshot), Err),
        }
    };
    if let Err(request_body_error) = &request_body_snapshot_result
        && let Some(mut response) = live_first_pool_response.take()
    {
        live_first_attempt_failed = true;
        live_first_request_body_first_byte_at =
            live_first_request_body_first_byte_at.or(response.live_request_body_first_byte_at);
        warn!(
            proxy_request_id,
            status = %request_body_error.status,
            failure_kind = request_body_error.failure_kind,
            "live-first request body failure cancelled a provisional upstream response"
        );
        reject_live_first_provisional_response_after_body_failure(
            state.as_ref(),
            &mut response,
            request_body_error,
        )
        .await;
    }

    PreparedCaptureRequestBody {
        request_body_snapshot_result,
        live_first_pool_response,
        prepared_live_request_streaming_decision,
        live_first_attempt_failed,
        live_first_failed_account_id,
        live_first_request_body_first_byte_at,
        live_oauth_rewrite_rx: Some(live_oauth_rewrite_rx),
        live_first_experiment_group,
        live_route_finalization_measurement: Some(LiveRequestStreamingMeasurement {
            route_finalization_ms: Some(live_route_finalization_ms),
            route_finalization_outcome: Some(if live_candidate {
                "buffered_eof_final_route"
            } else {
                "buffered_no_model"
            }),
            route_dependency_factors: live_route_dependency_factors(
                &live_body_key_probe,
                encrypted_owner_routing_enabled,
            ),
            routing_hot_cache_hit: live_route_lookup_cache_hit,
            routing_hot_cache_cold_load: live_route_lookup_cache_hit.map(|hit| !hit),
            ..LiveRequestStreamingMeasurement::default()
        }),
    }
}

fn live_route_dependency_factors(
    probe: &PoolReplayBodyKeyProbe,
    encrypted_owner_routing_enabled: bool,
) -> Vec<&'static str> {
    let mut factors = vec!["model"];
    if probe.sticky_key.is_some() {
        factors.push("sticky");
    }
    if probe.prompt_cache_key.is_some() {
        factors.push("prompt_cache");
    }
    if encrypted_owner_routing_enabled && probe.contains_encrypted_content {
        factors.push("encrypted_owner");
    }
    if matches!(
        probe.image_intent,
        ImageIntent::Yes | ImageIntent::DirectImage
    ) {
        factors.push("image_capability");
    }
    factors
}

/// A live probe is safe to commit only when the currently enabled routing
/// factors are absent from the unread portion. Sticky/prompt-cache ownership,
/// encrypted-session ownership, and known image intent remain buffered; the
/// incremental pipeline keeps those root fields in its precommit set.
fn live_route_probe_can_start_before_eof(probe: &PoolReplayBodyKeyProbe) -> bool {
    probe.model.is_some()
        && probe.sticky_key.is_none()
        && probe.prompt_cache_key.is_none()
        && !probe.contains_encrypted_content
        && probe.image_intent != ImageIntent::Yes
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn proxy_openai_v1_capture_target(
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
    mut proxy_request_permit: Option<ProxyRequestConcurrencyPermit>,
    admitted_runtime_snapshot: Option<AdmittedProxyRuntimeSnapshot>,
    downstream_request_observer: Option<DownstreamRequestObserver>,
    proxy_request_started_at: Instant,
) -> Result<Response, (StatusCode, String)> {
    if !pool_route_active {
        return Err((
            StatusCode::UNAUTHORIZED,
            "pool route key missing or invalid".to_string(),
        ));
    }
    let capture_started = Instant::now();
    let proxy_request_permit = take_or_acquire_proxy_request_concurrency_permit(
        &mut proxy_request_permit,
        state.as_ref(),
        proxy_request_id,
        &Method::POST,
        original_uri,
    )
    .await;
    let pool_routing_reservation_key = build_pool_routing_reservation_key(proxy_request_id);
    let occurred_at = admitted_runtime_snapshot
        .as_ref()
        .map(|snapshot| snapshot.occurred_at.clone())
        .unwrap_or_else(|| format_naive(Utc::now().with_timezone(&Shanghai).naive_local()));
    let mut pool_invocation_cleanup_guard = pool_route_active.then(|| {
        PoolInvocationCleanupGuard::new(
            state.clone(),
            InvocationRecoverySelector::new(invoke_id.clone(), occurred_at.clone()),
            "request_drop_guard",
        )
    });
    let body_limit = state.config.openai_proxy_max_request_body_bytes;
    let requester_ip = extract_requester_ip(&headers, peer_ip);
    let request_chain_metadata = request_chain_metadata_from_headers(&headers);
    let header_sticky_key = extract_sticky_key_from_headers(&headers);
    let header_prompt_cache_key = extract_prompt_cache_key_from_headers(&headers);
    let client_attribution_context = client_prompt_cache_attribution_context_from_headers(&headers);
    if admitted_runtime_snapshot.is_none() {
        let shell_started = Instant::now();
        let admitted_running_record = build_admitted_proxy_capture_runtime_snapshot(
            &invoke_id,
            &occurred_at,
            capture_target,
            requester_ip.as_deref(),
            header_sticky_key.as_deref(),
            header_prompt_cache_key.as_deref(),
        );
        if let Err(err) = persist_and_broadcast_proxy_capture_runtime_snapshot(
            state.as_ref(),
            admitted_running_record,
        )
        .await
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
                "admitted proxy capture handler emitted fallback running shell"
            );
        }
    }
    let proxy_settings = state.proxy_model_settings.read().await.clone();

    let req_read_started = Instant::now();
    let PreparedCaptureRequestBody {
        request_body_snapshot_result,
        mut live_first_pool_response,
        prepared_live_request_streaming_decision,
        live_first_attempt_failed,
        live_first_failed_account_id,
        live_first_request_body_first_byte_at,
        live_oauth_rewrite_rx,
        live_first_experiment_group,
        live_route_finalization_measurement,
    } = Box::pin(prepare_capture_request_body(
        state.clone(),
        proxy_request_id,
        &invoke_id,
        &occurred_at,
        original_uri,
        &headers,
        body,
        capture_target,
        body_limit,
        &runtime_timeouts,
        requester_ip.as_deref(),
        header_sticky_key.as_deref(),
        header_prompt_cache_key.as_deref(),
        req_read_started,
    ))
    .await;
    let request_body_snapshot = match request_body_snapshot_result {
        Ok(bytes) => bytes,
        Err(read_err) => {
            let response_envelope =
                build_local_capture_error_envelope(&invoke_id, read_err.status, &read_err.message);
            let t_req_read_ms = elapsed_ms(req_read_started);
            let request_info = RequestCaptureInfo::default();
            drop(proxy_request_permit);
            let req_raw = spawn_raw_payload_file_write(
                state.as_ref(),
                &invoke_id,
                "request",
                Bytes::from(read_err.partial_body),
                proxy_settings.request_body_logging_enabled,
            )
            .finish()
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
            let capture_failure_decision = prepared_live_request_streaming_decision
                .clone()
                .unwrap_or_else(|| {
                    LiveRequestStreamingDecision::buffered("request_body_capture_failed")
                });

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
                cost_breakdown: None,
                cost_estimated,
                price_version,
                status: if read_err.status.is_server_error() {
                    format!("http_{}", read_err.status.as_u16())
                } else {
                    "failed".to_string()
                },
                error_message: Some(error_message),
                failure_kind: Some(read_err.failure_kind.to_string()),
                payload: Some(with_live_request_streaming_payload_summary(
                    build_proxy_payload_summary(ProxyPayloadSummary {
                        target: capture_target,
                        status: read_err.status,
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
                        request_compression_algorithm: None,
                        request_compression_mode: None,
                        request_compression_logical_body_bytes: None,
                        request_compression_transmitted_body_bytes: None,
                        request_compression_transmission_complete: None,
                        failure_kind: Some(read_err.failure_kind),
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
                        sticky_key: header_sticky_key.as_deref(),
                        prompt_cache_key: header_prompt_cache_key.as_deref(),
                        prompt_cache_key_attribution_source: header_prompt_cache_key
                            .as_ref()
                            .map(|_| "request"),
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
                        downstream_status_code: Some(read_err.status),
                        downstream_error_message: Some(read_err.message.as_str()),
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
                        upstream_approx_upload_bytes: None,
                        upstream_approx_download_bytes: None,
                        proxy_display_name: None,
                        proxy_weight_delta: None,
                        pool_attempt_count: None,
                        pool_distinct_account_count: None,
                        pool_attempt_terminal_reason: None,
                        blocked_binding: None,
                    }),
                    &capture_failure_decision,
                    &LiveRequestStreamingMeasurement {
                        first_attempt_failed: live_first_attempt_failed,
                        fallback_or_retry: false,
                        capture_failed: true,
                        ambiguous_upstream_delivery: live_first_attempt_failed
                            && live_first_request_body_first_byte_at.is_some(),
                        experiment_account_group: live_first_experiment_group.clone(),
                        ..LiveRequestStreamingMeasurement::default()
                    },
                )),
                raw_response: response_envelope.body_text.clone(),
                response_body_preview_enabled: true,
                req_raw,
                resp_raw: build_local_capture_error_resp_raw(&response_envelope),
                timings: StageTimings {
                    t_total_ms: 0.0,
                    t_req_read_ms,
                    t_req_parse_ms: 0.0,
                    t_upstream_connect_ms: 0.0,
                    t_upstream_ttfb_ms: 0.0,
                    first_token_ms: None,
                    t_upstream_stream_ms: 0.0,
                    t_resp_parse_ms: 0.0,
                    t_persist_ms: 0.0,
                },
            };
            let terminal_invocation_persisted = if let Err(err) =
                persist_and_broadcast_proxy_capture(state.as_ref(), capture_started, record).await
            {
                warn!(proxy_request_id, error = %err, "failed to persist proxy capture record");
                false
            } else {
                true
            };
            if terminal_invocation_persisted {
                disarm_pool_invocation_cleanup_guard(&mut pool_invocation_cleanup_guard);
            }
            return Err((read_err.status, read_err.message));
        }
    };
    let t_req_read_ms = elapsed_ms(req_read_started);
    let request_body_snapshot_kind = pool_request_snapshot_kind(&request_body_snapshot);
    let request_body_bytes_len = pool_request_snapshot_body_bytes(&request_body_snapshot);
    let live_first_eligible = prepared_live_request_streaming_decision
        .as_ref()
        .is_some_and(|decision| decision.eligible);
    let live_first_reason = prepared_live_request_streaming_decision
        .as_ref()
        .map(|decision| decision.reason)
        .unwrap_or("not_live_candidate");
    if capture_request_body_read_log_at_info(request_body_bytes_len, t_req_read_ms) {
        info!(
            proxy_request_id,
            body_read_done = true,
            body_read_elapsed_ms = t_req_read_ms,
            request_body_bytes = request_body_bytes_len,
            body_size_bucket = request_body_size_bucket(request_body_bytes_len),
            request_body_snapshot_kind,
            live_first_eligible,
            live_first_reason,
            "openai proxy capture request body read completed"
        );
    } else {
        debug!(
            proxy_request_id,
            body_read_done = true,
            body_read_elapsed_ms = t_req_read_ms,
            request_body_bytes = request_body_bytes_len,
            body_size_bucket = request_body_size_bucket(request_body_bytes_len),
            request_body_snapshot_kind,
            live_first_eligible,
            live_first_reason,
            "openai proxy capture request body read completed"
        );
    }
    let req_parse_started = Instant::now();
    let semantic_projection = project_request_semantics(
        proxy_request_id,
        request_body_snapshot.clone(),
        capture_target,
        state.config.proxy_enforce_stream_include_usage,
    )
    .await;
    debug!(
        proxy_request_id,
        pipeline_mode = ?request_semantic_pipeline_mode(),
        json_parse_count = semantic_projection.json_parse_count,
        whole_body_materialization_count = semantic_projection.whole_body_materialization_count,
        materialization_bytes = semantic_projection.materialization_bytes,
        peak_business_buffer_bytes = semantic_projection.peak_business_buffer_bytes,
        parse_elapsed_ms = semantic_projection.parse_elapsed_ms,
        "proxy request semantic projection completed"
    );
    state.proxy_runtime_invocations.record_request_pipeline(
        request_body_snapshot_kind,
        semantic_projection.json_parse_count,
        semantic_projection.whole_body_materialization_count,
        semantic_projection.peak_business_buffer_bytes,
        semantic_projection
            .fallback_reason
            .or(semantic_projection.request_info.parse_error.as_deref()),
    );
    let mut request_info = semantic_projection.request_info.clone();
    let body_rewritten = semantic_projection.body_rewritten;
    let mut prompt_cache_key = request_info
        .prompt_cache_key
        .clone()
        .or_else(|| header_prompt_cache_key.clone());
    let mut sticky_key = request_info
        .sticky_key
        .clone()
        .or_else(|| header_sticky_key.clone());
    if prompt_cache_key.is_some() && request_info.prompt_cache_key_attribution_source.is_none() {
        request_info.prompt_cache_key_attribution_source = Some("request".to_string());
    }
    if capture_target == ProxyCaptureTarget::ResponsesCompact
        && prompt_cache_key.is_none()
        && let Some(attribution) =
            lookup_recent_prompt_cache_attribution(&client_attribution_context, Instant::now())
    {
        prompt_cache_key = Some(attribution.prompt_cache_key.clone());
        if sticky_key.is_none() {
            sticky_key = attribution.sticky_key.clone();
        }
        request_info.prompt_cache_key = prompt_cache_key.clone();
        request_info.sticky_key = sticky_key.clone();
        request_info.prompt_cache_key_attribution_source =
            Some("client_fingerprint_recent".to_string());
    }
    if capture_target == ProxyCaptureTarget::Responses
        && let Some(prompt_cache_key) = prompt_cache_key.as_deref()
    {
        remember_prompt_cache_attribution(
            &client_attribution_context,
            prompt_cache_key,
            sticky_key.as_deref(),
            Instant::now(),
        );
    }
    let (prompt_cache_binding_constraint, encrypted_owner_auto_guard_active) = if pool_route_active
    {
        let encrypted_owner_routing_enabled =
            encrypted_session_owner_routing_enabled(state.as_ref()).await;
        let binding_constraint_result = resolve_prompt_cache_effective_routing_constraint(
            &state.pool,
            prompt_cache_key.as_deref(),
            request_info.contains_encrypted_content,
            encrypted_owner_routing_enabled,
        )
        .await;
        match binding_constraint_result {
            Ok(value) => value,
            Err(err) => {
                let status = StatusCode::BAD_GATEWAY;
                let message = format!("failed to resolve prompt cache conversation binding: {err}");
                let terminal_invocation_persisted = persist_pre_attempt_proxy_capture_error(
                    state.clone(),
                    proxy_request_id,
                    capture_started,
                    &invoke_id,
                    &occurred_at,
                    capture_target,
                    &request_info,
                    requester_ip.as_deref(),
                    &request_chain_metadata,
                    sticky_key.as_deref(),
                    prompt_cache_key.as_deref(),
                    &client_attribution_context,
                    semantic_projection.request_body_for_capture.clone(),
                    semantic_projection.upstream_snapshot.clone(),
                    proxy_settings.request_body_logging_enabled,
                    t_req_read_ms,
                    elapsed_ms(req_parse_started),
                    status,
                    PROXY_FAILURE_POOL_ROUTING_BLOCKED,
                    &message,
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                )
                .await;
                if terminal_invocation_persisted {
                    disarm_pool_invocation_cleanup_guard(&mut pool_invocation_cleanup_guard);
                }
                return Err((status, message));
            }
        }
    } else {
        (None, false)
    };
    let prompt_cache_conversation_override = if pool_route_active {
        match load_prompt_cache_conversation_routing_override(
            &state.pool,
            prompt_cache_key.as_deref(),
        )
        .await
        {
            Ok(value) => value,
            Err(err) => {
                let status = StatusCode::BAD_GATEWAY;
                let message =
                    format!("failed to resolve prompt cache conversation overrides: {err}");
                let terminal_invocation_persisted = persist_pre_attempt_proxy_capture_error(
                    state.clone(),
                    proxy_request_id,
                    capture_started,
                    &invoke_id,
                    &occurred_at,
                    capture_target,
                    &request_info,
                    requester_ip.as_deref(),
                    &request_chain_metadata,
                    sticky_key.as_deref(),
                    prompt_cache_key.as_deref(),
                    &client_attribution_context,
                    semantic_projection.request_body_for_capture.clone(),
                    semantic_projection.upstream_snapshot.clone(),
                    proxy_settings.request_body_logging_enabled,
                    t_req_read_ms,
                    elapsed_ms(req_parse_started),
                    status,
                    PROXY_FAILURE_POOL_ROUTING_BLOCKED,
                    &message,
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                )
                .await;
                if terminal_invocation_persisted {
                    disarm_pool_invocation_cleanup_guard(&mut pool_invocation_cleanup_guard);
                }
                return Err((status, message));
            }
        }
    } else {
        None
    };
    let pool_attempt_trace_context = pool_route_active.then(|| PoolUpstreamAttemptTraceContext {
        invoke_id: invoke_id.clone(),
        occurred_at: occurred_at.clone(),
        endpoint: capture_target.endpoint().to_string(),
        sticky_key: sticky_key.clone(),
        requester_ip: requester_ip.clone(),
        upstream_base_url_host: None,
        request_model: request_info.model.clone(),
    });
    let t_req_parse_ms = elapsed_ms(req_parse_started);
    let upstream_body_snapshot = semantic_projection.upstream_snapshot.clone();

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
    if let Err(err) =
        persist_and_broadcast_proxy_capture_runtime_snapshot(state.as_ref(), initial_running_record)
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
            live_request_streaming_decision: prepared_live_request_streaming_decision.clone(),
            live_request_streaming_experiment_group: live_first_experiment_group.clone(),
            live_first_attempt_failed,
            live_first_request_body_first_byte_at,
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
    ) = if let Some(response) = live_first_pool_response.take() {
        (
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
        )
    } else if pool_route_active {
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
            PoolFailoverProgress {
                excluded_account_ids: live_first_failed_account_id.into_iter().collect(),
                attempt_count: usize::from(live_first_failed_account_id.is_some()),
                ..PoolFailoverProgress::default()
            },
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
                let live_request_streaming_measurement =
                    pool_attempt_runtime_snapshot.as_ref().and_then(|context| {
                        context.live_request_streaming_decision.as_ref().map(|_| {
                            let risk = live_request_streaming_risk_flags(
                                context.live_first_attempt_failed,
                                context.live_first_request_body_first_byte_at.is_some(),
                                err.attempt_summary.pool_attempt_count,
                            );
                            LiveRequestStreamingMeasurement {
                                first_attempt_failed: risk.first_attempt_failed,
                                fallback_or_retry: risk.fallback_or_retry,
                                ambiguous_upstream_delivery: risk.ambiguous_upstream_delivery,
                                upstream_account_group: err
                                    .account
                                    .as_ref()
                                    .and_then(|account| account.group_name.clone()),
                                experiment_account_group: context
                                    .live_request_streaming_experiment_group
                                    .clone(),
                                ..LiveRequestStreamingMeasurement::default()
                            }
                        })
                    });
                let mut record = ProxyCaptureRecord {
                    invoke_id,
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
                if let (Some(decision), Some(measurement)) = (
                    pool_attempt_runtime_snapshot
                        .as_ref()
                        .and_then(|context| context.live_request_streaming_decision.as_ref()),
                    live_request_streaming_measurement.as_ref(),
                ) && let Some(payload) = record.payload.take()
                {
                    record.payload = Some(with_live_request_streaming_payload_summary(
                        payload,
                        decision,
                        measurement,
                    ));
                }
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
            Some(upstream_body_snapshot.clone()),
            Some(UpstreamTrafficReporter::new(
                state.clone(),
                invoke_id.as_str(),
                occurred_at.as_str(),
                None,
                state.config.openai_upstream_base_url.host_str(),
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
                    invoke_id,
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
                return Err((err.status, err.message));
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
                invoke_id,
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
            return Err((StatusCode::BAD_GATEWAY, message));
        }
    };

    let upstream_connection_scoped = connection_scoped_header_names(upstream_response.headers());
    let response_is_event_stream = capture_target != ProxyCaptureTarget::StandaloneSearch
        && upstream_response
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
    let live_request_streaming_decision = if let Some(decision) =
        prepared_live_request_streaming_decision
    {
        decision
    } else if capture_target == ProxyCaptureTarget::Responses {
        match load_pool_routing_runtime_cache(state.as_ref()).await {
            Ok(snapshot) => decide_live_request_streaming(
                &snapshot.live_request_streaming,
                &invoke_id,
                capture_target,
                true,
                false,
            ),
            Err(error) => {
                warn!(?error, invoke_id = %invoke_id, "failed to load live request streaming settings");
                LiveRequestStreamingDecision::buffered("settings_unavailable")
            }
        }
    } else {
        LiveRequestStreamingDecision::buffered("endpoint_not_supported")
    };
    let logical_request_body_bytes = pool_request_snapshot_logical_body_bytes(
        &request_body_snapshot,
        headers
            .get(header::CONTENT_ENCODING)
            .and_then(|value| value.to_str().ok()),
    )
    .await
    .ok();
    let upstream_request_first_byte_ms = live_first_request_body_first_byte_at.map(|sent_at| {
        sent_at
            .saturating_duration_since(req_read_started)
            .as_secs_f64()
            * 1_000.0
    });
    let live_request_streaming_risk = live_request_streaming_risk_flags(
        live_first_attempt_failed,
        live_first_request_body_first_byte_at.is_some(),
        pending_pool_attempt_summary.pool_attempt_count,
    );
    let mut live_request_streaming_measurement = LiveRequestStreamingMeasurement {
        raw_body_bytes: Some(request_body_bytes_len),
        logical_body_bytes: logical_request_body_bytes,
        upstream_request_first_byte_ms,
        request_body_capture_complete_ms: Some(t_req_read_ms),
        request_upstream_overlap_ms: request_upstream_overlap_ms(
            Some(t_req_read_ms),
            upstream_request_first_byte_ms,
        ),
        first_response_byte_total_ms: prefetched_first_chunk_received_at.map(|received_at| {
            received_at
                .saturating_duration_since(req_read_started)
                .as_secs_f64()
                * 1_000.0
        }),
        first_attempt_failed: live_request_streaming_risk.first_attempt_failed,
        fallback_or_retry: live_request_streaming_risk.fallback_or_retry,
        capture_failed: false,
        ambiguous_upstream_delivery: live_request_streaming_risk.ambiguous_upstream_delivery,
        upstream_account_group: pool_account
            .as_ref()
            .and_then(|account| account.group_name.clone()),
        experiment_account_group: live_first_experiment_group,
        ..LiveRequestStreamingMeasurement::default()
    };
    if let Some(route_measurement) = live_route_finalization_measurement {
        live_request_streaming_measurement.route_finalization_ms =
            route_measurement.route_finalization_ms;
        live_request_streaming_measurement.route_finalization_outcome =
            route_measurement.route_finalization_outcome;
        live_request_streaming_measurement.route_dependency_factors =
            route_measurement.route_dependency_factors;
        live_request_streaming_measurement.routing_hot_cache_hit =
            route_measurement.routing_hot_cache_hit;
        live_request_streaming_measurement.routing_hot_cache_cold_load =
            route_measurement.routing_hot_cache_cold_load;
        live_request_streaming_measurement.route_finalization_raw_bytes =
            Some(request_body_bytes_len);
        live_request_streaming_measurement.route_finalization_logical_bytes =
            logical_request_body_bytes;
        live_request_streaming_measurement.route_finalization_raw_ratio = Some(1.0);
        live_request_streaming_measurement.route_finalization_logical_ratio =
            logical_request_body_bytes.map(|_| 1.0);
    }
    let mut response_running_record = build_running_proxy_capture_record(
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
    set_proxy_capture_record_request_compression_algorithm(
        &mut response_running_record,
        pending_pool_attempt_record
            .as_ref()
            .and_then(|pending| pending.upstream_request_compression_algorithm.as_deref()),
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
    if let Ok(header_value) = HeaderValue::from_str(&invoke_id) {
        response_builder =
            response_builder.header(HeaderName::from_static(CVM_INVOKE_ID_HEADER), header_value);
    }

    let state_for_task = state.clone();
    let request_info_for_task = request_info.clone();
    let client_attribution_context_for_task = client_attribution_context.clone();
    let req_raw_pending_for_task = req_raw_pending
        .take()
        .expect("request raw capture should still be pending when response stream starts");
    let invoke_id_for_task = invoke_id.clone();
    let occurred_at_for_task = occurred_at.clone();
    let upstream_content_encoding_for_task = upstream_content_encoding.clone();
    let requester_ip_for_task = requester_ip.clone();
    let request_chain_metadata_for_task = request_chain_metadata.clone();
    let sticky_key_for_task = sticky_key.clone();
    let reservation_key_for_task = pool_routing_reservation_key.clone();
    let prompt_cache_key_for_task = prompt_cache_key.clone();
    let selected_proxy_for_task = selected_proxy.clone();
    let selected_proxy_display_name_for_task = selected_proxy_display_name.clone();
    let pool_account_for_task = pool_account.clone();
    let image_tool_rewrite_for_task = image_tool_rewrite.clone();
    let mut oauth_responses_debug_for_task = oauth_responses_debug.clone();
    let live_oauth_rewrite_rx_for_task = live_oauth_rewrite_rx.clone();
    let downstream_request_observer_for_task = downstream_request_observer.clone();
    let attempt_already_recorded_for_task = attempt_already_recorded;
    let final_attempt_update_for_task = final_attempt_update;
    let direct_http_approx_for_task = direct_http_approx.clone();
    let transport_bytes_live_counted_for_task = transport_bytes_live_counted;
    let reservation_guard_for_task = reservation_guard;
    let mut pending_pool_attempt_record_for_task = pending_pool_attempt_record.clone();
    let mut deferred_pool_early_phase_cleanup_guard_for_task =
        deferred_pool_early_phase_cleanup_guard;
    let live_pool_attempt_activity_lease_for_task = live_pool_attempt_activity_lease;
    let pending_pool_attempt_summary_for_task = pending_pool_attempt_summary.clone();
    let prefetched_first_chunk_for_task = prefetched_first_chunk;
    let prefetched_first_chunk_received_at_for_task = prefetched_first_chunk_received_at;
    let mut prefetched_first_stream_chunk_received_at_for_task =
        prefetched_first_stream_chunk_received_at;
    let prefetched_stream_timeout_for_task = prefetched_stream_timeout;
    let prefetched_ttfb_ms_for_task = prefetched_ttfb_ms;
    let upstream_attempt_started_at_for_task = upstream_attempt_started_at;
    let upstream_attempt_started_at_utc_for_task = upstream_attempt_started_at_utc;
    let first_byte_timeout_for_task = first_byte_timeout;
    let request_endpoint_for_task = original_uri.path().to_string();
    let stream_timeout_for_task = stream_timeout;
    let response_is_event_stream_for_task = response_is_event_stream;
    let proxy_request_started_at_for_task = proxy_request_started_at;
    let request_body_consumption_started_at_for_task = req_read_started;
    let live_request_streaming_decision_for_task = live_request_streaming_decision.clone();
    let mut live_request_streaming_measurement_for_task = live_request_streaming_measurement;
    let proxy_request_permit_for_task = proxy_request_permit;
    let (tx, rx) = mpsc::channel::<Result<DownstreamResponseChunk, io::Error>>(16);
    let (downstream_body_terminal_tx, mut downstream_body_terminal_rx) =
        watch::channel(DownstreamBodyTerminalState::Open);
    if let Some(observation) = downstream_request_observer.as_ref() {
        observation.activate_reset_monitor();
    }

    tokio::spawn(async move {
        let mut reservation_guard = reservation_guard_for_task;
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
        let mut proxy_request_permit_for_task = Some(proxy_request_permit_for_task);
        let mut t_upstream_ttfb_ms = prefetched_ttfb_ms_for_task;
        let mut stream_started_at: Option<Instant> = None;
        let mut active_stream_timeout =
            prefetched_stream_timeout_for_task.or(stream_timeout_for_task);
        let mut response_preview = RawResponsePreviewBuffer::default();
        let mut response_raw_writer = AsyncStreamingRawPayloadWriter::new(
            state_for_task.as_ref(),
            &invoke_id_for_task,
            "response",
            proxy_settings.response_body_logging_enabled,
            upstream_content_encoding_for_task.as_deref(),
        );
        // A pool attempt observes the same downstream response stream as its invocation.
        // Capture the bytes once and attach the finalized payload to both records below.
        let attempt_response_capture_enabled = proxy_settings.response_body_logging_enabled
            && pending_pool_attempt_record_for_task.is_some();
        let mut stream_response_parser =
            StreamResponsePayloadChunkParser::for_target(capture_target);
        let mut stream_response_decoder =
            IncrementalResponsePayloadDecoder::new(upstream_content_encoding_for_task.as_deref());
        let mut first_token_ms: Option<f64> = None;
        let mut nonstream_parse_buffer = (!response_is_event_stream_for_task).then(|| {
            BoundedResponseParseBuffer::new(BOUNDED_NON_STREAM_RESPONSE_PARSE_LIMIT_BYTES)
        });
        let mut stream_error: Option<String> = None;
        let mut downstream_body_available = true;
        let mut downstream_closed = false;
        let mut successful_terminal_forwarded = false;
        let mut downstream_body_terminal_watch_open = true;
        let mut downstream_first_byte_logged = false;
        let mut forwarded_chunks = 0usize;
        let mut forwarded_bytes = 0usize;
        let mut stream_failure_origin: Option<&'static str> = None;
        let mut upstream_read_error_kind: Option<&'static str> = None;
        let mut downstream_write_error_kind: Option<&'static str> = None;
        let mut post_terminal_upstream_read_error_kind: Option<&'static str> = None;
        let mut post_terminal_upstream_read_error_message: Option<String> = None;
        let mut post_terminal_downstream_write_error_kind: Option<&'static str> = None;
        let mut post_terminal_downstream_write_error_message: Option<String> = None;
        let mut last_upstream_chunk_received_at: Option<Instant> = None;
        let mut last_downstream_forwarded_chunk_at: Option<Instant> = None;
        let mut last_upstream_chunk_gap_ms: Option<u64> = None;

        if let Some(chunk) = prefetched_first_chunk_for_task {
            let chunk_received_at =
                prefetched_first_chunk_received_at_for_task.unwrap_or_else(Instant::now);
            response_preview.append(&chunk);
            response_raw_writer.append(&chunk);
            let successful_terminal_seen_before_chunk =
                stream_response_parser.successful_terminal_seen();
            let decoded_chunk = stream_response_decoder.ingest(&chunk);
            let token_observed = stream_response_parser.ingest_bytes(&decoded_chunk);
            let successful_terminal_in_chunk = !successful_terminal_seen_before_chunk
                && stream_response_parser.successful_terminal_seen();
            if let Some(buffer) = nonstream_parse_buffer.as_mut() {
                buffer.append(&chunk);
            }
            forwarded_chunks = forwarded_chunks.saturating_add(1);
            forwarded_bytes = forwarded_bytes.saturating_add(chunk.len());
            if !transport_bytes_live_counted_for_task {
                state_for_task
                    .dashboard_network_speed_cache
                    .record_response_chunk_bytes(
                        &invoke_id_for_task,
                        &occurred_at_for_task,
                        pool_account_for_task
                            .as_ref()
                            .map(|account| account.account_id),
                        payload_summary_upstream_base_url_host(pool_account_for_task.as_ref())
                            .or_else(|| {
                                pool_account_for_task
                                    .is_none()
                                    .then(|| {
                                        state_for_task.config.openai_upstream_base_url.host_str()
                                    })
                                    .flatten()
                            }),
                        chunk.len(),
                        Utc::now(),
                    );
                schedule_dashboard_network_projection(state_for_task.as_ref());
            }
            stream_started_at = Some(Instant::now());
            last_upstream_chunk_received_at = Some(chunk_received_at);
            if downstream_body_available {
                if tx
                    .send(Ok(DownstreamResponseChunk {
                        bytes: chunk,
                        successful_terminal: successful_terminal_in_chunk,
                    }))
                    .await
                    .is_err()
                {
                    downstream_body_available = false;
                    downstream_closed = true;
                    downstream_write_error_kind = Some("receiver_dropped");
                    let _ = proxy_request_permit_for_task.take();
                } else {
                    let forwarded_at = Instant::now();
                    last_downstream_forwarded_chunk_at = Some(forwarded_at);
                    if first_token_ms.is_none()
                        && should_capture_ttft(capture_target, &request_info_for_task)
                        && token_observed
                    {
                        let observed_ms = forwarded_at
                            .saturating_duration_since(proxy_request_started_at_for_task)
                            .as_secs_f64()
                            * 1_000.0;
                        first_token_ms = Some(observed_ms);
                        live_request_streaming_measurement_for_task.first_token_ms = Some(
                            forwarded_at
                                .saturating_duration_since(
                                    request_body_consumption_started_at_for_task,
                                )
                                .as_secs_f64()
                                * 1_000.0,
                        );
                        broadcast_proxy_capture_first_token_runtime_snapshot(
                            state_for_task.as_ref(),
                            &invoke_id_for_task,
                            &occurred_at_for_task,
                            observed_ms,
                        );
                    }
                    if !downstream_first_byte_logged {
                        downstream_first_byte_logged = true;
                        let downstream_first_byte_elapsed =
                            stream_started.elapsed().as_millis() as u64;
                        if downstream_first_byte_log_at_info(downstream_first_byte_elapsed) {
                            info!(
                                invoke_id = %invoke_id_for_task,
                                downstream_first_byte_elapsed,
                                upstream_ttfb_ms = t_upstream_ttfb_ms,
                                forwarded_bytes,
                                "openai proxy capture streamed first byte downstream"
                            );
                        } else {
                            debug!(
                                invoke_id = %invoke_id_for_task,
                                downstream_first_byte_elapsed,
                                upstream_ttfb_ms = t_upstream_ttfb_ms,
                                forwarded_bytes,
                                "openai proxy capture streamed first byte downstream"
                            );
                        }
                    }
                }
            }
        }

        loop {
            let next_chunk = if let Some(stream_started_at) = stream_started_at {
                if let Some(stream_timeout) = active_stream_timeout {
                    let Some(timeout_budget) =
                        remaining_timeout_budget(stream_timeout, stream_started_at.elapsed())
                    else {
                        let message = pool_upstream_timeout_message(
                            stream_timeout,
                            "waiting for upstream stream completion",
                        );
                        last_upstream_chunk_gap_ms = last_upstream_chunk_received_at
                            .map(|instant| instant.elapsed().as_millis() as u64);
                        if stream_response_parser.successful_terminal_seen() {
                            post_terminal_upstream_read_error_kind = Some("timeout");
                            post_terminal_upstream_read_error_message = Some(message.clone());
                        } else {
                            stream_error = Some(message.clone());
                            stream_failure_origin = Some("proxy_timeout");
                        }
                        if stream_error.is_some()
                            && downstream_body_available
                            && tx.send(Err(io::Error::other(message))).await.is_err()
                        {
                            downstream_body_available = false;
                            downstream_closed = true;
                            downstream_write_error_kind = Some("receiver_dropped");
                        }
                        break;
                    };
                    let next_chunk = tokio::select! {
                        changed = downstream_body_terminal_rx.changed(), if downstream_body_terminal_watch_open => {
                            if changed.is_ok() {
                                proxy_stream_observe_downstream_body_terminal(
                                    *downstream_body_terminal_rx.borrow(),
                                    &mut successful_terminal_forwarded,
                                    &mut downstream_body_available,
                                    &mut downstream_closed,
                                    &mut downstream_write_error_kind,
                                    last_upstream_chunk_received_at,
                                    &mut last_upstream_chunk_gap_ms,
                                );
                            } else {
                                downstream_body_terminal_watch_open = false;
                            }
                            continue;
                        }
                        next_chunk = timeout(timeout_budget, stream.next()) => next_chunk,
                    };
                    match next_chunk {
                        Ok(next_chunk) => next_chunk,
                        Err(_) => {
                            let message = pool_upstream_timeout_message(
                                stream_timeout,
                                "waiting for upstream stream completion",
                            );
                            last_upstream_chunk_gap_ms = last_upstream_chunk_received_at
                                .map(|instant| instant.elapsed().as_millis() as u64);
                            if stream_response_parser.successful_terminal_seen() {
                                post_terminal_upstream_read_error_kind = Some("timeout");
                                post_terminal_upstream_read_error_message = Some(message.clone());
                            } else {
                                stream_error = Some(message.clone());
                                stream_failure_origin = Some("proxy_timeout");
                            }
                            if stream_error.is_some()
                                && downstream_body_available
                                && tx.send(Err(io::Error::other(message))).await.is_err()
                            {
                                downstream_body_available = false;
                                downstream_closed = true;
                                downstream_write_error_kind = Some("receiver_dropped");
                            }
                            break;
                        }
                    }
                } else {
                    tokio::select! {
                        changed = downstream_body_terminal_rx.changed(), if downstream_body_terminal_watch_open => {
                            if changed.is_ok() {
                                proxy_stream_observe_downstream_body_terminal(
                                    *downstream_body_terminal_rx.borrow(),
                                    &mut successful_terminal_forwarded,
                                    &mut downstream_body_available,
                                    &mut downstream_closed,
                                    &mut downstream_write_error_kind,
                                    last_upstream_chunk_received_at,
                                    &mut last_upstream_chunk_gap_ms,
                                );
                            } else {
                                downstream_body_terminal_watch_open = false;
                            }
                            continue;
                        }
                        next_chunk = stream.next() => next_chunk,
                    }
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
                    stream_failure_origin = Some("proxy_timeout");
                    if downstream_body_available
                        && tx.send(Err(io::Error::other(message))).await.is_err()
                    {
                        downstream_body_available = false;
                        downstream_closed = true;
                        downstream_write_error_kind = Some("receiver_dropped");
                    }
                    break;
                };
                let next_chunk = tokio::select! {
                    changed = downstream_body_terminal_rx.changed(), if downstream_body_terminal_watch_open => {
                        if changed.is_ok() {
                            proxy_stream_observe_downstream_body_terminal(
                                *downstream_body_terminal_rx.borrow(),
                                &mut successful_terminal_forwarded,
                                &mut downstream_body_available,
                                &mut downstream_closed,
                                &mut downstream_write_error_kind,
                                last_upstream_chunk_received_at,
                                &mut last_upstream_chunk_gap_ms,
                            );
                        } else {
                            downstream_body_terminal_watch_open = false;
                        }
                        continue;
                    }
                    next_chunk = timeout(timeout_budget, stream.next()) => next_chunk,
                };
                match next_chunk {
                    Ok(next_chunk) => next_chunk,
                    Err(_) => {
                        let message = pool_upstream_timeout_message(
                            first_byte_timeout_for_task,
                            "waiting for first upstream chunk",
                        );
                        stream_error = Some(message.clone());
                        stream_failure_origin = Some("proxy_timeout");
                        if downstream_body_available
                            && tx.send(Err(io::Error::other(message))).await.is_err()
                        {
                            downstream_body_available = false;
                            downstream_closed = true;
                            downstream_write_error_kind = Some("receiver_dropped");
                        }
                        break;
                    }
                }
            } else {
                tokio::select! {
                    changed = downstream_body_terminal_rx.changed(), if downstream_body_terminal_watch_open => {
                        if changed.is_ok() {
                            proxy_stream_observe_downstream_body_terminal(
                                *downstream_body_terminal_rx.borrow(),
                                &mut successful_terminal_forwarded,
                                &mut downstream_body_available,
                                &mut downstream_closed,
                                &mut downstream_write_error_kind,
                                last_upstream_chunk_received_at,
                                &mut last_upstream_chunk_gap_ms,
                            );
                        } else {
                            downstream_body_terminal_watch_open = false;
                        }
                        continue;
                    }
                    next_chunk = stream.next() => next_chunk,
                }
            };
            let Some(next_chunk) = next_chunk else {
                break;
            };
            match next_chunk {
                Ok(chunk) => {
                    let chunk_received_at = prefetched_first_stream_chunk_received_at_for_task
                        .take()
                        .unwrap_or_else(Instant::now);
                    let gap_before_send_ms = last_downstream_forwarded_chunk_at
                        .map(|instant| instant.elapsed().as_millis() as u64);
                    if stream_started_at.is_none() {
                        t_upstream_ttfb_ms = upstream_attempt_started_at_for_task
                            .map(elapsed_ms)
                            .unwrap_or_else(|| elapsed_ms(ttfb_started));
                        stream_started_at = Some(Instant::now());
                        active_stream_timeout =
                            prefetched_stream_timeout_for_task.or(stream_timeout_for_task);
                        let mut running_record = build_running_proxy_capture_record(
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
                            payload_summary_upstream_account_kind(pool_account_for_task.as_ref()),
                            payload_summary_upstream_base_url_host(pool_account_for_task.as_ref()),
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
                        set_proxy_capture_record_request_compression_algorithm(
                            &mut running_record,
                            pending_pool_attempt_record_for_task
                                .as_ref()
                                .and_then(|pending| {
                                    pending.upstream_request_compression_algorithm.as_deref()
                                }),
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
                    let successful_terminal_seen_before_chunk =
                        stream_response_parser.successful_terminal_seen();
                    let decoded_chunk = stream_response_decoder.ingest(&chunk);
                    let token_observed = stream_response_parser.ingest_bytes(&decoded_chunk);
                    let successful_terminal_in_chunk = !successful_terminal_seen_before_chunk
                        && stream_response_parser.successful_terminal_seen();
                    if let Some(buffer) = nonstream_parse_buffer.as_mut() {
                        buffer.append(&chunk);
                    }
                    forwarded_chunks = forwarded_chunks.saturating_add(1);
                    forwarded_bytes = forwarded_bytes.saturating_add(chunk.len());
                    if !transport_bytes_live_counted_for_task {
                        state_for_task
                            .dashboard_network_speed_cache
                            .record_response_chunk_bytes(
                                &invoke_id_for_task,
                                &occurred_at_for_task,
                                pool_account_for_task
                                    .as_ref()
                                    .map(|account| account.account_id),
                                payload_summary_upstream_base_url_host(
                                    pool_account_for_task.as_ref(),
                                )
                                .or_else(|| {
                                    pool_account_for_task
                                        .is_none()
                                        .then(|| {
                                            state_for_task
                                                .config
                                                .openai_upstream_base_url
                                                .host_str()
                                        })
                                        .flatten()
                                }),
                                chunk.len(),
                                Utc::now(),
                            );
                        schedule_dashboard_network_projection(state_for_task.as_ref());
                    }
                    last_upstream_chunk_received_at = Some(chunk_received_at);
                    if successful_terminal_in_chunk
                        && downstream_body_available
                        && let Some(observation) = downstream_request_observer_for_task.as_ref()
                        && let Some(write_error) = observation
                            .wait_for_write_error_window(Duration::ZERO)
                            .await
                    {
                        downstream_body_available = false;
                        downstream_closed = true;
                        downstream_write_error_kind = Some(write_error.kind);
                        last_upstream_chunk_gap_ms = gap_before_send_ms;
                        let _ = proxy_request_permit_for_task.take();
                    }
                    if downstream_body_available {
                        if tx
                            .send(Ok(DownstreamResponseChunk {
                                bytes: chunk,
                                successful_terminal: successful_terminal_in_chunk,
                            }))
                            .await
                            .is_err()
                        {
                            downstream_body_available = false;
                            if !successful_terminal_forwarded {
                                downstream_closed = true;
                                downstream_write_error_kind = Some("receiver_dropped");
                            }
                            last_upstream_chunk_gap_ms = gap_before_send_ms;
                            let _ = proxy_request_permit_for_task.take();
                        } else {
                            let forwarded_at = Instant::now();
                            last_downstream_forwarded_chunk_at = Some(forwarded_at);
                            if first_token_ms.is_none()
                                && should_capture_ttft(capture_target, &request_info_for_task)
                                && token_observed
                            {
                                let observed_ms = elapsed_ms(proxy_request_started_at_for_task);
                                first_token_ms = Some(observed_ms);
                                live_request_streaming_measurement_for_task.first_token_ms =
                                    Some(elapsed_ms(request_body_consumption_started_at_for_task));
                                broadcast_proxy_capture_first_token_runtime_snapshot(
                                    state_for_task.as_ref(),
                                    &invoke_id_for_task,
                                    &occurred_at_for_task,
                                    observed_ms,
                                );
                            }
                            if !downstream_first_byte_logged {
                                downstream_first_byte_logged = true;
                                let downstream_first_byte_elapsed =
                                    stream_started.elapsed().as_millis() as u64;
                                if downstream_first_byte_log_at_info(downstream_first_byte_elapsed)
                                {
                                    info!(
                                        invoke_id = %invoke_id_for_task,
                                        downstream_first_byte_elapsed,
                                        upstream_ttfb_ms = t_upstream_ttfb_ms,
                                        forwarded_bytes,
                                        "openai proxy capture streamed first byte downstream"
                                    );
                                } else {
                                    debug!(
                                        invoke_id = %invoke_id_for_task,
                                        downstream_first_byte_elapsed,
                                        upstream_ttfb_ms = t_upstream_ttfb_ms,
                                        forwarded_bytes,
                                        "openai proxy capture streamed first byte downstream"
                                    );
                                }
                            }
                        }
                    }
                }
                Err(err) => {
                    let msg = format!("upstream stream error: {err}");
                    stream_response_parser.flush_pending_line();
                    let read_error_kind = proxy_stream_upstream_read_error_kind(&err);
                    if stream_response_parser.successful_terminal_seen() {
                        post_terminal_upstream_read_error_kind = Some(read_error_kind);
                        post_terminal_upstream_read_error_message = Some(msg.clone());
                    } else {
                        stream_error = Some(msg.clone());
                        stream_failure_origin = Some("upstream_read");
                        upstream_read_error_kind = Some(read_error_kind);
                    }
                    last_upstream_chunk_gap_ms = last_upstream_chunk_received_at
                        .map(|instant| instant.elapsed().as_millis() as u64);
                    if stream_error.is_some()
                        && downstream_body_available
                        && tx.send(Err(io::Error::other(msg))).await.is_err()
                    {
                        downstream_body_available = false;
                        downstream_closed = true;
                        downstream_write_error_kind = Some("receiver_dropped");
                    }
                    break;
                }
            }
        }
        stream_response_parser.flush_pending_line();
        stream_response_decoder.finish();
        drop(tx);
        drop(proxy_request_permit_for_task.take());
        let downstream_terminal_grace_deadline =
            Instant::now() + PROXY_DOWNSTREAM_WRITE_ERROR_GRACE_PERIOD;
        if !downstream_closed && stream_error.is_none() {
            wait_for_downstream_body_terminal_until(
                &mut downstream_body_terminal_rx,
                downstream_terminal_grace_deadline,
                &mut successful_terminal_forwarded,
                &mut downstream_body_available,
                &mut downstream_closed,
                &mut downstream_write_error_kind,
                last_upstream_chunk_received_at,
                &mut last_upstream_chunk_gap_ms,
            )
            .await;
        }
        if !downstream_closed
            && stream_error.is_none()
            && let Some(observation) = downstream_request_observer_for_task.as_ref()
            && let Some(remaining_grace_period) =
                downstream_terminal_grace_deadline.checked_duration_since(Instant::now())
            && let Some(write_error) = observation
                .wait_for_write_error_window(remaining_grace_period)
                .await
        {
            if proxy_stream_write_error_is_post_terminal(successful_terminal_forwarded) {
                post_terminal_downstream_write_error_kind = Some(write_error.kind);
                post_terminal_downstream_write_error_message = Some(format!(
                    "downstream transport write failed after response.completed ({})",
                    write_error.kind
                ));
            } else {
                downstream_closed = true;
                downstream_write_error_kind = Some(write_error.kind);
                if last_upstream_chunk_gap_ms.is_none() {
                    last_upstream_chunk_gap_ms = last_upstream_chunk_received_at
                        .map(|instant| instant.elapsed().as_millis() as u64);
                }
            }
        }
        if let Some(observation) = downstream_request_observer_for_task.as_ref() {
            observation.finish_response_monitoring();
        }

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
        let req_raw_for_task = req_raw_pending_for_task.finish().await;
        let raw_response_finish_started = Instant::now();
        let resp_raw = response_raw_writer.finish().await;
        let attempt_resp_raw = if attempt_response_capture_enabled {
            resp_raw.clone()
        } else {
            RawPayloadMeta::default()
        };
        let raw_response_write_elapsed = raw_response_finish_started.elapsed().as_millis() as u64;
        if raw_response_write_log_at_info(raw_response_write_elapsed, resp_raw.size_bytes) {
            info!(
                invoke_id = %invoke_id_for_task,
                raw_response_write_elapsed,
                raw_response_bytes = resp_raw.size_bytes,
                raw_response_codec = raw_payload_meta_codec(&resp_raw),
                capture_strategy = if attempt_response_capture_enabled {
                    "shared_invocation_attempt"
                } else {
                    "invocation_only"
                },
                raw_response_truncated = resp_raw.truncated,
                "openai proxy capture response raw writer finished"
            );
        } else {
            debug!(
                invoke_id = %invoke_id_for_task,
                raw_response_write_elapsed,
                raw_response_bytes = resp_raw.size_bytes,
                raw_response_codec = raw_payload_meta_codec(&resp_raw),
                capture_strategy = if attempt_response_capture_enabled {
                    "shared_invocation_attempt"
                } else {
                    "invocation_only"
                },
                raw_response_truncated = resp_raw.truncated,
                "openai proxy capture response raw writer finished"
            );
        }
        let preview_bytes = response_preview.as_slice().to_vec();
        let raw_response_preview = response_preview.into_preview();
        let streamed_response_outcome = stream_response_parser.finish();
        let successful_terminal_seen_upstream = streamed_response_outcome.successful_terminal_seen;
        let preview_looks_like_sse = response_payload_looks_like_sse_after_decode(
            &preview_bytes,
            upstream_content_encoding_for_task.as_deref(),
        );
        let response_is_stream_hint = capture_target != ProxyCaptureTarget::StandaloneSearch
            && (response_is_event_stream_for_task
                || streamed_response_outcome.saw_stream_fields
                || preview_looks_like_sse);
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
        if let Some(reason) = stream_response_decoder.failure_reason()
            && !response_info
                .usage_missing_reason
                .as_deref()
                .is_some_and(|value| value.contains("response_decode_failed:"))
        {
            let existing = response_info.usage_missing_reason.take();
            response_info.usage_missing_reason = Some(match existing {
                Some(existing) => format!("response_decode_failed:{reason};{existing}"),
                None => format!("response_decode_failed:{reason}"),
            });
        }
        let t_resp_parse_ms = elapsed_ms(resp_parse_started);

        if response_info.model.is_none() {
            response_info.model = request_info_for_task.model.clone();
        }
        if response_info.usage_missing_reason.is_none() && stream_error.is_some() {
            response_info.usage_missing_reason = Some("upstream_stream_error".to_string());
        }
        let usage_observed = proxy_stream_usage_observed(&response_info);

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

        let downstream_error_message = if downstream_closed {
            Some(format!(
                "[{}] downstream closed while streaming upstream response",
                PROXY_STREAM_TERMINAL_DOWNSTREAM_CLOSED
            ))
        } else {
            None
        };
        let error_message = if let Some(err) = stream_error {
            Some(format!("[{}] {err}", PROXY_FAILURE_UPSTREAM_STREAM_ERROR))
        } else if had_logical_stream_failure {
            Some(format_upstream_response_failed_message(&response_info))
        } else if !upstream_status.is_success() {
            response_info
                .upstream_error_message
                .clone()
                .or_else(|| extract_error_message_from_response_preview(&preview_bytes))
        } else if pure_downstream_closed {
            downstream_error_message.clone()
        } else {
            None
        };
        let status = proxy_capture_invocation_status(
            upstream_status,
            error_message.is_some(),
            pure_downstream_closed,
        );
        if stream_failure_origin.is_none() {
            if had_logical_stream_failure {
                stream_failure_origin = Some("logical_terminal");
            } else if pure_downstream_closed {
                stream_failure_origin = Some("downstream_write");
            } else {
                stream_failure_origin = proxy_stream_failure_origin_from_usage_reason(
                    response_info.usage_missing_reason.as_deref(),
                );
            }
        }
        let downstream_close_phase = downstream_closed.then_some(if forwarded_chunks == 0 {
            "before_first_byte"
        } else {
            "after_first_byte"
        });
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
                let pool_route_success = !upstream_status.is_client_error()
                    && proxy_capture_response_status_is_success(
                        upstream_status,
                        had_stream_error,
                        had_logical_stream_failure,
                    );
                let request_image_intent = request_info_for_task
                    .image_intent
                    .as_deref()
                    .map(crate::ImageIntent::from_str)
                    .unwrap_or(crate::ImageIntent::Unknown);
                let route_result = if pool_route_success {
                    persist_pool_route_success_then_release(
                        state_for_task.as_ref(),
                        &reservation_key_for_task,
                        record_pool_route_success_for_endpoint_with_image_intent_and_affinity_generation_for_attempt_and_broadcast(
                        state_for_task.as_ref(),
                        account.account_id,
                        upstream_attempt_started_at_utc_for_task.unwrap_or_else(Utc::now),
                        sticky_key_for_task.as_deref(),
                        prompt_cache_key_for_task.as_deref(),
                        None,
                        request_endpoint_for_task.as_str(),
                        request_image_intent,
                        final_codex_imagegen_rewrite.as_ref(),
                        pending_pool_attempt_record_for_task
                            .as_ref()
                            .and_then(|pending| pending.attempt_id),
                        account.sticky_affinity_generation,
                        ),
                    )
                    .await
                } else if pure_downstream_closed {
                    release_pool_routing_reservation(
                        state_for_task.as_ref(),
                        &reservation_key_for_task,
                    );
                    Ok(())
                } else if had_stream_error {
                    let route_message = error_message
                        .as_deref()
                        .unwrap_or("upstream stream error")
                        .to_string();
                    persist_pool_route_failure_then_release_with_guard(
                        state_for_task.as_ref(),
                        &reservation_key_for_task,
                        reservation_guard.as_mut(),
                        record_pool_route_transport_failure_for_attempt_with_kind_and_broadcast(
                            state_for_task.as_ref(),
                            account.account_id,
                            sticky_key_for_task.as_deref(),
                            &route_message,
                            PROXY_FAILURE_UPSTREAM_STREAM_ERROR,
                            None,
                            pending_pool_attempt_record_for_task
                                .as_ref()
                                .and_then(|pending| pending.attempt_id),
                        ),
                    )
                    .await
                } else {
                    let route_message = error_message
                        .as_deref()
                        .unwrap_or("upstream request failed")
                        .to_string();
                    if response_info_is_retryable_responses_overload(
                        upstream_status,
                        &response_info,
                    ) {
                        persist_pool_route_failure_then_release_with_guard(
                            state_for_task.as_ref(),
                            &reservation_key_for_task,
                            reservation_guard.as_mut(),
                            record_pool_route_retryable_overload_failure_for_attempt_and_broadcast(
                                state_for_task.as_ref(),
                                account.account_id,
                                sticky_key_for_task.as_deref(),
                                &route_message,
                                None,
                                pending_pool_attempt_record_for_task
                                    .as_ref()
                                    .and_then(|pending| pending.attempt_id),
                            ),
                        )
                        .await
                    } else {
                        persist_pool_route_failure_then_release_with_guard(
                            state_for_task.as_ref(),
                            &reservation_key_for_task,
                            reservation_guard.as_mut(),
                            record_pool_route_http_failure_for_endpoint_with_image_intent_and_prompt_cache_key_for_attempt_and_broadcast(
                            state_for_task.as_ref(),
                            account.account_id,
                            &account.kind,
                            account.single_account_rotation_enabled,
                            sticky_key_for_task.as_deref(),
                            upstream_status,
                            &route_message,
                            None,
                            request_endpoint_for_task.as_str(),
                            request_image_intent,
                            pending_pool_attempt_record_for_task
                                .as_ref()
                                .and_then(|pending| pending.attempt_id),
                            account.sticky_affinity_generation,
                            prompt_cache_key_for_task
                                .as_deref()
                                .or(request_info_for_task.sticky_key.as_deref()),
                            ),
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
        if let Some(pending_attempt_record) = pending_pool_attempt_record_for_task.as_mut() {
            set_pending_pool_upstream_request_attempt_response_capture(
                pending_attempt_record,
                &attempt_resp_raw,
                Some(response_content_encoding.as_str()),
            );
            update_pending_pool_upstream_request_attempt_http_bytes(
                pending_attempt_record,
                pending_attempt_record
                    .upstream_request_logical_body_bytes
                    .and_then(|value| usize::try_from(value).ok()),
                pending_attempt_record
                    .upstream_request_transmitted_body_bytes
                    .and_then(|value| usize::try_from(value).ok()),
                pending_attempt_record
                    .upstream_request_header_bytes_approx
                    .and_then(|value| usize::try_from(value).ok()),
                Some(forwarded_bytes),
                pending_attempt_record
                    .upstream_response_header_bytes_approx
                    .and_then(|value| usize::try_from(value).ok()),
            );
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
                if downstream_closed {
                    Some(upstream_status)
                } else {
                    None
                },
                attempt_failure_kind,
                (!pure_downstream_closed)
                    .then_some(error_message.as_deref())
                    .flatten(),
                downstream_error_message.as_deref(),
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
        let pool_capture_success = proxy_capture_response_status_is_success(
            upstream_status,
            had_stream_error,
            had_logical_stream_failure,
        );
        if pool_capture_success
            && let (Some(prompt_cache_key), Some(account)) = (
                prompt_cache_key_for_task.as_deref(),
                pool_account_for_task.as_ref(),
            )
            && (request_info_for_task.contains_encrypted_content
                || response_info.contains_encrypted_content)
        {
            match confirm_prompt_cache_encrypted_session_owner_success_if_enabled(
                state_for_task.as_ref(),
                prompt_cache_key,
                account.account_id,
            )
            .await
            {
                Ok(true) => {
                    if let Err(err) =
                        promote_prompt_cache_group_binding_to_upstream_account_and_broadcast(
                            state_for_task.as_ref(),
                            prompt_cache_key,
                            account.account_id,
                        )
                        .await
                    {
                        warn!(
                            invoke_id = %invoke_id_for_task,
                            prompt_cache_key,
                            account_id = account.account_id,
                            error = %err,
                            "failed to promote prompt cache group binding after encrypted session success"
                        );
                    }
                }
                Ok(false) => {}
                Err(err) => {
                    warn!(
                        invoke_id = %invoke_id_for_task,
                        prompt_cache_key,
                        account_id = account.account_id,
                        error = %err,
                        "failed to persist encrypted session owner"
                    );
                }
            }
        }
        let (billing_service_tier, pricing_mode) =
            resolve_proxy_billing_service_tier_and_pricing_mode_for_account(
                None,
                request_info_for_task.requested_service_tier.as_deref(),
                response_info.service_tier.as_deref(),
                pool_account_for_task.as_ref(),
            );
        let (cost_breakdown, cost_estimated, price_version) =
            estimate_proxy_cost_breakdown_from_shared_catalog(
                &state_for_task.pricing_catalog,
                response_info.model.as_deref(),
                &response_info.usage,
                billing_service_tier.as_deref(),
                pricing_mode,
            )
            .await;
        let cost = cost_breakdown.map(ProxyCostBreakdown::total);
        let request_chain_metadata_for_payload = (had_stream_error
            || had_logical_stream_failure
            || pure_downstream_closed
            || !upstream_status.is_success())
        .then_some(&request_chain_metadata_for_task);
        if let (Some(debug), Some(rewrite_rx)) = (
            oauth_responses_debug_for_task.as_mut(),
            live_oauth_rewrite_rx_for_task.as_ref(),
        ) && let Some(rewrite) = rewrite_rx.borrow().clone()
        {
            debug.rewrite = rewrite;
            debug.responses_body_mode = Some("live_streaming_rewrite");
        }
        let payload = with_live_request_streaming_payload_summary(
            with_proxy_stream_terminal_diagnostics(
                with_codex_imagegen_rewrite_payload_summary(
                    with_image_tool_rewrite_payload_summary(
                        build_proxy_payload_summary(ProxyPayloadSummary {
                            target: capture_target,
                            status: upstream_status,
                            is_stream: request_info_for_task.is_stream,
                            request_contains_encrypted_content: request_info_for_task
                                .contains_encrypted_content,
                            response_contains_encrypted_content: response_info
                                .contains_encrypted_content,
                            compaction_request_kind: request_info_for_task.compaction_request_kind,
                            compaction_response_kind: resolve_compaction_response_kind_for_payload(
                                capture_target,
                                response_info.compaction_response_kind,
                            ),
                            image_intent: request_info_for_task.image_intent.as_deref(),
                            request_model: request_info_for_task.model.as_deref(),
                            requested_service_tier: request_info_for_task
                                .requested_service_tier
                                .as_deref(),
                            billing_service_tier: billing_service_tier.as_deref(),
                            reasoning_effort: request_info_for_task.reasoning_effort.as_deref(),
                            response_model: response_info.model.as_deref(),
                            usage_missing_reason: response_info.usage_missing_reason.as_deref(),
                            request_parse_error: request_info_for_task.parse_error.as_deref(),
                            request_compression_algorithm: pending_pool_attempt_record_for_task
                                .as_ref()
                                .and_then(|pending| {
                                    pending.upstream_request_compression_algorithm.as_deref()
                                })
                                .or_else(|| {
                                    pool_account_for_task
                                        .is_none()
                                        .then(|| {
                                            direct_http_approx_for_task
                                                .request_compression
                                                .as_ref()
                                                .map(|value| value.algorithm.as_str())
                                        })
                                        .flatten()
                                }),
                            request_compression_mode: if pool_account_for_task.is_none() {
                                direct_http_approx_for_task
                                    .request_compression
                                    .as_ref()
                                    .map(|value| value.mode.as_str())
                            } else {
                                None
                            },
                            request_compression_logical_body_bytes: if pool_account_for_task
                                .is_none()
                            {
                                direct_http_approx_for_task
                                    .request_compression
                                    .as_ref()
                                    .map(|value| value.logical_body_bytes)
                            } else {
                                None
                            },
                            request_compression_transmitted_body_bytes: if pool_account_for_task
                                .is_none()
                            {
                                direct_http_approx_for_task
                                    .request_compression
                                    .as_ref()
                                    .map(|value| value.transmitted_body_bytes)
                            } else {
                                None
                            },
                            request_compression_transmission_complete: pool_account_for_task
                                .is_none()
                                .then_some(
                                    direct_http_approx_for_task.request_transmission_complete,
                                ),
                            failure_kind,
                            requester_ip: requester_ip_for_task.as_deref(),
                            request_user_agent: request_chain_metadata_for_payload
                                .and_then(|metadata| metadata.user_agent.as_deref()),
                            request_x_forwarded_for: request_chain_metadata_for_payload
                                .and_then(|metadata| metadata.x_forwarded_for.as_deref()),
                            request_forwarded: request_chain_metadata_for_payload
                                .and_then(|metadata| metadata.forwarded.as_deref()),
                            request_x_real_ip: request_chain_metadata_for_payload
                                .and_then(|metadata| metadata.x_real_ip.as_deref()),
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
                            prompt_cache_key_attribution_source: request_info_for_task
                                .prompt_cache_key_attribution_source
                                .as_deref(),
                            client_fingerprint: client_attribution_context_for_task
                                .fingerprint
                                .as_deref(),
                            client_header_fingerprints: Some(
                                &client_attribution_context_for_task.header_fingerprints,
                            )
                            .filter(|fingerprints| !fingerprints.is_empty()),
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
                            oauth_account_header_attached:
                                oauth_account_header_attached_for_account(
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
                            stream_failure_origin,
                            upstream_read_error_kind,
                            content_encoding_chain: Some(response_content_encoding.as_str()),
                            forwarded_chunk_count: Some(forwarded_chunks),
                            forwarded_bytes: Some(forwarded_bytes),
                            usage_observed: Some(usage_observed),
                            downstream_close_phase,
                            downstream_write_error_kind,
                            last_upstream_chunk_gap_ms,
                            upstream_approx_upload_bytes: pool_account_for_task
                                .is_none()
                                .then_some(direct_http_approx_for_task.approx_upload_bytes),
                            upstream_approx_download_bytes: pool_account_for_task
                                .is_none()
                                .then_some(
                                    direct_http_approx_for_task
                                        .approx_download_bytes_before_response_body
                                        .saturating_add(forwarded_bytes),
                                ),
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
                                .and(pending_pool_attempt_terminal_reason.as_deref()),
                            blocked_binding: None,
                        }),
                        image_tool_rewrite_for_task.as_ref(),
                    ),
                    final_codex_imagegen_rewrite.as_ref(),
                ),
                successful_terminal_seen_upstream.then_some("completed"),
                post_terminal_upstream_read_error_kind,
                post_terminal_upstream_read_error_message.as_deref(),
                post_terminal_downstream_write_error_kind,
                post_terminal_downstream_write_error_message.as_deref(),
            ),
            &live_request_streaming_decision_for_task,
            &live_request_streaming_measurement_for_task,
        );

        let record = ProxyCaptureRecord {
            invoke_id: invoke_id_for_task,
            occurred_at: occurred_at_for_task,
            model: response_info.model,
            usage: response_info.usage,
            cost,
            cost_breakdown,
            cost_estimated,
            price_version,
            status,
            error_message,
            failure_kind: failure_kind.map(|kind| kind.to_string()),
            payload: Some(payload),
            raw_response: raw_response_preview,
            response_body_preview_enabled: proxy_settings.response_body_logging_enabled,
            req_raw: req_raw_for_task,
            resp_raw,
            timings: StageTimings {
                t_total_ms: 0.0,
                t_req_read_ms,
                t_req_parse_ms,
                t_upstream_connect_ms,
                t_upstream_ttfb_ms,
                first_token_ms,
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
            finalize_deferred_pool_early_phase_cleanup_guard_after_terminal_invocation(
                &mut deferred_pool_early_phase_cleanup_guard_for_task,
                terminal_invocation_persisted,
            );
            if !final_attempt_persisted {
                info!(
                    proxy_request_id,
                    "terminal invocation persisted but final pool attempt state did not; disarming deferred orphan recovery to preserve the completed invocation"
                );
            }
        } else if !terminal_invocation_persisted
            && deferred_pool_early_phase_cleanup_guard_for_task.is_some()
        {
            info!(
                proxy_request_id,
                "terminal invocation did not persist; keeping deferred orphan recovery armed until the pending attempt row is repaired"
            );
        }
    });
    disarm_pool_invocation_cleanup_guard(&mut pool_invocation_cleanup_guard);

    response_builder
        .body(Body::from_stream(TrackedDownstreamReceiverStream::new(
            ReceiverStream::new(rx),
            downstream_body_terminal_tx,
        )))
        .map_err(|err| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("failed to build proxy response: {err}"),
            )
        })
}

fn should_capture_ttft(
    capture_target: ProxyCaptureTarget,
    request_info: &RequestCaptureInfo,
) -> bool {
    if !request_info.is_stream {
        return false;
    }
    if matches!(
        capture_target,
        ProxyCaptureTarget::ImageGenerations | ProxyCaptureTarget::ImageEdits
    ) {
        return false;
    }
    !matches!(
        request_info
            .image_intent
            .as_deref()
            .map(ImageIntent::from_str),
        Some(ImageIntent::Yes | ImageIntent::DirectImage)
    )
}

pub(crate) async fn read_request_body_with_limit(
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

pub(crate) async fn read_request_body_snapshot_with_limit(
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

pub(crate) async fn read_request_body_snapshot_with_partial_limit(
    body: Body,
    body_limit: usize,
    request_read_timeout: Duration,
    proxy_request_id: u64,
) -> Result<PoolReplayBodySnapshot, RequestBodyReadError> {
    const ERROR_PARTIAL_BODY_LIMIT_BYTES: usize = 64 * 1024;

    let mut buffer = PoolReplayBodyBuffer::new(proxy_request_id);
    let mut partial_body = Vec::new();
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
                partial_body,
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
                    partial_body,
                });
            }
        };

        let Some(chunk) = next_chunk else {
            return buffer.finish().await.map_err(|err| RequestBodyReadError {
                status: StatusCode::BAD_GATEWAY,
                message: format!("failed to cache request body for replay: {err}"),
                failure_kind: PROXY_FAILURE_FAILED_CONTACT_UPSTREAM,
                partial_body,
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
                    partial_body,
                });
            }
        };

        if data_len.saturating_add(chunk.len()) > body_limit {
            let allowed = body_limit.saturating_sub(data_len);
            if allowed > 0 {
                append_bounded_partial_body(
                    &mut partial_body,
                    &chunk[..allowed.min(chunk.len())],
                    ERROR_PARTIAL_BODY_LIMIT_BYTES,
                );
            }
            return Err(RequestBodyReadError {
                status: StatusCode::PAYLOAD_TOO_LARGE,
                message: format!("request body exceeds {body_limit} bytes"),
                failure_kind: PROXY_FAILURE_BODY_TOO_LARGE,
                partial_body,
            });
        }
        data_len = data_len.saturating_add(chunk.len());
        append_bounded_partial_body(&mut partial_body, &chunk, ERROR_PARTIAL_BODY_LIMIT_BYTES);

        buffer
            .append(&chunk)
            .await
            .map_err(|err| RequestBodyReadError {
                status: StatusCode::BAD_GATEWAY,
                message: format!("failed to cache request body for replay: {err}"),
                failure_kind: PROXY_FAILURE_FAILED_CONTACT_UPSTREAM,
                partial_body: partial_body.clone(),
            })?;
    }
}

pub(crate) fn append_bounded_partial_body(partial_body: &mut Vec<u8>, chunk: &[u8], limit: usize) {
    let remaining = limit.saturating_sub(partial_body.len());
    if remaining > 0 {
        partial_body.extend_from_slice(&chunk[..chunk.len().min(remaining)]);
    }
}

pub(crate) fn request_body_size_bucket(bytes: usize) -> &'static str {
    match bytes {
        0 => "empty",
        1..=4096 => "le_4k",
        4097..=65536 => "le_64k",
        65537..=1048576 => "le_1m",
        1048577..=8388608 => "le_8m",
        _ => "gt_8m",
    }
}

pub(crate) fn capture_request_body_read_log_at_info(bytes: usize, elapsed_ms: f64) -> bool {
    bytes >= POOL_REQUEST_REPLAY_MEMORY_THRESHOLD_BYTES || elapsed_ms >= 1_000.0
}

pub(crate) fn downstream_first_byte_log_at_info(elapsed_ms: u64) -> bool {
    elapsed_ms >= 2_000
}

pub(crate) fn raw_response_write_log_at_info(elapsed_ms: u64, bytes: i64) -> bool {
    elapsed_ms >= 500 || bytes >= POOL_REQUEST_REPLAY_MEMORY_THRESHOLD_BYTES as i64
}

pub(crate) fn resolve_compaction_response_kind_for_payload(
    capture_target: ProxyCaptureTarget,
    parsed_kind: Option<CompactionKind>,
) -> Option<CompactionKind> {
    if capture_target == ProxyCaptureTarget::StandaloneSearch {
        None
    } else if capture_target == ProxyCaptureTarget::ResponsesCompact {
        Some(CompactionKind::Compact)
    } else {
        parsed_kind
    }
}

fn live_first_replay_exclusion(
    pool_attempt_count: usize,
    failure_kind: &str,
    message: &str,
    initial_account_id: i64,
) -> Option<i64> {
    (pool_attempt_count > 0 && !pool_failure_is_timeout_shaped(failure_kind, message))
        .then_some(initial_account_id)
}

#[cfg(test)]
mod dispatch_tests {
    use super::*;

    #[test]
    fn final_route_gate_starts_only_for_model_only_routes() {
        assert!(!live_route_probe_can_start_before_eof(
            &PoolReplayBodyKeyProbe::default()
        ));
        assert!(live_route_probe_can_start_before_eof(
            &PoolReplayBodyKeyProbe {
                model: Some("gpt-5.6".to_string()),
                ..PoolReplayBodyKeyProbe::default()
            }
        ));
        assert!(!live_route_probe_can_start_before_eof(
            &PoolReplayBodyKeyProbe {
                model: Some("gpt-5.6".to_string()),
                sticky_key: Some("sticky".to_string()),
                ..PoolReplayBodyKeyProbe::default()
            }
        ));
    }

    #[test]
    fn route_dependency_profile_uses_only_active_routing_factors() {
        let mut probe = PoolReplayBodyKeyProbe {
            sticky_key: Some("sticky".to_string()),
            prompt_cache_key: Some("prompt".to_string()),
            contains_encrypted_content: true,
            image_intent: ImageIntent::Yes,
            ..PoolReplayBodyKeyProbe::default()
        };
        assert_eq!(
            live_route_dependency_factors(&probe, false),
            vec!["model", "sticky", "prompt_cache", "image_capability"]
        );
        assert_eq!(
            live_route_dependency_factors(&probe, true),
            vec![
                "model",
                "sticky",
                "prompt_cache",
                "encrypted_owner",
                "image_capability"
            ]
        );
        probe.image_intent = ImageIntent::No;
        assert!(!live_route_dependency_factors(&probe, true).contains(&"image_capability"));
    }

    #[tokio::test]
    async fn wait_for_downstream_body_terminal_until_times_out_when_body_stays_open() {
        let (_tx, mut rx) = watch::channel(DownstreamBodyTerminalState::Open);
        let mut downstream_body_available = true;
        let mut downstream_closed = false;
        let mut downstream_write_error_kind = None;
        let mut last_upstream_chunk_gap_ms = None;

        tokio::time::timeout(
            Duration::from_millis(200),
            wait_for_downstream_body_terminal_until(
                &mut rx,
                Instant::now() + Duration::from_millis(25),
                &mut false,
                &mut downstream_body_available,
                &mut downstream_closed,
                &mut downstream_write_error_kind,
                None,
                &mut last_upstream_chunk_gap_ms,
            ),
        )
        .await
        .expect("body-terminal wait should not hang forever");
        assert!(!downstream_closed);
        assert!(downstream_body_available);
        assert!(downstream_write_error_kind.is_none());
        assert!(last_upstream_chunk_gap_ms.is_none());
    }

    #[tokio::test]
    async fn wait_for_downstream_body_terminal_until_marks_dropped_before_deadline() {
        let (tx, mut rx) = watch::channel(DownstreamBodyTerminalState::Open);
        let mut downstream_body_available = true;
        let mut downstream_closed = false;
        let mut downstream_write_error_kind = None;
        let mut last_upstream_chunk_gap_ms = None;
        let send_drop = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(10)).await;
            let _ = tx.send(DownstreamBodyTerminalState::Dropped);
        });

        wait_for_downstream_body_terminal_until(
            &mut rx,
            Instant::now() + Duration::from_millis(200),
            &mut false,
            &mut downstream_body_available,
            &mut downstream_closed,
            &mut downstream_write_error_kind,
            Some(Instant::now()),
            &mut last_upstream_chunk_gap_ms,
        )
        .await;
        send_drop.await.expect("join drop sender");
        assert!(!downstream_body_available);
        assert!(downstream_closed);
        assert_eq!(downstream_write_error_kind, Some("body_dropped"));
    }

    #[tokio::test]
    async fn downstream_terminal_chunk_preserves_success_latch_when_body_is_released() {
        let (terminal_tx, mut terminal_rx) = watch::channel(DownstreamBodyTerminalState::Open);
        let (response_tx, response_rx) = mpsc::channel(1);
        let mut stream =
            TrackedDownstreamReceiverStream::new(ReceiverStream::new(response_rx), terminal_tx);
        let mut successful_terminal_forwarded = false;
        let mut downstream_body_available = true;
        let mut downstream_closed = false;
        let mut downstream_write_error_kind = None;
        let mut last_upstream_chunk_gap_ms = None;

        response_tx
            .send(Ok(DownstreamResponseChunk {
                bytes: Bytes::from_static(b"event: response.completed\n\n"),
                successful_terminal: true,
            }))
            .await
            .expect("queue successful terminal chunk");
        let chunk = stream
            .next()
            .await
            .expect("terminal chunk should reach downstream body")
            .expect("terminal chunk should not be an error");
        assert!(chunk.starts_with(b"event: response.completed"));
        drop(stream);

        assert_eq!(
            *terminal_rx.borrow_and_update(),
            DownstreamBodyTerminalState::SuccessfulTerminalCompleted
        );
        wait_for_downstream_body_terminal_until(
            &mut terminal_rx,
            Instant::now() + Duration::from_millis(25),
            &mut successful_terminal_forwarded,
            &mut downstream_body_available,
            &mut downstream_closed,
            &mut downstream_write_error_kind,
            Some(Instant::now()),
            &mut last_upstream_chunk_gap_ms,
        )
        .await;

        assert!(successful_terminal_forwarded);
        assert!(downstream_body_available);
        assert!(!downstream_closed);
        assert!(downstream_write_error_kind.is_none());
    }

    #[tokio::test]
    async fn wait_for_downstream_body_terminal_marks_drop_before_successful_terminal() {
        let (tx, mut rx) = watch::channel(DownstreamBodyTerminalState::Dropped);
        let mut downstream_body_available = true;
        let mut downstream_closed = false;
        let mut downstream_write_error_kind = None;
        let mut last_upstream_chunk_gap_ms = None;

        wait_for_downstream_body_terminal_until(
            &mut rx,
            Instant::now() + Duration::from_millis(25),
            &mut true,
            &mut downstream_body_available,
            &mut downstream_closed,
            &mut downstream_write_error_kind,
            Some(Instant::now()),
            &mut last_upstream_chunk_gap_ms,
        )
        .await;

        drop(tx);
        assert!(!downstream_body_available);
        assert!(downstream_closed);
        assert_eq!(downstream_write_error_kind, Some("body_dropped"));
        assert!(last_upstream_chunk_gap_ms.is_some());
    }

    #[test]
    fn live_first_replay_exclusion_requires_a_real_non_timeout_attempt() {
        assert_eq!(
            live_first_replay_exclusion(0, PROXY_FAILURE_FAILED_CONTACT_UPSTREAM, "local error", 7),
            None,
            "a local preparation error must retain the selected route for replay"
        );
        assert_eq!(
            live_first_replay_exclusion(
                1,
                PROXY_FAILURE_UPSTREAM_HANDSHAKE_TIMEOUT,
                "timed out",
                7
            ),
            None,
            "a timeout-shaped upstream failure retains its same-route retry"
        );
        assert_eq!(
            live_first_replay_exclusion(
                1,
                PROXY_FAILURE_FAILED_CONTACT_UPSTREAM,
                "upstream reset",
                7
            ),
            Some(7),
            "a completed non-timeout upstream attempt excludes its failed route"
        );
    }

    #[test]
    fn downstream_write_error_is_post_terminal_only_after_successful_terminal_forwarding() {
        assert!(!proxy_stream_write_error_is_post_terminal(false));
        assert!(proxy_stream_write_error_is_post_terminal(true));
    }

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
