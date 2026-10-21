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
                let capacity_wait = crate::observability::diagnostics::waiting(
                    crate::observability::diagnostics::Resource::AccountCapacity,
                );
                tokio::select! {
                    changed = availability.changed() => {
                        // A release, recovery, reset, or settings change made capacity
                        // observable again. The next loop owns the fresh selection.
                        let _ = changed;
                    }
                    _ = tokio::time::sleep(wake_after) => {}
                }
                if let Some(guard) = capacity_wait {
                    guard.complete();
                }
            }
            _ => return Ok(PoolAccountResolutionWithWait::Resolution(resolution)),
        }
    }
}

pub(crate) async fn resolve_pool_account_for_failover_on_fresh_task(
    state: Arc<AppState>,
    sticky_key: Option<String>,
    requested_model: Option<String>,
    excluded_ids: Vec<i64>,
    excluded_upstream_route_keys: HashSet<String>,
    required_upstream_route_key: Option<String>,
    binding_constraint: Option<PromptCacheConversationBindingConstraint>,
    conversation_override: Option<ConversationRoutingOverride>,
    wait_for_no_available: bool,
    wait_deadline: Option<Instant>,
    total_timeout_deadline: Option<Instant>,
    endpoint: String,
    image_intent: crate::ImageIntent,
    codex_imagegen_request: bool,
    reservation_key: String,
) -> (Result<PoolAccountResolutionWithWait>, Option<Instant>) {
    let task = tokio_util::task::AbortOnDropHandle::new(crate::observability::diagnostics::spawn(
        async move {
            let mut wait_deadline = wait_deadline;
            let resolution = resolve_pool_account_for_request_with_wait_and_binding_constraint_with_image_intent_and_override_and_codex_imagegen_request_and_reservation(
            state.as_ref(),
            sticky_key.as_deref(),
            requested_model.as_deref(),
            &excluded_ids,
            &excluded_upstream_route_keys,
            required_upstream_route_key.as_deref(),
            binding_constraint.as_ref(),
            conversation_override.as_ref(),
            wait_for_no_available,
            &mut wait_deadline,
            total_timeout_deadline,
            endpoint.as_str(),
            image_intent,
            codex_imagegen_request,
            Some(reservation_key.as_str()),
        )
        .await;
            (resolution, wait_deadline)
        },
    ));

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

pub(crate) fn build_pool_attempt_journal_failure_error(
    account: &PoolResolvedAccount,
    error: &str,
    attempt_count: usize,
    distinct_account_count: usize,
) -> PoolUpstreamError {
    PoolUpstreamError {
        codex_imagegen_rewrite: None,
        account: Some(account.clone()),
        status: StatusCode::SERVICE_UNAVAILABLE,
        message: format!("proxy attempt durability unavailable: {error}"),
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
        attempt_summary: pool_attempt_summary(
            attempt_count,
            distinct_account_count,
            Some(PROXY_FAILURE_FAILED_CONTACT_UPSTREAM.to_string()),
        ),
        requested_service_tier: None,
        request_body_for_capture: None,
    }
}
