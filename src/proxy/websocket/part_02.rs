struct WebSocketUpstreamPreparationRequest<'a> {
    state: Arc<AppState>,
    proxy_request_id: u64,
    original_uri: &'a Uri,
    headers: &'a HeaderMap,
    runtime_timeouts: &'a PoolRoutingTimeoutSettingsResolved,
    sticky_key: Option<&'a str>,
    requested_model: Option<&'a str>,
    prompt_cache_key: Option<&'a str>,
    binding_constraint: Option<PromptCacheConversationBindingConstraint>,
    conversation_override: Option<ConversationRoutingOverride>,
    owner_auto_guard_active: bool,
    trace: &'a PoolUpstreamAttemptTraceContext,
    required_subprotocol: Option<&'a str>,
}

struct EncryptedSessionOwnerWsPrepareErrorRequest<'a> {
    state: Arc<AppState>,
    trace: &'a PoolUpstreamAttemptTraceContext,
    prompt_cache_key: Option<&'a str>,
    no_candidate_audit: Option<&'a PoolRoutingNoCandidateAudit>,
    retry_account_count: usize,
    distinct_account_count: usize,
}

struct WebSocketAccountResolutionRequest<'a> {
    state: Arc<AppState>,
    original_uri: &'a Uri,
    sticky_key: Option<&'a str>,
    requested_model: Option<&'a str>,
    excluded_account_ids: &'a [i64],
    excluded_upstream_route_keys: &'a HashSet<String>,
    binding_constraint: Option<&'a PromptCacheConversationBindingConstraint>,
    conversation_override: Option<&'a ConversationRoutingOverride>,
    owner_auto_guard_active: bool,
    trace: &'a PoolUpstreamAttemptTraceContext,
    prompt_cache_key: Option<&'a str>,
    reservation_key: &'a str,
    retry_account_count: usize,
    distinct_account_count: usize,
    last_failure: Option<&'a WsAttemptFailure>,
}

struct WebSocketAccountQueryRequest<'a> {
    state: &'a AppState,
    original_uri: &'a Uri,
    sticky_key: Option<&'a str>,
    requested_model: Option<&'a str>,
    excluded_account_ids: &'a [i64],
    excluded_upstream_route_keys: &'a HashSet<String>,
    binding_constraint: Option<&'a PromptCacheConversationBindingConstraint>,
    conversation_override: Option<&'a ConversationRoutingOverride>,
    reservation_key: &'a str,
}

struct WebSocketNoCandidateErrorRequest<'a> {
    context: WebSocketResolutionContext<'a>,
    audit: PoolRoutingNoCandidateAudit,
}

struct WebSocketResolutionContext<'a> {
    state: Arc<AppState>,
    trace: &'a PoolUpstreamAttemptTraceContext,
    prompt_cache_key: Option<&'a str>,
    owner_auto_guard_active: bool,
    retry_account_count: usize,
    distinct_account_count: usize,
    last_failure: Option<&'a WsAttemptFailure>,
}

enum WebSocketAccountAttemptPreparation {
    Prepared(Box<PreparedUpstreamWebSocket>),
    Retry(WsAttemptFailure),
}

struct WebSocketAccountAttemptPreparationRequest<'a> {
    state: Arc<AppState>,
    original_uri: &'a Uri,
    headers: &'a HeaderMap,
    trace: &'a PoolUpstreamAttemptTraceContext,
    prompt_cache_key: Option<&'a str>,
    reservation_guard: PoolRoutingReservationGuard,
    account: PoolResolvedAccount,
    attempt_index: usize,
    required_subprotocol: Option<&'a str>,
}

async fn encrypted_session_owner_ws_prepare_error(
    request: EncryptedSessionOwnerWsPrepareErrorRequest<'_>,
) -> WsPrepareError {
    let EncryptedSessionOwnerWsPrepareErrorRequest {
        state,
        trace,
        prompt_cache_key,
        no_candidate_audit,
        retry_account_count,
        distinct_account_count,
    } = request;
    let err = build_encrypted_session_owner_unavailable_error(
        None,
        retry_account_count,
        distinct_account_count,
    );
    if let Some(audit) = no_candidate_audit {
        let _ = persist_pool_routing_no_candidate_invocation_with_error(
            state.clone(),
            trace,
            prompt_cache_key,
            audit,
            err.status,
            err.failure_kind,
            &err.message,
            err.attempt_summary.pool_attempt_count,
            err.attempt_summary.pool_distinct_account_count,
            err.attempt_summary
                .pool_attempt_terminal_reason
                .as_deref()
                .unwrap_or(err.failure_kind),
        )
        .await;
    }
    let _ = insert_and_broadcast_pool_upstream_terminal_attempt(
        state.as_ref(),
        trace,
        &err,
        (retry_account_count + 1) as i64,
        distinct_account_count as i64,
        PROXY_FAILURE_ENCRYPTED_SESSION_OWNER_UNAVAILABLE,
    )
    .await;
    WsPrepareError {
        status: err.status,
        message: err.message,
    }
}

