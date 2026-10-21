use super::*;

pub(crate) async fn send_pool_request_with_failover_and_binding_constraint_inner(
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
    let mut reservation_guard = Some(PoolRoutingReservationDropGuard::new(
        state.clone(),
        reservation_key.clone(),
    ));
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
            let (resolution, updated_wait_deadline) =
                resolve_pool_account_for_failover_on_fresh_task(
                    state.clone(),
                    sticky_key.map(str::to_owned),
                    requested_model.clone(),
                    excluded_ids.clone(),
                    excluded_upstream_route_keys.clone(),
                    route_scoped_overload_selection.clone(),
                    binding_constraint.clone(),
                    conversation_override.clone(),
                    wait_for_no_available,
                    no_available_wait_deadline,
                    total_timeout_deadline,
                    capability_endpoint.to_string(),
                    image_intent,
                    codex_imagegen_request,
                    reservation_key.clone(),
                )
                .await;
            no_available_wait_deadline = updated_wait_deadline;
            match resolution {
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
                            None,
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
                    PoolAccountResolution::NoCandidate(no_candidate_audit),
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
                                    Some(no_candidate_audit.clone()),
                                )
                            });
                            err.attempt_summary = PoolAttemptSummary {
                                pool_routing_no_candidate_audit: Some(no_candidate_audit.clone()),
                                ..pool_attempt_summary(
                                    attempt_count,
                                    distinct_account_count,
                                    Some(err.failure_kind.to_string()),
                                )
                            };
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
                    return Err(build_pool_route_selection_failure_error(
                        &err,
                        attempt_count,
                        distinct_account_count,
                    ));
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
        let priority_handoff_admitted = priority_handoff_is_single_attempt(&account);
        let same_account_attempt_budget = if priority_handoff_admitted {
            1
        } else {
            pool_same_account_attempt_budget(
                original_uri,
                &method,
                distinct_account_count,
                initial_same_account_attempts,
            )
        };
        let overload_same_account_attempt_budget = if priority_handoff_admitted {
            0
        } else {
            pool_overload_same_account_attempt_budget(
                original_uri,
                &method,
                distinct_account_count,
                same_account_attempt_budget,
            )
        };
        let group_upstream_429_max_retries = if priority_handoff_admitted {
            0
        } else {
            account.effective_upstream_429_max_retries()
        };
        let model_mapping = load_model_mapping_for_account(
            state.as_ref(),
            account.account_id,
            requested_model.as_deref().or_else(|| {
                trace_context
                    .as_ref()
                    .and_then(|trace| trace.request_model.as_deref())
            }),
        )
        .await
        .map_err(|err| PoolUpstreamError {
            codex_imagegen_rewrite: None,
            account: Some(account.clone()),
            status: StatusCode::BAD_GATEWAY,
            message: format!("failed to resolve account model mapping: {err}"),
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
        })?;
        let upstream_request_model = model_mapping
            .as_ref()
            .map(|mapping| mapping.target_model.as_str())
            .or(requested_model.as_deref())
            .or_else(|| {
                trace_context
                    .as_ref()
                    .and_then(|trace| trace.request_model.as_deref())
            });
        let model_mapping_pattern = model_mapping
            .as_ref()
            .map(|mapping| mapping.source_model.as_str());
        let same_account_attempt_loop_budget = if priority_handoff_admitted {
            1
        } else {
            overload_same_account_attempt_budget
                .saturating_add(1)
                .saturating_add(group_upstream_429_max_retries)
        };
        let mut group_upstream_429_retry_count = 0_u8;
        let mut retried_upstream_413_for_account = last_error.as_ref().is_some_and(|err| {
            err.status == StatusCode::PAYLOAD_TOO_LARGE
                && err
                    .account
                    .as_ref()
                    .is_some_and(|err_account| err_account.account_id == account.account_id)
        });
        let mut first_response_attempt_started_at = None;
        let mut same_account_only_retry_active = false;
        let mut same_account_only_forward_proxy: Option<PoolSelectedForwardProxy> = None;

        match super::attempt_execution::execute_pool_account_attempts(
            super::attempt_execution::PoolAttemptExecutionContext {
                state: state.clone(),
                proxy_request_id,
                method: &method,
                original_uri,
                headers,
                body: &body,
                request_connection_scoped: &request_connection_scoped,
                direct_image_request,
                uses_timeout_route_failover,
                responses_total_timeout,
                responses_total_timeout_started_at,
                reservation_key: reservation_key.clone(),
                reservation_guard: &mut reservation_guard,
                trace_context: trace_context.as_ref(),
                runtime_snapshot_context: runtime_snapshot_context.as_ref(),
                sticky_key,
                codex_imagegen_request,
                request_compression_level_preset,
                requested_model: &requested_model,
                account,
                pre_first_byte_timeout,
                stream_timeout,
                send_timeout,
                upstream_route_key,
                api_key_target_url,
                priority_handoff_admitted,
                same_account_attempt_budget,
                overload_same_account_attempt_budget,
                group_upstream_429_max_retries,
                model_mapping: &model_mapping,
                upstream_request_model,
                model_mapping_pattern,
                same_account_attempt_loop_budget,
                attempt_count: &mut attempt_count,
                last_error: &mut last_error,
                preserve_sticky_owner_terminal_error: &mut preserve_sticky_owner_terminal_error,
                exhausted_accounts_all_rate_limited: &mut exhausted_accounts_all_rate_limited,
                excluded_upstream_route_keys: &mut excluded_upstream_route_keys,
                overload_required_upstream_route_key: &mut overload_required_upstream_route_key,
                timeout_route_failover_pending: &mut timeout_route_failover_pending,
                capability_endpoint,
                sticky_event_prompt_cache_key,
                prompt_cache_key,
                distinct_account_index,
                first_response_attempt_started_at: &mut first_response_attempt_started_at,
                same_account_only_retry_active: &mut same_account_only_retry_active,
                same_account_only_forward_proxy: &mut same_account_only_forward_proxy,
                group_upstream_429_retry_count: &mut group_upstream_429_retry_count,
                retried_upstream_413_for_account: &mut retried_upstream_413_for_account,
            },
            distinct_account_count,
        )
        .await?
        {
            Some(response) => return Ok(response),
            None => continue 'account_loop,
        }
    }
}

pub(crate) fn pool_upstream_413_retry_allowed(
    status: StatusCode,
    priority_handoff_admitted: bool,
    retried_upstream_413_for_account: bool,
) -> bool {
    status == StatusCode::PAYLOAD_TOO_LARGE
        && !priority_handoff_admitted
        && !retried_upstream_413_for_account
}
