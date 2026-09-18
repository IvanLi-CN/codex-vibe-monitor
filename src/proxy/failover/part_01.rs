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

pub(crate) fn no_candidate_next_eligible_delay(
    audit: &PoolRoutingNoCandidateAudit,
) -> Option<Duration> {
    const MIN_STALE_NEXT_ELIGIBLE_RESELECT_DELAY: Duration = Duration::from_millis(25);

    audit
        .next_eligible_at
        .as_deref()
        .and_then(parse_to_utc_datetime)
        .map(|eligible_at| {
            (eligible_at - Utc::now())
                .to_std()
                .unwrap_or(Duration::ZERO)
                .max(MIN_STALE_NEXT_ELIGIBLE_RESELECT_DELAY)
        })
}

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

struct PoolPrepareFailureAttemptRequest<'a> {
    state: &'a AppState,
    trace_context: Option<&'a PoolUpstreamAttemptTraceContext>,
    account: &'a PoolResolvedAccount,
    requested_model: Option<&'a str>,
    model_mapping_pattern: Option<&'a str>,
    attempt_index: i64,
    distinct_account_index: i64,
    same_account_retry_index: i64,
    status: StatusCode,
    message: &'a str,
}

async fn record_pool_request_prepare_failure_attempt(
    request: PoolPrepareFailureAttemptRequest<'_>,
) {
    let PoolPrepareFailureAttemptRequest {
        state,
        trace_context,
        account,
        requested_model,
        model_mapping_pattern,
        attempt_index,
        distinct_account_index,
        same_account_retry_index,
        status,
        message,
    } = request;
    let Some(trace) = trace_context else {
        return;
    };
    let mut attempt_trace = trace.clone();
    if attempt_trace.request_model.is_none() {
        attempt_trace.request_model = requested_model.map(ToOwned::to_owned);
    }
    attempt_trace.upstream_base_url_host = account
        .upstream_base_url
        .host_str()
        .and_then(normalize_upstream_base_url_host_value);
    let group_name_snapshot = normalize_pool_attempt_group_name(account.group_name.clone());
    let upstream_route_key = account.upstream_route_key();
    let started_at = format_naive_precise(Utc::now().with_timezone(&Shanghai).naive_local());
    let pending = begin_pool_upstream_request_attempt_with_scope_and_routing_source_and_audit(
        &state.pool,
        &attempt_trace,
        PoolAttemptStartScope {
            group_name_snapshot: group_name_snapshot.as_deref(),
            proxy_binding_key_snapshot: None,
            upstream_account_id: account.account_id,
            upstream_route_key: &upstream_route_key,
        },
        Some(account.routing_source),
        account.routing_selection_audit.as_ref(),
        PoolAttemptStartIndexes {
            attempt_index,
            distinct_account_index,
            same_account_retry_index,
        },
        &started_at,
    )
    .await;
    if let Err(err) = annotate_pool_upstream_request_attempt_model_mapping(
        &state.pool,
        &pending,
        None,
        model_mapping_pattern,
    )
    .await
    {
        warn!(
            invoke_id = %pending.invoke_id,
            error = %err,
            "failed to persist pre-send pool model mapping metadata"
        );
    }
    let finished_at = shanghai_now_string();
    if let Err(err) = finalize_pool_upstream_request_attempt(
        &state.pool,
        &pending,
        PoolAttemptFinalization {
            finished_at: &finished_at,
            status: POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_TRANSPORT_FAILURE,
            http_status: Some(status),
            downstream_http_status: None,
            failure_kind: Some(PROXY_FAILURE_FAILED_CONTACT_UPSTREAM),
            error_message: Some(message),
            downstream_error_message: None,
            connect_latency_ms: Some(0.0),
            first_byte_latency_ms: None,
            stream_latency_ms: None,
            upstream_request_id: None,
            compact_support_status: None,
            compact_support_reason: None,
        },
    )
    .await
    {
        warn!(
            invoke_id = %pending.invoke_id,
            error = %err,
            "failed to persist pre-send pool attempt"
        );
    }
    if let Err(err) = broadcast_pool_upstream_attempts_snapshot(state, &pending.invoke_id).await {
        warn!(
            invoke_id = %pending.invoke_id,
            error = %err,
            "failed to broadcast pre-send pool attempt snapshot"
        );
    }
}

