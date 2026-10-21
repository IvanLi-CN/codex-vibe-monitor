use super::*;

pub(crate) async fn maybe_build_and_record_single_account_binding_terminal_error(
    state: &AppState,
    trace_context: Option<&PoolUpstreamAttemptTraceContext>,
    binding_constraint: Option<&PromptCacheConversationBindingConstraint>,
    owner_auto_guard_active: bool,
    prompt_cache_key: Option<&str>,
    account: Option<PoolResolvedAccount>,
    message: Option<String>,
    attempt_count: usize,
    distinct_account_count: usize,
) -> Option<PoolUpstreamError> {
    let err = build_single_account_binding_blocked_error(
        state,
        binding_constraint,
        owner_auto_guard_active,
        account,
        prompt_cache_key,
        message,
        attempt_count,
        distinct_account_count,
    )
    .await?;
    if let Some(trace) = trace_context
        && let Err(record_err) = insert_and_broadcast_pool_upstream_terminal_attempt(
            state,
            trace,
            &err,
            (attempt_count + 1) as i64,
            distinct_account_count as i64,
            err.failure_kind,
        )
        .await
    {
        warn!(
            invoke_id = trace.invoke_id,
            error = %record_err,
            "failed to persist single-account binding terminal attempt"
        );
    }
    Some(err)
}

pub(crate) async fn persist_pool_failover_terminal_invocation(
    state: Arc<AppState>,
    proxy_request_id: u64,
    capture_started: Instant,
    original_uri: &Uri,
    headers: &HeaderMap,
    trace_context: Option<&PoolUpstreamAttemptTraceContext>,
    runtime_snapshot_context: Option<&PoolAttemptRuntimeSnapshotContext>,
    request_body_snapshot: PoolReplayBodySnapshot,
    error: &PoolUpstreamError,
) {
    let Some(trace) = trace_context else {
        return;
    };
    let request_info = runtime_snapshot_context
        .map(|context| context.request_info.clone())
        .unwrap_or_default();
    let capture_target = runtime_snapshot_context
        .map(|context| context.capture_target)
        .or_else(|| capture_target_for_request(original_uri.path(), &Method::POST))
        .unwrap_or(ProxyCaptureTarget::Responses);
    let header_prompt_cache_key = extract_prompt_cache_key_from_headers(headers);
    let prompt_cache_key = runtime_snapshot_context
        .and_then(|context| context.prompt_cache_key.as_deref())
        .or(header_prompt_cache_key.as_deref());
    let requester_ip = extract_requester_ip(headers, None);
    let request_chain_metadata = request_chain_metadata_from_headers(headers);
    let client_attribution_context = client_prompt_cache_attribution_context_from_headers(headers);
    let request_body = error.request_body_for_capture.clone();
    let request_body_logging_enabled = state
        .proxy_model_settings
        .read()
        .await
        .request_body_logging_enabled;
    let downstream_error = ProxyErrorResponse {
        status: error.status,
        message: error.message.clone(),
        cvm_id: None,
        retry_after_secs: retry_after_secs_for_proxy_error(error.status, &error.message),
        code: Some(error.failure_kind.to_string()),
        blocked_binding: error.blocked_binding.clone(),
    };
    let response_envelope =
        build_proxy_error_response_envelope(&downstream_error, &trace.invoke_id);
    let terminal_request_compression_algorithm = resolve_terminal_request_compression_algorithm(
        latest_pool_attempt_request_compression_algorithm(state.as_ref(), trace)
            .await
            .ok()
            .flatten(),
        pool_terminal_request_compression_algorithm(headers, error).map(str::to_string),
    );
    let _ = persist_pre_attempt_proxy_capture_error(
        state,
        proxy_request_id,
        capture_started,
        &trace.invoke_id,
        &trace.occurred_at,
        capture_target,
        &request_info,
        requester_ip.as_deref(),
        &request_chain_metadata,
        trace.sticky_key.as_deref(),
        prompt_cache_key,
        &client_attribution_context,
        request_body,
        request_body_snapshot,
        request_body_logging_enabled,
        runtime_snapshot_context
            .map(|context| context.t_req_read_ms)
            .unwrap_or_default(),
        runtime_snapshot_context
            .map(|context| context.t_req_parse_ms)
            .unwrap_or_default(),
        error.status,
        error.failure_kind,
        &error.message,
        Some(&error.attempt_summary),
        error.account.as_ref(),
        Some(error.connect_latency_ms),
        Some(error),
        terminal_request_compression_algorithm.as_deref(),
        Some(response_envelope),
    )
    .await;
}

