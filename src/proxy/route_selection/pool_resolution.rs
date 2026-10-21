use super::*;

pub(crate) async fn encrypted_session_owner_routing_enabled(state: &AppState) -> bool {
    state
        .proxy_model_settings
        .read()
        .await
        .encrypted_session_owner_routing_enabled
}

pub(crate) async fn confirm_prompt_cache_encrypted_session_owner_success_if_enabled(
    state: &AppState,
    prompt_cache_key: &str,
    owner_upstream_account_id: i64,
) -> Result<bool> {
    if !encrypted_session_owner_routing_enabled(state).await {
        return Ok(false);
    }
    let updated = confirm_prompt_cache_encrypted_session_owner_success(
        &state.pool,
        prompt_cache_key,
        owner_upstream_account_id,
    )
    .await?;
    if updated {
        broadcast_prompt_cache_conversation_changed(state, prompt_cache_key).await;
    }
    Ok(updated)
}

pub(crate) async fn load_via_pool_prompt_cache_binding_constraint(
    state: &AppState,
    prompt_cache_key: Option<&str>,
) -> Result<Option<PromptCacheConversationBindingConstraint>, (StatusCode, String)> {
    let encrypted_owner_routing_enabled = encrypted_session_owner_routing_enabled(state).await;
    resolve_prompt_cache_effective_routing_constraint(
        &state.pool,
        prompt_cache_key,
        false,
        encrypted_owner_routing_enabled,
    )
    .await
    .map(|(constraint, _owner_auto_guard_active)| constraint)
    .map_err(|err| {
        (
            StatusCode::BAD_GATEWAY,
            format!("failed to resolve prompt cache conversation binding: {err}"),
        )
    })
}

pub(crate) async fn load_via_pool_effective_routing_constraint(
    state: &AppState,
    prompt_cache_key: Option<&str>,
    request_contains_encrypted_content: bool,
) -> Result<(Option<PromptCacheConversationBindingConstraint>, bool), (StatusCode, String)> {
    let encrypted_owner_routing_enabled = encrypted_session_owner_routing_enabled(state).await;
    resolve_prompt_cache_effective_routing_constraint(
        &state.pool,
        prompt_cache_key,
        request_contains_encrypted_content,
        encrypted_owner_routing_enabled,
    )
    .await
    .map_err(|err| {
        (
            StatusCode::BAD_GATEWAY,
            format!("failed to resolve prompt cache conversation binding: {err}"),
        )
    })
}

pub(crate) async fn load_via_pool_effective_routing(
    state: &AppState,
    prompt_cache_key: Option<&str>,
    request_contains_encrypted_content: bool,
) -> Result<
    (
        Option<PromptCacheConversationBindingConstraint>,
        bool,
        Option<ConversationRoutingOverride>,
    ),
    (StatusCode, String),
> {
    let (constraint, owner_auto_guard_active) = load_via_pool_effective_routing_constraint(
        state,
        prompt_cache_key,
        request_contains_encrypted_content,
    )
    .await?;
    let conversation_override =
        load_prompt_cache_conversation_routing_override(&state.pool, prompt_cache_key)
            .await
            .map_err(|err| {
                (
                    StatusCode::BAD_GATEWAY,
                    format!("failed to resolve prompt cache conversation overrides: {err}"),
                )
            })?;
    Ok((constraint, owner_auto_guard_active, conversation_override))
}

pub(crate) fn should_prebuffer_for_body_sticky_probe(
    has_header_sticky_key: bool,
    content_type: Option<&str>,
    body_size_hint_exact: Option<usize>,
) -> bool {
    !has_header_sticky_key
        && content_type
            .is_some_and(|value| value.to_ascii_lowercase().starts_with("application/json"))
        && body_size_hint_exact
            .is_some_and(|value| value <= POOL_REQUEST_REPLAY_MEMORY_THRESHOLD_BYTES)
}

pub(crate) fn build_via_pool_attempt_trace_context(
    invoke_id: &str,
    endpoint: &str,
    sticky_key: Option<String>,
) -> PoolUpstreamAttemptTraceContext {
    PoolUpstreamAttemptTraceContext {
        invoke_id: invoke_id.to_string(),
        occurred_at: shanghai_now_string(),
        endpoint: endpoint.to_string(),
        sticky_key,
        requester_ip: None,
        upstream_base_url_host: None,
        request_model: None,
    }
}