async fn encrypted_session_owner_ws_resolution_error(
    state: Arc<AppState>,
    trace: &PoolUpstreamAttemptTraceContext,
    prompt_cache_key: Option<&str>,
    owner_auto_guard_active: bool,
    retry_account_count: usize,
    distinct_account_count: usize,
) -> Option<WsPrepareError> {
    if !owner_auto_guard_active {
        return None;
    }
    Some(
        encrypted_session_owner_ws_prepare_error(EncryptedSessionOwnerWsPrepareErrorRequest {
            state,
            trace,
            prompt_cache_key,
            no_candidate_audit: None,
            retry_account_count,
            distinct_account_count,
        })
        .await,
    )
}

async fn websocket_no_candidate_prepare_error(
    request: WebSocketNoCandidateErrorRequest<'_>,
) -> WsPrepareError {
    let WebSocketNoCandidateErrorRequest {
        context:
            WebSocketResolutionContext {
                state,
                trace,
                prompt_cache_key,
                owner_auto_guard_active,
                retry_account_count,
                distinct_account_count,
                last_failure,
            },
        audit,
    } = request;
    if owner_auto_guard_active {
        return encrypted_session_owner_ws_prepare_error(
            EncryptedSessionOwnerWsPrepareErrorRequest {
                state,
                trace,
                prompt_cache_key,
                no_candidate_audit: Some(&audit),
                retry_account_count,
                distinct_account_count,
            },
        )
        .await;
    }
    if let Some(failure) = last_failure {
        let _ = persist_pool_routing_no_candidate_invocation_with_error(
            state.clone(),
            trace,
            prompt_cache_key,
            &audit,
            failure.status,
            failure.failure_kind,
            &failure.message,
            retry_account_count,
            distinct_account_count,
            failure.failure_kind,
        )
        .await;
    } else {
        let _ = persist_pool_routing_no_candidate_invocation(
            state.clone(),
            trace,
            prompt_cache_key,
            &audit,
        )
        .await;
    }
    WsPrepareError {
        status: last_failure
            .map(|failure| failure.status)
            .unwrap_or(StatusCode::SERVICE_UNAVAILABLE),
        message: last_failure
            .map(|failure| failure.message.clone())
            .unwrap_or_else(|| POOL_NO_AVAILABLE_ACCOUNT_MESSAGE.to_string()),
    }
}

async fn websocket_resolution_error(
    WebSocketResolutionContext {
        state,
        trace,
        prompt_cache_key,
        owner_auto_guard_active,
        retry_account_count,
        distinct_account_count,
        last_failure,
    }: WebSocketResolutionContext<'_>,
    use_last_failure: bool,
    fallback_status: StatusCode,
    fallback_message: String,
) -> WsPrepareError {
    if let Some(err) = encrypted_session_owner_ws_resolution_error(
        state,
        trace,
        prompt_cache_key,
        owner_auto_guard_active,
        retry_account_count,
        distinct_account_count,
    )
    .await
    {
        return err;
    }
    let failure_status = use_last_failure
        .then_some(last_failure)
        .flatten()
        .map(|failure| failure.status)
        .unwrap_or(fallback_status);
    let failure_message = use_last_failure
        .then(|| last_failure.map(|failure| failure.message.clone()))
        .flatten()
        .unwrap_or(fallback_message);
    WsPrepareError {
        status: failure_status,
        message: failure_message,
    }
}