pub(crate) struct PoolAccountWaitRequest<'a> {
    pub(crate) state: &'a AppState,
    pub(crate) sticky_key: Option<&'a str>,
    pub(crate) requested_model: Option<&'a str>,
    pub(crate) excluded_ids: &'a [i64],
    pub(crate) excluded_upstream_route_keys: &'a HashSet<String>,
    pub(crate) required_upstream_route_key: Option<&'a str>,
    pub(crate) binding_constraint: Option<&'a PromptCacheConversationBindingConstraint>,
    pub(crate) conversation_override: Option<&'a ConversationRoutingOverride>,
    pub(crate) wait_for_no_available: bool,
    pub(crate) wait_deadline: &'a mut Option<Instant>,
    pub(crate) total_timeout_deadline: Option<Instant>,
    pub(crate) endpoint: &'a str,
    pub(crate) image_intent: crate::ImageIntent,
    pub(crate) codex_imagegen_request: bool,
    pub(crate) reservation_key: Option<&'a str>,
}

pub(crate) async fn resolve_pool_account_for_request_with_wait(
    mut request: PoolAccountWaitRequest<'_>,
) -> Result<PoolAccountResolutionWithWait> {
    request.binding_constraint = None;
    request.conversation_override = None;
    request.endpoint = "";
    request.image_intent = crate::ImageIntent::Unknown;
    request.codex_imagegen_request = false;
    request.reservation_key = None;
    resolve_pool_account_for_request_with_wait_and_binding_constraint_internal(request).await
}

pub(crate) async fn resolve_pool_account_for_request_with_wait_and_image_intent(
    mut request: PoolAccountWaitRequest<'_>,
) -> Result<PoolAccountResolutionWithWait> {
    request.binding_constraint = None;
    request.conversation_override = None;
    request.codex_imagegen_request = false;
    request.reservation_key = None;
    resolve_pool_account_for_request_with_wait_and_binding_constraint_internal(request).await
}

pub(crate) async fn resolve_pool_account_for_request_with_wait_and_image_intent_and_codex_imagegen_request(
    mut request: PoolAccountWaitRequest<'_>,
) -> Result<PoolAccountResolutionWithWait> {
    request.binding_constraint = None;
    request.conversation_override = None;
    request.reservation_key = None;
    resolve_pool_account_for_request_with_wait_and_binding_constraint_internal(request).await
}

pub(crate) async fn resolve_pool_account_for_request_with_wait_and_image_intent_and_codex_imagegen_request_and_reservation(
    mut request: PoolAccountWaitRequest<'_>,
) -> Result<PoolAccountResolutionWithWait> {
    request.binding_constraint = None;
    request.conversation_override = None;
    resolve_pool_account_for_request_with_wait_and_binding_constraint_internal(request).await
}

pub(crate) async fn resolve_pool_account_for_request_with_wait_and_binding_constraint(
    mut request: PoolAccountWaitRequest<'_>,
) -> Result<PoolAccountResolutionWithWait> {
    request.conversation_override = None;
    request.endpoint = "";
    request.image_intent = crate::ImageIntent::Unknown;
    request.codex_imagegen_request = false;
    request.reservation_key = None;
    resolve_pool_account_for_request_with_wait_and_binding_constraint_internal(request).await
}

pub(crate) async fn resolve_pool_account_for_request_with_wait_and_binding_constraint_with_image_intent(
    mut request: PoolAccountWaitRequest<'_>,
) -> Result<PoolAccountResolutionWithWait> {
    request.conversation_override = None;
    request.codex_imagegen_request = false;
    request.reservation_key = None;
    resolve_pool_account_for_request_with_wait_and_binding_constraint_internal(request).await
}

pub(crate) async fn resolve_pool_account_for_request_with_wait_and_binding_constraint_with_image_intent_and_override(
    mut request: PoolAccountWaitRequest<'_>,
) -> Result<PoolAccountResolutionWithWait> {
    request.codex_imagegen_request = false;
    request.reservation_key = None;
    resolve_pool_account_for_request_with_wait_and_binding_constraint_internal(request).await
}

pub(crate) async fn resolve_pool_account_for_request_with_wait_and_binding_constraint_with_image_intent_and_override_and_reservation(
    mut request: PoolAccountWaitRequest<'_>,
) -> Result<PoolAccountResolutionWithWait> {
    request.codex_imagegen_request = false;
    resolve_pool_account_for_request_with_wait_and_binding_constraint_internal(request).await
}

