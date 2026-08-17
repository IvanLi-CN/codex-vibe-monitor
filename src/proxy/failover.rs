use super::*;

#[cfg(test)]
pub(crate) fn pool_no_available_wait_hooks()
-> &'static std::sync::Mutex<std::collections::HashMap<usize, Vec<std::sync::mpsc::Sender<()>>>> {
    static HOOKS: std::sync::OnceLock<
        std::sync::Mutex<std::collections::HashMap<usize, Vec<std::sync::mpsc::Sender<()>>>>,
    > = std::sync::OnceLock::new();
    HOOKS.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()))
}

#[cfg(test)]
pub(crate) fn register_pool_no_available_wait_hook(
    state: &Arc<AppState>,
) -> std::sync::mpsc::Receiver<()> {
    let (tx, rx) = std::sync::mpsc::channel();
    pool_no_available_wait_hooks()
        .lock()
        .expect("lock pool no-available wait hooks")
        .entry(Arc::as_ptr(state) as usize)
        .or_default()
        .push(tx);
    rx
}

#[cfg(test)]
pub(crate) fn notify_pool_no_available_wait_hook(state: &AppState) {
    let listeners = pool_no_available_wait_hooks()
        .lock()
        .expect("lock pool no-available wait hooks")
        .remove(&(state as *const AppState as usize));
    if let Some(listeners) = listeners {
        for listener in listeners {
            let _ = listener.send(());
        }
    }
}

#[cfg(not(test))]
pub(crate) fn notify_pool_no_available_wait_hook(_state: &AppState) {}

pub(crate) fn parse_retry_after_delay(value: &HeaderValue) -> Option<Duration> {
    let text = value.to_str().ok()?.trim();
    if text.is_empty() {
        return None;
    }

    if let Ok(seconds) = text.parse::<u64>() {
        return Some(Duration::from_secs(seconds).min(Duration::from_secs(
            MAX_PROXY_UPSTREAM_429_RETRY_AFTER_DELAY_SECS,
        )));
    }

    let retry_at = httpdate::parse_http_date(text).ok()?;
    let delay = retry_at.duration_since(std::time::SystemTime::now()).ok()?;
    Some(delay.min(Duration::from_secs(
        MAX_PROXY_UPSTREAM_429_RETRY_AFTER_DELAY_SECS,
    )))
}

pub(crate) async fn canonical_pool_attempt_proxy_binding_key(
    state: &AppState,
    selected_proxy_key: &str,
) -> Option<String> {
    let manager = state.forward_proxy.lock().await;
    manager.canonicalize_bound_proxy_key(selected_proxy_key, None)
}

pub(crate) fn normalize_pool_attempt_group_name(group_name: Option<String>) -> Option<String> {
    group_name
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

pub(crate) async fn resolve_pool_account_for_request_with_wait(
    state: &AppState,
    sticky_key: Option<&str>,
    requested_model: Option<&str>,
    excluded_ids: &[i64],
    excluded_upstream_route_keys: &HashSet<String>,
    required_upstream_route_key: Option<&str>,
    wait_for_no_available: bool,
    wait_deadline: &mut Option<Instant>,
    total_timeout_deadline: Option<Instant>,
) -> Result<PoolAccountResolutionWithWait> {
    resolve_pool_account_for_request_with_wait_and_binding_constraint_internal(
        state,
        sticky_key,
        requested_model,
        excluded_ids,
        excluded_upstream_route_keys,
        required_upstream_route_key,
        None,
        None,
        wait_for_no_available,
        wait_deadline,
        total_timeout_deadline,
        "",
        crate::ImageIntent::Unknown,
        false,
        None,
    )
    .await
}

pub(crate) async fn resolve_pool_account_for_request_with_wait_and_image_intent(
    state: &AppState,
    sticky_key: Option<&str>,
    requested_model: Option<&str>,
    excluded_ids: &[i64],
    excluded_upstream_route_keys: &HashSet<String>,
    required_upstream_route_key: Option<&str>,
    wait_for_no_available: bool,
    wait_deadline: &mut Option<Instant>,
    total_timeout_deadline: Option<Instant>,
    endpoint: &str,
    image_intent: crate::ImageIntent,
) -> Result<PoolAccountResolutionWithWait> {
    resolve_pool_account_for_request_with_wait_and_image_intent_and_codex_imagegen_request(
        state,
        sticky_key,
        requested_model,
        excluded_ids,
        excluded_upstream_route_keys,
        required_upstream_route_key,
        wait_for_no_available,
        wait_deadline,
        total_timeout_deadline,
        endpoint,
        image_intent,
        false,
    )
    .await
}

pub(crate) async fn resolve_pool_account_for_request_with_wait_and_image_intent_and_codex_imagegen_request(
    state: &AppState,
    sticky_key: Option<&str>,
    requested_model: Option<&str>,
    excluded_ids: &[i64],
    excluded_upstream_route_keys: &HashSet<String>,
    required_upstream_route_key: Option<&str>,
    wait_for_no_available: bool,
    wait_deadline: &mut Option<Instant>,
    total_timeout_deadline: Option<Instant>,
    endpoint: &str,
    image_intent: crate::ImageIntent,
    codex_imagegen_request: bool,
) -> Result<PoolAccountResolutionWithWait> {
    resolve_pool_account_for_request_with_wait_and_image_intent_and_codex_imagegen_request_and_reservation(
        state,
        sticky_key,
        requested_model,
        excluded_ids,
        excluded_upstream_route_keys,
        required_upstream_route_key,
        wait_for_no_available,
        wait_deadline,
        total_timeout_deadline,
        endpoint,
        image_intent,
        codex_imagegen_request,
        None,
    )
    .await
}

pub(crate) async fn resolve_pool_account_for_request_with_wait_and_image_intent_and_codex_imagegen_request_and_reservation(
    state: &AppState,
    sticky_key: Option<&str>,
    requested_model: Option<&str>,
    excluded_ids: &[i64],
    excluded_upstream_route_keys: &HashSet<String>,
    required_upstream_route_key: Option<&str>,
    wait_for_no_available: bool,
    wait_deadline: &mut Option<Instant>,
    total_timeout_deadline: Option<Instant>,
    endpoint: &str,
    image_intent: crate::ImageIntent,
    codex_imagegen_request: bool,
    reservation_key: Option<&str>,
) -> Result<PoolAccountResolutionWithWait> {
    resolve_pool_account_for_request_with_wait_and_binding_constraint_with_image_intent_and_override_and_codex_imagegen_request_and_reservation(
        state,
        sticky_key,
        requested_model,
        excluded_ids,
        excluded_upstream_route_keys,
        required_upstream_route_key,
        None,
        None,
        wait_for_no_available,
        wait_deadline,
        total_timeout_deadline,
        endpoint,
        image_intent,
        codex_imagegen_request,
        reservation_key,
    )
    .await
}

pub(crate) async fn resolve_pool_account_for_request_with_wait_and_binding_constraint(
    state: &AppState,
    sticky_key: Option<&str>,
    requested_model: Option<&str>,
    excluded_ids: &[i64],
    excluded_upstream_route_keys: &HashSet<String>,
    required_upstream_route_key: Option<&str>,
    binding_constraint: Option<&PromptCacheConversationBindingConstraint>,
    wait_for_no_available: bool,
    wait_deadline: &mut Option<Instant>,
    total_timeout_deadline: Option<Instant>,
) -> Result<PoolAccountResolutionWithWait> {
    resolve_pool_account_for_request_with_wait_and_binding_constraint_internal(
        state,
        sticky_key,
        requested_model,
        excluded_ids,
        excluded_upstream_route_keys,
        required_upstream_route_key,
        binding_constraint,
        None,
        wait_for_no_available,
        wait_deadline,
        total_timeout_deadline,
        "",
        crate::ImageIntent::Unknown,
        false,
        None,
    )
    .await
}

pub(crate) async fn resolve_pool_account_for_request_with_wait_and_binding_constraint_with_image_intent(
    state: &AppState,
    sticky_key: Option<&str>,
    requested_model: Option<&str>,
    excluded_ids: &[i64],
    excluded_upstream_route_keys: &HashSet<String>,
    required_upstream_route_key: Option<&str>,
    binding_constraint: Option<&PromptCacheConversationBindingConstraint>,
    wait_for_no_available: bool,
    wait_deadline: &mut Option<Instant>,
    total_timeout_deadline: Option<Instant>,
    endpoint: &str,
    image_intent: crate::ImageIntent,
) -> Result<PoolAccountResolutionWithWait> {
    resolve_pool_account_for_request_with_wait_and_binding_constraint_with_image_intent_and_override_and_codex_imagegen_request(
        state,
        sticky_key,
        requested_model,
        excluded_ids,
        excluded_upstream_route_keys,
        required_upstream_route_key,
        binding_constraint,
        None,
        wait_for_no_available,
        wait_deadline,
        total_timeout_deadline,
        endpoint,
        image_intent,
        false,
    )
    .await
}

pub(crate) async fn resolve_pool_account_for_request_with_wait_and_binding_constraint_with_image_intent_and_override(
    state: &AppState,
    sticky_key: Option<&str>,
    requested_model: Option<&str>,
    excluded_ids: &[i64],
    excluded_upstream_route_keys: &HashSet<String>,
    required_upstream_route_key: Option<&str>,
    binding_constraint: Option<&PromptCacheConversationBindingConstraint>,
    conversation_override: Option<&ConversationRoutingOverride>,
    wait_for_no_available: bool,
    wait_deadline: &mut Option<Instant>,
    total_timeout_deadline: Option<Instant>,
    endpoint: &str,
    image_intent: crate::ImageIntent,
) -> Result<PoolAccountResolutionWithWait> {
    resolve_pool_account_for_request_with_wait_and_binding_constraint_with_image_intent_and_override_and_codex_imagegen_request_and_reservation(
        state,
        sticky_key,
        requested_model,
        excluded_ids,
        excluded_upstream_route_keys,
        required_upstream_route_key,
        binding_constraint,
        conversation_override,
        wait_for_no_available,
        wait_deadline,
        total_timeout_deadline,
        endpoint,
        image_intent,
        false,
        None,
    )
    .await
}

pub(crate) async fn resolve_pool_account_for_request_with_wait_and_binding_constraint_with_image_intent_and_override_and_reservation(
    state: &AppState,
    sticky_key: Option<&str>,
    requested_model: Option<&str>,
    excluded_ids: &[i64],
    excluded_upstream_route_keys: &HashSet<String>,
    required_upstream_route_key: Option<&str>,
    binding_constraint: Option<&PromptCacheConversationBindingConstraint>,
    conversation_override: Option<&ConversationRoutingOverride>,
    wait_for_no_available: bool,
    wait_deadline: &mut Option<Instant>,
    total_timeout_deadline: Option<Instant>,
    endpoint: &str,
    image_intent: crate::ImageIntent,
    reservation_key: Option<&str>,
) -> Result<PoolAccountResolutionWithWait> {
    resolve_pool_account_for_request_with_wait_and_binding_constraint_with_image_intent_and_override_and_codex_imagegen_request_and_reservation(
        state,
        sticky_key,
        requested_model,
        excluded_ids,
        excluded_upstream_route_keys,
        required_upstream_route_key,
        binding_constraint,
        conversation_override,
        wait_for_no_available,
        wait_deadline,
        total_timeout_deadline,
        endpoint,
        image_intent,
        false,
        reservation_key,
    )
    .await
}

pub(crate) async fn resolve_pool_account_for_request_with_wait_and_binding_constraint_with_image_intent_and_override_and_codex_imagegen_request(
    state: &AppState,
    sticky_key: Option<&str>,
    requested_model: Option<&str>,
    excluded_ids: &[i64],
    excluded_upstream_route_keys: &HashSet<String>,
    required_upstream_route_key: Option<&str>,
    binding_constraint: Option<&PromptCacheConversationBindingConstraint>,
    conversation_override: Option<&ConversationRoutingOverride>,
    wait_for_no_available: bool,
    wait_deadline: &mut Option<Instant>,
    total_timeout_deadline: Option<Instant>,
    endpoint: &str,
    image_intent: crate::ImageIntent,
    codex_imagegen_request: bool,
) -> Result<PoolAccountResolutionWithWait> {
    resolve_pool_account_for_request_with_wait_and_binding_constraint_with_image_intent_and_override_and_codex_imagegen_request_and_reservation(
        state,
        sticky_key,
        requested_model,
        excluded_ids,
        excluded_upstream_route_keys,
        required_upstream_route_key,
        binding_constraint,
        conversation_override,
        wait_for_no_available,
        wait_deadline,
        total_timeout_deadline,
        endpoint,
        image_intent,
        codex_imagegen_request,
        None,
    )
    .await
}

pub(crate) async fn resolve_pool_account_for_request_with_wait_and_binding_constraint_with_image_intent_and_override_and_codex_imagegen_request_and_reservation(
    state: &AppState,
    sticky_key: Option<&str>,
    requested_model: Option<&str>,
    excluded_ids: &[i64],
    excluded_upstream_route_keys: &HashSet<String>,
    required_upstream_route_key: Option<&str>,
    binding_constraint: Option<&PromptCacheConversationBindingConstraint>,
    conversation_override: Option<&ConversationRoutingOverride>,
    wait_for_no_available: bool,
    wait_deadline: &mut Option<Instant>,
    total_timeout_deadline: Option<Instant>,
    endpoint: &str,
    image_intent: crate::ImageIntent,
    codex_imagegen_request: bool,
    reservation_key: Option<&str>,
) -> Result<PoolAccountResolutionWithWait> {
    resolve_pool_account_for_request_with_wait_and_binding_constraint_internal(
        state,
        sticky_key,
        requested_model,
        excluded_ids,
        excluded_upstream_route_keys,
        required_upstream_route_key,
        binding_constraint,
        conversation_override,
        wait_for_no_available,
        wait_deadline,
        total_timeout_deadline,
        endpoint,
        image_intent,
        codex_imagegen_request,
        reservation_key,
    )
    .await
}

pub(crate) async fn resolve_pool_account_for_request_with_wait_and_binding_constraint_internal(
    state: &AppState,
    sticky_key: Option<&str>,
    requested_model: Option<&str>,
    excluded_ids: &[i64],
    excluded_upstream_route_keys: &HashSet<String>,
    required_upstream_route_key: Option<&str>,
    binding_constraint: Option<&PromptCacheConversationBindingConstraint>,
    conversation_override: Option<&ConversationRoutingOverride>,
    wait_for_no_available: bool,
    wait_deadline: &mut Option<Instant>,
    total_timeout_deadline: Option<Instant>,
    endpoint: &str,
    image_intent: crate::ImageIntent,
    codex_imagegen_request: bool,
    reservation_key: Option<&str>,
) -> Result<PoolAccountResolutionWithWait> {
    let poll_interval = state.pool_no_available_wait.normalized_poll_interval();

    loop {
        let now = Instant::now();
        if total_timeout_deadline.is_some_and(|deadline| now >= deadline) {
            return Ok(PoolAccountResolutionWithWait::TotalTimeoutExpired);
        }
        let resolution =
            resolve_pool_account_for_request_with_route_requirement_and_image_intent_and_override_and_codex_imagegen_request_and_reservation(
                state,
                sticky_key,
                requested_model,
                excluded_ids,
                excluded_upstream_route_keys,
                required_upstream_route_key,
                binding_constraint,
                conversation_override,
                endpoint,
                image_intent,
                codex_imagegen_request,
                reservation_key,
            )
            .await?;
        if wait_for_no_available
            && matches!(
                resolution,
                PoolAccountResolution::Unavailable | PoolAccountResolution::NoCandidate
            )
            && wait_deadline.is_none()
        {
            *wait_deadline = Some(Instant::now() + state.pool_no_available_wait.timeout);
        }
        if total_timeout_deadline.is_some_and(|deadline| Instant::now() >= deadline) {
            return Ok(PoolAccountResolutionWithWait::TotalTimeoutExpired);
        }
        match resolution {
            resolution @ (PoolAccountResolution::Unavailable
            | PoolAccountResolution::NoCandidate)
                if wait_for_no_available =>
            {
                let wait_deadline = if let Some(deadline) = *wait_deadline {
                    deadline
                } else {
                    let deadline = Instant::now() + state.pool_no_available_wait.timeout;
                    *wait_deadline = Some(deadline);
                    deadline
                };
                let effective_deadline = total_timeout_deadline
                    .map(|deadline| std::cmp::min(wait_deadline, deadline))
                    .unwrap_or(wait_deadline);
                let now = Instant::now();
                if now >= effective_deadline {
                    if total_timeout_deadline.is_some_and(|deadline| deadline <= wait_deadline) {
                        return Ok(PoolAccountResolutionWithWait::TotalTimeoutExpired);
                    }
                    return Ok(PoolAccountResolutionWithWait::Resolution(resolution));
                }
                notify_pool_no_available_wait_hook(state);
                tokio::time::sleep(
                    poll_interval.min(effective_deadline.saturating_duration_since(now)),
                )
                .await;
            }
            _ => return Ok(PoolAccountResolutionWithWait::Resolution(resolution)),
        }
    }
}

async fn maybe_build_and_record_single_account_binding_terminal_error(
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

pub(crate) async fn send_pool_request_with_failover(
    state: Arc<AppState>,
    proxy_request_id: u64,
    method: Method,
    original_uri: &Uri,
    headers: &HeaderMap,
    body: Option<PoolReplayBodySnapshot>,
    handshake_timeout: Duration,
    trace_context: Option<PoolUpstreamAttemptTraceContext>,
    runtime_snapshot_context: Option<PoolAttemptRuntimeSnapshotContext>,
    sticky_key: Option<&str>,
    preferred_account: Option<PoolResolvedAccount>,
    failover_progress: PoolFailoverProgress,
    same_account_attempts: u8,
) -> Result<PoolUpstreamResponse, PoolUpstreamError> {
    send_pool_request_with_failover_and_binding_constraint(
        state,
        proxy_request_id,
        method,
        original_uri,
        headers,
        body,
        handshake_timeout,
        trace_context,
        runtime_snapshot_context,
        sticky_key,
        None,
        None,
        None,
        preferred_account,
        failover_progress,
        same_account_attempts,
        true,
    )
    .await
}

pub(crate) fn send_pool_request_with_failover_and_binding_constraint<'a>(
    state: Arc<AppState>,
    proxy_request_id: u64,
    method: Method,
    original_uri: &'a Uri,
    headers: &'a HeaderMap,
    body: Option<PoolReplayBodySnapshot>,
    handshake_timeout: Duration,
    trace_context: Option<PoolUpstreamAttemptTraceContext>,
    runtime_snapshot_context: Option<PoolAttemptRuntimeSnapshotContext>,
    sticky_key: Option<&'a str>,
    sticky_event_prompt_cache_key: Option<&'a str>,
    binding_constraint: Option<PromptCacheConversationBindingConstraint>,
    conversation_override: Option<ConversationRoutingOverride>,
    preferred_account: Option<PoolResolvedAccount>,
    failover_progress: PoolFailoverProgress,
    same_account_attempts: u8,
    persist_terminal_invocation: bool,
) -> Pin<Box<dyn Future<Output = Result<PoolUpstreamResponse, PoolUpstreamError>> + Send + 'a>> {
    // Keep this large state machine off the request task stack. The imagegen audit
    // extends its state enough to overflow normal test and Axum worker stacks.
    Box::pin(async move {
        let capture_started = Instant::now();
        let state_for_terminal_capture = state.clone();
        let body_for_terminal_capture = body.clone();
        let trace_for_terminal_capture = trace_context.clone();
        let runtime_context_for_terminal_capture = runtime_snapshot_context.clone();
        let result = send_pool_request_with_failover_and_binding_constraint_inner(
            state,
            proxy_request_id,
            method,
            original_uri,
            headers,
            body,
            handshake_timeout,
            trace_context,
            runtime_snapshot_context,
            sticky_key,
            sticky_event_prompt_cache_key,
            binding_constraint,
            conversation_override,
            preferred_account,
            failover_progress,
            same_account_attempts,
        )
        .await;
        if persist_terminal_invocation && let Err(error) = &result {
            persist_pool_failover_terminal_invocation(
                state_for_terminal_capture,
                proxy_request_id,
                capture_started,
                original_uri,
                headers,
                trace_for_terminal_capture.as_ref(),
                runtime_context_for_terminal_capture.as_ref(),
                body_for_terminal_capture.unwrap_or(PoolReplayBodySnapshot::Empty),
                error,
            )
            .await;
        }
        result
    })
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
    let live_request_streaming_decision = runtime_snapshot_context
        .and_then(|context| context.live_request_streaming_decision.as_ref());
    let live_request_streaming_measurement =
        live_request_streaming_decision.map(|_| LiveRequestStreamingMeasurement {
            first_attempt_failed: runtime_snapshot_context
                .is_some_and(|context| context.live_first_attempt_failed),
            fallback_or_retry: error.attempt_summary.pool_attempt_count > 1,
            ambiguous_upstream_delivery: runtime_snapshot_context
                .is_some_and(|context| context.live_first_attempt_failed),
            experiment_account_group: runtime_snapshot_context
                .and_then(|context| context.live_request_streaming_experiment_group.clone()),
            ..LiveRequestStreamingMeasurement::default()
        });
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
        live_request_streaming_decision,
        live_request_streaming_measurement.as_ref(),
    )
    .await;
}