async fn query_websocket_account(
    request: WebSocketAccountQueryRequest<'_>,
) -> Result<PoolAccountResolutionWithWait> {
    let WebSocketAccountQueryRequest {
        state,
        original_uri,
        sticky_key,
        requested_model,
        excluded_account_ids,
        excluded_upstream_route_keys,
        binding_constraint,
        conversation_override,
        reservation_key,
    } = request;
    let mut no_available_wait_deadline = None;
    resolve_pool_account_for_request_with_wait_and_binding_constraint_with_image_intent_and_override_and_reservation(
        PoolAccountWaitRequest {
            state,
            sticky_key,
            requested_model,
            excluded_ids: excluded_account_ids,
            excluded_upstream_route_keys,
            required_upstream_route_key: None,
            binding_constraint,
            conversation_override,
            wait_for_no_available: true,
            wait_deadline: &mut no_available_wait_deadline,
            total_timeout_deadline: None,
            endpoint: original_uri.path(),
            image_intent: crate::ImageIntent::Unknown,
            codex_imagegen_request: false,
            reservation_key: Some(reservation_key),
        },
    )
    .await
}

async fn websocket_assigned_blocked_error(
    context: WebSocketResolutionContext<'_>,
    blocked: PoolAssignedBlockedAccount,
) -> WsPrepareError {
    let WebSocketResolutionContext {
        state,
        trace,
        prompt_cache_key,
        owner_auto_guard_active,
        retry_account_count,
        distinct_account_count,
        ..
    } = context;
    if let Some(err) = encrypted_session_owner_ws_resolution_error(
        state.clone(),
        trace,
        prompt_cache_key,
        owner_auto_guard_active,
        retry_account_count,
        distinct_account_count,
    )
    .await
    {
        return err;
    }
    let terminal = ViaPoolResolutionTerminalError::assigned_blocked(blocked);
    terminal
        .persist_if_needed(state.as_ref(), Some(trace))
        .await;
    WsPrepareError {
        status: terminal.status,
        message: terminal.message,
    }
}

async fn prepare_websocket_account_attempt(
    request: WebSocketAccountAttemptPreparationRequest<'_>,
) -> Result<WebSocketAccountAttemptPreparation, WsPrepareError> {
    let WebSocketAccountAttemptPreparationRequest {
        state,
        original_uri,
        headers,
        trace,
        prompt_cache_key,
        reservation_guard,
        account,
        attempt_index,
        required_subprotocol,
    } = request;
    match account_supports_upstream_websocket(state.as_ref(), &account).await {
        Ok(true) => {}
        Ok(false) => {
            return Ok(WebSocketAccountAttemptPreparation::Retry(
                WsAttemptFailure {
                    status: StatusCode::SERVICE_UNAVAILABLE,
                    message: "selected upstream account is tagged as not supporting websocket"
                        .to_string(),
                    failure_kind: PROXY_FAILURE_FAILED_CONTACT_UPSTREAM,
                    retryable: true,
                    account_id: Some(account.account_id),
                    upstream_route_key: Some(account.upstream_route_key()),
                },
            ));
        }
        Err(err) => {
            return Err(WsPrepareError {
                status: StatusCode::BAD_GATEWAY,
                message: format!("failed to inspect websocket support tag: {err}"),
            });
        }
    }
    let runtime_timeouts = &load_effective_request_path_timeouts_for_account(
        &state.pool,
        &state.config,
        account.account_id,
        prompt_cache_key,
    )
    .await
    .map_err(|err| WsPrepareError {
        status: StatusCode::BAD_GATEWAY,
        message: format!("failed to resolve effective request-path timeouts: {err}"),
    })?
    .2;
    match prepare_single_upstream_websocket_attempt(WebSocketSingleAttemptRequest {
        state,
        original_uri,
        headers,
        runtime_timeouts,
        trace,
        prompt_cache_key,
        reservation_guard,
        account,
        attempt_index,
        required_subprotocol,
    })
    .await
    {
        Ok(prepared) => Ok(WebSocketAccountAttemptPreparation::Prepared(Box::new(
            prepared,
        ))),
        Err(failure) if failure.retryable => Ok(WebSocketAccountAttemptPreparation::Retry(failure)),
        Err(failure) => Err(WsPrepareError {
            status: failure.status,
            message: failure.message,
        }),
    }
}

