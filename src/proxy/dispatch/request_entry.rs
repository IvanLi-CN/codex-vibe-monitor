use super::*;
pub(crate) async fn proxy_openai_v1_inner(
    state: Arc<AppState>,
    proxy_request_id: u64,
    original_uri: Uri,
    method: Method,
    headers: HeaderMap,
    body: Body,
    target_url: Url,
    peer_ip: Option<IpAddr>,
    pool_route_active: bool,
    runtime_timeouts: PoolRoutingTimeoutSettingsResolved,
    proxy_request_permit: Option<ProxyRequestConcurrencyPermit>,
    downstream_request_observer: Option<DownstreamRequestObserver>,
    proxy_request_started_at: Instant,
) -> Result<Response, ProxyErrorResponse> {
    crate::observability::observed_future(
        state.observability.enabled,
        "proxy_request_dispatch",
        proxy_openai_v1_inner_impl(
            state,
            proxy_request_id,
            original_uri,
            method,
            headers,
            body,
            target_url,
            peer_ip,
            pool_route_active,
            runtime_timeouts,
            proxy_request_permit,
            downstream_request_observer,
            proxy_request_started_at,
        ),
    )
    .await
}

async fn proxy_openai_v1_inner_impl(
    state: Arc<AppState>,
    proxy_request_id: u64,
    original_uri: Uri,
    method: Method,
    headers: HeaderMap,
    body: Body,
    target_url: Url,
    peer_ip: Option<IpAddr>,
    pool_route_active: bool,
    runtime_timeouts: PoolRoutingTimeoutSettingsResolved,
    mut proxy_request_permit: Option<ProxyRequestConcurrencyPermit>,
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
        return proxy_openai_v1_capture_target(
            state,
            proxy_request_id,
            &original_uri,
            headers,
            body,
            target,
            target_url,
            peer_ip,
            pool_route_active,
            runtime_timeouts,
            proxy_request_permit.take(),
            downstream_request_observer,
            proxy_request_started_at,
        )
        .await
        .map_err(|error| ProxyErrorResponse {
            retry_after_secs: retry_after_secs_for_proxy_error(error.status, &error.message),
            status: error.status,
            message: error.message,
            cvm_id: error.invoke_id,
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

pub(crate) struct PreparedProxyCaptureRequest {
    pub(crate) state: Arc<AppState>,
    pub(crate) proxy_request_id: u64,
    pub(crate) original_uri: Uri,
    pub(crate) headers: HeaderMap,
    pub(crate) capture_target: ProxyCaptureTarget,
    pub(crate) target_url: Url,
    pub(crate) pool_route_active: bool,
    pub(crate) runtime_timeouts: PoolRoutingTimeoutSettingsResolved,
    pub(crate) proxy_request_permit: ProxyRequestConcurrencyPermit,
    pub(crate) downstream_request_observer: Option<DownstreamRequestObserver>,
    pub(crate) proxy_request_started_at: Instant,
    pub(crate) capture_started: Instant,
    pub(crate) pool_routing_reservation_key: String,
    pub(crate) requester_ip: Option<String>,
    pub(crate) request_chain_metadata: RequestChainMetadata,
    pub(crate) sticky_key: Option<String>,
    pub(crate) prompt_cache_key: Option<String>,
    pub(crate) client_attribution_context: ClientPromptCacheAttributionContext,
    pub(crate) proxy_settings: ProxyModelSettings,
    pub(crate) request_info: RequestCaptureInfo,
    pub(crate) semantic_projection: RequestSemanticProjection,
    pub(crate) prompt_cache_binding_constraint: Option<PromptCacheConversationBindingConstraint>,
    pub(crate) encrypted_owner_auto_guard_active: bool,
    pub(crate) prompt_cache_conversation_override: Option<ConversationRoutingOverride>,
    pub(crate) pool_attempt_trace_context: Option<PoolUpstreamAttemptTraceContext>,
    pub(crate) t_req_read_ms: f64,
    pub(crate) t_req_parse_ms: f64,
    pub(crate) invoke_id: String,
    pub(crate) occurred_at: String,
    pub(crate) pool_invocation_cleanup_guard: Option<PoolInvocationCleanupGuard>,
}

pub(crate) enum PreparedProxyCaptureRequestResult {
    Ready(Box<PreparedProxyCaptureRequest>),
    Response(Response),
}

pub(crate) async fn prepare_proxy_capture_request(
    state: Arc<AppState>,
    proxy_request_id: u64,
    original_uri: &Uri,
    headers: HeaderMap,
    body: Body,
    capture_target: ProxyCaptureTarget,
    target_url: Url,
    peer_ip: Option<IpAddr>,
    pool_route_active: bool,
    runtime_timeouts: PoolRoutingTimeoutSettingsResolved,
    mut proxy_request_permit: Option<ProxyRequestConcurrencyPermit>,
    downstream_request_observer: Option<DownstreamRequestObserver>,
    proxy_request_started_at: Instant,
) -> Result<PreparedProxyCaptureRequestResult, ProxyCaptureDispatchError> {
    if !pool_route_active {
        return Err(ProxyCaptureDispatchError {
            status: StatusCode::UNAUTHORIZED,
            message: "pool route key missing or invalid".to_string(),
            invoke_id: None,
        });
    }
    if prompt_cache_key_headers_are_oversized(&headers) {
        return Err(ProxyCaptureDispatchError {
            status: StatusCode::PAYLOAD_TOO_LARGE,
            message: "prompt-cache key exceeds the supported 512-byte limit".to_string(),
            invoke_id: None,
        });
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
    let body_limit = state.config.openai_proxy_max_request_body_bytes;
    let requester_ip = extract_requester_ip(&headers, peer_ip);
    let request_chain_metadata = request_chain_metadata_from_headers(&headers);
    let header_sticky_key = extract_sticky_key_from_headers(&headers);
    let header_prompt_cache_key = extract_prompt_cache_key_from_headers(&headers);
    let client_attribution_context = client_prompt_cache_attribution_context_from_headers(&headers);
    let proxy_settings = state.proxy_model_settings.read().await.clone();

    let diagnostic_read = crate::observability::diagnostics::phase(
        crate::observability::diagnostics::Phase::RequestRead,
    );
    let req_read_started = Instant::now();
    let request_body_snapshot = match read_request_body_snapshot_with_partial_limit(
        body,
        body_limit,
        runtime_timeouts.request_read_timeout,
        proxy_request_id,
    )
    .await
    {
        Ok(bytes) => bytes,
        Err(read_err) => {
            let invoke_id = match allocate_proxy_invoke_id_with_active_lease(
                state.as_ref(),
                header_prompt_cache_key.as_deref(),
            )
            .await
            {
                Ok(invoke_id) => invoke_id,
                Err(err) => {
                    warn!(
                        proxy_request_id,
                        error = %err,
                        "failed to allocate proxy invoke id after request body read failure"
                    );
                    drop(proxy_request_permit);
                    return Ok(PreparedProxyCaptureRequestResult::Response(
                        build_proxy_invoke_id_allocation_error_response(err),
                    ));
                }
            };
            let occurred_at = format_naive(Utc::now().with_timezone(&Shanghai).naive_local());
            let mut pool_invocation_cleanup_guard = Some({
                PoolInvocationCleanupGuard::new(
                    state.clone(),
                    InvocationRecoverySelector::new(invoke_id.clone(), occurred_at.clone()),
                    "request_drop_guard",
                    header_prompt_cache_key.as_deref(),
                )
            });
            emit_admitted_proxy_capture_runtime_snapshot(
                state.as_ref(),
                &invoke_id,
                &occurred_at,
                capture_target,
                requester_ip.as_deref(),
                header_sticky_key.as_deref(),
                header_prompt_cache_key.as_deref(),
            )
            .await;
            let response_envelope =
                build_local_capture_error_envelope(&invoke_id, read_err.status, &read_err.message);
            if let Some(guard) = diagnostic_read {
                guard.complete();
            }
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

            warn!(
                proxy_request_id,
                status = %read_err.status,
                failure_kind = read_err.failure_kind,
                error = %read_err.message,
                elapsed_ms = t_req_read_ms,
                "openai proxy request body read failed"
            );

            let record = ProxyCaptureRecord {
                invoke_id: invoke_id.clone(),
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
                payload: Some(build_proxy_payload_summary(ProxyPayloadSummary {
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
                })),
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
            return Err(ProxyCaptureDispatchError {
                status: read_err.status,
                message: read_err.message,
                invoke_id: Some(invoke_id),
            });
        }
    };
    let t_req_read_ms = elapsed_ms(req_read_started);
    let request_body_snapshot_kind = pool_request_snapshot_kind(&request_body_snapshot);
    let request_body_bytes_len = pool_request_snapshot_body_bytes(&request_body_snapshot);
    if capture_request_body_read_log_at_info(request_body_bytes_len, t_req_read_ms) {
        info!(
            proxy_request_id,
            body_read_done = true,
            body_read_elapsed_ms = t_req_read_ms,
            request_body_bytes = request_body_bytes_len,
            body_size_bucket = request_body_size_bucket(request_body_bytes_len),
            request_body_snapshot_kind,
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
            "openai proxy capture request body read completed"
        );
    }
    let diagnostic_parse = crate::observability::diagnostics::phase(
        crate::observability::diagnostics::Phase::RequestParse,
    );
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
    if let Some(guard) = diagnostic_parse {
        guard.complete();
    }
    let mut request_info = semantic_projection.request_info.clone();
    if let Some(context) = crate::observability::diagnostics::current() {
        context.ttft_applicable(should_capture_ttft(capture_target, &request_info));
    }
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
    let invoke_id =
        match allocate_proxy_invoke_id_with_active_lease(&state, prompt_cache_key.as_deref()).await
        {
            Ok(invoke_id) => invoke_id,
            Err(err) => {
                warn!(
                    proxy_request_id,
                    prompt_cache_key_fingerprint = prompt_cache_key
                        .as_deref()
                        .map(prompt_cache_key_fingerprint),
                    error = %err,
                    "failed to allocate proxy invoke id after request semantic projection"
                );
                return Err(ProxyCaptureDispatchError {
                    status: StatusCode::SERVICE_UNAVAILABLE,
                    message: format!("failed to allocate proxy invoke id: {err}"),
                    invoke_id: None,
                });
            }
        };
    let occurred_at = format_naive(Utc::now().with_timezone(&Shanghai).naive_local());
    let mut pool_invocation_cleanup_guard = Some({
        PoolInvocationCleanupGuard::new(
            state.clone(),
            InvocationRecoverySelector::new(invoke_id.clone(), occurred_at.clone()),
            "request_drop_guard",
            prompt_cache_key.as_deref(),
        )
    });
    emit_admitted_proxy_capture_runtime_snapshot(
        state.as_ref(),
        &invoke_id,
        &occurred_at,
        capture_target,
        requester_ip.as_deref(),
        sticky_key.as_deref(),
        prompt_cache_key.as_deref(),
    )
    .await;
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
                )
                .await;
                if terminal_invocation_persisted {
                    disarm_pool_invocation_cleanup_guard(&mut pool_invocation_cleanup_guard);
                }
                return Err(ProxyCaptureDispatchError {
                    status,
                    message,
                    invoke_id: Some(invoke_id.clone()),
                });
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
                )
                .await;
                if terminal_invocation_persisted {
                    disarm_pool_invocation_cleanup_guard(&mut pool_invocation_cleanup_guard);
                }
                return Err(ProxyCaptureDispatchError {
                    status,
                    message,
                    invoke_id: Some(invoke_id.clone()),
                });
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

    Ok(PreparedProxyCaptureRequestResult::Ready(Box::new(
        PreparedProxyCaptureRequest {
            state,
            proxy_request_id,
            original_uri: original_uri.clone(),
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
        },
    )))
}

pub(crate) async fn proxy_openai_v1_capture_target(
    state: Arc<AppState>,
    proxy_request_id: u64,
    original_uri: &Uri,
    headers: HeaderMap,
    body: Body,
    capture_target: ProxyCaptureTarget,
    target_url: Url,
    peer_ip: Option<IpAddr>,
    pool_route_active: bool,
    runtime_timeouts: PoolRoutingTimeoutSettingsResolved,
    proxy_request_permit: Option<ProxyRequestConcurrencyPermit>,
    downstream_request_observer: Option<DownstreamRequestObserver>,
    proxy_request_started_at: Instant,
) -> Result<Response, ProxyCaptureDispatchError> {
    let prepared = match prepare_proxy_capture_request(
        state,
        proxy_request_id,
        original_uri,
        headers,
        body,
        capture_target,
        target_url,
        peer_ip,
        pool_route_active,
        runtime_timeouts,
        proxy_request_permit,
        downstream_request_observer,
        proxy_request_started_at,
    )
    .await?
    {
        PreparedProxyCaptureRequestResult::Ready(prepared) => *prepared,
        PreparedProxyCaptureRequestResult::Response(response) => return Ok(response),
    };
    let upstream = dispatch_proxy_upstream(prepared).await?;
    forward_proxy_upstream(upstream).await
}