pub(crate) async fn resolve_pool_account_for_request_with_wait_and_binding_constraint_with_image_intent_and_override_and_codex_imagegen_request(
    mut request: PoolAccountWaitRequest<'_>,
) -> Result<PoolAccountResolutionWithWait> {
    request.reservation_key = None;
    resolve_pool_account_for_request_with_wait_and_binding_constraint_internal(request).await
}

pub(crate) async fn resolve_pool_account_for_request_with_wait_and_binding_constraint_with_image_intent_and_override_and_codex_imagegen_request_and_reservation(
    request: PoolAccountWaitRequest<'_>,
) -> Result<PoolAccountResolutionWithWait> {
    resolve_pool_account_for_request_with_wait_and_binding_constraint_internal(request).await
}

pub(crate) async fn resolve_pool_account_for_request_with_wait_and_binding_constraint_internal(
    request: PoolAccountWaitRequest<'_>,
) -> Result<PoolAccountResolutionWithWait> {
    let PoolAccountWaitRequest {
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
    } = request;
    let mut availability = state.pool_routing_availability.subscribe();
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
                PoolAccountResolution::Unavailable | PoolAccountResolution::NoCandidate(_)
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
            | PoolAccountResolution::NoCandidate(_))
                if wait_for_no_available =>
            {
                let next_eligible_delay = match &resolution {
                    PoolAccountResolution::NoCandidate(audit) => {
                        no_candidate_next_eligible_delay(audit)
                    }
                    _ => None,
                };
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
                let remaining = effective_deadline.saturating_duration_since(now);
                let wake_after = next_eligible_delay
                    .map(|delay| delay.min(remaining))
                    .unwrap_or(remaining);
                tokio::select! {
                    changed = availability.changed() => {
                        // A release, recovery, reset, or settings change made capacity
                        // observable again. The next loop owns the fresh selection.
                        let _ = changed;
                    }
                    _ = tokio::time::sleep(wake_after) => {}
                }
            }
            _ => return Ok(PoolAccountResolutionWithWait::Resolution(resolution)),
        }
    }
}

pub(crate) struct PoolAccountFreshTaskRequest {
    pub(crate) state: Arc<AppState>,
    pub(crate) sticky_key: Option<String>,
    pub(crate) requested_model: Option<String>,
    pub(crate) excluded_ids: Vec<i64>,
    pub(crate) excluded_upstream_route_keys: HashSet<String>,
    pub(crate) required_upstream_route_key: Option<String>,
    pub(crate) binding_constraint: Option<PromptCacheConversationBindingConstraint>,
    pub(crate) conversation_override: Option<ConversationRoutingOverride>,
    pub(crate) wait_for_no_available: bool,
    pub(crate) wait_deadline: Option<Instant>,
    pub(crate) total_timeout_deadline: Option<Instant>,
    pub(crate) endpoint: String,
    pub(crate) image_intent: crate::ImageIntent,
    pub(crate) codex_imagegen_request: bool,
    pub(crate) reservation_key: String,
}

struct PoolSingleAccountBindingTerminalRequest<'a> {
    state: &'a AppState,
    trace_context: Option<&'a PoolUpstreamAttemptTraceContext>,
    binding_constraint: Option<&'a PromptCacheConversationBindingConstraint>,
    owner_auto_guard_active: bool,
    prompt_cache_key: Option<&'a str>,
    account: Option<PoolResolvedAccount>,
    message: Option<String>,
    attempt_count: usize,
    distinct_account_count: usize,
}

pub(crate) async fn resolve_pool_account_for_failover_on_fresh_task(
    request: PoolAccountFreshTaskRequest,
) -> (Result<PoolAccountResolutionWithWait>, Option<Instant>) {
    let PoolAccountFreshTaskRequest {
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
    } = request;
    let task = tokio_util::task::AbortOnDropHandle::new(tokio::spawn(async move {
        let mut wait_deadline = wait_deadline;
        let resolution = resolve_pool_account_for_request_with_wait_and_binding_constraint_with_image_intent_and_override_and_codex_imagegen_request_and_reservation(PoolAccountWaitRequest {
            state: state.as_ref(),
            sticky_key: sticky_key.as_deref(),
            requested_model: requested_model.as_deref(),
            excluded_ids: &excluded_ids,
            excluded_upstream_route_keys: &excluded_upstream_route_keys,
            required_upstream_route_key: required_upstream_route_key.as_deref(),
            binding_constraint: binding_constraint.as_ref(),
            conversation_override: conversation_override.as_ref(),
            wait_for_no_available,
            wait_deadline: &mut wait_deadline,
            total_timeout_deadline,
            endpoint: endpoint.as_str(),
            image_intent,
            codex_imagegen_request,
            reservation_key: Some(reservation_key.as_str()),
        })
        .await;
        (resolution, wait_deadline)
    }));

    await_pool_route_selection_task(task, wait_deadline).await
}