async fn query_websocket_account_with_context(
    request: WebSocketAccountResolutionRequest<'_>,
) -> (
    Result<PoolAccountResolutionWithWait>,
    WebSocketResolutionContext<'_>,
) {
    let WebSocketAccountResolutionRequest {
        state,
        original_uri,
        sticky_key,
        requested_model,
        excluded_account_ids,
        excluded_upstream_route_keys,
        binding_constraint,
        conversation_override,
        owner_auto_guard_active,
        trace,
        prompt_cache_key,
        reservation_key,
        retry_account_count,
        distinct_account_count,
        last_failure,
    } = request;
    let resolution = query_websocket_account(WebSocketAccountQueryRequest {
        state: state.as_ref(),
        original_uri,
        sticky_key,
        requested_model,
        excluded_account_ids,
        excluded_upstream_route_keys,
        binding_constraint,
        conversation_override,
        reservation_key,
    })
    .await;
    let context = WebSocketResolutionContext {
        state,
        trace,
        prompt_cache_key,
        owner_auto_guard_active,
        retry_account_count,
        distinct_account_count,
        last_failure,
    };
    (resolution, context)
}

async fn ensure_upstream_websocket_enabled(state: &AppState) -> Result<(), WsPrepareError> {
    if state
        .proxy_model_settings
        .read()
        .await
        .upstream_websocket_default_enabled
    {
        return Ok(());
    }
    Err(WsPrepareError {
        status: StatusCode::SERVICE_UNAVAILABLE,
        message: format!(
            "upstream websocket transport is disabled in Settings; enable it or set {ENV_OPENAI_PROXY_UPSTREAM_WEBSOCKET_DEFAULT_ENABLED}=true before first startup"
        ),
    })
}

fn register_retryable_websocket_failure(
    failure: WsAttemptFailure,
    retry_account_ids: &mut HashSet<i64>,
    excluded_account_ids: &mut Vec<i64>,
    excluded_upstream_route_keys: &mut HashSet<String>,
    last_failure: &mut Option<WsAttemptFailure>,
    proxy_request_id: u64,
) -> Result<(), WsPrepareError> {
    if let Some(account_id) = failure.account_id {
        retry_account_ids.insert(account_id);
    }
    if let Err(err) = exclude_retryable_ws_attempt_failure(
        &failure,
        excluded_account_ids,
        excluded_upstream_route_keys,
    ) {
        warn!(
            proxy_request_id,
            error = %failure.message,
            "websocket upstream attempt failed without account exclusion context"
        );
        return Err(err);
    }
    *last_failure = Some(failure);
    Ok(())
}

async fn resolve_websocket_account(
    request: WebSocketAccountResolutionRequest<'_>,
) -> Result<PoolResolvedAccount, WsPrepareError> {
    let (resolution, context) = query_websocket_account_with_context(request).await;
    match resolution {
        Ok(PoolAccountResolutionWithWait::Resolution(PoolAccountResolution::Resolved(account))) => {
            Ok(account)
        }
        Ok(PoolAccountResolutionWithWait::Resolution(PoolAccountResolution::NoCandidate(
            audit,
        ))) => Err(
            websocket_no_candidate_prepare_error(WebSocketNoCandidateErrorRequest {
                context,
                audit,
            })
            .await,
        ),
        Ok(PoolAccountResolutionWithWait::Resolution(PoolAccountResolution::Unavailable)) => {
            Err(websocket_resolution_error(
                context,
                true,
                StatusCode::SERVICE_UNAVAILABLE,
                POOL_NO_AVAILABLE_ACCOUNT_MESSAGE.to_string(),
            )
            .await)
        }
        Ok(PoolAccountResolutionWithWait::Resolution(PoolAccountResolution::RateLimited)) => {
            Err(websocket_resolution_error(
                context,
                false,
                StatusCode::TOO_MANY_REQUESTS,
                POOL_ALL_ACCOUNTS_RATE_LIMITED_MESSAGE.to_string(),
            )
            .await)
        }
        Ok(PoolAccountResolutionWithWait::Resolution(PoolAccountResolution::DegradedOnly)) => {
            Err(websocket_resolution_error(
                context,
                false,
                StatusCode::SERVICE_UNAVAILABLE,
                POOL_ALL_ACCOUNTS_DEGRADED_MESSAGE.to_string(),
            )
            .await)
        }
        Ok(PoolAccountResolutionWithWait::Resolution(PoolAccountResolution::AssignedBlocked(
            blocked,
        ))) => Err(websocket_assigned_blocked_error(context, blocked).await),
        Ok(PoolAccountResolutionWithWait::Resolution(PoolAccountResolution::BlockedByPolicy(
            message,
        ))) => Err(websocket_resolution_error(
            context,
            false,
            StatusCode::SERVICE_UNAVAILABLE,
            message,
        )
        .await),
        Ok(PoolAccountResolutionWithWait::TotalTimeoutExpired) => Err(WsPrepareError {
            status: StatusCode::GATEWAY_TIMEOUT,
            message: "pool websocket account selection timed out".to_string(),
        }),
        Err(err) => Err(WsPrepareError {
            status: StatusCode::BAD_GATEWAY,
            message: format!("failed to resolve pool account: {err}"),
        }),
    }
}