pub(crate) fn forwarded_request_compression_observation(
    headers: &HeaderMap,
) -> Option<(&'static str, &'static str)> {
    let encodings = parse_content_encodings(
        headers
            .get(header::CONTENT_ENCODING)
            .and_then(|value| value.to_str().ok()),
    );
    if encodings.is_empty() || encodings.iter().all(|encoding| encoding == "identity") {
        return Some(("identity", "identity"));
    }

    match encodings.as_slice() {
        [encoding] => match encoding.as_str() {
            "gzip" | "x-gzip" => Some(("gzip", "passthrough")),
            "deflate" => Some(("deflate", "passthrough")),
            "zstd" => Some(("zstd", "passthrough")),
            _ => None,
        },
        _ => None,
    }
}

async fn latest_pool_attempt_request_compression_algorithm(
    state: &AppState,
    trace: &PoolUpstreamAttemptTraceContext,
) -> Result<Option<Option<String>>, sqlx::Error> {
    sqlx::query_scalar::<_, Option<String>>(
        r#"
        SELECT NULLIF(TRIM(upstream_request_compression_algorithm), '')
        FROM pool_upstream_request_attempts
        WHERE invoke_id = ?1
          AND occurred_at = ?2
          AND LOWER(TRIM(COALESCE(status, ''))) <> 'budget_exhausted_final'
        ORDER BY attempt_index DESC, id DESC
        LIMIT 1
        "#,
    )
    .bind(&trace.invoke_id)
    .bind(&trace.occurred_at)
    .fetch_optional(&state.pool)
    .await
}

pub(crate) fn resolve_terminal_request_compression_algorithm(
    latest_attempt_algorithm: Option<Option<String>>,
    fallback_algorithm: Option<String>,
) -> Option<String> {
    latest_attempt_algorithm.unwrap_or(fallback_algorithm)
}

fn pool_terminal_request_compression_algorithm(
    headers: &HeaderMap,
    error: &PoolUpstreamError,
) -> Option<&'static str> {
    if error.attempt_summary.pool_attempt_count == 0 {
        return None;
    }

    let account = error.account.as_ref()?;
    resolved_pool_request_compression_algorithm(
        headers,
        account.auth.is_oauth(),
        account.request_compression_algorithm,
    )
}

pub(crate) fn resolved_pool_request_compression_algorithm(
    headers: &HeaderMap,
    is_oauth: bool,
    configured_algorithm: RequestCompressionAlgorithm,
) -> Option<&'static str> {
    if is_oauth || configured_algorithm == RequestCompressionAlgorithm::Follow {
        forwarded_request_compression_observation(headers).map(|(algorithm, _)| algorithm)
    } else {
        Some(configured_algorithm.as_str())
    }
}

pub(crate) async fn persist_pool_attempt_response_capture(
    state: Arc<AppState>,
    pending: &mut PendingPoolAttemptRecord,
    response_body: Bytes,
    response_body_logging_enabled: bool,
    response_content_encoding: Option<String>,
) {
    let capture_key = pool_attempt_response_capture_key(pending);
    let raw_meta = spawn_raw_payload_file_write(
        state.as_ref(),
        &capture_key,
        "response",
        response_body,
        response_body_logging_enabled,
    )
    .finish()
    .await;
    // The terminal handoff must own completed metadata even when this smaller
    // runtime update cannot acquire capacity. Callers finalize and broadcast next.
    set_pending_pool_upstream_request_attempt_response_capture(
        pending,
        &raw_meta,
        response_content_encoding.as_deref(),
    );
    if let Err(err) =
        persist_pool_upstream_request_attempt_response_capture_in_memory(state.as_ref(), pending)
            .await
    {
        warn!(
            invoke_id = %pending.invoke_id,
            error = %err,
            "failed to persist pool attempt response capture"
        );
        if pending.attempt_id.is_none() {
            return;
        }
        let mut fallback_persisted = false;
        for _ in 0..3 {
            tokio::time::sleep(Duration::from_millis(20)).await;
            if persist_pool_upstream_request_attempt_response_capture_fallback(&state.pool, pending)
                .await
                .is_ok()
            {
                fallback_persisted = true;
                break;
            }
        }
        if !fallback_persisted {
            warn!(invoke_id = %pending.invoke_id, "pool attempt response capture fallback did not persist metadata");
        }
    }
}

pub(crate) fn take_priority_handoff_terminal_error(
    account: &PoolResolvedAccount,
    last_error: &mut Option<PoolUpstreamError>,
    attempt_count: usize,
    distinct_account_count: usize,
) -> Option<PoolUpstreamError> {
    if !priority_handoff_is_single_attempt(account) {
        return None;
    }
    let mut error = last_error.take()?;
    error.attempt_summary = pool_attempt_summary(
        attempt_count,
        distinct_account_count,
        Some(error.failure_kind.to_string()),
    );
    Some(error)
}

pub(crate) fn priority_handoff_is_single_attempt(account: &PoolResolvedAccount) -> bool {
    account.routing_source == PoolRoutingSelectionSource::PriorityHandoff
        && account
            .priority_handoff_permit
            .as_ref()
            .is_some_and(|permit| permit.is_sticky_migration())
}
