use super::*;

#[expect(
    clippy::too_many_arguments,
    reason = "Runtime adapters preserve the established proxy route outcome contract."
)]
pub(crate) async fn record_runtime_pool_route_success(
    state: &AppState,
    account_id: i64,
    request_started_at_utc: DateTime<Utc>,
    sticky_key: Option<&str>,
    prompt_cache_key: Option<&str>,
    invoke_id: Option<&str>,
    endpoint: &str,
    image_intent: ImageIntent,
    codex_imagegen_rewrite: Option<&Value>,
    _attempt_id: Option<i64>,
    sticky_affinity_generation: Option<i64>,
    request_model: Option<&str>,
) -> Result<bool> {
    let runtime = runtime_route_facts(state).await?;
    let gate = runtime.admission_gate.lock().await;
    let account = load_account_projection(state, Some(&runtime), account_id, true)
        .await?
        .context("route account not found")?;
    if runtime_request_control_is_stale(state, &account, invoke_id) {
        return Ok(false);
    }
    let observed_at = format_utc_iso_precise(Utc::now());
    let health_before = RuntimeAccountHealth::from_row(&account);
    let can_recover = account
        .last_route_failure_at
        .as_deref()
        .is_none_or(|failure| {
            super::super::failure_recording::route_failure_precedes_request(
                failure,
                request_started_at_utc,
            )
        });
    let health_after = can_recover.then(|| RuntimeAccountHealth {
        status: UPSTREAM_ACCOUNT_STATUS_ACTIVE.to_string(),
        last_error_at: None,
        last_route_failure_at: None,
        last_route_failure_kind: None,
        cooldown_until: None,
        consecutive_route_failures: 0,
        temporary_route_failure_streak_started_at: None,
    });
    let requirements =
        RequestCapabilityRequirements::from_endpoint_and_image_intent(endpoint, image_intent);
    let axes = [
        (
            requirements.response_endpoint,
            UpstreamCapabilityAxis::ResponseEndpoint,
        ),
        (
            requirements.chat_completions_endpoint,
            UpstreamCapabilityAxis::ChatCompletionsEndpoint,
        ),
        (
            requirements.image_endpoint,
            UpstreamCapabilityAxis::ImageEndpoint,
        ),
        (
            requirements.response_image_tool,
            UpstreamCapabilityAxis::ResponseImageTool,
        ),
        (
            requirements.standalone_search && account.kind == UPSTREAM_ACCOUNT_KIND_API_KEY_CODEX,
            UpstreamCapabilityAxis::StandaloneSearch,
        ),
        (
            codex_imagegen_rewrite.is_some(),
            UpstreamCapabilityAxis::CodexImagegen,
        ),
    ];
    let capabilities = axes
        .into_iter()
        .filter(|(required, _)| *required)
        .map(|(_, axis)| RuntimeCapabilityFact {
            axis,
            support: "supported".to_string(),
            observed_at: format_utc_iso_precise(request_started_at_utc),
            reason: axis.success_reason().to_string(),
        })
        .collect();
    let sticky = if can_recover {
        build_sticky_fact(
            state,
            &runtime,
            sticky_key,
            request_model,
            sticky_affinity_generation,
            Some(account_id),
            &observed_at,
            account_id,
            prompt_cache_key,
            invoke_id,
            Some(request_started_at_utc),
        )
        .await?
    } else {
        None
    };
    let model = model::outcome_fact(
        state,
        &runtime,
        &account,
        request_model,
        request_started_at_utc,
        None,
        Utc::now(),
    )
    .await?;
    let model_recovered = model
        .as_ref()
        .is_none_or(|fact| fact.after.state == MODEL_ROUTE_STATE_AVAILABLE);
    let event = RuntimeRouteEvent {
        sequence: 0,
        event_id: String::new(),
        account_id,
        control_fingerprint: route_account_control_fingerprint(&account),
        request_started_at: format_utc_iso_precise(request_started_at_utc),
        observed_at,
        health_before,
        health_after,
        capabilities,
        sticky,
        invoke_id: invoke_id.map(ToOwned::to_owned),
        reason_code: None,
        action: can_recover.then(|| UPSTREAM_ACCOUNT_ACTION_ROUTE_RECOVERED.to_string()),
        update_latest_action: can_recover,
        http_status: None,
        unsupported_model: None,
        model,
        memory_error: None,
    };
    let acknowledgement = runtime.append(event)?;
    drop(gate);
    confirm(state, &runtime, acknowledgement).await?;
    complete_runtime_priority_handoff(_attempt_id, invoke_id, can_recover && model_recovered);
    if can_recover {
        publish_pool_routing_availability(state);
    }
    Ok(can_recover)
}