pub(super) async fn allocate_via_pool_invoke_id(
    state: &AppState,
    proxy_request_id: u64,
    prompt_cache_key: Option<&str>,
) -> Result<String, ProxyErrorResponse> {
    match allocate_proxy_invoke_id_with_active_lease(state, prompt_cache_key).await {
        Ok(invoke_id) => Ok(invoke_id),
        Err(err) => {
            warn!(
                proxy_request_id,
                prompt_cache_key_fingerprint = prompt_cache_key.map(prompt_cache_key_fingerprint),
                error = %err,
                "failed to allocate via-pool proxy invoke id; rejecting request"
            );
            Err(plain_proxy_error(
                StatusCode::SERVICE_UNAVAILABLE,
                format!("failed to allocate proxy invoke id: {err}"),
            ))
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct ViaPoolResolutionTerminalError {
    pub(crate) status: StatusCode,
    pub(crate) message: String,
    pub(crate) assigned_blocked: Option<PoolAssignedBlockedAccount>,
}

impl ViaPoolResolutionTerminalError {
    pub(crate) fn plain(status: StatusCode, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
            assigned_blocked: None,
        }
    }

    pub(crate) fn assigned_blocked(blocked: PoolAssignedBlockedAccount) -> Self {
        Self {
            status: StatusCode::SERVICE_UNAVAILABLE,
            message: blocked.message.clone(),
            assigned_blocked: Some(blocked),
        }
    }

    pub(crate) async fn persist_if_needed(
        &self,
        state: &AppState,
        trace_context: Option<&PoolUpstreamAttemptTraceContext>,
    ) {
        let Some(trace_context) = trace_context else {
            return;
        };
        let Some(blocked) = self.assigned_blocked.clone() else {
            return;
        };

        let terminal_failure_kind = blocked.failure_kind;
        let err = build_pool_assigned_account_blocked_error(
            blocked.account,
            blocked.message,
            terminal_failure_kind,
            1,
            1,
        );
        if let Err(record_err) = insert_and_broadcast_pool_upstream_terminal_attempt(
            state,
            trace_context,
            &err,
            1,
            1,
            terminal_failure_kind,
        )
        .await
        {
            warn!(
                invoke_id = trace_context.invoke_id,
                error = %record_err,
                "failed to persist via-pool preflight terminal attempt"
            );
        }
    }
}

pub(crate) async fn maybe_persist_single_account_binding_terminal_error(
    state: &AppState,
    trace_context: Option<&PoolUpstreamAttemptTraceContext>,
    binding_constraint: Option<&PromptCacheConversationBindingConstraint>,
    owner_auto_guard_active: bool,
    prompt_cache_key: Option<&str>,
    account: Option<PoolResolvedAccount>,
    message: Option<String>,
    attempt_count: usize,
    distinct_account_count: usize,
) -> Option<ProxyErrorResponse> {
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
    if let Some(trace_context) = trace_context
        && let Err(record_err) = insert_and_broadcast_pool_upstream_terminal_attempt(
            state,
            trace_context,
            &err,
            attempt_count as i64,
            distinct_account_count as i64,
            err.failure_kind,
        )
        .await
    {
        warn!(
            invoke_id = trace_context.invoke_id,
            error = %record_err,
            "failed to persist via-pool single-account binding terminal attempt"
        );
    }
    Some(proxy_error_response_from_pool_upstream_error(err, None))
}

pub(crate) async fn unwrap_via_pool_initial_account(
    state: &AppState,
    trace_context: Option<&PoolUpstreamAttemptTraceContext>,
    resolution: Result<PoolAccountResolutionWithWait>,
    no_available_wait_deadline: Option<Instant>,
    responses_total_timeout: Option<Duration>,
    binding_constraint: Option<&PromptCacheConversationBindingConstraint>,
    owner_auto_guard_active: bool,
    prompt_cache_key: Option<&str>,
) -> Result<(PoolResolvedAccount, Option<Instant>), ProxyErrorResponse> {
    let initial_account = match resolution {
        Ok(PoolAccountResolutionWithWait::Resolution(PoolAccountResolution::Resolved(account))) => {
            account
        }
        Ok(PoolAccountResolutionWithWait::TotalTimeoutExpired) => {
            let total_timeout = responses_total_timeout
                .expect("pre-attempt total-timeout expiry requires responses timeout");
            return Err(plain_proxy_error(
                StatusCode::GATEWAY_TIMEOUT,
                pool_total_timeout_exhausted_message(total_timeout),
            ));
        }
        Ok(PoolAccountResolutionWithWait::Resolution(PoolAccountResolution::Unavailable))
        | Ok(PoolAccountResolutionWithWait::Resolution(PoolAccountResolution::NoCandidate(_))) => {
            if let Some(err) = maybe_persist_single_account_binding_terminal_error(
                state,
                trace_context,
                binding_constraint,
                owner_auto_guard_active,
                prompt_cache_key,
                None,
                None,
                1,
                1,
            )
            .await
            {
                return Err(err);
            }
            return Err(plain_proxy_error(
                StatusCode::SERVICE_UNAVAILABLE,
                POOL_NO_AVAILABLE_ACCOUNT_MESSAGE.to_string(),
            ));
        }
        Ok(PoolAccountResolutionWithWait::Resolution(PoolAccountResolution::RateLimited)) => {
            if let Some(err) = maybe_persist_single_account_binding_terminal_error(
                state,
                trace_context,
                binding_constraint,
                owner_auto_guard_active,
                prompt_cache_key,
                None,
                None,
                1,
                1,
            )
            .await
            {
                return Err(err);
            }
            return Err(plain_proxy_error(
                StatusCode::TOO_MANY_REQUESTS,
                POOL_ALL_ACCOUNTS_RATE_LIMITED_MESSAGE.to_string(),
            ));
        }
        Ok(PoolAccountResolutionWithWait::Resolution(PoolAccountResolution::DegradedOnly)) => {
            if let Some(err) = maybe_persist_single_account_binding_terminal_error(
                state,
                trace_context,
                binding_constraint,
                owner_auto_guard_active,
                prompt_cache_key,
                None,
                None,
                1,
                1,
            )
            .await
            {
                return Err(err);
            }
            return Err(plain_proxy_error(
                StatusCode::SERVICE_UNAVAILABLE,
                POOL_ALL_ACCOUNTS_DEGRADED_MESSAGE.to_string(),
            ));
        }
        Ok(PoolAccountResolutionWithWait::Resolution(PoolAccountResolution::AssignedBlocked(
            blocked,
        ))) => {
            if let Some(err) = maybe_persist_single_account_binding_terminal_error(
                state,
                trace_context,
                binding_constraint,
                owner_auto_guard_active,
                prompt_cache_key,
                Some(blocked.account.clone()),
                Some(blocked.message.clone()),
                1,
                1,
            )
            .await
            {
                return Err(err);
            }
            let terminal_error = ViaPoolResolutionTerminalError::assigned_blocked(blocked);
            terminal_error.persist_if_needed(state, trace_context).await;
            return Err(plain_proxy_error(
                terminal_error.status,
                terminal_error.message,
            ));
        }
        Ok(PoolAccountResolutionWithWait::Resolution(PoolAccountResolution::BlockedByPolicy(
            message,
        ))) => {
            if let Some(err) = maybe_persist_single_account_binding_terminal_error(
                state,
                trace_context,
                binding_constraint,
                owner_auto_guard_active,
                prompt_cache_key,
                None,
                Some(message.clone()),
                1,
                1,
            )
            .await
            {
                return Err(err);
            }
            return Err(plain_proxy_error(StatusCode::SERVICE_UNAVAILABLE, message));
        }
        Err(err) => {
            return Err(plain_proxy_error(
                StatusCode::BAD_GATEWAY,
                format!("failed to resolve pool account: {err}"),
            ));
        }
    };
    Ok((initial_account, no_available_wait_deadline))
}

pub(crate) async fn finalize_tracked_pool_attempt(
    state: &AppState,
    pending_attempt_record: Option<&PendingPoolAttemptRecord>,
    status: &str,
    http_status: Option<StatusCode>,
    downstream_http_status: Option<StatusCode>,
    failure_kind: Option<&str>,
    error_message: Option<&str>,
    downstream_error_message: Option<&str>,
    connect_latency_ms: Option<f64>,
    first_byte_latency_ms: Option<f64>,
    stream_latency_ms: Option<f64>,
    upstream_request_id: Option<&str>,
    context: &'static str,
) {
    let Some(pending_attempt_record) = pending_attempt_record else {
        return;
    };
    let priority_handoff =
        pending_attempt_record.routing_source.as_deref() == Some(PRIORITY_HANDOFF_ROUTING_SOURCE);
    let priority_handoff_client_cancelled = priority_handoff
        && priority_handoff_client_cancellation(status, downstream_http_status, failure_kind);
    let priority_handoff_success =
        priority_handoff && status == POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS;
    let priority_handoff_cooldown = !priority_handoff_success
        && (http_status.is_some_and(|status| {
            status == StatusCode::TOO_MANY_REQUESTS || status.is_server_error()
        }) || matches!(
            failure_kind,
            Some(
                PROXY_FAILURE_FAILED_CONTACT_UPSTREAM
                    | PROXY_FAILURE_UPSTREAM_HANDSHAKE_TIMEOUT
                    | PROXY_FAILURE_UPSTREAM_STREAM_ERROR
                    | PROXY_FAILURE_UPSTREAM_RESPONSE_FAILED
            )
        ));
    let priority_handoff_generation = priority_handoff.then(|| {
        priority_handoff_generation_from_audit_json(
            pending_attempt_record
                .routing_selection_audit_json
                .as_deref(),
        )
    });
    let finished_at = shanghai_now_string();
    if let Err(err) = finalize_pool_upstream_request_attempt_in_memory(
        state,
        pending_attempt_record,
        finished_at.as_str(),
        status,
        http_status,
        downstream_http_status,
        failure_kind,
        error_message,
        downstream_error_message,
        connect_latency_ms,
        first_byte_latency_ms,
        stream_latency_ms,
        upstream_request_id,
        None,
        None,
    )
    .await
    {
        warn!(
            context,
            invoke_id = %pending_attempt_record.invoke_id,
            error = %err,
            "failed to persist tracked pool attempt"
        );
    }
    if let Err(err) =
        broadcast_pool_upstream_attempts_snapshot(state, &pending_attempt_record.invoke_id).await
    {
        warn!(
            context,
            invoke_id = %pending_attempt_record.invoke_id,
            error = %err,
            "failed to broadcast tracked pool attempt snapshot"
        );
    }
    if priority_handoff {
        // A pure downstream close can leave a successful upstream status in the
        // capture record. It is client cancellation, not handoff evidence, so it
        // must only release the local permit through the owning guard.
        if priority_handoff_client_cancelled {
            forget_priority_handoff_attempt(pending_attempt_record.attempt_id);
            forget_priority_handoff_attempt_for_invoke(&pending_attempt_record.invoke_id);
        } else if let Some(Some(generation)) = priority_handoff_generation {
            if priority_handoff_success {
                // The attempt row is finalized before success completion, so a
                // complete sticky write can advance verification. If either
                // persistence step failed, completion releases without credit.
                complete_runtime_priority_handoff(
                    pending_attempt_record.attempt_id,
                    Some(&pending_attempt_record.invoke_id),
                    true,
                );
                if let Some(model_key) = pending_attempt_record.request_model.as_deref()
                    && !model_key.trim().is_empty()
                {
                    release_priority_handoff_for_key(
                        pending_attempt_record.upstream_account_id,
                        model_key,
                        generation,
                    );
                }
            } else {
                defer_priority_handoff_failure_for_key(
                    pending_attempt_record.upstream_account_id,
                    pending_attempt_record
                        .request_model
                        .as_deref()
                        .unwrap_or_default(),
                    generation,
                    priority_handoff_cooldown,
                );
            }
            forget_priority_handoff_attempt(pending_attempt_record.attempt_id);
            forget_priority_handoff_attempt_for_invoke(&pending_attempt_record.invoke_id);
        } else {
            forget_priority_handoff_attempt(pending_attempt_record.attempt_id);
            forget_priority_handoff_attempt_for_invoke(&pending_attempt_record.invoke_id);
        }
    }
}

pub(crate) async fn maybe_backfill_oauth_request_debug_from_replay_status(
    debug: &mut Option<oauth_bridge::OauthResponsesDebugInfo>,
    original_uri: &Uri,
    replay_status_rx: &watch::Receiver<PoolReplayBodyStatus>,
    crypto_key: Option<&[u8; 32]>,
) {
    let Some(debug) = debug.as_mut() else {
        return;
    };
    if debug.request_body_prefix_fingerprint.is_some() || crypto_key.is_none() {
        return;
    }

    let replay_status = { replay_status_rx.borrow().clone() };
    let PoolReplayBodyStatus::Complete(snapshot) = replay_status else {
        return;
    };
    let Ok(prefix) = snapshot
        .to_prefix_bytes(oauth_bridge::OAUTH_REQUEST_BODY_PREFIX_FINGERPRINT_MAX_BYTES)
        .await
    else {
        return;
    };
    oauth_bridge::backfill_oauth_request_debug_body_prefix(
        debug,
        original_uri.path(),
        prefix.as_ref(),
        crypto_key,
    );
}

pub(crate) fn infer_request_image_intent(
    capture_target: Option<ProxyCaptureTarget>,
    parsed_request_body: Option<&Value>,
) -> crate::ImageIntent {
    match capture_target {
        Some(ProxyCaptureTarget::ImageGenerations | ProxyCaptureTarget::ImageEdits) => {
            crate::ImageIntent::DirectImage
        }
        Some(target) => parsed_request_body
            .map(|value| infer_hosted_image_intent_from_request_body(target, value))
            .unwrap_or(crate::ImageIntent::Unknown),
        None => crate::ImageIntent::Unknown,
    }
}

pub(crate) fn infer_request_compaction_kind(
    capture_target: Option<ProxyCaptureTarget>,
    parsed_request_body: Option<&Value>,
) -> Option<CompactionKind> {
    match capture_target {
        Some(ProxyCaptureTarget::ResponsesCompact) => Some(CompactionKind::Compact),
        Some(ProxyCaptureTarget::Responses) => parsed_request_body
            .filter(|value| request_declares_remote_v2_compaction(value))
            .map(|_| CompactionKind::RemoteV2),
        _ => None,
    }
}