fn forwarded_request_compression_observation(
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

fn resolve_terminal_request_compression_algorithm(
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

fn resolved_pool_request_compression_algorithm(
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

fn spawn_pool_attempt_response_capture(
    state: Arc<AppState>,
    pending: PendingPoolAttemptRecord,
    response_body: Bytes,
    response_body_logging_enabled: bool,
    response_content_encoding: Option<String>,
) {
    let capture_key = pool_attempt_response_capture_key(&pending);
    tokio::spawn(async move {
        let raw_meta = spawn_raw_payload_file_write(
            state.as_ref(),
            &capture_key,
            "response",
            response_body,
            response_body_logging_enabled,
        )
        .finish()
        .await;
        let mut pending = pending;
        set_pending_pool_upstream_request_attempt_response_capture(
            &mut pending,
            &raw_meta,
            response_content_encoding.as_deref(),
        );
        match persist_pool_upstream_request_attempt_response_capture(&state.pool, &pending).await {
            Ok(()) => {
                if let Err(err) =
                    broadcast_pool_upstream_attempts_snapshot(state.as_ref(), &pending.invoke_id)
                        .await
                {
                    warn!(
                        invoke_id = %pending.invoke_id,
                        error = %err,
                        "failed to broadcast asynchronous pool attempt response capture"
                    );
                }
            }
            Err(err) => {
                warn!(
                    invoke_id = %pending.invoke_id,
                    error = %err,
                    "failed to persist asynchronous pool attempt response capture"
                );
            }
        }
    });
}

async fn send_pool_request_with_failover_and_binding_constraint_inner(
    state: Arc<AppState>,
    proxy_request_id: u64,
    method: Method,
    original_uri: &Uri,
    headers: &HeaderMap,
    body: Option<PoolReplayBodySnapshot>,
    handshake_timeout: Duration,
    trace_context: Option<PoolUpstreamAttemptTraceContext>,
    runtime_snapshot_context: Option<PoolAttemptRuntimeSnapshotContext>,
    sticky_key: Option<&str>,
    sticky_event_prompt_cache_key: Option<&str>,
    binding_constraint: Option<PromptCacheConversationBindingConstraint>,
    conversation_override: Option<ConversationRoutingOverride>,
    preferred_account: Option<PoolResolvedAccount>,
    failover_progress: PoolFailoverProgress,
    same_account_attempts: u8,
) -> Result<PoolUpstreamResponse, PoolUpstreamError> {
    let request_connection_scoped = connection_scoped_header_names(headers);
    let codex_imagegen_request = codex_imagegen_protocol_from_headers(headers).is_some();
    let reservation_key = build_pool_routing_reservation_key(proxy_request_id);
    let mut reservation_guard =
        PoolRoutingReservationDropGuard::new(state.clone(), reservation_key.clone());
    let direct_image_request = matches!(
        capture_target_for_request(original_uri.path(), &method),
        Some(ProxyCaptureTarget::ImageGenerations | ProxyCaptureTarget::ImageEdits)
    );
    let capability_endpoint = if method == Method::POST {
        original_uri.path()
    } else {
        ""
    };
    let uses_timeout_route_failover =
        pool_uses_responses_timeout_failover_policy(original_uri, &method);
    let responses_total_timeout =
        pool_upstream_responses_total_timeout(&state.config, original_uri, &method);
    let pre_attempt_total_timeout_deadline =
        responses_total_timeout.map(|total_timeout| Instant::now() + total_timeout);
    let pre_attempt_total_timeout_started_at = responses_total_timeout
        .zip(pre_attempt_total_timeout_deadline)
        .map(|(total_timeout, deadline)| deadline - total_timeout);
    let mut responses_total_timeout_started_at =
        failover_progress.responses_total_timeout_started_at;
    let mut excluded_ids = failover_progress.excluded_account_ids;
    let mut excluded_upstream_route_keys = failover_progress.excluded_upstream_route_keys;
    let mut last_error = failover_progress.last_error;
    let mut preserve_sticky_owner_terminal_error = failover_progress
        .preserve_sticky_owner_terminal_error
        || pool_upstream_error_preserves_existing_sticky_owner(last_error.as_ref());
    let mut overload_required_upstream_route_key =
        failover_progress.overload_required_upstream_route_key;
    let mut attempted_account_ids = excluded_ids.iter().copied().collect::<HashSet<_>>();
    if let Some(account_id) = last_error
        .as_ref()
        .and_then(|error| error.account.as_ref())
        .map(|account| account.account_id)
    {
        attempted_account_ids.insert(account_id);
    }
    let initial_errors_all_rate_limited = if failover_progress.attempt_count == 0 {
        true
    } else {
        last_error
            .as_ref()
            .is_some_and(pool_upstream_error_is_rate_limited)
    };
    let mut preferred_account = preferred_account
        .filter(|account| !excluded_upstream_route_keys.contains(&account.upstream_route_key()))
        .filter(|account| {
            binding_constraint
                .as_ref()
                .is_none_or(|constraint| match constraint {
                    PromptCacheConversationBindingConstraint::Group(group_name) => account
                        .group_name
                        .as_deref()
                        .map(str::trim)
                        .is_some_and(|value| value == group_name),
                    PromptCacheConversationBindingConstraint::UpstreamAccount(account_id) => {
                        account.account_id == *account_id
                    }
                })
        });
    let initial_same_account_attempts = same_account_attempts.max(1);
    let mut attempt_count = failover_progress.attempt_count;
    let mut timeout_route_failover_pending = failover_progress.timeout_route_failover_pending;
    let mut exhausted_accounts_all_rate_limited = initial_errors_all_rate_limited;
    let mut no_available_wait_deadline = failover_progress.no_available_wait_deadline;
    let requested_model = runtime_snapshot_context
        .as_ref()
        .and_then(|ctx| ctx.request_info.model.as_deref())
        .map(str::to_string);
    let image_intent = runtime_snapshot_context
        .as_ref()
        .and_then(|ctx| ctx.request_info.image_intent.as_deref())
        .map(crate::ImageIntent::from_str)
        .unwrap_or(crate::ImageIntent::Unknown);
    let encrypted_session_owner_guard_active = runtime_snapshot_context
        .as_ref()
        .is_some_and(|ctx| ctx.owner_auto_guard_active);
    let prompt_cache_key = runtime_snapshot_context
        .as_ref()
        .and_then(|ctx| ctx.prompt_cache_key.as_deref());
    let request_compression_level_preset = load_pool_routing_runtime_cache(state.as_ref())
        .await
        .map_err(|err| PoolUpstreamError {
            codex_imagegen_rewrite: None,
            account: None,
            status: StatusCode::BAD_GATEWAY,
            message: format!("failed to load pool routing runtime cache: {err}"),
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
            requested_service_tier: None,
            request_body_for_capture: None,
        })?
        .request_compression
        .level_preset;

    'account_loop: loop {
        let mut distinct_account_count = attempted_account_ids.len();
        if let (Some(total_timeout), Some(started_at)) =
            (responses_total_timeout, responses_total_timeout_started_at)
            && pool_total_timeout_exhausted(total_timeout, started_at)
        {
            let final_error = build_pool_total_timeout_exhausted_error(
                total_timeout,
                last_error,
                attempt_count,
                distinct_account_count,
            );
            if let Some(trace) = trace_context.as_ref()
                && let Err(err) = insert_pool_upstream_terminal_attempt(
                    &state.pool,
                    trace,
                    &final_error,
                    (attempt_count + 1) as i64,
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
        }
        if preferred_account.is_none()
            && (excluded_ids.len() >= POOL_UPSTREAM_MAX_DISTINCT_ACCOUNTS
                || (uses_timeout_route_failover
                    && timeout_route_failover_pending
                    && excluded_upstream_route_keys.len()
                        >= POOL_UPSTREAM_RESPONSES_MAX_TIMEOUT_ROUTE_KEYS))
        {
            let terminal_failure_kind =
                if uses_timeout_route_failover && timeout_route_failover_pending {
                    PROXY_FAILURE_POOL_NO_ALTERNATE_UPSTREAM_AFTER_TIMEOUT
                } else {
                    PROXY_FAILURE_POOL_MAX_DISTINCT_ACCOUNTS_EXHAUSTED
                };
            if terminal_failure_kind == PROXY_FAILURE_POOL_MAX_DISTINCT_ACCOUNTS_EXHAUSTED
                && let Some(err) = take_and_record_sticky_owner_terminal_error(
                    state.as_ref(),
                    trace_context.as_ref(),
                    preserve_sticky_owner_terminal_error,
                    &mut last_error,
                    attempt_count,
                    distinct_account_count,
                )
                .await
            {
                return Err(err);
            }
            let terminal_message = if terminal_failure_kind
                == PROXY_FAILURE_POOL_NO_ALTERNATE_UPSTREAM_AFTER_TIMEOUT
            {
                "no alternate upstream route is available after timeout".to_string()
            } else {
                "pool distinct-account retry budget exhausted".to_string()
            };
            let mut final_error = last_error.unwrap_or(PoolUpstreamError {
                codex_imagegen_rewrite: None,
                account: None,
                status: StatusCode::BAD_GATEWAY,
                message: terminal_message.clone(),
                canonical_error_message: None,
                failure_kind: terminal_failure_kind,
                blocked_binding: None,
                connect_latency_ms: 0.0,
                upstream_error_code: None,
                upstream_error_message: None,
                downstream_error_message: None,
                upstream_request_id: None,
                proxy_binding_key_snapshot: None,
                oauth_responses_debug: None,
                attempt_summary: PoolAttemptSummary::default(),
                requested_service_tier: None,
                request_body_for_capture: None,
            });
            if exhausted_accounts_all_rate_limited && distinct_account_count > 0 {
                final_error.status = StatusCode::TOO_MANY_REQUESTS;
                final_error.message = POOL_ALL_ACCOUNTS_RATE_LIMITED_MESSAGE.to_string();
                final_error.failure_kind = PROXY_FAILURE_POOL_ALL_ACCOUNTS_RATE_LIMITED;
                final_error.upstream_error_code = None;
                final_error.upstream_error_message = None;
                final_error.upstream_request_id = None;
            } else if terminal_failure_kind
                == PROXY_FAILURE_POOL_NO_ALTERNATE_UPSTREAM_AFTER_TIMEOUT
                || !matches!(
                    final_error.status,
                    StatusCode::TOO_MANY_REQUESTS | StatusCode::PAYLOAD_TOO_LARGE
                )
            {
                final_error.status = StatusCode::BAD_GATEWAY;
                final_error.message = terminal_message;
                final_error.failure_kind = terminal_failure_kind;
                final_error.upstream_error_code = None;
                final_error.upstream_error_message = None;
                final_error.upstream_request_id = None;
            }
            final_error.attempt_summary = pool_attempt_summary(
                attempt_count,
                distinct_account_count,
                Some(terminal_failure_kind.to_string()),
            );
            if let Some(trace) = trace_context.as_ref()
                && let Err(err) = insert_and_broadcast_pool_upstream_terminal_attempt(
                    state.as_ref(),
                    trace,
                    &final_error,
                    (attempt_count + 1) as i64,
                    distinct_account_count as i64,
                    terminal_failure_kind,
                )
                .await
            {
                warn!(
                    invoke_id = trace.invoke_id,
                    error = %err,
                    "failed to persist pool budget exhaustion attempt"
                );
            }
            return Err(final_error);
        }

        let account = if let Some(account) = preferred_account.take() {
            account
        } else {
            // Pre-attempt no-account gaps should wait across all routes. After a concrete
            // upstream failure, only responses-family failover keeps the bounded wait so a
            // recovered alternate can still take over; non-responses routes preserve the last
            // upstream failure instead of re-entering the generic no-account wait loop.
            let wait_for_no_available = attempt_count == 0
                || (uses_timeout_route_failover && !timeout_route_failover_pending);
            let total_timeout_deadline = responses_total_timeout_started_at
                .zip(responses_total_timeout)
                .map(|(started_at, total_timeout)| started_at + total_timeout)
                .or({
                    if attempt_count == 0 {
                        pre_attempt_total_timeout_deadline
                    } else {
                        None
                    }
                });
            let route_scoped_overload_selection = overload_required_upstream_route_key.clone();
            match resolve_pool_account_for_request_with_wait_and_binding_constraint_with_image_intent_and_override_and_codex_imagegen_request_and_reservation(
                state.as_ref(),
                sticky_key,
                requested_model.as_deref(),
                &excluded_ids,
                &excluded_upstream_route_keys,
                route_scoped_overload_selection.as_deref(),
                binding_constraint.as_ref(),
                conversation_override.as_ref(),
                wait_for_no_available,
                &mut no_available_wait_deadline,
                total_timeout_deadline,
                capability_endpoint,
                image_intent,
                codex_imagegen_request,
                Some(&reservation_key),
            )
            .await
            {
                Ok(PoolAccountResolutionWithWait::Resolution(PoolAccountResolution::Resolved(
                    account,
                ))) => account,
                Ok(PoolAccountResolutionWithWait::Resolution(_))
                    if route_scoped_overload_selection.is_some() =>
                {
                    let exhausted_route_key = overload_required_upstream_route_key
                        .take()
                        .expect("route-scoped overload selection should be present");
                    excluded_upstream_route_keys.insert(exhausted_route_key);
                    continue 'account_loop;
                }
                Ok(PoolAccountResolutionWithWait::TotalTimeoutExpired) => {
                    let final_error = build_pool_total_timeout_exhausted_error(
                        responses_total_timeout.expect("responses total timeout should be present"),
                        last_error,
                        attempt_count,
                        distinct_account_count,
                    );
                    if let Some(trace) = trace_context.as_ref()
                        && let Err(err) = insert_pool_upstream_terminal_attempt(
                            &state.pool,
                            trace,
                            &final_error,
                            (attempt_count + 1) as i64,
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
                }
                Ok(PoolAccountResolutionWithWait::Resolution(
                    PoolAccountResolution::RateLimited,
                )) => {
                    if let Some(err) = maybe_build_and_record_single_account_binding_terminal_error(
                        state.as_ref(),
                        trace_context.as_ref(),
                        binding_constraint.as_ref(),
                        encrypted_session_owner_guard_active,
                        prompt_cache_key,
                        None,
                        None,
                        attempt_count,
                        distinct_account_count,
                    )
                    .await
                    {
                        return Err(err);
                    }
                    if let Some(err) = take_and_record_sticky_owner_terminal_error(
                        state.as_ref(),
                        trace_context.as_ref(),
                        preserve_sticky_owner_terminal_error,
                        &mut last_error,
                        attempt_count,
                        distinct_account_count,
                    )
                    .await
                    {
                        return Err(err);
                    }
                    return Err(build_pool_rate_limited_error(
                        attempt_count,
                        distinct_account_count,
                        PROXY_FAILURE_POOL_ALL_ACCOUNTS_RATE_LIMITED,
                    ));
                }
                Ok(PoolAccountResolutionWithWait::Resolution(
                    PoolAccountResolution::DegradedOnly,
                )) => {
                    if let Some(err) = maybe_build_and_record_single_account_binding_terminal_error(
                        state.as_ref(),
                        trace_context.as_ref(),
                        binding_constraint.as_ref(),
                        encrypted_session_owner_guard_active,
                        prompt_cache_key,
                        None,
                        None,
                        attempt_count,
                        distinct_account_count,
                    )
                    .await
                    {
                        return Err(err);
                    }
                    if let Some(err) = take_and_record_sticky_owner_terminal_error(
                        state.as_ref(),
                        trace_context.as_ref(),
                        preserve_sticky_owner_terminal_error,
                        &mut last_error,
                        attempt_count,
                        distinct_account_count,
                    )
                    .await
                    {
                        return Err(err);
                    }
                    return Err(build_pool_degraded_only_error(
                        attempt_count,
                        distinct_account_count,
                    ));
                }
                Ok(PoolAccountResolutionWithWait::Resolution(
                    PoolAccountResolution::Unavailable,
                )) => {
                    if let Some(err) = maybe_build_and_record_single_account_binding_terminal_error(
                        state.as_ref(),
                        trace_context.as_ref(),
                        binding_constraint.as_ref(),
                        encrypted_session_owner_guard_active,
                        prompt_cache_key,
                        None,
                        None,
                        attempt_count,
                        distinct_account_count,
                    )
                    .await
                    {
                        return Err(err);
                    }
                    let terminal_failure_kind =
                        if uses_timeout_route_failover && timeout_route_failover_pending {
                            PROXY_FAILURE_POOL_NO_ALTERNATE_UPSTREAM_AFTER_TIMEOUT
                        } else {
                            PROXY_FAILURE_POOL_NO_AVAILABLE_ACCOUNT
                        };
                    if terminal_failure_kind == PROXY_FAILURE_POOL_NO_AVAILABLE_ACCOUNT
                        && let Some(err) = take_and_record_sticky_owner_terminal_error(
                            state.as_ref(),
                            trace_context.as_ref(),
                            preserve_sticky_owner_terminal_error,
                            &mut last_error,
                            attempt_count,
                            distinct_account_count,
                        )
                        .await
                    {
                        return Err(err);
                    }
                    let mut err = if terminal_failure_kind
                        == PROXY_FAILURE_POOL_NO_ALTERNATE_UPSTREAM_AFTER_TIMEOUT
                    {
                        last_error.unwrap_or(PoolUpstreamError {
        codex_imagegen_rewrite: None,
                            account: None,
                            status: StatusCode::BAD_GATEWAY,
                            message: "no alternate upstream route is available after timeout"
                                .to_string(),
                            canonical_error_message: None,
                            failure_kind: terminal_failure_kind,
                            blocked_binding: None,
                            connect_latency_ms: 0.0,
                            upstream_error_code: None,
                            upstream_error_message: None,
                            downstream_error_message: None,
                            upstream_request_id: None,
                            proxy_binding_key_snapshot: None,
                            oauth_responses_debug: None,
                            attempt_summary: PoolAttemptSummary::default(),
                            requested_service_tier: None,
                            request_body_for_capture: None,
                        })
                    } else {
                        build_pool_no_available_account_error(
                            attempt_count,
                            distinct_account_count,
                            state.pool_no_available_wait.retry_after_secs,
                        )
                    };
                    if terminal_failure_kind
                        == PROXY_FAILURE_POOL_NO_ALTERNATE_UPSTREAM_AFTER_TIMEOUT
                    {
                        err.status = StatusCode::BAD_GATEWAY;
                        err.message =
                            "no alternate upstream route is available after timeout".to_string();
                        err.failure_kind = terminal_failure_kind;
                        err.upstream_error_code = None;
                        err.upstream_error_message = None;
                        err.upstream_request_id = None;
                    }
                    err.attempt_summary = pool_attempt_summary(
                        attempt_count,
                        distinct_account_count,
                        Some(terminal_failure_kind.to_string()),
                    );
                    if terminal_failure_kind
                        == PROXY_FAILURE_POOL_NO_ALTERNATE_UPSTREAM_AFTER_TIMEOUT
                        && let Some(trace) = trace_context.as_ref()
                        && let Err(record_err) =
                            insert_and_broadcast_pool_upstream_terminal_attempt(
                                state.as_ref(),
                                trace,
                                &err,
                                (attempt_count + 1) as i64,
                                distinct_account_count as i64,
                                terminal_failure_kind,
                            )
                            .await
                    {
                        warn!(
                            invoke_id = trace.invoke_id,
                            error = %record_err,
                            "failed to persist pool no-alternate-after-timeout attempt"
                        );
                    }
                    return Err(err);
                }
                Ok(PoolAccountResolutionWithWait::Resolution(
                    PoolAccountResolution::NoCandidate,
                )) => {
                    if let Some(err) = maybe_build_and_record_single_account_binding_terminal_error(
                        state.as_ref(),
                        trace_context.as_ref(),
                        binding_constraint.as_ref(),
                        encrypted_session_owner_guard_active,
                        prompt_cache_key,
                        None,
                        None,
                        attempt_count,
                        distinct_account_count,
                    )
                    .await
                    {
                        return Err(err);
                    }
                    if uses_timeout_route_failover && timeout_route_failover_pending {
                        let mut err = last_error.unwrap_or(PoolUpstreamError {
        codex_imagegen_rewrite: None,
                            account: None,
                            status: StatusCode::BAD_GATEWAY,
                            message: "no alternate upstream route is available after timeout"
                                .to_string(),
                            canonical_error_message: None,
                            failure_kind: PROXY_FAILURE_POOL_NO_ALTERNATE_UPSTREAM_AFTER_TIMEOUT,
                            blocked_binding: None,
                            connect_latency_ms: 0.0,
                            upstream_error_code: None,
                            upstream_error_message: None,
                            downstream_error_message: None,
                            upstream_request_id: None,
                            proxy_binding_key_snapshot: None,
                            oauth_responses_debug: None,
                            attempt_summary: PoolAttemptSummary::default(),
                            requested_service_tier: None,
                            request_body_for_capture: None,
                        });
                        err.status = StatusCode::BAD_GATEWAY;
                        err.message =
                            "no alternate upstream route is available after timeout".to_string();
                        err.failure_kind = PROXY_FAILURE_POOL_NO_ALTERNATE_UPSTREAM_AFTER_TIMEOUT;
                        err.upstream_error_code = None;
                        err.upstream_error_message = None;
                        err.upstream_request_id = None;
                        err.attempt_summary = pool_attempt_summary(
                            attempt_count,
                            distinct_account_count,
                            Some(
                                PROXY_FAILURE_POOL_NO_ALTERNATE_UPSTREAM_AFTER_TIMEOUT.to_string(),
                            ),
                        );
                        if let Some(trace) = trace_context.as_ref()
                            && let Err(record_err) =
                                insert_and_broadcast_pool_upstream_terminal_attempt(
                                    state.as_ref(),
                                    trace,
                                    &err,
                                    (attempt_count + 1) as i64,
                                    distinct_account_count as i64,
                                    PROXY_FAILURE_POOL_NO_ALTERNATE_UPSTREAM_AFTER_TIMEOUT,
                                )
                                .await
                        {
                            warn!(
                                invoke_id = trace.invoke_id,
                                error = %record_err,
                                "failed to persist pool no-candidate no-alternate attempt"
                            );
                        }
                        return Err(err);
                    }

                    if let Some(err) = take_and_record_sticky_owner_terminal_error(
                        state.as_ref(),
                        trace_context.as_ref(),
                        preserve_sticky_owner_terminal_error,
                        &mut last_error,
                        attempt_count,
                        distinct_account_count,
                    )
                    .await
                    {
                        return Err(err);
                    }

                    return Err(
                        if exhausted_accounts_all_rate_limited && distinct_account_count > 0 {
                            build_pool_rate_limited_error(
                                attempt_count,
                                distinct_account_count,
                                PROXY_FAILURE_POOL_ALL_ACCOUNTS_RATE_LIMITED,
                            )
                        } else {
                            let mut err = last_error.unwrap_or_else(|| {
                                build_pool_no_available_account_error(
                                    attempt_count,
                                    distinct_account_count,
                                    state.pool_no_available_wait.retry_after_secs,
                                )
                            });
                            err.attempt_summary = pool_attempt_summary(
                                attempt_count,
                                distinct_account_count,
                                Some(err.failure_kind.to_string()),
                            );
                            err
                        },
                    );
                }
                Ok(PoolAccountResolutionWithWait::Resolution(
                    PoolAccountResolution::AssignedBlocked(blocked),
                )) => {
                    if let Some(err) = maybe_build_and_record_single_account_binding_terminal_error(
                        state.as_ref(),
                        trace_context.as_ref(),
                        binding_constraint.as_ref(),
                        encrypted_session_owner_guard_active,
                        prompt_cache_key,
                        Some(blocked.account.clone()),
                        Some(blocked.message.clone()),
                        attempt_count,
                        distinct_account_count,
                    )
                    .await
                    {
                        return Err(err);
                    }
                    if let Some(err) = take_and_record_sticky_owner_terminal_error(
                        state.as_ref(),
                        trace_context.as_ref(),
                        preserve_sticky_owner_terminal_error,
                        &mut last_error,
                        attempt_count,
                        distinct_account_count,
                    )
                    .await
                    {
                        return Err(err);
                    }
                    let terminal_failure_kind = blocked.failure_kind;
                    let err = build_pool_assigned_account_blocked_error(
                        blocked.account,
                        blocked.message,
                        terminal_failure_kind,
                        attempt_count,
                        distinct_account_count,
                    );
                    if let Some(trace) = trace_context.as_ref()
                        && let Err(record_err) =
                            insert_and_broadcast_pool_upstream_terminal_attempt(
                                state.as_ref(),
                                trace,
                                &err,
                                (attempt_count + 1) as i64,
                                distinct_account_count as i64,
                                terminal_failure_kind,
                            )
                            .await
                    {
                        warn!(
                            invoke_id = trace.invoke_id,
                            error = %record_err,
                            "failed to persist pool assigned-blocked terminal attempt"
                        );
                    }
                    return Err(err);
                }
                Ok(PoolAccountResolutionWithWait::Resolution(
                    PoolAccountResolution::BlockedByPolicy(message),
                )) => {
                    if let Some(err) = maybe_build_and_record_single_account_binding_terminal_error(
                        state.as_ref(),
                        trace_context.as_ref(),
                        binding_constraint.as_ref(),
                        encrypted_session_owner_guard_active,
                        prompt_cache_key,
                        None,
                        Some(message.clone()),
                        attempt_count,
                        distinct_account_count,
                    )
                    .await
                    {
                        return Err(err);
                    }
                    if let Some(err) = take_and_record_sticky_owner_terminal_error(
                        state.as_ref(),
                        trace_context.as_ref(),
                        preserve_sticky_owner_terminal_error,
                        &mut last_error,
                        attempt_count,
                        distinct_account_count,
                    )
                    .await
                    {
                        return Err(err);
                    }
                    let terminal_failure_kind = PROXY_FAILURE_POOL_ROUTING_BLOCKED;
                    let mut err = PoolUpstreamError {
        codex_imagegen_rewrite: None,
                        account: None,
                        status: StatusCode::SERVICE_UNAVAILABLE,
                        message,
                        failure_kind: terminal_failure_kind,
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
                    };
                    err.attempt_summary = pool_attempt_summary(
                        attempt_count,
                        distinct_account_count,
                        Some(terminal_failure_kind.to_string()),
                    );
                    if let Some(trace) = trace_context.as_ref()
                        && let Err(record_err) =
                            insert_and_broadcast_pool_upstream_terminal_attempt(
                                state.as_ref(),
                                trace,
                                &err,
                                (attempt_count + 1) as i64,
                                distinct_account_count as i64,
                                terminal_failure_kind,
                            )
                            .await
                    {
                        warn!(
                            invoke_id = trace.invoke_id,
                            error = %record_err,
                            "failed to persist pool blocked-policy terminal attempt"
                        );
                    }
                    return Err(err);
                }
                Err(err) => {
                    return Err(PoolUpstreamError {
        codex_imagegen_rewrite: None,
                        account: None,
                        status: StatusCode::BAD_GATEWAY,
                        message: format!("failed to resolve pool account: {err}"),
                        canonical_error_message: None,
                        failure_kind: PROXY_FAILURE_POOL_NO_AVAILABLE_ACCOUNT,
                        blocked_binding: None,
                        connect_latency_ms: 0.0,
                        upstream_error_code: None,
                        upstream_error_message: None,
                        downstream_error_message: None,
                        upstream_request_id: None,
                        proxy_binding_key_snapshot: None,
                        oauth_responses_debug: None,
                        attempt_summary: pool_attempt_summary(
                            attempt_count,
                            distinct_account_count,
                            Some(PROXY_FAILURE_POOL_NO_AVAILABLE_ACCOUNT.to_string()),
                        ),
                        requested_service_tier: None,
                        request_body_for_capture: None,
                    });
                }
            }
        };
        if responses_total_timeout_started_at.is_none() && no_available_wait_deadline.is_some() {
            responses_total_timeout_started_at = pre_attempt_total_timeout_started_at;
        }
        reserve_pool_routing_account_for_model(
            state.as_ref(),
            &reservation_key,
            &account,
            trace_context
                .as_ref()
                .and_then(|trace| trace.request_model.as_deref()),
        );
        timeout_route_failover_pending = false;

        let (_, _, runtime_timeouts) = load_effective_request_path_timeouts_for_account(
            &state.pool,
            &state.config,
            account.account_id,
            prompt_cache_key,
        )
        .await
        .map_err(|err| PoolUpstreamError {
            codex_imagegen_rewrite: None,
            account: Some(account.clone()),
            status: StatusCode::BAD_GATEWAY,
            message: format!("failed to resolve effective request-path timeouts: {err}"),
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
            attempt_summary: pool_attempt_summary(
                attempt_count,
                distinct_account_count,
                Some(PROXY_FAILURE_FAILED_CONTACT_UPSTREAM.to_string()),
            ),
            requested_service_tier: None,
            request_body_for_capture: None,
        })?;
        let pre_first_byte_timeout =
            pool_upstream_first_chunk_timeout(&runtime_timeouts, original_uri, &method);
        let stream_timeout = capture_target_for_request(original_uri.path(), &method)
            .and_then(|target| proxy_capture_target_stream_timeout(&runtime_timeouts, target));
        let send_timeout = pool_upstream_send_timeout(
            original_uri,
            &method,
            handshake_timeout,
            pre_first_byte_timeout,
        );

        excluded_ids.push(account.account_id);
        attempted_account_ids.insert(account.account_id);
        distinct_account_count = attempted_account_ids.len();
        let distinct_account_index = distinct_account_count as i64;
        let upstream_route_key = account.upstream_route_key();
        let api_key_target_url = match &account.auth {
            PoolResolvedAuth::ApiKey { .. } => {
                match build_proxy_upstream_url(&account.upstream_base_url, original_uri) {
                    Ok(url) => Some(url),
                    Err(err) => {
                        return Err(PoolUpstreamError {
                            codex_imagegen_rewrite: None,
                            account: Some(account),
                            status: StatusCode::BAD_GATEWAY,
                            message: format!("failed to build pool upstream url: {err}"),
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
                            attempt_summary: pool_attempt_summary(
                                attempt_count,
                                distinct_account_count,
                                Some(PROXY_FAILURE_FAILED_CONTACT_UPSTREAM.to_string()),
                            ),
                            requested_service_tier: None,
                            request_body_for_capture: None,
                        });
                    }
                }
            }
            PoolResolvedAuth::Oauth { .. } => None,
        };
        let same_account_attempt_budget = pool_same_account_attempt_budget(
            original_uri,
            &method,
            distinct_account_count,
            initial_same_account_attempts,
        );
        let overload_same_account_attempt_budget = pool_overload_same_account_attempt_budget(
            original_uri,
            &method,
            distinct_account_count,
            same_account_attempt_budget,
        );
        let group_upstream_429_max_retries = account.effective_upstream_429_max_retries();
        let same_account_attempt_loop_budget = overload_same_account_attempt_budget
            .saturating_add(1)
            .saturating_add(group_upstream_429_max_retries);
        let mut group_upstream_429_retry_count = 0_u8;
        let mut retried_upstream_413_for_account = last_error.as_ref().is_some_and(|err| {
            err.status == StatusCode::PAYLOAD_TOO_LARGE
                && err
                    .account
                    .as_ref()
                    .is_some_and(|err_account| err_account.account_id == account.account_id)
        });
        let mut first_response_attempt_started_at = None;

        for same_account_attempt in 0..same_account_attempt_loop_budget {
            if uses_timeout_route_failover && first_response_attempt_started_at.is_none() {
                first_response_attempt_started_at = Some(Instant::now());
            }
            let attempt_total_timeout_started_at = if uses_timeout_route_failover {
                if let Some(started_at) = responses_total_timeout_started_at {
                    Some(started_at)
                } else if same_account_attempt > 0 {
                    first_response_attempt_started_at
                } else {
                    None
                }
            } else {
                None
            };
            let Some(attempt_pre_first_byte_timeout) = pool_timeout_budget_with_total_limit(
                pre_first_byte_timeout,
                responses_total_timeout,
                attempt_total_timeout_started_at,
            ) else {
                let final_error = build_pool_total_timeout_exhausted_error(
                    responses_total_timeout.expect("responses total timeout should be present"),
                    last_error,
                    attempt_count,
                    distinct_account_count,
                );
                if attempt_count > 0
                    && let Some(trace) = trace_context.as_ref()
                    && let Err(err) = insert_pool_upstream_terminal_attempt(
                        &state.pool,
                        trace,
                        &final_error,
                        (attempt_count + 1) as i64,
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
                send_timeout,
                responses_total_timeout,
                attempt_total_timeout_started_at,
            ) else {
                let final_error = build_pool_total_timeout_exhausted_error(
                    responses_total_timeout.expect("responses total timeout should be present"),
                    last_error,
                    attempt_count,
                    distinct_account_count,
                );
                if let Some(trace) = trace_context.as_ref()
                    && let Err(err) = insert_pool_upstream_terminal_attempt(
                        &state.pool,
                        trace,
                        &final_error,
                        (attempt_count + 1) as i64,
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
            let connect_started = Instant::now();
            let attempt_started_at: String;
            let attempt_index: i64;
            let mut pending_attempt_record: Option<PendingPoolAttemptRecord>;
            let mut early_phase_cleanup_guard: Option<PoolEarlyPhaseOrphanCleanupGuard>;
            let live_attempt_activity_lease: Option<PoolLiveAttemptActivityLease>;
            let prepared_request_body = match prepare_pool_request_body_for_account(
                proxy_request_id,
                body.as_ref(),
                original_uri,
                &method,
                headers
                    .get(header::CONTENT_ENCODING)
                    .and_then(|value| value.to_str().ok()),
                account.fast_mode_rewrite_mode,
                account.image_tool_rewrite_mode,
                account.codex_imagegen_rewrite_mode,
                codex_imagegen_protocol_from_headers(headers),
                runtime_snapshot_context
                    .as_ref()
                    .map(|context| &context.request_info),
                runtime_snapshot_context
                    .as_ref()
                    .and_then(|context| context.hosted_image_intent),
            )
            .await
            {
                Ok(prepared) => prepared,
                Err(err) => {
                    release_pool_routing_reservation(state.as_ref(), &reservation_key);
                    return Err(PoolUpstreamError {
                        codex_imagegen_rewrite: None,
                        account: Some(account.clone()),
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
            let attempted_requested_service_tier =
                prepared_request_body.requested_service_tier.clone();
            let attempted_requested_image_intent = prepared_request_body.requested_image_intent;
            let attempted_requested_hosted_image_intent =
                prepared_request_body.requested_hosted_image_intent;
            let attempted_request_body_for_capture =
                prepared_request_body.request_body_for_capture.clone();
            let attempted_codex_imagegen_rewrite =
                prepared_request_body.codex_imagegen_rewrite.clone();
            let (forward_proxy_scope, selected_proxy, _client) =
                match select_pool_account_forward_proxy_client(state.as_ref(), &account).await {
                    Ok(selection) => selection,
                    Err(message) => {
                        if let Err(route_err) = record_pool_route_transport_failure_for_model(
                            &state.pool,
                            account.account_id,
                            sticky_key,
                            &message,
                            trace_context.as_ref().map(|trace| trace.invoke_id.as_str()),
                            requested_model.as_deref(),
                        )
                        .await
                        {
                            warn!(account_id = account.account_id, error = %route_err, "failed to record pool forward proxy selection failure");
                        }
                        store_pool_failover_error(
                            &mut last_error,
                            &mut preserve_sticky_owner_terminal_error,
                            PoolUpstreamError {
                                codex_imagegen_rewrite: attempted_codex_imagegen_rewrite.clone(),
                                account: Some(account.clone()),
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
                                request_body_for_capture: attempted_request_body_for_capture
                                    .clone(),
                            },
                            attempted_codex_imagegen_rewrite.as_ref(),
                        );
                        release_pool_routing_reservation(state.as_ref(), &reservation_key);
                        exhausted_accounts_all_rate_limited = false;
                        continue 'account_loop;
                    }
                };
            let (
                response,
                oauth_responses_debug,
                forward_proxy_selection,
                transport_bytes_live_counted,
                codex_imagegen_retest_claimed,
            ) = match &account.auth {
                PoolResolvedAuth::ApiKey { authorization } => {
                    attempt_count += 1;
                    attempt_index = attempt_count as i64;
                    attempt_started_at =
                        format_naive_precise(Utc::now().with_timezone(&Shanghai).naive_local());
                    let outbound_request_body = build_pool_upstream_request_body(
                        &prepared_request_body,
                        account.request_compression_algorithm,
                        request_compression_level_preset,
                        headers
                            .get(header::CONTENT_ENCODING)
                            .and_then(|value| value.to_str().ok()),
                    )
                    .await
                    .map_err(|err| {
                        release_pool_routing_reservation(state.as_ref(), &reservation_key);
                        PoolUpstreamError {
                            codex_imagegen_rewrite: attempted_codex_imagegen_rewrite.clone(),
                            account: Some(account.clone()),
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
                    let codex_imagegen_retest_claimed =
                        if codex_imagegen_audit_has_canonical_namespace(
                            attempted_codex_imagegen_rewrite.as_ref(),
                        ) {
                            match claim_codex_imagegen_supported_retest_override(
                                &state.pool,
                                account.account_id,
                            )
                            .await
                            {
                                Ok(CodexImagegenRetestClaim::Claimed) => true,
                                Ok(CodexImagegenRetestClaim::NotNeeded) => false,
                                Ok(CodexImagegenRetestClaim::AlreadyClaimed) => {
                                    release_pool_routing_reservation(
                                        state.as_ref(),
                                        &reservation_key,
                                    );
                                    continue 'account_loop;
                                }
                                Err(claim_err) => {
                                    warn!(
                                        account_id = account.account_id,
                                        error = %claim_err,
                                        "failed to atomically claim Codex imagegen supported retest override"
                                    );
                                    release_pool_routing_reservation(
                                        state.as_ref(),
                                        &reservation_key,
                                    );
                                    continue 'account_loop;
                                }
                            }
                        } else {
                            false
                        };
                    let forwarded_content_length = headers
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
                    for (name, value) in headers {
                        if *name == header::AUTHORIZATION || *name == header::CONTENT_ENCODING {
                            continue;
                        }
                        if *name == header::CONTENT_LENGTH && !preserve_content_length {
                            continue;
                        }
                        if should_forward_proxy_header(name, &request_connection_scoped) {
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
                    record_account_selected(state.as_ref(), account.account_id).await;
                    let group_name_snapshot =
                        normalize_pool_attempt_group_name(account.group_name.clone());
                    let proxy_binding_key_snapshot = canonical_pool_attempt_proxy_binding_key(
                        state.as_ref(),
                        selected_proxy.key.as_str(),
                    )
                    .await;
                    pending_attempt_record = if let Some(trace) = trace_context.as_ref() {
                        let mut attempt_trace = trace.clone();
                        if attempt_trace.request_model.is_none() {
                            attempt_trace.request_model = runtime_snapshot_context
                                .as_ref()
                                .and_then(|context| context.request_info.model.clone());
                        }
                        attempt_trace.upstream_base_url_host = account
                            .upstream_base_url
                            .host_str()
                            .and_then(normalize_upstream_base_url_host_value);
                        Some(
                            begin_pool_upstream_request_attempt_with_scope_and_routing_source_and_audit(
                                &state.pool,
                                &attempt_trace,
                                group_name_snapshot.as_deref(),
                                proxy_binding_key_snapshot.as_deref(),
                                Some(account.routing_source),
                                account.routing_selection_audit.as_ref(),
                                account.account_id,
                                upstream_route_key.as_str(),
                                attempt_index,
                                distinct_account_index,
                                same_account_retry_index,
                                attempt_started_at.as_str(),
                            )
                            .await,
                        )
                    } else {
                        None
                    };
                    if let Some(pending_attempt_record) = pending_attempt_record.as_mut()
                        && let Err(err) =
                            annotate_pool_upstream_request_attempt_request_compression(
                                &state.pool,
                                pending_attempt_record,
                                outbound_request_body.content_encoding.algorithm().as_str(),
                                outbound_request_body.compression_mode.as_str(),
                            )
                            .await
                    {
                        warn!(
                            invoke_id = %pending_attempt_record.invoke_id,
                            error = %err,
                            "failed to persist pool request compression metadata"
                        );
                    }
                    if let Some(pending_attempt_record) = pending_attempt_record.as_ref()
                        && let Err(err) =
                            annotate_pool_upstream_request_attempt_codex_imagegen_rewrite(
                                &state.pool,
                                pending_attempt_record,
                                attempted_codex_imagegen_rewrite.as_ref(),
                            )
                            .await
                    {
                        warn!(
                            invoke_id = %pending_attempt_record.invoke_id,
                            error = %err,
                            "failed to persist pool Codex imagegen rewrite audit"
                        );
                    }
                    let attempt_runtime_snapshot = runtime_snapshot_context.as_ref().map(|ctx| {
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
                            PoolLiveAttemptActivityLease::new(state.clone(), attempt_id)
                        });
                    if let (Some(trace), Some(runtime_snapshot)) =
                        (trace_context.as_ref(), attempt_runtime_snapshot.as_ref())
                    {
                        broadcast_pool_attempt_started_runtime_snapshot(
                            state.as_ref(),
                            trace,
                            runtime_snapshot,
                            &account,
                            attempt_count,
                            distinct_account_count,
                            Some(outbound_request_body.content_encoding.algorithm().as_str()),
                        )
                        .await;
                    }
                    early_phase_cleanup_guard = pending_attempt_record.as_ref().map(|pending| {
                        PoolEarlyPhaseOrphanCleanupGuard::new(state.clone(), pending.clone())
                    });
                    if let Some(pending_attempt_record) = pending_attempt_record.as_ref()
                        && let Err(err) = advance_pool_upstream_request_attempt_phase(
                            state.as_ref(),
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
                        invoke_id = trace_context
                            .as_ref()
                            .map(|trace| trace.invoke_id.as_str())
                            .unwrap_or(""),
                        account_id = account.account_id,
                        endpoint = original_uri.path(),
                        attempt_index = pending_attempt_record
                            .as_ref()
                            .map(|pending| pending.attempt_index),
                        upstream_attempt_started = true,
                        request_compression_algorithm =
                            outbound_request_body.content_encoding.as_str(),
                        request_compression_mode = outbound_request_body.compression_mode.as_str(),
                        prepared_body_bytes,
                        outbound_content_length,
                        "pool upstream attempt started"
                    );
                    let live_reporter = trace_context.as_ref().map(|trace| {
                        UpstreamTrafficReporter::new(
                            state.clone(),
                            trace.invoke_id.as_str(),
                            trace.occurred_at.as_str(),
                            Some(account.account_id),
                            account.upstream_base_url.host_str(),
                        )
                    });
                    match timeout(
                        attempt_send_timeout,
                        send_counted_upstream_http_request(
                            method.clone(),
                            api_key_target_url
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
                            }
                            record_pool_account_forward_proxy_result(
                                state.as_ref(),
                                &forward_proxy_scope,
                                &selected_proxy,
                                ForwardProxyRouteResultKind::NetworkFailure,
                            )
                            .await;
                            warn!(
                                invoke_id = trace_context
                                    .as_ref()
                                    .map(|trace| trace.invoke_id.as_str())
                                    .unwrap_or(""),
                                account_id = account.account_id,
                                endpoint = original_uri.path(),
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
                                direct_image_request && err.is_timeout();
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
                                original_uri,
                                None,
                                Some(message.as_str()),
                            );
                            let timeout_shaped_failure =
                                pool_failure_is_timeout_shaped(failure_kind, &message);
                            let should_timeout_route_failover =
                                uses_timeout_route_failover && timeout_shaped_failure;
                            let finished_at = shanghai_now_string();
                            if let Some(pending_attempt_record) = pending_attempt_record.as_ref()
                                && let Err(record_err) = finalize_pool_upstream_request_attempt(
                                    &state.pool,
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
                                    state.as_ref(),
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
                            let has_retry_budget = same_account_attempt + 1
                                < same_account_attempt_budget
                                && !direct_image_handshake_timeout
                                && !codex_imagegen_retest_claimed;
                            if has_retry_budget && !should_timeout_route_failover {
                                let retry_delay = fallback_proxy_429_retry_delay_for_state(
                                    state.as_ref(),
                                    u32::from(same_account_attempt) + 1,
                                );
                                info!(
                                    account_id = account.account_id,
                                    retry_index = same_account_attempt + 1,
                                    max_same_account_attempts = same_account_attempt_budget,
                                    retry_after_ms = retry_delay.as_millis(),
                                    "pool upstream transport failure; retrying same account"
                                );
                                disarm_pool_early_phase_cleanup_guard(
                                    &mut early_phase_cleanup_guard,
                                );
                                sleep(retry_delay).await;
                                continue;
                            }
                            if let Err(route_err) =
                                record_pool_route_transport_failure_for_attempt_with_kind(
                                    &state.pool,
                                    account.account_id,
                                    sticky_key,
                                    &message,
                                    failure_kind,
                                    trace_context.as_ref().map(|trace| trace.invoke_id.as_str()),
                                    pending_attempt_record
                                        .as_ref()
                                        .and_then(|pending| pending.attempt_id),
                                )
                                .await
                            {
                                warn!(account_id = account.account_id, error = %route_err, "failed to record pool transport failure");
                            }
                            store_pool_failover_error(
                                &mut last_error,
                                &mut preserve_sticky_owner_terminal_error,
                                PoolUpstreamError {
                                    codex_imagegen_rewrite: attempted_codex_imagegen_rewrite
                                        .clone(),
                                    account: Some(account.clone()),
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
                                    upstream_error_code: direct_image_handshake_timeout.then(
                                        || PROXY_FAILURE_UPSTREAM_HANDSHAKE_TIMEOUT.to_string(),
                                    ),
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
                                },
                                attempted_codex_imagegen_rewrite.as_ref(),
                            );
                            if direct_image_handshake_timeout {
                                let mut final_error = last_error.take().expect(
                                    "direct-image timeout stores a terminal upstream error",
                                );
                                final_error.attempt_summary = pool_attempt_summary(
                                    attempt_count,
                                    distinct_account_count,
                                    Some(PROXY_FAILURE_UPSTREAM_HANDSHAKE_TIMEOUT.to_string()),
                                );
                                disarm_pool_early_phase_cleanup_guard(
                                    &mut early_phase_cleanup_guard,
                                );
                                return Err(final_error);
                            }
                            exhausted_accounts_all_rate_limited = false;
                            if should_timeout_route_failover {
                                excluded_upstream_route_keys.insert(upstream_route_key.clone());
                                overload_required_upstream_route_key = None;
                                timeout_route_failover_pending = true;
                            }
                            disarm_pool_early_phase_cleanup_guard(&mut early_phase_cleanup_guard);
                            continue 'account_loop;
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
                            }
                            record_pool_account_forward_proxy_result(
                                state.as_ref(),
                                &forward_proxy_scope,
                                &selected_proxy,
                                ForwardProxyRouteResultKind::NetworkFailure,
                            )
                            .await;
                            warn!(
                                invoke_id = trace_context
                                    .as_ref()
                                    .map(|trace| trace.invoke_id.as_str())
                                    .unwrap_or(""),
                                account_id = account.account_id,
                                endpoint = original_uri.path(),
                                requested_service_tier =
                                    attempted_requested_service_tier.as_deref(),
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
                                capture_target_for_request(original_uri.path(), &method),
                                attempt_send_timeout,
                            );
                            let compact_support_observation = classify_compact_support_observation(
                                original_uri,
                                None,
                                Some(message.as_str()),
                            );
                            let should_timeout_route_failover = uses_timeout_route_failover;
                            let finished_at = shanghai_now_string();
                            if let Some(pending_attempt_record) = pending_attempt_record.as_ref()
                                && let Err(record_err) = finalize_pool_upstream_request_attempt(
                                    &state.pool,
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
                                    state.as_ref(),
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
                            if let (Some(total_timeout), Some(started_at)) =
                                (responses_total_timeout, attempt_total_timeout_started_at)
                                && pool_total_timeout_exhausted(total_timeout, started_at)
                            {
                                let final_error = build_pool_total_timeout_exhausted_error(
                                    total_timeout,
                                    Some(PoolUpstreamError {
                                        codex_imagegen_rewrite: attempted_codex_imagegen_rewrite
                                            .clone(),
                                        account: Some(account.clone()),
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
                                        proxy_binding_key_snapshot: proxy_binding_key_snapshot
                                            .clone(),
                                        oauth_responses_debug: None,
                                        attempt_summary: PoolAttemptSummary::default(),
                                        requested_service_tier: attempted_requested_service_tier
                                            .clone(),
                                        request_body_for_capture:
                                            attempted_request_body_for_capture.clone(),
                                    }),
                                    attempt_count,
                                    distinct_account_count,
                                );
                                if let Some(trace) = trace_context.as_ref()
                                    && let Err(err) = insert_pool_upstream_terminal_attempt(
                                        &state.pool,
                                        trace,
                                        &final_error,
                                        (attempt_count + 1) as i64,
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
                                disarm_pool_early_phase_cleanup_guard(
                                    &mut early_phase_cleanup_guard,
                                );
                                return Err(final_error);
                            }
                            let has_retry_budget = same_account_attempt + 1
                                < same_account_attempt_budget
                                && !direct_image_request
                                && !codex_imagegen_retest_claimed;
                            if has_retry_budget && !should_timeout_route_failover {
                                let retry_delay = fallback_proxy_429_retry_delay_for_state(
                                    state.as_ref(),
                                    u32::from(same_account_attempt) + 1,
                                );
                                info!(
                                    account_id = account.account_id,
                                    retry_index = same_account_attempt + 1,
                                    max_same_account_attempts = same_account_attempt_budget,
                                    retry_after_ms = retry_delay.as_millis(),
                                    "pool upstream handshake timeout; retrying same account"
                                );
                                disarm_pool_early_phase_cleanup_guard(
                                    &mut early_phase_cleanup_guard,
                                );
                                sleep(retry_delay).await;
                                continue;
                            }
                            if let Err(route_err) =
                                record_pool_route_transport_failure_for_attempt_with_kind(
                                    &state.pool,
                                    account.account_id,
                                    sticky_key,
                                    &message,
                                    PROXY_FAILURE_UPSTREAM_HANDSHAKE_TIMEOUT,
                                    trace_context.as_ref().map(|trace| trace.invoke_id.as_str()),
                                    pending_attempt_record
                                        .as_ref()
                                        .and_then(|pending| pending.attempt_id),
                                )
                                .await
                            {
                                warn!(account_id = account.account_id, error = %route_err, "failed to record pool handshake timeout");
                            }
                            store_pool_failover_error(
                                &mut last_error,
                                &mut preserve_sticky_owner_terminal_error,
                                PoolUpstreamError {
                                    codex_imagegen_rewrite: attempted_codex_imagegen_rewrite
                                        .clone(),
                                    account: Some(account.clone()),
                                    status: if direct_image_request {
                                        StatusCode::GATEWAY_TIMEOUT
                                    } else {
                                        StatusCode::BAD_GATEWAY
                                    },
                                    message: if direct_image_request {
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
                                    upstream_error_code: direct_image_request.then(|| {
                                        PROXY_FAILURE_UPSTREAM_HANDSHAKE_TIMEOUT.to_string()
                                    }),
                                    upstream_error_message: None,
                                    downstream_error_message: None,
                                    upstream_request_id: None,
                                    proxy_binding_key_snapshot: None,
                                    oauth_responses_debug: None,
                                    attempt_summary: PoolAttemptSummary::default(),
                                    requested_service_tier: attempted_requested_service_tier
                                        .clone(),
                                    request_body_for_capture: attempted_request_body_for_capture
                                        .clone(),
                                },
                                attempted_codex_imagegen_rewrite.as_ref(),
                            );
                            if direct_image_request {
                                let mut final_error = last_error.take().expect(
                                    "direct-image timeout stores a terminal upstream error",
                                );
                                final_error.attempt_summary = pool_attempt_summary(
                                    attempt_count,
                                    distinct_account_count,
                                    Some(PROXY_FAILURE_UPSTREAM_HANDSHAKE_TIMEOUT.to_string()),
                                );
                                disarm_pool_early_phase_cleanup_guard(
                                    &mut early_phase_cleanup_guard,
                                );
                                return Err(final_error);
                            }
                            exhausted_accounts_all_rate_limited = false;
                            if should_timeout_route_failover {
                                excluded_upstream_route_keys.insert(upstream_route_key.clone());
                                overload_required_upstream_route_key = None;
                                timeout_route_failover_pending = true;
                            }
                            disarm_pool_early_phase_cleanup_guard(&mut early_phase_cleanup_guard);
                            continue 'account_loop;
                        }
                    }
                }
                PoolResolvedAuth::Oauth {
                    access_token,
                    chatgpt_account_id,
                } => {
                    let proxy_binding_key_snapshot = canonical_pool_attempt_proxy_binding_key(
                        state.as_ref(),
                        selected_proxy.key.as_str(),
                    )
                    .await;
                    let oauth_body = match &prepared_request_body.snapshot {
                        snapshot @ (PoolReplayBodySnapshot::Empty
                        | PoolReplayBodySnapshot::Memory(_))
                            if original_uri.path() == "/v1/responses" =>
                        {
                            oauth_bridge::CountedOauthUpstreamRequestBody::Bytes(
                                snapshot.to_bytes().await.map_err(|err| PoolUpstreamError {
        codex_imagegen_rewrite: attempted_codex_imagegen_rewrite.clone(),
                                    account: Some(account.clone()),
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
                                        attempt_count,
                                        distinct_account_count,
                                        Some(PROXY_FAILURE_FAILED_CONTACT_UPSTREAM.to_string()),
                                    ),
                                    requested_service_tier:
                                        attempted_requested_service_tier.clone(),
                                    request_body_for_capture:
                                        attempted_request_body_for_capture.clone(),
                                })?,
                            )
                        }
                        snapshot @ PoolReplayBodySnapshot::File { size, .. }
                            if original_uri.path() == "/v1/responses"
                                && *size <= OAUTH_RESPONSES_MAX_REWRITE_BODY_BYTES
                                && headers
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
                                    account: Some(account.clone()),
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
                                        attempt_count,
                                        distinct_account_count,
                                        Some(PROXY_FAILURE_FAILED_CONTACT_UPSTREAM.to_string()),
                                    ),
                                    requested_service_tier:
                                        attempted_requested_service_tier.clone(),
                                    request_body_for_capture:
                                        attempted_request_body_for_capture.clone(),
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
        codex_imagegen_rewrite: attempted_codex_imagegen_rewrite.clone(),
                                        account: Some(account.clone()),
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
                                        proxy_binding_key_snapshot: proxy_binding_key_snapshot
                                            .clone(),
                                        oauth_responses_debug: None,
                                        attempt_summary: pool_attempt_summary(
                                            attempt_count,
                                            distinct_account_count,
                                            Some(PROXY_FAILURE_FAILED_CONTACT_UPSTREAM.to_string()),
                                        ),
                                        requested_service_tier:
                                            attempted_requested_service_tier.clone(),
                                        request_body_for_capture:
                                        attempted_request_body_for_capture.clone(),
                                    })?,
                            ),
                            request_is_stream: if original_uri.path() == "/v1/responses" {
                                snapshot
                                    .extract_request_stream_flag(
                                        headers
                                            .get(header::CONTENT_ENCODING)
                                            .and_then(|value| value.to_str().ok()),
                                    )
                                    .await
                            } else {
                                None
                            },
                            request_is_stream_rx: None,
                            snapshot_kind: if original_uri.path() == "/v1/responses" {
                                Some(pool_request_snapshot_kind(snapshot))
                            } else {
                                None
                            },
                            live_rewrite_pending: false,
                            body: snapshot.to_http_body(),
                        },
                    };
                    let codex_imagegen_retest_claimed =
                        if codex_imagegen_audit_has_canonical_namespace(
                            attempted_codex_imagegen_rewrite.as_ref(),
                        ) {
                            match claim_codex_imagegen_supported_retest_override(
                                &state.pool,
                                account.account_id,
                            )
                            .await
                            {
                                Ok(CodexImagegenRetestClaim::Claimed) => true,
                                Ok(CodexImagegenRetestClaim::NotNeeded) => false,
                                Ok(CodexImagegenRetestClaim::AlreadyClaimed) => {
                                    release_pool_routing_reservation(
                                        state.as_ref(),
                                        &reservation_key,
                                    );
                                    continue 'account_loop;
                                }
                                Err(claim_err) => {
                                    warn!(
                                        account_id = account.account_id,
                                        error = %claim_err,
                                        "failed to atomically claim Codex imagegen supported retest override"
                                    );
                                    release_pool_routing_reservation(
                                        state.as_ref(),
                                        &reservation_key,
                                    );
                                    continue 'account_loop;
                                }
                            }
                        } else {
                            false
                        };
                    attempt_count += 1;
                    attempt_index = attempt_count as i64;
                    attempt_started_at =
                        format_naive_precise(Utc::now().with_timezone(&Shanghai).naive_local());
                    record_account_selected(state.as_ref(), account.account_id).await;
                    let group_name_snapshot =
                        normalize_pool_attempt_group_name(account.group_name.clone());
                    pending_attempt_record = if let Some(trace) = trace_context.as_ref() {
                        let mut attempt_trace = trace.clone();
                        if attempt_trace.request_model.is_none() {
                            attempt_trace.request_model = runtime_snapshot_context
                                .as_ref()
                                .and_then(|context| context.request_info.model.clone());
                        }
                        attempt_trace.upstream_base_url_host = account
                            .upstream_base_url
                            .host_str()
                            .and_then(normalize_upstream_base_url_host_value);
                        Some(
                            begin_pool_upstream_request_attempt_with_scope_and_routing_source_and_audit(
                                &state.pool,
                                &attempt_trace,
                                group_name_snapshot.as_deref(),
                                proxy_binding_key_snapshot.as_deref(),
                                Some(account.routing_source),
                                account.routing_selection_audit.as_ref(),
                                account.account_id,
                                upstream_route_key.as_str(),
                                attempt_index,
                                distinct_account_index,
                                same_account_retry_index,
                                attempt_started_at.as_str(),
                            )
                            .await,
                        )
                    } else {
                        None
                    };
                    let forwarded_request_compression = (method != Method::GET)
                        .then(|| forwarded_request_compression_observation(headers))
                        .flatten();
                    if let (Some(pending_attempt_record), Some((algorithm, mode))) = (
                        pending_attempt_record.as_mut(),
                        forwarded_request_compression,
                    ) && let Err(err) =
                        annotate_pool_upstream_request_attempt_request_compression(
                            &state.pool,
                            pending_attempt_record,
                            algorithm,
                            mode,
                        )
                        .await
                    {
                        warn!(
                            invoke_id = %pending_attempt_record.invoke_id,
                            error = %err,
                            "failed to persist pool OAuth request compression metadata"
                        );
                    }
                    if let Some(pending_attempt_record) = pending_attempt_record.as_ref()
                        && let Err(err) =
                            annotate_pool_upstream_request_attempt_codex_imagegen_rewrite(
                                &state.pool,
                                pending_attempt_record,
                                attempted_codex_imagegen_rewrite.as_ref(),
                            )
                            .await
                    {
                        warn!(
                            invoke_id = %pending_attempt_record.invoke_id,
                            error = %err,
                            "failed to persist pool Codex imagegen rewrite audit"
                        );
                    }
                    let attempt_runtime_snapshot = runtime_snapshot_context.as_ref().map(|ctx| {
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
                            PoolLiveAttemptActivityLease::new(state.clone(), attempt_id)
                        });
                    if let (Some(trace), Some(runtime_snapshot)) =
                        (trace_context.as_ref(), attempt_runtime_snapshot.as_ref())
                    {
                        broadcast_pool_attempt_started_runtime_snapshot(
                            state.as_ref(),
                            trace,
                            runtime_snapshot,
                            &account,
                            attempt_count,
                            distinct_account_count,
                            forwarded_request_compression.map(|(algorithm, _)| algorithm),
                        )
                        .await;
                    }
                    early_phase_cleanup_guard = pending_attempt_record.as_ref().map(|pending| {
                        PoolEarlyPhaseOrphanCleanupGuard::new(state.clone(), pending.clone())
                    });
                    if let Some(pending_attempt_record) = pending_attempt_record.as_ref()
                        && let Err(err) = advance_pool_upstream_request_attempt_phase(
                            state.as_ref(),
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
                        let live_reporter = trace_context.as_ref().map(|trace| {
                            UpstreamTrafficReporter::new(
                                state.clone(),
                                trace.invoke_id.as_str(),
                                trace.occurred_at.as_str(),
                                Some(account.account_id),
                                account.upstream_base_url.host_str(),
                            )
                        });
                        let oauth_response = oauth_bridge::send_counted_oauth_upstream_request(
                            method.clone(),
                            original_uri,
                            headers,
                            oauth_body,
                            attempt_send_timeout,
                            attempt_pre_first_byte_timeout,
                            Some(account.account_id),
                            access_token,
                            chatgpt_account_id.as_deref(),
                            Some(&state.oauth_installation_seed),
                            state.upstream_accounts.crypto_key.as_ref(),
                            selected_proxy.endpoint_url.as_ref(),
                            live_reporter,
                        )
                        .await;
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
                let has_retry_budget = same_account_attempt + 1 < same_account_attempt_budget;
                let has_upstream_413_retry_budget =
                    status == StatusCode::PAYLOAD_TOO_LARGE && !retried_upstream_413_for_account;
                let has_group_upstream_429_retry_budget = status == StatusCode::TOO_MANY_REQUESTS
                    && group_upstream_429_retry_count < group_upstream_429_max_retries;
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
                    && let Some((forward_proxy_scope, selected_proxy)) =
                        forward_proxy_selection.as_ref()
                {
                    record_pool_account_forward_proxy_result(
                        state.as_ref(),
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
                        let (
                            upstream_error_code,
                            upstream_error_message,
                            upstream_request_id,
                            message,
                        ) = summarize_pool_upstream_http_failure(
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
                let requested_model_for_failure = requested_model.as_deref().or_else(|| {
                    trace_context
                        .as_ref()
                        .and_then(|trace| trace.request_model.as_deref())
                });
                let explicit_model_failure = requested_model_for_failure.map_or_else(
                    || is_explicit_model_failure(status, Some(&route_error_message)),
                    |model| {
                        is_explicit_model_failure_for_model(
                            status,
                            Some(&route_error_message),
                            Some(model),
                        )
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
                                state.as_ref(),
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
                        // The caller prepends `first_chunk` to this response body. Keep the
                        // body empty here so buffered 4xx payloads are not emitted twice.
                        .body(Body::empty())
                        .map_err(|err| PoolUpstreamError {
                            codex_imagegen_rewrite: attempted_codex_imagegen_rewrite.clone(),
                            account: Some(account.clone()),
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

                    let mut deferred_early_phase_cleanup_guard = None;
                    if let Some(pending_attempt_record) = pending_attempt_record.as_ref() {
                        if pending_attempt_record.attempt_id.is_none() {
                            deferred_early_phase_cleanup_guard = early_phase_cleanup_guard.take();
                        }
                        let phase_enqueued = enqueue_pool_upstream_request_attempt_progress(
                            state.as_ref(),
                            pending_attempt_record,
                            POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_STREAMING_RESPONSE,
                            Some(connect_latency_ms),
                            Some(first_byte_latency_ms),
                            None,
                            None,
                        );
                        if phase_enqueued {
                            debug!(
                                invoke_id = %pending_attempt_record.invoke_id,
                                attempt_id = pending_attempt_record.attempt_id,
                                "queued pool attempt streaming phase progress"
                            );
                            if pending_attempt_record.attempt_id.is_some() {
                                deferred_early_phase_cleanup_guard =
                                    early_phase_cleanup_guard.take();
                            }
                        } else {
                            info!(
                                invoke_id = %pending_attempt_record.invoke_id,
                                attempt_id = pending_attempt_record.attempt_id,
                                "streaming phase was not enqueued; relying on invocation cleanup guards for post-first-byte recovery"
                            );
                            if pending_attempt_record.attempt_id.is_some() {
                                deferred_early_phase_cleanup_guard =
                                    early_phase_cleanup_guard.take();
                            }
                        }
                    } else {
                        disarm_pool_early_phase_cleanup_guard(&mut early_phase_cleanup_guard);
                    }

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
                    let capability_requirements =
                        RequestCapabilityRequirements::from_endpoint_and_image_intent_for_method(
                            original_uri.path(),
                            attempted_requested_hosted_image_intent,
                            method == Method::POST,
                        );
                    if capability_requirements.response_endpoint
                        && classify_response_endpoint_capability_observation(
                            status,
                            Some(route_error_message.as_str()),
                        ) == CapabilitySupport::Unsupported
                        && let Err(observation_err) = record_capability_observation(
                            &state.pool,
                            account.account_id,
                            UpstreamCapabilityAxis::ResponseEndpoint,
                            CapabilitySupport::Unsupported,
                            Some(route_error_message.as_str()),
                        )
                        .await
                    {
                        warn!(
                            account_id = account.account_id,
                            error = %observation_err,
                            "failed to record response endpoint capability observation"
                        );
                    }
                    if capability_requirements.chat_completions_endpoint
                        && classify_chat_completions_capability_observation(
                            status,
                            Some(route_error_message.as_str()),
                        ) == CapabilitySupport::Unsupported
                        && let Err(observation_err) = record_capability_observation(
                            &state.pool,
                            account.account_id,
                            UpstreamCapabilityAxis::ChatCompletionsEndpoint,
                            CapabilitySupport::Unsupported,
                            Some(route_error_message.as_str()),
                        )
                        .await
                    {
                        warn!(
                            account_id = account.account_id,
                            error = %observation_err,
                            "failed to record chat completions capability observation"
                        );
                    }
                    if capability_requirements.image_endpoint
                        && classify_image_endpoint_capability_observation(
                            status,
                            Some(route_error_message.as_str()),
                        ) == CapabilitySupport::Unsupported
                        && let Err(observation_err) = record_capability_observation(
                            &state.pool,
                            account.account_id,
                            UpstreamCapabilityAxis::ImageEndpoint,
                            CapabilitySupport::Unsupported,
                            Some(route_error_message.as_str()),
                        )
                        .await
                    {
                        warn!(
                            account_id = account.account_id,
                            error = %observation_err,
                            "failed to record image endpoint capability observation"
                        );
                    }
                    if capability_requirements.response_image_tool
                        && classify_response_image_tool_capability_observation(
                            status,
                            Some(route_error_message.as_str()),
                        ) == CapabilitySupport::Unsupported
                        && let Err(observation_err) = record_capability_observation(
                            &state.pool,
                            account.account_id,
                            UpstreamCapabilityAxis::ResponseImageTool,
                            CapabilitySupport::Unsupported,
                            Some(route_error_message.as_str()),
                        )
                        .await
                    {
                        warn!(
                            account_id = account.account_id,
                            error = %observation_err,
                            "failed to record response image-tool capability observation"
                        );
                    }
                    if capability_requirements.standalone_search
                        && classify_standalone_search_capability_observation(
                            status,
                            Some(route_error_message.as_str()),
                        ) == CapabilitySupport::Unsupported
                        && let Err(observation_err) = record_capability_observation(
                            &state.pool,
                            account.account_id,
                            UpstreamCapabilityAxis::StandaloneSearch,
                            CapabilitySupport::Unsupported,
                            Some(route_error_message.as_str()),
                        )
                        .await
                    {
                        warn!(
                            account_id = account.account_id,
                            error = %observation_err,
                            "failed to record standalone search capability observation"
                        );
                    }

                    if let Some((forward_proxy_scope, selected_proxy)) =
                        forward_proxy_selection.as_ref()
                    {
                        record_pool_account_forward_proxy_result(
                            state.as_ref(),
                            forward_proxy_scope,
                            selected_proxy,
                            ForwardProxyRouteResultKind::CompletedRequest,
                        )
                        .await;
                    }
                    reservation_guard.disarm();
                    return Ok(PoolUpstreamResponse {
                        account: account.clone(),
                        response: ProxyUpstreamResponseBody::Axum(response),
                        transport_bytes_live_counted,
                        stream_timeout,
                        oauth_responses_debug,
                        connect_latency_ms,
                        attempt_started_at_utc,
                        first_byte_latency_ms,
                        live_request_body_first_byte_at: None,
                        first_chunk,
                        first_chunk_received_at: None,
                        first_stream_chunk_received_at: None,
                        pending_attempt_record: pending_attempt_record.map(|mut pending| {
                            pending.connect_latency_ms = connect_latency_ms;
                            pending.first_byte_latency_ms = first_byte_latency_ms;
                            pending.compact_support_status = compact_support_observation
                                .as_ref()
                                .map(|value| value.status.to_string());
                            pending.compact_support_reason = compact_support_observation
                                .as_ref()
                                .and_then(|value| value.reason.clone());
                            pending
                        }),
                        deferred_early_phase_cleanup_guard,
                        live_attempt_activity_lease,
                        attempt_summary: pool_attempt_summary(
                            attempt_count,
                            distinct_account_count,
                            None,
                        ),
                        requested_service_tier: attempted_requested_service_tier,
                        request_body_for_capture: attempted_request_body_for_capture,
                        codex_imagegen_rewrite: attempted_codex_imagegen_rewrite.clone(),
                    });
                }
                let http_failure_classification =
                    classify_pool_account_http_failure(&account.kind, status, &route_error_message);
                let failure_kind = oauth_transport_failure_kind
                    .unwrap_or(http_failure_classification.failure_kind);
                let normalized_failure = normalize_pool_upstream_failure_record(
                    status,
                    oauth_transport_failure_kind,
                    &message,
                    upstream_error_message.as_deref(),
                );
                let canonical_override = (normalized_failure.canonical_error_message != message)
                    .then(|| normalized_failure.canonical_error_message.clone());
                let compact_support_observation = classify_compact_support_observation(
                    original_uri,
                    Some(status),
                    Some(route_error_message.as_str()),
                );
                let compact_support_is_unsupported = compact_support_observation
                    .as_ref()
                    .is_some_and(|value| value.status == COMPACT_SUPPORT_STATUS_UNSUPPORTED);
                let timeout_shaped_failure = status.is_server_error()
                    && pool_failure_is_timeout_shaped(failure_kind, &message);
                let should_timeout_route_failover =
                    uses_timeout_route_failover && timeout_shaped_failure;
                let direct_image_handshake_timeout = direct_image_request
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
                                state.as_ref(),
                                u32::from(same_account_attempt) + 1,
                            )
                        })
                });
                let finished_at = shanghai_now_string();
                if let Some(pending_attempt_record) = pending_attempt_record.as_ref()
                    && let Err(record_err) = finalize_pool_upstream_request_attempt(
                        &state.pool,
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
                    && let Err(observation_err) = record_capability_observation(
                        &state.pool,
                        account.account_id,
                        UpstreamCapabilityAxis::CodexImagegen,
                        CapabilitySupport::Unsupported,
                        Some(route_error_message.as_str()),
                    )
                    .await
                {
                    warn!(
                        account_id = account.account_id,
                        error = %observation_err,
                        "failed to record Codex imagegen capability observation"
                    );
                }
                if let Some(response_body) = error_body_bytes.as_ref()
                    && let Some(pending_attempt_record) = pending_attempt_record.as_ref()
                {
                    let response_body_logging_enabled = state
                        .proxy_model_settings
                        .read()
                        .await
                        .response_body_logging_enabled;
                    let response_content_encoding = response_headers
                        .get(header::CONTENT_ENCODING)
                        .and_then(|value| value.to_str().ok());
                    spawn_pool_attempt_response_capture(
                        state.clone(),
                        pending_attempt_record.clone(),
                        response_body.clone(),
                        response_body_logging_enabled,
                        response_content_encoding.map(str::to_string),
                    );
                }
                if let Some(pending_attempt_record) = pending_attempt_record.as_ref()
                    && let Err(err) = broadcast_pool_upstream_attempts_snapshot(
                        state.as_ref(),
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
                    let retry_delay = pool_group_upstream_429_retry_delay(state.as_ref());
                    let group_retry_index = group_upstream_429_retry_count + 1;
                    info!(
                        account_id = account.account_id,
                        status = status.as_u16(),
                        retry_index = same_account_attempt + 1,
                        group_retry_index,
                        max_same_account_attempts = same_account_attempt_loop_budget,
                        group_upstream_429_max_retries,
                        retry_after_ms = retry_delay.as_millis(),
                        "pool upstream responded with group retryable 429; retrying same account"
                    );
                    group_upstream_429_retry_count += 1;
                    disarm_pool_early_phase_cleanup_guard(&mut early_phase_cleanup_guard);
                    sleep(retry_delay).await;
                    continue;
                }
                if has_upstream_413_retry_budget && !codex_imagegen_retest_claimed {
                    let retry_delay = fallback_proxy_429_retry_delay_for_state(
                        state.as_ref(),
                        u32::from(same_account_attempt) + 1,
                    );
                    info!(
                        account_id = account.account_id,
                        status = status.as_u16(),
                        retry_index = same_account_attempt + 1,
                        max_same_account_attempts = 2,
                        retry_after_ms = retry_delay.as_millis(),
                        "pool upstream responded with 413; retrying same account once"
                    );
                    retried_upstream_413_for_account = true;
                    disarm_pool_early_phase_cleanup_guard(&mut early_phase_cleanup_guard);
                    sleep(retry_delay).await;
                    continue;
                }
                if let Some(retry_delay) = retry_delay
                    && !codex_imagegen_retest_claimed
                {
                    info!(
                        account_id = account.account_id,
                        status = status.as_u16(),
                        retry_index = same_account_attempt + 1,
                        max_same_account_attempts = same_account_attempt_budget,
                        retry_after_ms = retry_delay.as_millis(),
                        "pool upstream responded with retryable status; retrying same account"
                    );
                    disarm_pool_early_phase_cleanup_guard(&mut early_phase_cleanup_guard);
                    sleep(retry_delay).await;
                    continue;
                }
                let route_failure_result = if codex_imagegen_upstream_incompatible {
                    Ok(())
                } else if oauth_transport_failure_kind.is_some() {
                    record_pool_route_transport_failure_for_attempt(
                        &state.pool,
                        account.account_id,
                        sticky_key,
                        normalized_failure.canonical_error_message.as_str(),
                        trace_context.as_ref().map(|trace| trace.invoke_id.as_str()),
                        pending_attempt_record
                            .as_ref()
                            .and_then(|pending| pending.attempt_id),
                    )
                    .await
                } else {
                    record_pool_route_http_failure_for_endpoint_with_image_intent_and_prompt_cache_key_for_attempt_and_broadcast(
                        state.as_ref(),
                        account.account_id,
                        &account.kind,
                        account.single_account_rotation_enabled,
                        sticky_key,
                        status,
                        &route_error_message,
                        trace_context.as_ref().map(|trace| trace.invoke_id.as_str()),
                        capability_endpoint,
                        attempted_requested_hosted_image_intent,
                        pending_attempt_record
                            .as_ref()
                            .and_then(|pending| pending.attempt_id),
                        account.sticky_affinity_generation,
                        prompt_cache_key.or(sticky_event_prompt_cache_key),
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
                store_pool_failover_error(
                    &mut last_error,
                    &mut preserve_sticky_owner_terminal_error,
                    PoolUpstreamError {
                        codex_imagegen_rewrite: attempted_codex_imagegen_rewrite.clone(),
                        account: Some(account.clone()),
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
                                state.as_ref(),
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
                if direct_image_request && failure_kind == PROXY_FAILURE_UPSTREAM_HANDSHAKE_TIMEOUT
                {
                    let mut final_error = last_error
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
                        attempt_count,
                        distinct_account_count,
                        Some(PROXY_FAILURE_UPSTREAM_HANDSHAKE_TIMEOUT.to_string()),
                    );
                    disarm_pool_early_phase_cleanup_guard(&mut early_phase_cleanup_guard);
                    return Err(final_error);
                }
                exhausted_accounts_all_rate_limited &= status == StatusCode::TOO_MANY_REQUESTS;
                if should_timeout_route_failover {
                    excluded_upstream_route_keys.insert(upstream_route_key.clone());
                    overload_required_upstream_route_key = None;
                    timeout_route_failover_pending = true;
                }
                disarm_pool_early_phase_cleanup_guard(&mut early_phase_cleanup_guard);
                continue 'account_loop;
            }

            if let Some(pending_attempt_record) = pending_attempt_record.as_ref()
                && let Err(err) = advance_pool_upstream_request_attempt_phase(
                    state.as_ref(),
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
                                state.as_ref(),
                                forward_proxy_scope,
                                selected_proxy,
                                ForwardProxyRouteResultKind::NetworkFailure,
                            )
                            .await;
                        }
                        let message = format!("upstream stream error before first chunk: {err}");
                        let compact_support_observation = classify_compact_support_observation(
                            original_uri,
                            None,
                            Some(message.as_str()),
                        );
                        let timeout_shaped_failure = pool_failure_is_timeout_shaped(
                            PROXY_FAILURE_UPSTREAM_STREAM_ERROR,
                            &message,
                        );
                        let should_timeout_route_failover =
                            uses_timeout_route_failover && timeout_shaped_failure;
                        let finished_at = shanghai_now_string();
                        if let Some(pending_attempt_record) = pending_attempt_record.as_ref()
                            && let Err(record_err) = finalize_pool_upstream_request_attempt(
                                &state.pool,
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
                                state.as_ref(),
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
                        if let (Some(total_timeout), Some(started_at)) =
                            (responses_total_timeout, attempt_total_timeout_started_at)
                            && pool_total_timeout_exhausted(total_timeout, started_at)
                        {
                            let final_error = build_pool_total_timeout_exhausted_error(
                                total_timeout,
                                Some(PoolUpstreamError {
                                    codex_imagegen_rewrite: attempted_codex_imagegen_rewrite
                                        .clone(),
                                    account: Some(account.clone()),
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
                                            state.as_ref(),
                                            selected_proxy.key.as_str(),
                                        )
                                        .await
                                    } else {
                                        None
                                    },
                                    oauth_responses_debug: oauth_responses_debug.clone(),
                                    attempt_summary: PoolAttemptSummary::default(),
                                    requested_service_tier: attempted_requested_service_tier
                                        .clone(),
                                    request_body_for_capture: attempted_request_body_for_capture
                                        .clone(),
                                }),
                                attempt_count,
                                distinct_account_count,
                            );
                            if let Some(trace) = trace_context.as_ref()
                                && let Err(err) = insert_pool_upstream_terminal_attempt(
                                    &state.pool,
                                    trace,
                                    &final_error,
                                    (attempt_count + 1) as i64,
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
                        let has_retry_budget =
                            same_account_attempt + 1 < same_account_attempt_budget;
                        if has_retry_budget && !should_timeout_route_failover {
                            let retry_delay = fallback_proxy_429_retry_delay_for_state(
                                state.as_ref(),
                                u32::from(same_account_attempt) + 1,
                            );
                            info!(
                                account_id = account.account_id,
                                retry_index = same_account_attempt + 1,
                                max_same_account_attempts = same_account_attempt_budget,
                                retry_after_ms = retry_delay.as_millis(),
                                "pool upstream first chunk failed; retrying same account"
                            );
                            disarm_pool_early_phase_cleanup_guard(&mut early_phase_cleanup_guard);
                            sleep(retry_delay).await;
                            continue;
                        }
                        if let Err(route_err) =
                            record_pool_route_transport_failure_for_attempt_with_kind(
                                &state.pool,
                                account.account_id,
                                sticky_key,
                                &message,
                                PROXY_FAILURE_UPSTREAM_STREAM_ERROR,
                                trace_context.as_ref().map(|trace| trace.invoke_id.as_str()),
                                pending_attempt_record
                                    .as_ref()
                                    .and_then(|pending| pending.attempt_id),
                            )
                            .await
                        {
                            warn!(account_id = account.account_id, error = %route_err, "failed to record pool first chunk failure");
                        }
                        store_pool_failover_error(
                            &mut last_error,
                            &mut preserve_sticky_owner_terminal_error,
                            PoolUpstreamError {
                                codex_imagegen_rewrite: attempted_codex_imagegen_rewrite.clone(),
                                account: Some(account.clone()),
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
                                        state.as_ref(),
                                        selected_proxy.key.as_str(),
                                    )
                                    .await
                                } else {
                                    None
                                },
                                oauth_responses_debug: oauth_responses_debug.clone(),
                                attempt_summary: PoolAttemptSummary::default(),
                                requested_service_tier: attempted_requested_service_tier.clone(),
                                request_body_for_capture: attempted_request_body_for_capture
                                    .clone(),
                            },
                            attempted_codex_imagegen_rewrite.as_ref(),
                        );
                        exhausted_accounts_all_rate_limited = false;
                        if should_timeout_route_failover {
                            excluded_upstream_route_keys.insert(upstream_route_key.clone());
                            overload_required_upstream_route_key = None;
                            timeout_route_failover_pending = true;
                        }
                        disarm_pool_early_phase_cleanup_guard(&mut early_phase_cleanup_guard);
                        continue 'account_loop;
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
            let initial_gate_outcome = if original_uri.path() == "/v1/responses"
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
            } else if original_uri.path() == "/v1/responses/compact" && status == StatusCode::OK {
                Ok(gate_pool_initial_compact_response(
                    status,
                    response.headers(),
                    first_chunk.as_ref(),
                )
                .unwrap_or(PoolInitialResponseGateOutcome::Forward {
                    response,
                    prefetched_bytes: first_chunk,
                    prefetched_bytes_received_at: first_chunk_received_at,
                    replayed_bytes_received_at: None,
                }))
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
                        message,
                        upstream_error_code,
                        upstream_error_message,
                        upstream_request_id,
                        raw_body,
                    }) => {
                        let finished_at = shanghai_now_string();
                        if let Some(pending_attempt_record) = pending_attempt_record.as_ref()
                            && let Err(record_err) = finalize_pool_upstream_request_attempt(
                                &state.pool,
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
                        if let Some(pending_attempt_record) = pending_attempt_record.as_ref() {
                            let response_body_logging_enabled = state
                                .proxy_model_settings
                                .read()
                                .await
                                .response_body_logging_enabled;
                            spawn_pool_attempt_response_capture(
                                state.clone(),
                                pending_attempt_record.clone(),
                                raw_body.clone(),
                                response_body_logging_enabled,
                                response_content_encoding_for_attempt.clone(),
                            );
                        }
                        if let Some(pending_attempt_record) = pending_attempt_record.as_ref()
                            && let Err(err) = broadcast_pool_upstream_attempts_snapshot(
                                state.as_ref(),
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

                        let has_retry_budget =
                            same_account_attempt + 1 < overload_same_account_attempt_budget;
                        if has_retry_budget {
                            let retry_delay = fallback_proxy_429_retry_delay_for_state(
                                state.as_ref(),
                                u32::from(same_account_attempt) + 1,
                            );
                            info!(
                                account_id = account.account_id,
                                retry_index = same_account_attempt + 1,
                                max_same_account_attempts = overload_same_account_attempt_budget,
                                retry_after_ms = retry_delay.as_millis(),
                                "pool upstream reported retryable response.failed before forwarding; retrying same account"
                            );
                            disarm_pool_early_phase_cleanup_guard(&mut early_phase_cleanup_guard);
                            sleep(retry_delay).await;
                            continue;
                        }

                        if let Err(route_err) =
                            record_pool_route_retryable_overload_failure_for_attempt(
                                &state.pool,
                                account.account_id,
                                sticky_key,
                                &message,
                                trace_context.as_ref().map(|trace| trace.invoke_id.as_str()),
                                pending_attempt_record
                                    .as_ref()
                                    .and_then(|pending| pending.attempt_id),
                            )
                            .await
                        {
                            warn!(account_id = account.account_id, error = %route_err, "failed to record retryable response.failed route state");
                        }
                        store_pool_failover_error(
                            &mut last_error,
                            &mut preserve_sticky_owner_terminal_error,
                            PoolUpstreamError {
                                codex_imagegen_rewrite: attempted_codex_imagegen_rewrite.clone(),
                                account: Some(account.clone()),
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
                                        state.as_ref(),
                                        selected_proxy.key.as_str(),
                                    )
                                    .await
                                } else {
                                    None
                                },
                                oauth_responses_debug: oauth_responses_debug.clone(),
                                attempt_summary: PoolAttemptSummary::default(),
                                requested_service_tier: attempted_requested_service_tier.clone(),
                                request_body_for_capture: attempted_request_body_for_capture
                                    .clone(),
                            },
                            attempted_codex_imagegen_rewrite.as_ref(),
                        );
                        exhausted_accounts_all_rate_limited = false;
                        overload_required_upstream_route_key = Some(upstream_route_key.clone());
                        disarm_pool_early_phase_cleanup_guard(&mut early_phase_cleanup_guard);
                        continue 'account_loop;
                    }
                    Err(err) => {
                        let message = format!("failed to gate initial upstream response: {err}");
                        let finished_at = shanghai_now_string();
                        if let Some(pending_attempt_record) = pending_attempt_record.as_ref()
                            && let Err(record_err) = finalize_pool_upstream_request_attempt(
                                &state.pool,
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
                                state.as_ref(),
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
                        if let Err(route_err) =
                            record_pool_route_transport_failure_for_attempt_with_kind(
                                &state.pool,
                                account.account_id,
                                sticky_key,
                                &message,
                                PROXY_FAILURE_UPSTREAM_STREAM_ERROR,
                                trace_context.as_ref().map(|trace| trace.invoke_id.as_str()),
                                pending_attempt_record
                                    .as_ref()
                                    .and_then(|pending| pending.attempt_id),
                            )
                            .await
                        {
                            warn!(account_id = account.account_id, error = %route_err, "failed to record first-event gate transport failure");
                        }
                        store_pool_failover_error(
                            &mut last_error,
                            &mut preserve_sticky_owner_terminal_error,
                            PoolUpstreamError {
                                codex_imagegen_rewrite: attempted_codex_imagegen_rewrite.clone(),
                                account: Some(account.clone()),
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
                                        state.as_ref(),
                                        selected_proxy.key.as_str(),
                                    )
                                    .await
                                } else {
                                    None
                                },
                                oauth_responses_debug: oauth_responses_debug.clone(),
                                attempt_summary: PoolAttemptSummary::default(),
                                requested_service_tier: attempted_requested_service_tier.clone(),
                                request_body_for_capture: attempted_request_body_for_capture
                                    .clone(),
                            },
                            attempted_codex_imagegen_rewrite.as_ref(),
                        );
                        exhausted_accounts_all_rate_limited = false;
                        disarm_pool_early_phase_cleanup_guard(&mut early_phase_cleanup_guard);
                        continue 'account_loop;
                    }
                };

            let mut deferred_early_phase_cleanup_guard = None;
            if let Some(pending_attempt_record) = pending_attempt_record.as_ref() {
                if pending_attempt_record.attempt_id.is_none() {
                    deferred_early_phase_cleanup_guard = early_phase_cleanup_guard.take();
                }
                let phase_enqueued = enqueue_pool_upstream_request_attempt_progress(
                    state.as_ref(),
                    pending_attempt_record,
                    POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_STREAMING_RESPONSE,
                    Some(connect_latency_ms),
                    Some(first_byte_latency_ms),
                    None,
                    None,
                );
                if phase_enqueued {
                    debug!(
                        invoke_id = %pending_attempt_record.invoke_id,
                        attempt_id = pending_attempt_record.attempt_id,
                        "queued pool attempt streaming phase progress"
                    );
                    if pending_attempt_record.attempt_id.is_some() {
                        deferred_early_phase_cleanup_guard = early_phase_cleanup_guard.take();
                    }
                } else {
                    info!(
                        invoke_id = %pending_attempt_record.invoke_id,
                        attempt_id = pending_attempt_record.attempt_id,
                        "streaming phase was not enqueued; relying on invocation cleanup guards for post-first-byte recovery"
                    );
                    if pending_attempt_record.attempt_id.is_some() {
                        deferred_early_phase_cleanup_guard = early_phase_cleanup_guard.take();
                    }
                }
            } else {
                disarm_pool_early_phase_cleanup_guard(&mut early_phase_cleanup_guard);
            }

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
            reservation_guard.disarm();
            return Ok(PoolUpstreamResponse {
                account: account.clone(),
                response,
                transport_bytes_live_counted,
                stream_timeout,
                oauth_responses_debug,
                connect_latency_ms,
                attempt_started_at_utc,
                first_byte_latency_ms,
                live_request_body_first_byte_at: None,
                first_chunk,
                first_chunk_received_at,
                first_stream_chunk_received_at,
                pending_attempt_record: pending_attempt_record.map(|mut pending| {
                    pending.connect_latency_ms = connect_latency_ms;
                    pending.first_byte_latency_ms = first_byte_latency_ms;
                    pending.compact_support_status = compact_support_observation
                        .as_ref()
                        .map(|value| value.status.to_string());
                    pending.compact_support_reason = compact_support_observation
                        .as_ref()
                        .and_then(|value| value.reason.clone());
                    pending
                }),
                deferred_early_phase_cleanup_guard,
                live_attempt_activity_lease,
                attempt_summary: pool_attempt_summary(attempt_count, distinct_account_count, None),
                requested_service_tier: attempted_requested_service_tier,
                request_body_for_capture: attempted_request_body_for_capture,
                codex_imagegen_rewrite: attempted_codex_imagegen_rewrite,
            });
        }
    }
}

#[cfg(test)]
mod request_compression_tests {
    use super::*;

    #[test]
    fn forwarded_oauth_request_compression_is_recorded_as_passthrough() {
        let mut headers = HeaderMap::new();
        headers.insert(header::CONTENT_ENCODING, HeaderValue::from_static("zstd"));

        assert_eq!(
            forwarded_request_compression_observation(&headers),
            Some(("zstd", "passthrough"))
        );
        headers.insert(header::CONTENT_ENCODING, HeaderValue::from_static("x-gzip"));
        assert_eq!(
            forwarded_request_compression_observation(&headers),
            Some(("gzip", "passthrough"))
        );
        assert_eq!(
            resolved_pool_request_compression_algorithm(
                &headers,
                true,
                RequestCompressionAlgorithm::Identity,
            ),
            Some("gzip")
        );
    }

    #[test]
    fn terminal_pool_request_compression_uses_actual_forwarding_mode() {
        let mut headers = HeaderMap::new();
        headers.insert(header::CONTENT_ENCODING, HeaderValue::from_static("gzip"));

        assert_eq!(
            resolved_pool_request_compression_algorithm(
                &headers,
                false,
                RequestCompressionAlgorithm::Follow,
            ),
            Some("gzip")
        );
        assert_eq!(
            resolved_pool_request_compression_algorithm(
                &headers,
                false,
                RequestCompressionAlgorithm::Zstd,
            ),
            Some("zstd")
        );
    }

    #[test]
    fn terminal_pool_request_compression_keeps_unknown_attempt_metadata_unknown() {
        assert_eq!(
            resolve_terminal_request_compression_algorithm(Some(None), Some("zstd".to_string())),
            None
        );
        assert_eq!(
            resolve_terminal_request_compression_algorithm(
                Some(Some("gzip".to_string())),
                Some("zstd".to_string())
            ),
            Some("gzip".to_string())
        );
        assert_eq!(
            resolve_terminal_request_compression_algorithm(None, Some("zstd".to_string())),
            Some("zstd".to_string())
        );
    }
}