#[expect(
    clippy::too_many_arguments,
    reason = "Sticky facts retain causal identity and scope."
)]
async fn build_sticky_fact(
    state: &AppState,
    runtime: &RuntimeRouteFacts,
    sticky_key: Option<&str>,
    model: Option<&str>,
    expected: Option<i64>,
    account_id: Option<i64>,
    observed_at: &str,
    cause_account: i64,
    prompt_cache_key: Option<&str>,
    invoke_id: Option<&str>,
    request_started: Option<DateTime<Utc>>,
) -> Result<Option<RuntimeStickyFact>> {
    let Some(key) = sticky_key else {
        return Ok(None);
    };
    let (route, mut generation, exact) =
        sticky::load_projection(state, Some(runtime), key, model, true).await?;
    if expected.is_some_and(|expected| generation != expected) {
        return Ok(None);
    }
    let owner_before = route.as_ref().map(|route| route.account_id);
    if account_id.is_none()
        && (owner_before != Some(cause_account)
            || request_started.is_some_and(|started| {
                route
                    .as_ref()
                    .and_then(|route| parse_to_utc_datetime(&route.updated_at))
                    .is_some_and(|updated| updated > started)
            }))
    {
        return Ok(None);
    }
    let mut model_key = normalize_sticky_model_key(model);
    if account_id.is_none() && !exact && model_key.is_some() {
        generation = unpack_sticky_affinity_token(generation).0;
        model_key = None;
    }
    let changed =
        owner_before != account_id || (account_id.is_some() && model_key.is_some() && !exact);
    let generation_after = if changed {
        if model_key.is_some() {
            let (epoch, model_generation) = unpack_sticky_affinity_token(generation);
            pack_sticky_affinity_token(epoch, model_generation.saturating_add(1))
        } else {
            generation.saturating_add(1)
        }
    } else {
        generation
    };
    Ok(Some(RuntimeStickyFact {
        sticky_key: key.to_string(),
        model_key,
        generation_before: generation,
        generation_after,
        account_id,
        owner_before,
        prompt_cache_key: prompt_cache_key.map(ToOwned::to_owned),
        attempt_public_id: invoke_id
            .and_then(|invoke| {
                state
                    .proxy_runtime_attempts
                    .list_for_invocation(invoke, None)
                    .into_iter()
                    .rev()
                    .find(|attempt| attempt.upstream_account_id == cause_account)
            })
            .map(|attempt| attempt.attempt_public_id),
        routing_source: invoke_id
            .and_then(|invoke| {
                state
                    .proxy_runtime_attempts
                    .list_for_invocation(invoke, None)
                    .into_iter()
                    .rev()
                    .find(|attempt| attempt.upstream_account_id == cause_account)
            })
            .and_then(|attempt| attempt.routing_source),
        created_at: route.map_or_else(|| observed_at.to_string(), |route| route.created_at),
    }))
}