pub(crate) async fn await_pool_route_selection_task(
    task: tokio_util::task::AbortOnDropHandle<(
        Result<PoolAccountResolutionWithWait>,
        Option<Instant>,
    )>,
    fallback_wait_deadline: Option<Instant>,
) -> (Result<PoolAccountResolutionWithWait>, Option<Instant>) {
    match task.await {
        Ok(result) => result,
        Err(err) => (
            Err(anyhow!("pool route selection task failed: {err}")),
            fallback_wait_deadline,
        ),
    }
}

pub(crate) fn build_pool_route_selection_failure_error(
    err: &anyhow::Error,
    attempt_count: usize,
    distinct_account_count: usize,
) -> PoolUpstreamError {
    PoolUpstreamError {
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
    }
}

async fn maybe_build_and_record_single_account_binding_terminal_error(
    request: PoolSingleAccountBindingTerminalRequest<'_>,
) -> Option<PoolUpstreamError> {
    let PoolSingleAccountBindingTerminalRequest {
        state,
        trace_context,
        binding_constraint,
        owner_auto_guard_active,
        prompt_cache_key,
        account,
        message,
        attempt_count,
        distinct_account_count,
    } = request;
    let err = build_single_account_binding_blocked_error(SingleAccountBindingBlockedErrorRequest {
        state,
        binding_constraint,
        owner_auto_guard_active,
        account,
        prompt_cache_key,
        message,
        attempt_count,
        distinct_account_count,
    })
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

pub(crate) struct PoolFailoverRequest<'a> {
    pub(crate) state: Arc<AppState>,
    pub(crate) proxy_request_id: u64,
    pub(crate) method: Method,
    pub(crate) original_uri: &'a Uri,
    pub(crate) headers: &'a HeaderMap,
    pub(crate) body: Option<PoolReplayBodySnapshot>,
    pub(crate) handshake_timeout: Duration,
    pub(crate) trace_context: Option<PoolUpstreamAttemptTraceContext>,
    pub(crate) runtime_snapshot_context: Option<PoolAttemptRuntimeSnapshotContext>,
    pub(crate) sticky_key: Option<&'a str>,
    pub(crate) preferred_account: Option<PoolResolvedAccount>,
    pub(crate) failover_progress: PoolFailoverProgress,
    pub(crate) same_account_attempts: u8,
}

pub(crate) struct PoolFailoverBindingRequest<'a> {
    pub(crate) state: Arc<AppState>,
    pub(crate) proxy_request_id: u64,
    pub(crate) method: Method,
    pub(crate) original_uri: &'a Uri,
    pub(crate) headers: &'a HeaderMap,
    pub(crate) body: Option<PoolReplayBodySnapshot>,
    pub(crate) handshake_timeout: Duration,
    pub(crate) trace_context: Option<PoolUpstreamAttemptTraceContext>,
    pub(crate) runtime_snapshot_context: Option<PoolAttemptRuntimeSnapshotContext>,
    pub(crate) sticky_key: Option<&'a str>,
    pub(crate) sticky_event_prompt_cache_key: Option<&'a str>,
    pub(crate) binding_constraint: Option<PromptCacheConversationBindingConstraint>,
    pub(crate) conversation_override: Option<ConversationRoutingOverride>,
    pub(crate) preferred_account: Option<PoolResolvedAccount>,
    pub(crate) failover_progress: PoolFailoverProgress,
    pub(crate) same_account_attempts: u8,
    pub(crate) persist_terminal_invocation: bool,
}