async fn prepare_upstream_websocket(
    request: WebSocketUpstreamPreparationRequest<'_>,
) -> Result<PreparedUpstreamWebSocket, WsPrepareError> {
    let WebSocketUpstreamPreparationRequest {
        state,
        proxy_request_id,
        original_uri,
        headers,
        runtime_timeouts: _runtime_timeouts,
        sticky_key,
        requested_model,
        prompt_cache_key,
        binding_constraint,
        conversation_override,
        owner_auto_guard_active,
        trace,
        required_subprotocol,
    } = request;
    let mut excluded_account_ids = Vec::new();
    let mut excluded_upstream_route_keys = HashSet::new();
    let mut ws_retry_account_ids = HashSet::new();
    let mut last_failure: Option<WsAttemptFailure> = None;
    let reservation_key = build_pool_routing_reservation_key(proxy_request_id);
    ensure_upstream_websocket_enabled(state.as_ref()).await?;

    loop {
        let distinct_account_count = ws_retry_account_ids.len();
        if distinct_account_count >= POOL_UPSTREAM_MAX_DISTINCT_ACCOUNTS {
            return Err(WsPrepareError {
                status: last_failure
                    .as_ref()
                    .map(|failure| failure.status)
                    .unwrap_or(StatusCode::BAD_GATEWAY),
                message: last_failure
                    .map(|failure| failure.message)
                    .unwrap_or_else(|| "pool distinct-account retry budget exhausted".to_string()),
            });
        }

        let account = resolve_websocket_account(WebSocketAccountResolutionRequest {
            state: state.clone(),
            original_uri,
            sticky_key,
            requested_model,
            excluded_account_ids: &excluded_account_ids,
            excluded_upstream_route_keys: &excluded_upstream_route_keys,
            binding_constraint: binding_constraint.as_ref(),
            conversation_override: conversation_override.as_ref(),
            owner_auto_guard_active,
            trace,
            prompt_cache_key,
            reservation_key: &reservation_key,
            retry_account_count: ws_retry_account_ids.len(),
            distinct_account_count,
            last_failure: last_failure.as_ref(),
        })
        .await?;
        let reservation_guard =
            PoolRoutingReservationGuard::new(state.clone(), reservation_key.clone());
        match prepare_websocket_account_attempt(WebSocketAccountAttemptPreparationRequest {
            state: state.clone(),
            original_uri,
            headers,
            trace,
            prompt_cache_key,
            reservation_guard,
            account,
            attempt_index: ws_retry_account_ids.len() + 1,
            required_subprotocol,
        })
        .await
        {
            Ok(WebSocketAccountAttemptPreparation::Prepared(prepared)) => return Ok(*prepared),
            Ok(WebSocketAccountAttemptPreparation::Retry(failure)) => {
                register_retryable_websocket_failure(
                    failure,
                    &mut ws_retry_account_ids,
                    &mut excluded_account_ids,
                    &mut excluded_upstream_route_keys,
                    &mut last_failure,
                    proxy_request_id,
                )?;
            }
            Err(failure) => {
                return Err(WsPrepareError {
                    status: failure.status,
                    message: failure.message,
                });
            }
        }
    }
}

pub(crate) fn exclude_retryable_ws_attempt_failure(
    failure: &WsAttemptFailure,
    excluded_account_ids: &mut Vec<i64>,
    excluded_upstream_route_keys: &mut HashSet<String>,
) -> Result<(), WsPrepareError> {
    let Some(account_id) = failure.account_id else {
        return Err(WsPrepareError {
            status: failure.status,
            message: failure.message.clone(),
        });
    };

    excluded_account_ids.push(account_id);
    if let Some(route_key) = failure.upstream_route_key.as_deref() {
        excluded_upstream_route_keys.insert(route_key.to_string());
    }
    Ok(())
}

pub(crate) async fn account_supports_upstream_websocket(
    state: &AppState,
    account: &PoolResolvedAccount,
) -> Result<bool> {
    Ok(!account_has_websocket_unsupported_tag(&state.pool, account.account_id).await?)
}