#[expect(
    clippy::too_many_arguments,
    reason = "Runtime adapters preserve the established proxy route failure contract."
)]
pub(crate) async fn record_runtime_pool_route_http_failure(
    state: &AppState,
    account_id: i64,
    account_kind: &str,
    single_account_rotation_enabled: bool,
    sticky_key: Option<&str>,
    status: StatusCode,
    error_message: &str,
    invoke_id: Option<&str>,
    endpoint: &str,
    image_intent: ImageIntent,
    _attempt_id: Option<i64>,
    sticky_affinity_generation: Option<i64>,
    prompt_cache_key: Option<&str>,
    request_model: Option<&str>,
) -> Result<()> {
    let runtime = runtime_route_facts(state).await?;
    let gate = runtime.admission_gate.lock().await;
    let account = load_account_projection(state, Some(&runtime), account_id, true)
        .await?
        .context("route account not found")?;
    if runtime_request_control_is_stale(state, &account, invoke_id) {
        return Ok(());
    }
    let health_before = RuntimeAccountHealth::from_row(&account);
    let now = Utc::now();
    let started = runtime_request_started_at(state, account_id, invoke_id).unwrap_or(now);
    let observed_at = format_utc_iso_precise(now);
    let classification = classify_pool_account_http_failure(account_kind, status, error_message);
    let enabled = account_status_change_reason_is_enabled(
        &state.pool,
        account_id,
        classification.reason_code,
    )
    .await?;
    let overload = route_http_failure_is_retryable_responses_overload(status, error_message);
    let hard_failure = classification.disposition
        == UpstreamAccountFailureDisposition::HardUnavailable
        && !overload;
    let explicit_model =
        is_explicit_model_failure_for_model(status, Some(error_message), request_model);
    let changes_account = enabled
        && (account_kind != UPSTREAM_ACCOUNT_KIND_API_KEY_CODEX
            || (hard_failure && !explicit_model));
    let health_after = changes_account.then(|| {
        if hard_failure {
            RuntimeAccountHealth {
                status: classification
                    .next_account_status
                    .unwrap_or(UPSTREAM_ACCOUNT_STATUS_ERROR)
                    .to_string(),
                last_error_at: Some(observed_at.clone()),
                last_route_failure_at: Some(observed_at.clone()),
                last_route_failure_kind: Some(classification.failure_kind.to_string()),
                cooldown_until: None,
                consecutive_route_failures: account.consecutive_route_failures.saturating_add(1),
                temporary_route_failure_streak_started_at: None,
            }
        } else {
            let continuing = account.consecutive_route_failures > 0
                && route_failure_kind_is_temporary(account.last_route_failure_kind.as_deref());
            let count = if continuing {
                account.consecutive_route_failures.max(0).saturating_add(1)
            } else {
                1
            };
            let started = continuing
                .then(|| {
                    account
                        .temporary_route_failure_streak_started_at
                        .as_deref()
                        .or(account.last_route_failure_at.as_deref())
                        .and_then(parse_to_utc_datetime)
                })
                .flatten()
                .unwrap_or(now);
            let cooling = count >= POOL_ROUTE_TEMPORARY_FAILURE_STREAK_THRESHOLD
                || now.signed_duration_since(started).num_seconds()
                    >= POOL_ROUTE_TEMPORARY_FAILURE_DEGRADED_WINDOW_SECS;
            let base = if status == StatusCode::TOO_MANY_REQUESTS {
                15
            } else {
                5
            };
            let seconds = (base * (1_i64 << (count - 1).clamp(0, 5) as u32))
                .min(POOL_ROUTE_TEMPORARY_FAILURE_COOLDOWN_MAX_SECS);
            RuntimeAccountHealth {
                status: if account_kind == UPSTREAM_ACCOUNT_KIND_OAUTH_CODEX
                    && classification.failure_kind
                        == FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED
                {
                    UPSTREAM_ACCOUNT_STATUS_ERROR
                } else {
                    UPSTREAM_ACCOUNT_STATUS_ACTIVE
                }
                .to_string(),
                last_error_at: Some(observed_at.clone()),
                last_route_failure_at: Some(observed_at.clone()),
                last_route_failure_kind: Some(classification.failure_kind.to_string()),
                cooldown_until: cooling
                    .then(|| format_utc_iso_precise(now + ChronoDuration::seconds(seconds))),
                consecutive_route_failures: count,
                temporary_route_failure_streak_started_at: Some(format_utc_iso_precise(started)),
            }
        }
    });
    let requirements =
        RequestCapabilityRequirements::from_endpoint_and_image_intent(endpoint, image_intent);
    let axes = [
        (
            requirements.response_endpoint,
            UpstreamCapabilityAxis::ResponseEndpoint,
            classify_response_endpoint_capability_observation(status, Some(error_message)),
        ),
        (
            requirements.chat_completions_endpoint,
            UpstreamCapabilityAxis::ChatCompletionsEndpoint,
            classify_chat_completions_capability_observation(status, Some(error_message)),
        ),
        (
            requirements.image_endpoint,
            UpstreamCapabilityAxis::ImageEndpoint,
            classify_image_endpoint_capability_observation(status, Some(error_message)),
        ),
        (
            requirements.response_image_tool,
            UpstreamCapabilityAxis::ResponseImageTool,
            classify_response_image_tool_capability_observation(status, Some(error_message)),
        ),
        (
            requirements.standalone_search && account_kind == UPSTREAM_ACCOUNT_KIND_API_KEY_CODEX,
            UpstreamCapabilityAxis::StandaloneSearch,
            classify_standalone_search_capability_observation(status, Some(error_message)),
        ),
    ];
    let capabilities = axes
        .into_iter()
        .filter(|(required, _, support)| *required && *support == CapabilitySupport::Unsupported)
        .map(|(_, axis, _)| RuntimeCapabilityFact {
            axis,
            support: "unsupported".to_string(),
            observed_at: observed_at.clone(),
            reason: classification.reason_code.to_string(),
        })
        .collect();
    let clears_sticky = changes_account
        && ((hard_failure && is_scope_permission_error_message(error_message))
            || (single_account_rotation_enabled && status == StatusCode::TOO_MANY_REQUESTS));
    let sticky = if clears_sticky {
        build_sticky_fact(
            state,
            &runtime,
            sticky_key,
            request_model,
            sticky_affinity_generation,
            None,
            &observed_at,
            account_id,
            prompt_cache_key,
            invoke_id,
            runtime_request_started_at(state, account_id, invoke_id),
        )
        .await?
    } else {
        None
    };
    let model = model::outcome_fact(
        state,
        &runtime,
        &account,
        request_model,
        started,
        Some((
            classification.failure_kind,
            explicit_model
                || status == StatusCode::TOO_MANY_REQUESTS
                || status.is_server_error()
                || overload,
        )),
        now,
    )
    .await?;
    defer_priority_handoff_failure_for_attempt_or_invoke(
        _attempt_id,
        invoke_id,
        overload || status == StatusCode::TOO_MANY_REQUESTS || status.is_server_error(),
    );
    let event = RuntimeRouteEvent {
        sequence: 0,
        event_id: String::new(),
        account_id,
        control_fingerprint: route_account_control_fingerprint(&account),
        request_started_at: format_utc_iso_precise(started),
        observed_at,
        health_before,
        health_after: health_after.clone(),
        capabilities,
        sticky,
        invoke_id: invoke_id.map(ToOwned::to_owned),
        reason_code: Some(classification.reason_code.to_string()),
        action: Some(
            if changes_account {
                if hard_failure {
                    UPSTREAM_ACCOUNT_ACTION_ROUTE_HARD_UNAVAILABLE
                } else {
                    if health_after
                        .as_ref()
                        .is_some_and(|health| health.cooldown_until.is_some())
                    {
                        UPSTREAM_ACCOUNT_ACTION_ROUTE_COOLDOWN_STARTED
                    } else {
                        UPSTREAM_ACCOUNT_ACTION_ROUTE_RETRYABLE_FAILURE
                    }
                }
            } else {
                UPSTREAM_ACCOUNT_ACTION_STATUS_CHANGE_SUPPRESSED
            }
            .to_string(),
        ),
        http_status: Some(i64::from(status.as_u16())),
        update_latest_action: changes_account,
        unsupported_model: (account_kind != UPSTREAM_ACCOUNT_KIND_API_KEY_CODEX)
            .then(|| extract_unsupported_model_from_route_error(status, error_message))
            .flatten(),
        model,
        memory_error: Some(error_message.to_string()),
    };
    let acknowledgement = runtime.append(event)?;
    drop(gate);
    confirm(state, &runtime, acknowledgement).await?;
    if clears_sticky && prompt_cache_key.is_some_and(|key| Some(key) == sticky_key) {
        broadcast_prompt_cache_conversation_changed(state, prompt_cache_key.expect("checked key"))
            .await;
    }
    Ok(())
}