pub(crate) async fn send_pool_request_with_failover(
    request: PoolFailoverRequest<'_>,
) -> Result<PoolUpstreamResponse, PoolUpstreamError> {
    let PoolFailoverRequest {
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
        preferred_account,
        failover_progress,
        same_account_attempts,
    } = request;
    send_pool_request_with_failover_and_binding_constraint(PoolFailoverBindingRequest {
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
        sticky_event_prompt_cache_key: None,
        binding_constraint: None,
        conversation_override: None,
        preferred_account,
        failover_progress,
        same_account_attempts,
        persist_terminal_invocation: true,
    })
    .await
}

pub(crate) fn send_pool_request_with_failover_and_binding_constraint<'a>(
    request: PoolFailoverBindingRequest<'a>,
) -> Pin<Box<dyn Future<Output = Result<PoolUpstreamResponse, PoolUpstreamError>> + Send + 'a>> {
    // Keep this large state machine off the request task stack. The imagegen audit
    // extends its state enough to overflow normal test and Axum worker stacks.
    Box::pin(async move {
        let PoolFailoverBindingRequest {
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
            persist_terminal_invocation,
        } = request;
        let capture_started = Instant::now();
        let state_for_terminal_capture = state.clone();
        let body_for_terminal_capture = body.clone();
        let trace_for_terminal_capture = trace_context.clone();
        let runtime_context_for_terminal_capture = runtime_snapshot_context.clone();
        let result = boxed_send_pool_request_with_failover_and_binding_constraint_inner(
            PoolFailoverBindingRequest {
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
                persist_terminal_invocation,
            },
        )
        .await;
        if persist_terminal_invocation && let Err(error) = &result {
            persist_pool_failover_terminal_invocation(PoolFailoverTerminalInvocationRequest {
                state: state_for_terminal_capture,
                proxy_request_id,
                capture_started,
                original_uri,
                headers,
                trace_context: trace_for_terminal_capture.as_ref(),
                runtime_snapshot_context: runtime_context_for_terminal_capture.as_ref(),
                request_body_snapshot: body_for_terminal_capture
                    .unwrap_or(PoolReplayBodySnapshot::Empty),
                error,
            })
            .await;
        }
        result
    })
}

struct PoolFailoverTerminalInvocationRequest<'a> {
    state: Arc<AppState>,
    proxy_request_id: u64,
    capture_started: Instant,
    original_uri: &'a Uri,
    headers: &'a HeaderMap,
    trace_context: Option<&'a PoolUpstreamAttemptTraceContext>,
    runtime_snapshot_context: Option<&'a PoolAttemptRuntimeSnapshotContext>,
    request_body_snapshot: PoolReplayBodySnapshot,
    error: &'a PoolUpstreamError,
}

async fn persist_pool_failover_terminal_invocation(
    request: PoolFailoverTerminalInvocationRequest<'_>,
) {
    let PoolFailoverTerminalInvocationRequest {
        state,
        proxy_request_id,
        capture_started,
        original_uri,
        headers,
        trace_context,
        runtime_snapshot_context,
        request_body_snapshot,
        error,
    } = request;
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
    let _ = persist_pre_attempt_proxy_capture_error(PreAttemptProxyCaptureError {
        state,
        proxy_request_id,
        capture_started,
        invoke_id: &trace.invoke_id,
        occurred_at: &trace.occurred_at,
        capture_target,
        request_info: &request_info,
        requester_ip: requester_ip.as_deref(),
        request_chain_metadata: &request_chain_metadata,
        sticky_key: trace.sticky_key.as_deref(),
        prompt_cache_key,
        client_attribution_context: &client_attribution_context,
        request_body_for_capture: request_body,
        request_body_snapshot,
        request_body_logging_enabled,
        t_req_read_ms: runtime_snapshot_context
            .map(|context| context.t_req_read_ms)
            .unwrap_or_default(),
        t_req_parse_ms: runtime_snapshot_context
            .map(|context| context.t_req_parse_ms)
            .unwrap_or_default(),
        status: error.status,
        failure_kind: error.failure_kind,
        error_message: &error.message,
        terminal_attempt_summary: Some(&error.attempt_summary),
        terminal_account: error.account.as_ref(),
        terminal_connect_latency_ms: Some(error.connect_latency_ms),
        terminal_error: Some(error),
        terminal_request_compression_algorithm: terminal_request_compression_algorithm.as_deref(),
        response_envelope_override: Some(response_envelope),
    })
    .await;
}
