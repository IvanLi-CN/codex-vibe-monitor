use super::*;

pub(crate) struct PoolAttemptExecutionContext<'a> {
    pub(crate) state: Arc<AppState>,
    pub(crate) proxy_request_id: u64,
    pub(crate) method: &'a Method,
    pub(crate) original_uri: &'a Uri,
    pub(crate) headers: &'a HeaderMap,
    pub(crate) body: &'a Option<PoolReplayBodySnapshot>,
    pub(crate) request_connection_scoped: &'a HashSet<HeaderName>,
    pub(crate) direct_image_request: bool,
    pub(crate) uses_timeout_route_failover: bool,
    pub(crate) responses_total_timeout: Option<Duration>,
    pub(crate) responses_total_timeout_started_at: Option<Instant>,
    pub(crate) reservation_key: String,
    pub(crate) reservation_guard: &'a mut Option<PoolRoutingReservationDropGuard>,
    pub(crate) trace_context: Option<&'a PoolUpstreamAttemptTraceContext>,
    pub(crate) runtime_snapshot_context: Option<&'a PoolAttemptRuntimeSnapshotContext>,
    pub(crate) sticky_key: Option<&'a str>,
    pub(crate) codex_imagegen_request: bool,
    pub(crate) request_compression_level_preset: RequestCompressionLevelPreset,
    pub(crate) requested_model: &'a Option<String>,
    pub(crate) account: PoolResolvedAccount,
    pub(crate) pre_first_byte_timeout: Duration,
    pub(crate) stream_timeout: Option<Duration>,
    pub(crate) send_timeout: Duration,
    pub(crate) upstream_route_key: String,
    pub(crate) api_key_target_url: Option<Url>,
    pub(crate) priority_handoff_admitted: bool,
    pub(crate) same_account_attempt_budget: u8,
    pub(crate) overload_same_account_attempt_budget: u8,
    pub(crate) group_upstream_429_max_retries: u8,
    pub(crate) model_mapping: &'a Option<ResolvedModelMapping>,
    pub(crate) upstream_request_model: Option<&'a str>,
    pub(crate) model_mapping_pattern: Option<&'a str>,
    pub(crate) same_account_attempt_loop_budget: u8,
    pub(crate) attempt_count: &'a mut usize,
    pub(crate) last_error: &'a mut Option<PoolUpstreamError>,
    pub(crate) preserve_sticky_owner_terminal_error: &'a mut bool,
    pub(crate) exhausted_accounts_all_rate_limited: &'a mut bool,
    pub(crate) excluded_upstream_route_keys: &'a mut HashSet<String>,
    pub(crate) overload_required_upstream_route_key: &'a mut Option<String>,
    pub(crate) timeout_route_failover_pending: &'a mut bool,
    pub(crate) capability_endpoint: &'a str,
    pub(crate) sticky_event_prompt_cache_key: Option<&'a str>,
    pub(crate) prompt_cache_key: Option<&'a str>,
    pub(crate) distinct_account_index: i64,
    pub(crate) first_response_attempt_started_at: &'a mut Option<Instant>,
    pub(crate) same_account_only_retry_active: &'a mut bool,
    pub(crate) same_account_only_forward_proxy: &'a mut Option<PoolSelectedForwardProxy>,
    pub(crate) group_upstream_429_retry_count: &'a mut u8,
    pub(crate) retried_upstream_413_for_account: &'a mut bool,
}