pub(super) fn runtime_request_started_at(
    state: &AppState,
    account: i64,
    invoke: Option<&str>,
) -> Option<DateTime<Utc>> {
    state
        .proxy_runtime_attempts
        .list_for_invocation(invoke?, None)
        .into_iter()
        .rev()
        .find(|attempt| attempt.upstream_account_id == account)
        .and_then(|attempt| parse_to_utc_datetime(&attempt.started_at))
}

pub(super) fn runtime_request_control_is_stale(
    state: &AppState,
    account: &UpstreamAccountRow,
    invoke: Option<&str>,
) -> bool {
    invoke
        .and_then(|invoke| {
            state
                .proxy_runtime_attempts
                .list_for_invocation(invoke, None)
                .into_iter()
                .rev()
                .find(|attempt| attempt.upstream_account_id == account.id)
        })
        .and_then(|attempt| attempt.routing_control_fingerprint)
        .is_some_and(|fingerprint| fingerprint != route_account_control_fingerprint(account))
}

#[expect(
    clippy::too_many_arguments,
    reason = "Established proxy transport outcome contract."
)]
pub(crate) async fn record_runtime_pool_route_transport_failure(
    state: &AppState,
    account_id: i64,
    sticky_key: Option<&str>,
    message: &str,
    failure_kind: &str,
    invoke_id: Option<&str>,
    attempt_id: Option<i64>,
    model: Option<&str>,
) -> Result<()> {
    record_runtime_temporary_failure(
        state,
        account_id,
        sticky_key,
        message,
        failure_kind,
        UPSTREAM_ACCOUNT_ACTION_REASON_TRANSPORT_FAILURE,
        invoke_id,
        attempt_id,
        model,
    )
    .await
}

pub(crate) async fn record_runtime_pool_route_overload_failure(
    state: &AppState,
    account_id: i64,
    sticky_key: Option<&str>,
    message: &str,
    invoke_id: Option<&str>,
    attempt_id: Option<i64>,
    model: Option<&str>,
) -> Result<()> {
    record_runtime_temporary_failure(
        state,
        account_id,
        sticky_key,
        message,
        PROXY_FAILURE_UPSTREAM_RESPONSE_FAILED,
        UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_SERVER_OVERLOADED,
        invoke_id,
        attempt_id,
        model,
    )
    .await
}

#[expect(
    clippy::too_many_arguments,
    reason = "Temporary route failure reducer carries the existing evidence fields."
)]
async fn record_runtime_temporary_failure(
    state: &AppState,
    account_id: i64,
    _sticky_key: Option<&str>,
    message: &str,
    kind: &str,
    reason: &str,
    invoke_id: Option<&str>,
    attempt_id: Option<i64>,
    requested_model: Option<&str>,
) -> Result<()> {
    let runtime = runtime_route_facts(state).await?;
    let gate = runtime.admission_gate.lock().await;
    let account = load_account_projection(state, Some(&runtime), account_id, true)
        .await?
        .context("route account not found")?;
    if runtime_request_control_is_stale(state, &account, invoke_id) {
        return Ok(());
    }
    let now = Utc::now();
    let started = runtime_request_started_at(state, account_id, invoke_id).unwrap_or(now);
    let observed_at = format_utc_iso_precise(now);
    let enabled = account_status_change_reason_is_enabled(&state.pool, account_id, reason).await?;
    let health_before = RuntimeAccountHealth::from_row(&account);
    let health_after =
        (enabled && account.kind != UPSTREAM_ACCOUNT_KIND_API_KEY_CODEX).then(|| {
            let continuing = account.consecutive_route_failures > 0
                && route_failure_kind_is_temporary(account.last_route_failure_kind.as_deref());
            let count = if continuing {
                account.consecutive_route_failures.saturating_add(1)
            } else {
                1
            };
            let start = if continuing {
                account
                    .temporary_route_failure_streak_started_at
                    .as_deref()
                    .or(account.last_route_failure_at.as_deref())
                    .and_then(parse_to_utc_datetime)
                    .unwrap_or(now)
            } else {
                now
            };
            let cooling = count >= POOL_ROUTE_TEMPORARY_FAILURE_STREAK_THRESHOLD
                || now.signed_duration_since(start).num_seconds()
                    >= POOL_ROUTE_TEMPORARY_FAILURE_DEGRADED_WINDOW_SECS;
            RuntimeAccountHealth {
                status: UPSTREAM_ACCOUNT_STATUS_ACTIVE.to_string(),
                last_error_at: Some(observed_at.clone()),
                last_route_failure_at: Some(observed_at.clone()),
                last_route_failure_kind: Some(
                    if account.kind == UPSTREAM_ACCOUNT_KIND_OAUTH_CODEX {
                        PROXY_FAILURE_FAILED_CONTACT_UPSTREAM.to_string()
                    } else {
                        kind.to_string()
                    },
                ),
                cooldown_until: cooling.then(|| {
                    format_utc_iso_precise(
                        now + ChronoDuration::seconds(
                            (5 * (1_i64 << (count - 1).clamp(0, 5) as u32))
                                .min(POOL_ROUTE_TEMPORARY_FAILURE_COOLDOWN_MAX_SECS),
                        ),
                    )
                }),
                consecutive_route_failures: count,
                temporary_route_failure_streak_started_at: Some(format_utc_iso_precise(start)),
            }
        });
    let model = model::outcome_fact(
        state,
        &runtime,
        &account,
        requested_model,
        started,
        Some((kind, true)),
        now,
    )
    .await?;
    let event = RuntimeRouteEvent {
        sequence: 0,
        event_id: String::new(),
        account_id,
        control_fingerprint: route_account_control_fingerprint(&account),
        request_started_at: format_utc_iso_precise(started),
        observed_at,
        health_before,
        health_after: health_after.clone(),
        capabilities: Vec::new(),
        sticky: None,
        invoke_id: invoke_id.map(ToOwned::to_owned),
        reason_code: Some(reason.to_string()),
        action: Some(
            if health_after
                .as_ref()
                .is_some_and(|health| health.cooldown_until.is_some())
            {
                UPSTREAM_ACCOUNT_ACTION_ROUTE_COOLDOWN_STARTED
            } else {
                UPSTREAM_ACCOUNT_ACTION_ROUTE_RETRYABLE_FAILURE
            }
            .to_string(),
        ),
        http_status: Some(
            if reason == UPSTREAM_ACCOUNT_ACTION_REASON_TRANSPORT_FAILURE {
                502
            } else {
                200
            },
        ),
        update_latest_action: enabled && account.kind != UPSTREAM_ACCOUNT_KIND_API_KEY_CODEX,
        unsupported_model: None,
        model,
        memory_error: Some(message.to_string()),
    };
    defer_priority_handoff_failure_for_attempt_or_invoke(attempt_id, invoke_id, true);
    let acknowledgement = runtime.append(event)?;
    drop(gate);
    confirm(state, &runtime, acknowledgement).await?;
    Ok(())
}

pub(super) async fn confirm(
    state: &AppState,
    runtime: &RuntimeRouteFacts,
    acknowledgement: oneshot::Receiver<std::result::Result<(), String>>,
) -> Result<()> {
    acknowledgement
        .await
        .context("route journal confirmation cancelled")?
        .map_err(anyhow::Error::msg)?;
    #[cfg(test)]
    if state.sqlite_batch_writer.auto_flush_terminal_for_test() {
        runtime.materialize_batch(&state.pool).await?;
    }
    #[cfg(not(test))]
    let _ = (state, runtime);
    Ok(())
}