pub(crate) async fn execute_pool_account_attempts<'a>(
    mut context: PoolAttemptExecutionContext<'a>,
    distinct_account_count: usize,
) -> Result<Option<PoolUpstreamResponse>, PoolUpstreamError> {
    for same_account_attempt in 0..context.same_account_attempt_loop_budget {
        if *context.same_account_only_retry_active
            && same_account_attempt >= context.overload_same_account_attempt_budget
        {
            return Err(take_same_account_only_terminal_error(
                context.last_error,
                *context.attempt_count,
                distinct_account_count,
            ));
        }
        let same_account_retry_budget = if *context.same_account_only_retry_active {
            context.overload_same_account_attempt_budget
        } else {
            context.same_account_attempt_budget
        };
        if context.uses_timeout_route_failover
            && context.first_response_attempt_started_at.is_none()
        {
            *context.first_response_attempt_started_at = Some(Instant::now());
        }
        let attempt_total_timeout_started_at = if context.uses_timeout_route_failover {
            if let Some(started_at) = context.responses_total_timeout_started_at {
                Some(started_at)
            } else if same_account_attempt > 0 {
                *context.first_response_attempt_started_at
            } else {
                None
            }
        } else {
            None
        };
        let apply_retry_delay_budget = |retry_delay| {
            pool_retry_delay_with_total_timeout(
                retry_delay,
                *context.same_account_only_retry_active,
                context.responses_total_timeout,
                attempt_total_timeout_started_at,
            )
        };
        let Some(attempt_pre_first_byte_timeout) = pool_timeout_budget_with_total_limit(
            context.pre_first_byte_timeout,
            context.responses_total_timeout,
            attempt_total_timeout_started_at,
        ) else {
            let final_error = build_pool_total_timeout_exhausted_error(
                context
                    .responses_total_timeout
                    .expect("responses total timeout should be present"),
                context.last_error.take(),
                *context.attempt_count,
                distinct_account_count,
            );
            if *context.attempt_count > 0
                && let Some(trace) = context.trace_context
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
            return Err(final_error);
        };
        let Some(attempt_send_timeout) = pool_timeout_budget_with_total_limit(
            context.send_timeout,
            context.responses_total_timeout,
            attempt_total_timeout_started_at,
        ) else {
            let final_error = build_pool_total_timeout_exhausted_error(
                context
                    .responses_total_timeout
                    .expect("responses total timeout should be present"),
                context.last_error.take(),
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
            return Err(final_error);
        };
        let same_account_retry_index = i64::from(same_account_attempt) + 1;
        let attempt_started_at_utc = Utc::now();
        let attempt_observed_at = format_utc_iso_precise(attempt_started_at_utc);
        let connect_started = Instant::now();
        let attempt_started_at: String;
        let attempt_index: i64;
        let mut pending_attempt_record: Option<PendingPoolAttemptRecord>;
        let mut early_phase_cleanup_guard: Option<PoolEarlyPhaseOrphanCleanupGuard> = None;
        let live_attempt_activity_lease: Option<PoolLiveAttemptActivityLease>;
        let prepared_request_body = match prepare_pool_request_body_for_account(
            context.proxy_request_id,
            context.body.as_ref(),
            context.original_uri,
            context.method,
            context
                .headers
                .get(header::CONTENT_ENCODING)
                .and_then(|value| value.to_str().ok()),
            context.account.fast_mode_rewrite_mode,
            context.account.image_tool_rewrite_mode,
            context.account.codex_imagegen_rewrite_mode,
            codex_imagegen_protocol_from_headers(context.headers),
            context
                .runtime_snapshot_context
                .as_ref()
                .map(|context| &context.request_info),
            context
                .runtime_snapshot_context
                .as_ref()
                .and_then(|context| context.hosted_image_intent),
            context.model_mapping.as_ref(),
        )
        .await
        {
            Ok(prepared) => prepared,
            Err(err) => {
                record_pool_request_prepare_failure_attempt(
                    context.state.as_ref(),
                    context.trace_context,
                    &context.account,
                    context.requested_model.as_deref(),
                    context.model_mapping_pattern,
                    (*context.attempt_count + 1) as i64,
                    distinct_account_count as i64,
                    same_account_retry_index,
                    err.status,
                    &err.message,
                )
                .await;
                release_pool_routing_reservation(context.state.as_ref(), &context.reservation_key);
                return Err(PoolUpstreamError {
                    codex_imagegen_rewrite: None,
                    account: Some(context.account.clone()),
                    status: err.status,
                    message: err.message,
                    failure_kind: PROXY_FAILURE_FAILED_CONTACT_UPSTREAM,
                    blocked_binding: None,
                    connect_latency_ms: 0.0,
                    upstream_error_code: None,
                    upstream_error_message: None,
                    canonical_error_message: None,
                    downstream_error_message: None,
                    upstream_request_id: None,
                    proxy_binding_key_snapshot: None,
                    oauth_responses_debug: None,
                    attempt_summary: PoolAttemptSummary::default(),
                    requested_service_tier: None,
                    request_body_for_capture: None,
                });
            }
        };
        let attempted_requested_service_tier = prepared_request_body.requested_service_tier.clone();
        let attempted_requested_image_intent = prepared_request_body.requested_image_intent;
        let attempted_requested_hosted_image_intent =
            prepared_request_body.requested_hosted_image_intent;
        let attempted_request_body_for_capture =
            prepared_request_body.request_body_for_capture.clone();
        let attempted_codex_imagegen_rewrite = prepared_request_body.codex_imagegen_rewrite.clone();
        let (forward_proxy_scope, selected_proxy, _client) = if *context
            .same_account_only_retry_active
        {
            (*context.same_account_only_forward_proxy)
                .clone()
                .expect("same-account-only retry should retain forward proxy selection")
        } else {
            match select_pool_account_forward_proxy_client(context.state.as_ref(), &context.account)
                .await
            {
                Ok(selection) => selection,
                Err(message) => {
                    // The drop guard releases silently if cancellation interrupts this fence.
                    let failure_recorded = context
                        .reservation_guard
                        .as_mut()
                        .expect("pool routing reservation should be active")
                        .fence_failure(record_runtime_pool_route_transport_failure(
                            context.state.as_ref(),
                            context.account.account_id,
                            context.sticky_key,
                            &message,
                            PROXY_FAILURE_FAILED_CONTACT_UPSTREAM,
                            context.trace_context.map(|trace| trace.invoke_id.as_str()),
                            None,
                            context.requested_model.as_deref(),
                        ))
                        .await;
                    if let Err(ref route_err) = failure_recorded {
                        warn!(account_id = context.account.account_id, error = %route_err, "failed to record pool forward proxy selection failure");
                    }
                    store_pool_failover_error(
                        context.last_error,
                        &mut *context.preserve_sticky_owner_terminal_error,
                        PoolUpstreamError {
                            codex_imagegen_rewrite: attempted_codex_imagegen_rewrite.clone(),
                            account: Some(context.account.clone()),
                            status: StatusCode::BAD_GATEWAY,
                            message,
                            canonical_error_message: None,
                            failure_kind: PROXY_FAILURE_FAILED_CONTACT_UPSTREAM,
                            blocked_binding: None,
                            connect_latency_ms: 0.0,
                            upstream_error_code: None,
                            upstream_error_message: None,
                            downstream_error_message: None,
                            upstream_request_id: None,
                            proxy_binding_key_snapshot: None,
                            oauth_responses_debug: None,
                            attempt_summary: PoolAttemptSummary::default(),
                            requested_service_tier: attempted_requested_service_tier.clone(),
                            request_body_for_capture: attempted_request_body_for_capture.clone(),
                        },
                        attempted_codex_imagegen_rewrite.as_ref(),
                    );
                    if failure_recorded.is_ok() {
                        release_pool_routing_reservation(
                            context.state.as_ref(),
                            &context.reservation_key,
                        );
                    } else {
                        release_pool_routing_reservation_without_availability(
                            context.state.as_ref(),
                            &context.reservation_key,
                        );
                    }
                    *context.exhausted_accounts_all_rate_limited = false;
                    if *context.same_account_only_retry_active {
                        return Err(take_same_account_only_terminal_error(
                            context.last_error,
                            *context.attempt_count,
                            distinct_account_count,
                        ));
                    }
                    return Ok(None);
                }
            }
        };
        let same_account_only_forward_proxy_candidate = (
            forward_proxy_scope.clone(),
            selected_proxy.clone(),
            _client.clone(),
        );
        let (
            response,
            oauth_responses_debug,
            forward_proxy_selection,
            transport_bytes_live_counted,
            codex_imagegen_retest_claimed,
        ) = match &context.account.auth {
            PoolResolvedAuth::ApiKey { authorization } => {
                if *context.attempt_count > 0 {
                    let reason = match context.last_error.as_ref().map(|error| error.status) {
                        Some(StatusCode::TOO_MANY_REQUESTS) => "rate_limit",
                        Some(StatusCode::SERVICE_UNAVAILABLE) => "overload",
                        _ => "route_failover",
                    };
                    context.state.observability.counter(
                        "cvm_proxy_retries_total",
                        &[
                            (
                                "endpoint",
                                crate::observability::endpoint_from_path(
                                    context.original_uri.path(),
                                ),
                            ),
                            ("reason", reason),
                        ],
                        1,
                    );
                }
                *context.attempt_count += 1;
                attempt_index = *context.attempt_count as i64;
                attempt_started_at =
                    format_naive_precise(Utc::now().with_timezone(&Shanghai).naive_local());
                let outbound_request_body = build_pool_upstream_request_body(
                    &prepared_request_body,
                    context.account.request_compression_algorithm,
                    context.request_compression_level_preset,
                    context
                        .headers
                        .get(header::CONTENT_ENCODING)
                        .and_then(|value| value.to_str().ok()),
                )
                .await
                .map_err(|err| {
                    release_pool_routing_reservation(
                        context.state.as_ref(),
                        &context.reservation_key,
                    );
                    PoolUpstreamError {
                        codex_imagegen_rewrite: attempted_codex_imagegen_rewrite.clone(),
                        account: Some(context.account.clone()),
                        status: err.status,
                        message: err.message,
                        canonical_error_message: None,
                        failure_kind: PROXY_FAILURE_FAILED_CONTACT_UPSTREAM,
                        blocked_binding: None,
                        connect_latency_ms: 0.0,
                        upstream_error_code: None,
                        upstream_error_message: None,
                        downstream_error_message: None,
                        upstream_request_id: None,
                        proxy_binding_key_snapshot: None,
                        oauth_responses_debug: None,
                        attempt_summary: PoolAttemptSummary::default(),
                        requested_service_tier: attempted_requested_service_tier.clone(),
                        request_body_for_capture: attempted_request_body_for_capture.clone(),
                    }
                })?;
                let codex_imagegen_retest_claimed = if codex_imagegen_audit_has_canonical_namespace(
                    attempted_codex_imagegen_rewrite.as_ref(),
                ) {
                    match claim_codex_imagegen_supported_retest_override(
                        &context.state.pool,
                        context.account.account_id,
                    )
                    .await
                    {
                        Ok(CodexImagegenRetestClaim::Claimed) => true,
                        Ok(CodexImagegenRetestClaim::NotNeeded) => false,
                        Ok(CodexImagegenRetestClaim::AlreadyClaimed) => {
                            release_pool_routing_reservation(
                                context.state.as_ref(),
                                &context.reservation_key,
                            );
                            if *context.same_account_only_retry_active {
                                return Err(take_same_account_only_terminal_error(
                                    context.last_error,
                                    *context.attempt_count,
                                    distinct_account_count,
                                ));
                            }
                            return Ok(None);
                        }
                        Err(claim_err) => {
                            warn!(
                                account_id = context.account.account_id,
                                error = %claim_err,
                                "failed to atomically claim Codex imagegen supported retest override"
                            );
                            release_pool_routing_reservation(
                                context.state.as_ref(),
                                &context.reservation_key,
                            );
                            if *context.same_account_only_retry_active {
                                return Err(take_same_account_only_terminal_error(
                                    context.last_error,
                                    *context.attempt_count,
                                    distinct_account_count,
                                ));
                            }
                            return Ok(None);
                        }
                    }
                } else {
                    false
                };
                let forwarded_content_length = context
                    .headers
                    .get(header::CONTENT_LENGTH)
                    .and_then(|value| value.to_str().ok())
                    .map(str::to_string);
                let outbound_snapshot_kind =
                    pool_request_snapshot_kind(&prepared_request_body.snapshot);
                let prepared_body_bytes =
                    pool_request_snapshot_body_bytes(&prepared_request_body.snapshot);
                let outbound_content_length = outbound_request_body.content_length;
                let preserve_content_length = outbound_content_length.is_some()
                    && forwarded_content_length
                        .as_deref()
                        .and_then(|value| value.parse::<usize>().ok())
                        == outbound_content_length;
                let mut outbound_headers = HeaderMap::new();
                for (name, value) in context.headers {
                    if *name == header::AUTHORIZATION || *name == header::CONTENT_ENCODING {
                        continue;
                    }
                    if *name == header::CONTENT_LENGTH && !preserve_content_length {
                        continue;
                    }
                    if should_forward_proxy_header(name, context.request_connection_scoped) {
                        outbound_headers.append(name.clone(), value.clone());
                    }
                }
                if let Ok(value) = HeaderValue::from_str(authorization) {
                    outbound_headers.insert(header::AUTHORIZATION, value);
                }
                if let Some(content_length) = outbound_content_length
                    && let Ok(value) = HeaderValue::from_str(&content_length.to_string())
                {
                    outbound_headers.insert(header::CONTENT_LENGTH, value);
                }
                if let Some(content_encoding) =
                    outbound_request_body.content_encoding.header_value()
                {
                    outbound_headers.insert(
                        header::CONTENT_ENCODING,
                        HeaderValue::from_static(content_encoding),
                    );
                }
                let request_header_bytes_approx =
                    http_visible_header_bytes_approx(&outbound_headers);
                record_account_selected(context.state.as_ref(), context.account.account_id).await;
                let group_name_snapshot =
                    normalize_pool_attempt_group_name(context.account.group_name.clone());
                let proxy_binding_key_snapshot = canonical_pool_attempt_proxy_binding_key(
                    context.state.as_ref(),
                    selected_proxy.key.as_str(),
                )
                .await;
                let _attempt_recovery_gate =
                    crate::runtime::pool_attempt_recovery_gate().read().await;
                pending_attempt_record = if let Some(trace) = context.trace_context {
                    let mut attempt_trace = trace.clone();
                    if attempt_trace.request_model.is_none() {
                        attempt_trace.request_model = context
                            .runtime_snapshot_context
                            .as_ref()
                            .and_then(|context| context.request_info.model.clone())
                            .or_else(|| context.requested_model.clone());
                    }
                    attempt_trace.upstream_base_url_host = context
                        .account
                        .upstream_base_url
                        .host_str()
                        .and_then(normalize_upstream_base_url_host_value);
                    begin_pool_upstream_request_attempt_in_memory(
                        context.state.as_ref(),
                        &attempt_trace,
                        group_name_snapshot.as_deref(),
                        proxy_binding_key_snapshot.as_deref(),
                        Some(context.account.routing_source),
                        context.account.routing_selection_audit.as_ref(),
                        context.account.routing_control_fingerprint.as_deref(),
                        context.account.sticky_affinity_generation,
                        context.account.account_id,
                        context.upstream_route_key.as_str(),
                        attempt_index,
                        context.distinct_account_index,
                        same_account_retry_index,
                        attempt_started_at.as_str(),
                    )
                    .await
                } else {
                    None
                };
                if context.trace_context.is_some() && pending_attempt_record.is_none() {
                    release_pool_routing_reservation_without_availability(
                        context.state.as_ref(),
                        &context.reservation_key,
                    );
                    return Err(build_pool_attempt_journal_failure_error(
                        &context.account,
                        "start event was rejected by the bounded runtime journal",
                        *context.attempt_count,
                        distinct_account_count,
                    ));
                }
                if let Some(pending_attempt_record) = pending_attempt_record.as_mut() {
                    if let Err(err) = annotate_pool_upstream_request_attempt_model_mapping(
                        pending_attempt_record,
                        context.upstream_request_model,
                        context.model_mapping_pattern,
                    ) {
                        warn!(
                            invoke_id = %pending_attempt_record.invoke_id,
                            error = %err,
                            "failed to prepare pool model mapping attempt metadata"
                        );
                    }
                    if let Err(err) = annotate_pool_upstream_request_attempt_request_compression(
                        pending_attempt_record,
                        outbound_request_body.content_encoding.algorithm().as_str(),
                        outbound_request_body.compression_mode.as_str(),
                    ) {
                        warn!(
                            invoke_id = %pending_attempt_record.invoke_id,
                            error = %err,
                            "failed to prepare pool request compression metadata"
                        );
                    }
                    if let Err(err) = annotate_pool_upstream_request_attempt_codex_imagegen_rewrite(
                        pending_attempt_record,
                        attempted_codex_imagegen_rewrite.as_ref(),
                    ) {
                        warn!(
                            invoke_id = %pending_attempt_record.invoke_id,
                            error = %err,
                            "failed to prepare pool Codex imagegen rewrite audit"
                        );
                    }
                    early_phase_cleanup_guard = Some(PoolEarlyPhaseOrphanCleanupGuard::new(
                        context.state.clone(),
                        pending_attempt_record.clone(),
                    ));
                    enqueue_pool_upstream_request_attempt_snapshot(
                        context.state.as_ref(),
                        pending_attempt_record,
                    );
                }
                let attempt_runtime_snapshot = context.runtime_snapshot_context.map(|ctx| {
                    let mut ctx = ctx.clone();
                    ctx.request_info.requested_service_tier =
                        attempted_requested_service_tier.clone();
                    ctx.request_info.image_intent =
                        Some(attempted_requested_image_intent.as_str().to_string());
                    ctx
                });
                live_attempt_activity_lease = pending_attempt_record
                    .as_ref()
                    .and_then(|pending| pending.attempt_id)
                    .map(|attempt_id| {
                        PoolLiveAttemptActivityLease::new(context.state.clone(), attempt_id)
                    });
                drop(_attempt_recovery_gate);
                if let (Some(trace), Some(runtime_snapshot)) =
                    (context.trace_context, attempt_runtime_snapshot.as_ref())
                {
                    broadcast_pool_attempt_started_runtime_snapshot(
                        context.state.as_ref(),
                        trace,
                        runtime_snapshot,
                        &context.account,
                        *context.attempt_count,
                        distinct_account_count,
                        Some(outbound_request_body.content_encoding.algorithm().as_str()),
                    )
                    .await;
                }
                if let Some(pending_attempt_record) = pending_attempt_record.as_ref()
                    && let Err(err) = advance_pool_upstream_request_attempt_phase(
                        context.state.as_ref(),
                        pending_attempt_record,
                        POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_SENDING_REQUEST,
                    )
                    .await
                {
                    warn!(
                        invoke_id = %pending_attempt_record.invoke_id,
                        error = %err,
                        "failed to advance pool attempt into sending-request phase"
                    );
                }
                debug!(
                    invoke_id = context
                        .trace_context
                        .as_ref()
                        .map(|trace| trace.invoke_id.as_str())
                        .unwrap_or(""),
                    account_id = context.account.account_id,
                    endpoint = context.original_uri.path(),
                    attempt_index = pending_attempt_record
                        .as_ref()
                        .map(|pending| pending.attempt_index),
                    upstream_attempt_started = true,
                    request_compression_algorithm = outbound_request_body.content_encoding.as_str(),
                    request_compression_mode = outbound_request_body.compression_mode.as_str(),
                    prepared_body_bytes,
                    outbound_content_length,
                    "pool upstream attempt started"
                );
                let live_reporter = context.trace_context.map(|trace| {
                    UpstreamTrafficReporter::new(
                        context.state.clone(),
                        trace.invoke_id.as_str(),
                        trace.occurred_at.as_str(),
                        Some(context.account.account_id),
                        context.account.upstream_base_url.host_str(),
                        context.original_uri.path(),
                    )
                });
                match timeout(
                    attempt_send_timeout,
                    send_counted_upstream_http_request(
                        context.method.clone(),
                        context
                            .api_key_target_url
                            .as_ref()
                            .expect("api key pool route should always have an upstream url"),
                        &outbound_headers,
                        outbound_request_body.body,
                        selected_proxy.endpoint_url.as_ref(),
                        live_reporter,
                    ),
                )
                .await
                {
                    Ok(Ok(response)) => {
                        let request_logical_body_bytes = outbound_request_body
                            .byte_observation
                            .logical_body_bytes
                            .load();
                        let request_transmitted_body_bytes = outbound_request_body
                            .byte_observation
                            .transmitted_body_bytes
                            .load();
                        let response_header_bytes_approx =
                            http_visible_header_bytes_approx(response.response.headers());
                        if let Some(pending_attempt_record) = pending_attempt_record.as_mut() {
                            update_pending_pool_upstream_request_attempt_http_bytes(
                                pending_attempt_record,
                                Some(request_logical_body_bytes),
                                Some(request_transmitted_body_bytes),
                                Some(request_header_bytes_approx),
                                None,
                                Some(response_header_bytes_approx),
                            );
                            enqueue_pool_upstream_request_attempt_snapshot(
                                context.state.as_ref(),
                                pending_attempt_record,
                            );
                        }
                        (
                            ProxyUpstreamResponseBody::Axum(response.response),
                            None,
                            Some((forward_proxy_scope, selected_proxy)),
                            true,
                            codex_imagegen_retest_claimed,
                        )
                    }
                    Ok(Err(err)) => {
                        let request_logical_body_bytes = outbound_request_body
                            .byte_observation
                            .logical_body_bytes
                            .load();
                        let request_transmitted_body_bytes = outbound_request_body
                            .byte_observation
                            .transmitted_body_bytes
                            .load();
                        if let Some(pending_attempt_record) = pending_attempt_record.as_mut() {
                            update_pending_pool_upstream_request_attempt_http_bytes(
                                pending_attempt_record,
                                Some(request_logical_body_bytes),
                                Some(request_transmitted_body_bytes),
                                Some(request_header_bytes_approx),
                                None,
                                None,
                            );
                            enqueue_pool_upstream_request_attempt_snapshot(
                                context.state.as_ref(),
                                pending_attempt_record,
                            );
                        }
                        record_pool_account_forward_proxy_result(
                            context.state.as_ref(),
                            &forward_proxy_scope,
                            &selected_proxy,
                            ForwardProxyRouteResultKind::NetworkFailure,
                        )
                        .await;
                        warn!(
                            invoke_id = context.trace_context
                                .as_ref()
                                .map(|trace| trace.invoke_id.as_str())
                                .unwrap_or(""),
                            account_id = context.account.account_id,
                            endpoint = context.original_uri.path(),
                            requested_service_tier = attempted_requested_service_tier.as_deref(),
                            request_compression_algorithm =
                                outbound_request_body.content_encoding.as_str(),
                            request_compression_mode =
                                outbound_request_body.compression_mode.as_str(),
                            snapshot_kind = outbound_snapshot_kind,
                            prepared_body_bytes,
                            outbound_content_length,
                            forwarded_content_length = forwarded_content_length.as_deref(),
                            preserved_content_length = preserve_content_length,
                            error = %err.message,
                            "pool upstream request send failed before response"
                        );
                        let direct_image_handshake_timeout =
                            context.direct_image_request && err.is_timeout();
                        let failure_kind = if direct_image_handshake_timeout {
                            PROXY_FAILURE_UPSTREAM_HANDSHAKE_TIMEOUT
                        } else {
                            PROXY_FAILURE_FAILED_CONTACT_UPSTREAM
                        };
                        let message = if direct_image_handshake_timeout {
                            format!(
                                "[{PROXY_FAILURE_UPSTREAM_HANDSHAKE_TIMEOUT}] failed to contact upstream: {err}"
                            )
                        } else {
                            format!("failed to contact upstream: {err}")
                        };
                        let compact_support_observation = classify_compact_support_observation(
                            context.original_uri,
                            None,
                            Some(message.as_str()),
                        );
                        let timeout_shaped_failure =
                            pool_failure_is_timeout_shaped(failure_kind, &message);
                        let should_timeout_route_failover =
                            context.uses_timeout_route_failover && timeout_shaped_failure;
                        let finished_at = shanghai_now_string();
                        if let Some(pending_attempt_record) = pending_attempt_record.as_ref()
                            && let Err(record_err) =
                                finalize_pool_upstream_request_attempt_in_memory(
                                    context.state.as_ref(),
                                    pending_attempt_record,
                                    finished_at.as_str(),
                                    POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_TRANSPORT_FAILURE,
                                    None,
                                    None,
                                    Some(failure_kind),
                                    Some(message.as_str()),
                                    None,
                                    Some(elapsed_ms(connect_started)),
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
                                "failed to persist pool transport attempt"
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
                                "failed to broadcast pool transport attempt snapshot"
                            );
                        }
                        let has_retry_budget = same_account_attempt + 1 < same_account_retry_budget
                            && !direct_image_handshake_timeout
                            && !codex_imagegen_retest_claimed;
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
                                "pool upstream transport failure; retrying same account"
                            );
                            disarm_pool_early_phase_cleanup_guard(&mut early_phase_cleanup_guard);
                            crate::observability::diagnostics::wait(
                                crate::observability::diagnostics::Resource::RetryBackoff,
                                sleep(retry_delay),
                            )
                            .await;
                            continue;
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
                                failure_kind,
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
                            warn!(account_id = context.account.account_id, error = %route_err, "failed to record pool transport failure");
                        }
                        store_pool_failover_error(
                            context.last_error,
                            &mut *context.preserve_sticky_owner_terminal_error,
                            PoolUpstreamError {
                                codex_imagegen_rewrite: attempted_codex_imagegen_rewrite.clone(),
                                account: Some(context.account.clone()),
                                status: if direct_image_handshake_timeout {
                                    StatusCode::GATEWAY_TIMEOUT
                                } else {
                                    StatusCode::BAD_GATEWAY
                                },
                                message: message.clone(),
                                canonical_error_message: None,
                                failure_kind,
                                blocked_binding: None,
                                connect_latency_ms: elapsed_ms(connect_started),
                                upstream_error_code: direct_image_handshake_timeout
                                    .then(|| PROXY_FAILURE_UPSTREAM_HANDSHAKE_TIMEOUT.to_string()),
                                upstream_error_message: None,
                                downstream_error_message: None,
                                upstream_request_id: None,
                                proxy_binding_key_snapshot: proxy_binding_key_snapshot.clone(),
                                oauth_responses_debug: None,
                                attempt_summary: PoolAttemptSummary::default(),
                                requested_service_tier: attempted_requested_service_tier.clone(),
                                request_body_for_capture: attempted_request_body_for_capture
                                    .clone(),
                            },
                            attempted_codex_imagegen_rewrite.as_ref(),
                        );
                        if direct_image_handshake_timeout {
                            let mut final_error = context
                                .last_error
                                .take()
                                .expect("direct-image timeout stores a terminal upstream error");
                            final_error.attempt_summary = pool_attempt_summary(
                                *context.attempt_count,
                                distinct_account_count,
                                Some(PROXY_FAILURE_UPSTREAM_HANDSHAKE_TIMEOUT.to_string()),
                            );
                            disarm_pool_early_phase_cleanup_guard(&mut early_phase_cleanup_guard);
                            return Err(final_error);
                        }
                        *context.exhausted_accounts_all_rate_limited = false;
                        if should_timeout_route_failover {
                            context
                                .excluded_upstream_route_keys
                                .insert(context.upstream_route_key.clone());
                            *context.overload_required_upstream_route_key = None;
                            *context.timeout_route_failover_pending = true;
                        }
                        disarm_pool_early_phase_cleanup_guard(&mut early_phase_cleanup_guard);
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
                        return Ok(None);
                    }
                    Err(_) => {
                        let request_logical_body_bytes = outbound_request_body
                            .byte_observation
                            .logical_body_bytes
                            .load();
                        let request_transmitted_body_bytes = outbound_request_body
                            .byte_observation
                            .transmitted_body_bytes
                            .load();
                        if let Some(pending_attempt_record) = pending_attempt_record.as_mut() {
                            update_pending_pool_upstream_request_attempt_http_bytes(
                                pending_attempt_record,
                                Some(request_logical_body_bytes),
                                Some(request_transmitted_body_bytes),
                                Some(request_header_bytes_approx),
                                None,
                                None,
                            );
                            enqueue_pool_upstream_request_attempt_snapshot(
                                context.state.as_ref(),
                                pending_attempt_record,
                            );
                        }
                        record_pool_account_forward_proxy_result(
                            context.state.as_ref(),
                            &forward_proxy_scope,
                            &selected_proxy,
                            ForwardProxyRouteResultKind::NetworkFailure,
                        )
                        .await;
                        warn!(
                            invoke_id = context
                                .trace_context
                                .as_ref()
                                .map(|trace| trace.invoke_id.as_str())
                                .unwrap_or(""),
                            account_id = context.account.account_id,
                            endpoint = context.original_uri.path(),
                            requested_service_tier = attempted_requested_service_tier.as_deref(),
                            request_compression_algorithm =
                                outbound_request_body.content_encoding.as_str(),
                            request_compression_mode =
                                outbound_request_body.compression_mode.as_str(),
                            snapshot_kind = outbound_snapshot_kind,
                            prepared_body_bytes,
                            outbound_content_length,
                            forwarded_content_length = forwarded_content_length.as_deref(),
                            preserved_content_length = preserve_content_length,
                            timeout_ms = attempt_send_timeout.as_millis(),
                            "pool upstream request send timed out before response"
                        );
                        let message = proxy_request_send_timeout_message(
                            capture_target_for_request(context.original_uri.path(), context.method),
                            attempt_send_timeout,
                        );
                        let compact_support_observation = classify_compact_support_observation(
                            context.original_uri,
                            None,
                            Some(message.as_str()),
                        );
                        let should_timeout_route_failover = context.uses_timeout_route_failover;
                        let finished_at = shanghai_now_string();
                        if let Some(pending_attempt_record) = pending_attempt_record.as_ref()
                            && let Err(record_err) =
                                finalize_pool_upstream_request_attempt_in_memory(
                                    context.state.as_ref(),
                                    pending_attempt_record,
                                    finished_at.as_str(),
                                    POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_TRANSPORT_FAILURE,
                                    None,
                                    None,
                                    Some(PROXY_FAILURE_UPSTREAM_HANDSHAKE_TIMEOUT),
                                    Some(message.as_str()),
                                    None,
                                    Some(elapsed_ms(connect_started)),
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
                                "failed to persist pool handshake timeout attempt"
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
                                "failed to broadcast pool handshake timeout snapshot"
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
                                    codex_imagegen_rewrite: attempted_codex_imagegen_rewrite
                                        .clone(),
                                    account: Some(context.account.clone()),
                                    status: StatusCode::BAD_GATEWAY,
                                    message: message.clone(),
                                    canonical_error_message: None,
                                    failure_kind: PROXY_FAILURE_UPSTREAM_HANDSHAKE_TIMEOUT,
                                    blocked_binding: None,
                                    connect_latency_ms: elapsed_ms(connect_started),
                                    upstream_error_code: None,
                                    upstream_error_message: None,
                                    downstream_error_message: None,
                                    upstream_request_id: None,
                                    proxy_binding_key_snapshot: proxy_binding_key_snapshot.clone(),
                                    oauth_responses_debug: None,
                                    attempt_summary: PoolAttemptSummary::default(),
                                    requested_service_tier: attempted_requested_service_tier
                                        .clone(),
                                    request_body_for_capture: attempted_request_body_for_capture
                                        .clone(),
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
                            disarm_pool_early_phase_cleanup_guard(&mut early_phase_cleanup_guard);
                            return Err(final_error);
                        }
                        let has_retry_budget = same_account_attempt + 1 < same_account_retry_budget
                            && !context.direct_image_request
                            && !codex_imagegen_retest_claimed;
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
                                "pool upstream handshake timeout; retrying same account"
                            );
                            disarm_pool_early_phase_cleanup_guard(&mut early_phase_cleanup_guard);
                            crate::observability::diagnostics::wait(
                                crate::observability::diagnostics::Resource::RetryBackoff,
                                sleep(retry_delay),
                            )
                            .await;
                            continue;
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
                                PROXY_FAILURE_UPSTREAM_HANDSHAKE_TIMEOUT,
                                context.trace_context.map(|trace| trace.invoke_id.as_str()),
                                pending_attempt_record
                                    .as_ref()
                                    .and_then(|pending| pending.attempt_id),
                                context.requested_model.as_deref(),
                            ))
                            .await
                        {
                            warn!(account_id = context.account.account_id, error = %route_err, "failed to record pool handshake timeout");
                        }
                        store_pool_failover_error(
                            context.last_error,
                            &mut *context.preserve_sticky_owner_terminal_error,
                            PoolUpstreamError {
                                codex_imagegen_rewrite: attempted_codex_imagegen_rewrite.clone(),
                                account: Some(context.account.clone()),
                                status: if context.direct_image_request {
                                    StatusCode::GATEWAY_TIMEOUT
                                } else {
                                    StatusCode::BAD_GATEWAY
                                },
                                message: if context.direct_image_request {
                                    format!(
                                        "[{PROXY_FAILURE_UPSTREAM_HANDSHAKE_TIMEOUT}] {message}"
                                    )
                                } else {
                                    message.clone()
                                },
                                canonical_error_message: None,
                                failure_kind: PROXY_FAILURE_UPSTREAM_HANDSHAKE_TIMEOUT,
                                blocked_binding: None,
                                connect_latency_ms: elapsed_ms(connect_started),
                                upstream_error_code: context
                                    .direct_image_request
                                    .then(|| PROXY_FAILURE_UPSTREAM_HANDSHAKE_TIMEOUT.to_string()),
                                upstream_error_message: None,
                                downstream_error_message: None,
                                upstream_request_id: None,
                                proxy_binding_key_snapshot: None,
                                oauth_responses_debug: None,
                                attempt_summary: PoolAttemptSummary::default(),
                                requested_service_tier: attempted_requested_service_tier.clone(),
                                request_body_for_capture: attempted_request_body_for_capture
                                    .clone(),
                            },
                            attempted_codex_imagegen_rewrite.as_ref(),
                        );
                        if context.direct_image_request {
                            let mut final_error = context
                                .last_error
                                .take()
                                .expect("direct-image timeout stores a terminal upstream error");
                            final_error.attempt_summary = pool_attempt_summary(
                                *context.attempt_count,
                                distinct_account_count,
                                Some(PROXY_FAILURE_UPSTREAM_HANDSHAKE_TIMEOUT.to_string()),
                            );
                            disarm_pool_early_phase_cleanup_guard(&mut early_phase_cleanup_guard);
                            return Err(final_error);
                        }
                        *context.exhausted_accounts_all_rate_limited = false;
                        if should_timeout_route_failover {
                            context
                                .excluded_upstream_route_keys
                                .insert(context.upstream_route_key.clone());
                            *context.overload_required_upstream_route_key = None;
                            *context.timeout_route_failover_pending = true;
                        }
                        disarm_pool_early_phase_cleanup_guard(&mut early_phase_cleanup_guard);
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
                        return Ok(None);
                    }
                }
            }
            PoolResolvedAuth::Oauth {
                access_token,
                chatgpt_account_id,
            } => {
                let proxy_binding_key_snapshot = canonical_pool_attempt_proxy_binding_key(
                    context.state.as_ref(),
                    selected_proxy.key.as_str(),
                )
                .await;
                if !matches!(
                    &prepared_request_body.snapshot,
                    PoolReplayBodySnapshot::Empty
                ) {
                    validate_pool_request_snapshot_transcode(
                        &prepared_request_body.snapshot,
                        context
                            .headers
                            .get(header::CONTENT_ENCODING)
                            .and_then(|value| value.to_str().ok()),
                        context.account.oauth_request_compression_algorithm,
                        context.state.config.openai_proxy_max_request_body_bytes,
                    )
                    .await
                    .map_err(|err| {
                        release_pool_routing_reservation(
                            context.state.as_ref(),
                            &context.reservation_key,
                        );
                        PoolUpstreamError {
                            codex_imagegen_rewrite: attempted_codex_imagegen_rewrite.clone(),
                            account: Some(context.account.clone()),
                            status: err.status,
                            message: err.message,
                            canonical_error_message: None,
                            failure_kind: PROXY_FAILURE_FAILED_CONTACT_UPSTREAM,
                            blocked_binding: None,
                            connect_latency_ms: 0.0,
                            upstream_error_code: None,
                            upstream_error_message: None,
                            downstream_error_message: None,
                            upstream_request_id: None,
                            proxy_binding_key_snapshot: proxy_binding_key_snapshot.clone(),
                            oauth_responses_debug: None,
                            attempt_summary: pool_attempt_summary(
                                *context.attempt_count,
                                distinct_account_count,
                                Some(PROXY_FAILURE_FAILED_CONTACT_UPSTREAM.to_string()),
                            ),
                            requested_service_tier: attempted_requested_service_tier.clone(),
                            request_body_for_capture: attempted_request_body_for_capture.clone(),
                        }
                    })?;
                }
                let oauth_body = match &prepared_request_body.snapshot {
                    snapshot @ (PoolReplayBodySnapshot::Empty
                    | PoolReplayBodySnapshot::Memory(_))
                        if context.original_uri.path() == "/v1/responses" =>
                    {
                        oauth_bridge::CountedOauthUpstreamRequestBody::Bytes(
                            snapshot.to_bytes().await.map_err(|err| PoolUpstreamError {
                                codex_imagegen_rewrite: attempted_codex_imagegen_rewrite.clone(),
                                account: Some(context.account.clone()),
                                status: StatusCode::BAD_GATEWAY,
                                message: format!("failed to replay oauth request body: {err}"),
                                canonical_error_message: None,
                                failure_kind: PROXY_FAILURE_FAILED_CONTACT_UPSTREAM,
                                blocked_binding: None,
                                connect_latency_ms: 0.0,
                                upstream_error_code: None,
                                upstream_error_message: None,
                                downstream_error_message: None,
                                upstream_request_id: None,
                                proxy_binding_key_snapshot: proxy_binding_key_snapshot.clone(),
                                oauth_responses_debug: None,
                                attempt_summary: pool_attempt_summary(
                                    *context.attempt_count,
                                    distinct_account_count,
                                    Some(PROXY_FAILURE_FAILED_CONTACT_UPSTREAM.to_string()),
                                ),
                                requested_service_tier: attempted_requested_service_tier.clone(),
                                request_body_for_capture: attempted_request_body_for_capture
                                    .clone(),
                            })?,
                        )
                    }
                    snapshot @ PoolReplayBodySnapshot::File { size, .. }
                        if context.original_uri.path() == "/v1/responses"
                            && *size <= OAUTH_RESPONSES_MAX_REWRITE_BODY_BYTES
                            && context
                                .headers
                                .get(header::CONTENT_ENCODING)
                                .and_then(|value| value.to_str().ok())
                                .map(|value| parse_content_encodings(Some(value)))
                                .is_none_or(|encodings| {
                                    encodings.iter().all(|encoding| encoding == "identity")
                                }) =>
                    {
                        oauth_bridge::CountedOauthUpstreamRequestBody::Bytes(
                            snapshot.to_bytes().await.map_err(|err| PoolUpstreamError {
                                codex_imagegen_rewrite: attempted_codex_imagegen_rewrite.clone(),
                                account: Some(context.account.clone()),
                                status: StatusCode::BAD_GATEWAY,
                                message: format!("failed to replay oauth request body: {err}"),
                                canonical_error_message: None,
                                failure_kind: PROXY_FAILURE_FAILED_CONTACT_UPSTREAM,
                                blocked_binding: None,
                                connect_latency_ms: 0.0,
                                upstream_error_code: None,
                                upstream_error_message: None,
                                downstream_error_message: None,
                                upstream_request_id: None,
                                proxy_binding_key_snapshot: proxy_binding_key_snapshot.clone(),
                                oauth_responses_debug: None,
                                attempt_summary: pool_attempt_summary(
                                    *context.attempt_count,
                                    distinct_account_count,
                                    Some(PROXY_FAILURE_FAILED_CONTACT_UPSTREAM.to_string()),
                                ),
                                requested_service_tier: attempted_requested_service_tier.clone(),
                                request_body_for_capture: attempted_request_body_for_capture
                                    .clone(),
                            })?,
                        )
                    }
                    snapshot => oauth_bridge::CountedOauthUpstreamRequestBody::Stream {
                        debug_body_prefix: Some(
                            snapshot
                                .to_prefix_bytes(
                                    oauth_bridge::OAUTH_REQUEST_BODY_PREFIX_FINGERPRINT_MAX_BYTES,
                                )
                                .await
                                .map_err(|err| PoolUpstreamError {
                                    codex_imagegen_rewrite: attempted_codex_imagegen_rewrite
                                        .clone(),
                                    account: Some(context.account.clone()),
                                    status: StatusCode::BAD_GATEWAY,
                                    message: format!(
                                        "failed to replay oauth request body prefix: {err}"
                                    ),
                                    canonical_error_message: None,
                                    failure_kind: PROXY_FAILURE_FAILED_CONTACT_UPSTREAM,
                                    blocked_binding: None,
                                    connect_latency_ms: 0.0,
                                    upstream_error_code: None,
                                    upstream_error_message: None,
                                    downstream_error_message: None,
                                    upstream_request_id: None,
                                    proxy_binding_key_snapshot: proxy_binding_key_snapshot.clone(),
                                    oauth_responses_debug: None,
                                    attempt_summary: pool_attempt_summary(
                                        *context.attempt_count,
                                        distinct_account_count,
                                        Some(PROXY_FAILURE_FAILED_CONTACT_UPSTREAM.to_string()),
                                    ),
                                    requested_service_tier: attempted_requested_service_tier
                                        .clone(),
                                    request_body_for_capture: attempted_request_body_for_capture
                                        .clone(),
                                })?,
                        ),
                        request_is_stream: if context.original_uri.path() == "/v1/responses" {
                            snapshot
                                .extract_request_stream_flag(
                                    context
                                        .headers
                                        .get(header::CONTENT_ENCODING)
                                        .and_then(|value| value.to_str().ok()),
                                )
                                .await
                        } else {
                            None
                        },
                        snapshot_kind: if context.original_uri.path() == "/v1/responses" {
                            Some(pool_request_snapshot_kind(snapshot))
                        } else {
                            None
                        },
                        body: snapshot.to_http_body(),
                    },
                };
                let codex_imagegen_retest_claimed = if codex_imagegen_audit_has_canonical_namespace(
                    attempted_codex_imagegen_rewrite.as_ref(),
                ) {
                    match claim_codex_imagegen_supported_retest_override(
                        &context.state.pool,
                        context.account.account_id,
                    )
                    .await
                    {
                        Ok(CodexImagegenRetestClaim::Claimed) => true,
                        Ok(CodexImagegenRetestClaim::NotNeeded) => false,
                        Ok(CodexImagegenRetestClaim::AlreadyClaimed) => {
                            release_pool_routing_reservation(
                                context.state.as_ref(),
                                &context.reservation_key,
                            );
                            if *context.same_account_only_retry_active {
                                return Err(take_same_account_only_terminal_error(
                                    context.last_error,
                                    *context.attempt_count,
                                    distinct_account_count,
                                ));
                            }
                            return Ok(None);
                        }
                        Err(claim_err) => {
                            warn!(
                                account_id = context.account.account_id,
                                error = %claim_err,
                                "failed to atomically claim Codex imagegen supported retest override"
                            );
                            release_pool_routing_reservation(
                                context.state.as_ref(),
                                &context.reservation_key,
                            );
                            if *context.same_account_only_retry_active {
                                return Err(take_same_account_only_terminal_error(
                                    context.last_error,
                                    *context.attempt_count,
                                    distinct_account_count,
                                ));
                            }
                            return Ok(None);
                        }
                    }
                } else {
                    false
                };
                if *context.attempt_count > 0 {
                    let reason = match context.last_error.as_ref().map(|error| error.status) {
                        Some(StatusCode::TOO_MANY_REQUESTS) => "rate_limit",
                        Some(StatusCode::SERVICE_UNAVAILABLE) => "overload",
                        _ => "route_failover",
                    };
                    context.state.observability.counter(
                        "cvm_proxy_retries_total",
                        &[
                            (
                                "endpoint",
                                crate::observability::endpoint_from_path(
                                    context.original_uri.path(),
                                ),
                            ),
                            ("reason", reason),
                        ],
                        1,
                    );
                }
                *context.attempt_count += 1;
                attempt_index = *context.attempt_count as i64;
                attempt_started_at =
                    format_naive_precise(Utc::now().with_timezone(&Shanghai).naive_local());
                record_account_selected(context.state.as_ref(), context.account.account_id).await;
                let group_name_snapshot =
                    normalize_pool_attempt_group_name(context.account.group_name.clone());
                let _attempt_recovery_gate =
                    crate::runtime::pool_attempt_recovery_gate().read().await;
                pending_attempt_record = if let Some(trace) = context.trace_context {
                    let mut attempt_trace = trace.clone();
                    if attempt_trace.request_model.is_none() {
                        attempt_trace.request_model = context
                            .runtime_snapshot_context
                            .as_ref()
                            .and_then(|context| context.request_info.model.clone())
                            .or_else(|| context.requested_model.clone());
                    }
                    attempt_trace.upstream_base_url_host = context
                        .account
                        .upstream_base_url
                        .host_str()
                        .and_then(normalize_upstream_base_url_host_value);
                    begin_pool_upstream_request_attempt_in_memory(
                        context.state.as_ref(),
                        &attempt_trace,
                        group_name_snapshot.as_deref(),
                        proxy_binding_key_snapshot.as_deref(),
                        Some(context.account.routing_source),
                        context.account.routing_selection_audit.as_ref(),
                        context.account.routing_control_fingerprint.as_deref(),
                        context.account.sticky_affinity_generation,
                        context.account.account_id,
                        context.upstream_route_key.as_str(),
                        attempt_index,
                        context.distinct_account_index,
                        same_account_retry_index,
                        attempt_started_at.as_str(),
                    )
                    .await
                } else {
                    None
                };
                if context.trace_context.is_some() && pending_attempt_record.is_none() {
                    release_pool_routing_reservation_without_availability(
                        context.state.as_ref(),
                        &context.reservation_key,
                    );
                    return Err(build_pool_attempt_journal_failure_error(
                        &context.account,
                        "start event was rejected by the bounded runtime journal",
                        *context.attempt_count,
                        distinct_account_count,
                    ));
                }
                if let Some(pending_attempt_record) = pending_attempt_record.as_mut()
                    && let Err(err) = annotate_pool_upstream_request_attempt_model_mapping(
                        pending_attempt_record,
                        context.upstream_request_model,
                        context.model_mapping_pattern,
                    )
                {
                    warn!(
                        invoke_id = %pending_attempt_record.invoke_id,
                        error = %err,
                        "failed to prepare pool model mapping attempt metadata"
                    );
                }
                let forwarded_request_compression = if context.method == Method::GET {
                    None
                } else if context.account.auth.is_oauth() {
                    match context.account.oauth_request_compression_algorithm {
                        RequestCompressionAlgorithm::Follow => {
                            forwarded_request_compression_observation(context.headers)
                        }
                        RequestCompressionAlgorithm::Identity => Some(("identity", "identity")),
                        algorithm => Some((algorithm.as_str(), "recompressed")),
                    }
                } else {
                    forwarded_request_compression_observation(context.headers)
                };
                if let (Some(pending_attempt_record), Some((algorithm, mode))) = (
                    pending_attempt_record.as_mut(),
                    forwarded_request_compression,
                ) && let Err(err) = annotate_pool_upstream_request_attempt_request_compression(
                    pending_attempt_record,
                    algorithm,
                    mode,
                ) {
                    warn!(
                        invoke_id = %pending_attempt_record.invoke_id,
                        error = %err,
                        "failed to prepare pool OAuth request compression metadata"
                    );
                }
                if let Some(pending_attempt_record) = pending_attempt_record.as_mut()
                    && let Err(err) = annotate_pool_upstream_request_attempt_codex_imagegen_rewrite(
                        pending_attempt_record,
                        attempted_codex_imagegen_rewrite.as_ref(),
                    )
                {
                    warn!(
                    invoke_id = %pending_attempt_record.invoke_id,
                    error = %err,
                    "failed to prepare pool Codex imagegen rewrite audit"
                    );
                }
                early_phase_cleanup_guard = pending_attempt_record.as_ref().map(|pending| {
                    PoolEarlyPhaseOrphanCleanupGuard::new(context.state.clone(), pending.clone())
                });
                if let Some(pending_attempt_record) = pending_attempt_record.as_ref() {
                    enqueue_pool_upstream_request_attempt_snapshot(
                        context.state.as_ref(),
                        pending_attempt_record,
                    );
                }
                let attempt_runtime_snapshot = context.runtime_snapshot_context.map(|ctx| {
                    let mut ctx = ctx.clone();
                    ctx.request_info.requested_service_tier =
                        attempted_requested_service_tier.clone();
                    ctx.request_info.image_intent =
                        Some(attempted_requested_image_intent.as_str().to_string());
                    ctx
                });
                live_attempt_activity_lease = pending_attempt_record
                    .as_ref()
                    .and_then(|pending| pending.attempt_id)
                    .map(|attempt_id| {
                        PoolLiveAttemptActivityLease::new(context.state.clone(), attempt_id)
                    });
                drop(_attempt_recovery_gate);
                if let (Some(trace), Some(runtime_snapshot)) =
                    (context.trace_context, attempt_runtime_snapshot.as_ref())
                {
                    broadcast_pool_attempt_started_runtime_snapshot(
                        context.state.as_ref(),
                        trace,
                        runtime_snapshot,
                        &context.account,
                        *context.attempt_count,
                        distinct_account_count,
                        forwarded_request_compression.map(|(algorithm, _)| algorithm),
                    )
                    .await;
                }
                if let Some(pending_attempt_record) = pending_attempt_record.as_ref()
                    && let Err(err) = advance_pool_upstream_request_attempt_phase(
                        context.state.as_ref(),
                        pending_attempt_record,
                        POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_SENDING_REQUEST,
                    )
                    .await
                {
                    warn!(
                        invoke_id = %pending_attempt_record.invoke_id,
                        error = %err,
                        "failed to advance pool oauth attempt into sending-request phase"
                    );
                }
                {
                    let live_reporter = context.trace_context.map(|trace| {
                        UpstreamTrafficReporter::new(
                            context.state.clone(),
                            trace.invoke_id.as_str(),
                            trace.occurred_at.as_str(),
                            Some(context.account.account_id),
                            context.account.upstream_base_url.host_str(),
                            context.original_uri.path(),
                        )
                    });
                    let oauth_response =
                        oauth_bridge::send_counted_oauth_upstream_request_with_compression(
                            context.method.clone(),
                            context.original_uri,
                            context.headers,
                            oauth_body,
                            attempt_send_timeout,
                            attempt_pre_first_byte_timeout,
                            Some(context.account.account_id),
                            access_token,
                            chatgpt_account_id.as_deref(),
                            Some(&context.state.oauth_installation_seed),
                            context.state.upstream_accounts.crypto_key.as_ref(),
                            context.account.oauth_request_compression_algorithm,
                            context.request_compression_level_preset,
                            selected_proxy.endpoint_url.as_ref(),
                            live_reporter,
                        )
                        .await;
                    if let Some(debug) = oauth_response.request_debug.as_ref() {
                        let request_header_bytes_approx = pending_attempt_record
                            .as_ref()
                            .and_then(|pending| pending.upstream_request_header_bytes_approx)
                            .and_then(|value| usize::try_from(value).ok());
                        let response_body_bytes = pending_attempt_record
                            .as_ref()
                            .and_then(|pending| pending.upstream_response_body_bytes)
                            .and_then(|value| usize::try_from(value).ok());
                        let response_header_bytes_approx = pending_attempt_record
                            .as_ref()
                            .and_then(|pending| pending.upstream_response_header_bytes_approx)
                            .and_then(|value| usize::try_from(value).ok());
                        if let Some(pending_attempt_record) = pending_attempt_record.as_mut() {
                            update_pending_pool_upstream_request_attempt_http_bytes(
                                pending_attempt_record,
                                debug.request_body_logical_bytes,
                                debug.request_body_transmitted_bytes,
                                request_header_bytes_approx,
                                response_body_bytes,
                                response_header_bytes_approx,
                            );
                            enqueue_pool_upstream_request_attempt_snapshot(
                                context.state.as_ref(),
                                pending_attempt_record,
                            );
                        }
                    }
                    (
                        ProxyUpstreamResponseBody::Axum(oauth_response.response),
                        oauth_response.request_debug,
                        Some((forward_proxy_scope, selected_proxy)),
                        true,
                        codex_imagegen_retest_claimed,
                    )
                }
            }
        };

        match handle_pool_attempt_response(PoolAttemptResponseContext {
            context: &mut context,
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
            pending_attempt_record: &mut pending_attempt_record,
            early_phase_cleanup_guard: &mut early_phase_cleanup_guard,
            live_attempt_activity_lease,
            attempted_requested_hosted_image_intent,
            attempted_requested_service_tier,
            attempted_request_body_for_capture,
            attempted_codex_imagegen_rewrite,
        })
        .await?
        {
            PoolAttemptResponseOutcome::Retry => continue,
            PoolAttemptResponseOutcome::Failover => return Ok(None),
            PoolAttemptResponseOutcome::Response(response) => return Ok(Some(*response)),
        }
    }
    if *context.same_account_only_retry_active {
        return Err(take_same_account_only_terminal_error(
            context.last_error,
            *context.attempt_count,
            distinct_account_count,
        ));
    }
    Ok(None)
}
