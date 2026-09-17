use super::*;

pub(crate) async fn health_check(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    if state.startup_ready.load(Ordering::Acquire) {
        (StatusCode::OK, "ok")
    } else {
        (StatusCode::SERVICE_UNAVAILABLE, "starting")
    }
}

#[cfg(test)]
pub(crate) async fn proxy_openai_v1(
    State(state): State<Arc<AppState>>,
    OriginalUri(original_uri): OriginalUri,
    method: Method,
    headers: HeaderMap,
    body: Body,
) -> Response {
    Box::pin(proxy_openai_v1_common(
        state,
        original_uri,
        method,
        headers,
        body,
        None,
        None,
    ))
    .await
}

pub(crate) struct ProxyOpenaiV1HandlerRequest {
    pub(crate) connect_info: Option<ConnectInfo<SocketAddr>>,
    pub(crate) downstream_transport: Option<Extension<DownstreamTransportObserver>>,
    pub(crate) ws: Option<WebSocketUpgrade>,
    pub(crate) original_uri: OriginalUri,
    pub(crate) method: Method,
    pub(crate) headers: HeaderMap,
    pub(crate) body: Body,
}

#[axum::async_trait]
impl FromRequest<Arc<AppState>> for ProxyOpenaiV1HandlerRequest {
    type Rejection = Infallible;

    async fn from_request(
        request: Request<Body>,
        state: &Arc<AppState>,
    ) -> Result<Self, Self::Rejection> {
        let (mut parts, body) = request.into_parts();
        let connect_info =
            Option::<ConnectInfo<SocketAddr>>::from_request_parts(&mut parts, state).await?;
        let downstream_transport =
            Option::<Extension<DownstreamTransportObserver>>::from_request_parts(&mut parts, state)
                .await?;
        let ws = Option::<WebSocketUpgrade>::from_request_parts(&mut parts, state).await?;
        let original_uri = OriginalUri::from_request_parts(&mut parts, state).await?;
        Ok(Self {
            connect_info,
            downstream_transport,
            ws,
            original_uri,
            method: parts.method,
            headers: parts.headers,
            body,
        })
    }
}

pub(crate) async fn proxy_openai_v1_with_connect_info(
    State(state): State<Arc<AppState>>,
    request: ProxyOpenaiV1HandlerRequest,
) -> Response {
    let ProxyOpenaiV1HandlerRequest {
        connect_info,
        downstream_transport,
        ws,
        original_uri: OriginalUri(original_uri),
        method,
        headers,
        body,
    } = request;
    if let Some(ws) = ws
        && is_websocket_upgrade_request(&headers)
    {
        let websocket_enabled = state.proxy_model_settings.read().await.websocket_enabled;
        if !websocket_enabled {
            let invoke_id = format!("proxy-ws-disabled-{}", Utc::now().timestamp_millis());
            return build_proxy_error_response(
                ProxyErrorResponse {
                    status: StatusCode::SERVICE_UNAVAILABLE,
                    message: format!(
                        "OpenAI proxy WebSocket support is disabled; enable it in Settings or set {ENV_OPENAI_PROXY_WEBSOCKET_ENABLED}=true before first startup"
                    ),
                    cvm_id: None,
                    retry_after_secs: None,
                    code: None,
                    blocked_binding: None,
                },
                &invoke_id,
            );
        }
        return proxy_openai_v1_ws_common(
            state,
            connect_info.map(|info| info.0.ip()),
            ws,
            original_uri,
            method,
            headers,
        )
        .await;
    }
    Box::pin(proxy_openai_v1_common(
        state,
        original_uri,
        method,
        headers,
        body,
        connect_info.map(|info| info.0.ip()),
        downstream_transport.map(|Extension(observer)| observer),
    ))
    .await
}

pub(crate) async fn proxy_openai_v1_common(
    state: Arc<AppState>,
    original_uri: Uri,
    method: Method,
    headers: HeaderMap,
    body: Body,
    peer_ip: Option<IpAddr>,
    downstream_transport: Option<DownstreamTransportObserver>,
) -> Response {
    let proxy_request_id = next_proxy_request_id();
    let started_at = Instant::now();
    let invoke_id = generate_unique_proxy_invoke_id(&state.pool).await;
    let method_for_log = method.clone();
    let uri_for_log = original_uri.clone();
    log_proxy_request_started(
        proxy_request_id,
        &method_for_log,
        &uri_for_log,
        &headers,
        peer_ip,
    );
    let target_url = match build_proxy_target_url(&state, &original_uri, &invoke_id) {
        Ok(url) => url,
        Err(response) => return response,
    };
    let proxy_request_permit = acquire_proxy_request_permit_for_request(
        state.as_ref(),
        proxy_request_id,
        &method_for_log,
        &uri_for_log,
    )
    .await;
    let capture_target = capture_target_for_request(original_uri.path(), &method);
    let transport_request_observer = downstream_transport
        .as_ref()
        .map(DownstreamTransportObserver::begin_request);
    let downstream_request_observer = capture_target
        .is_some()
        .then_some(transport_request_observer)
        .flatten();
    let admitted_runtime_snapshot = emit_admitted_proxy_runtime_snapshot(
        state.as_ref(),
        proxy_request_id,
        &invoke_id,
        capture_target,
        &headers,
        peer_ip,
    )
    .await;

    if let Some(response) = reject_missing_proxy_bearer(
        state.as_ref(),
        &invoke_id,
        admitted_runtime_snapshot.as_ref(),
        &headers,
    ) {
        return response;
    }

    let route_context_started = Instant::now();
    let runtime_timeouts = match resolve_proxy_route_context(RouteContextRequest {
        state: state.as_ref(),
        proxy_request_id,
        method: &method_for_log,
        uri: &uri_for_log,
        headers: &headers,
        invoke_id: &invoke_id,
        started_at,
        route_context_started,
        admitted_runtime_snapshot: admitted_runtime_snapshot.as_ref(),
    })
    .await
    {
        Ok(route_context) => route_context,
        Err(response) => return response,
    };
    debug!(
        proxy_request_id,
        route_context_elapsed = route_context_started.elapsed().as_millis() as u64,
        "proxy route context resolved"
    );
    let pool_route_active = true;

    let response = Box::pin(proxy_openai_v1_inner(ProxyOpenaiV1Request {
        state,
        proxy_request_id,
        invoke_id: invoke_id.clone(),
        original_uri,
        method,
        headers,
        body,
        target_url,
        peer_ip,
        pool_route_active,
        runtime_timeouts,
        proxy_request_permit,
        admitted_runtime_snapshot,
        downstream_request_observer,
        proxy_request_started_at: started_at,
    }))
    .await;
    finish_proxy_openai_v1_common(
        response,
        proxy_request_id,
        &method_for_log,
        &uri_for_log,
        started_at,
        &invoke_id,
    )
}

fn log_proxy_request_started(
    proxy_request_id: u64,
    method: &Method,
    uri: &Uri,
    headers: &HeaderMap,
    peer_ip: Option<IpAddr>,
) {
    let request_content_length = headers
        .get(header::CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<usize>().ok());
    info!(
        proxy_request_id,
        method = %method,
        uri = %uri,
        proxy_request_started = true,
        has_body = request_may_have_body(method, headers),
        content_length = ?request_content_length,
        peer_ip = ?peer_ip,
        "openai proxy request started"
    );
}

fn build_proxy_target_url(
    state: &AppState,
    original_uri: &Uri,
    invoke_id: &str,
) -> Result<Url, Response> {
    build_proxy_upstream_url(&state.config.openai_upstream_base_url, original_uri).map_err(|err| {
        let error_text = err.to_string();
        let status = if error_text.contains(PROXY_DOT_SEGMENT_PATH_NOT_ALLOWED)
            || error_text.contains(PROXY_INVALID_REQUEST_TARGET)
            || error_text.contains("failed to parse proxy upstream url")
        {
            StatusCode::BAD_REQUEST
        } else {
            StatusCode::INTERNAL_SERVER_ERROR
        };
        build_proxy_error_response(
            ProxyErrorResponse {
                status,
                message: format!("failed to build upstream url: {err}"),
                cvm_id: None,
                retry_after_secs: None,
                code: None,
                blocked_binding: None,
            },
            invoke_id,
        )
    })
}

async fn emit_admitted_proxy_runtime_snapshot(
    state: &AppState,
    proxy_request_id: u64,
    invoke_id: &str,
    capture_target: Option<ProxyCaptureTarget>,
    headers: &HeaderMap,
    peer_ip: Option<IpAddr>,
) -> Option<AdmittedProxyRuntimeSnapshot> {
    let target = capture_target?;
    let occurred_at = format_naive(Utc::now().with_timezone(&Shanghai).naive_local());
    let requester_ip = extract_requester_ip(headers, peer_ip);
    let sticky_key = extract_sticky_key_from_headers(headers);
    let prompt_cache_key = extract_prompt_cache_key_from_headers(headers);
    let shell_started = Instant::now();
    let record = build_admitted_proxy_capture_runtime_snapshot(
        invoke_id,
        &occurred_at,
        target,
        requester_ip.as_deref(),
        sticky_key.as_deref(),
        prompt_cache_key.as_deref(),
    );
    match persist_and_broadcast_proxy_capture_runtime_snapshot(state, record).await {
        Ok(()) => debug!(
            proxy_request_id,
            invoke_id,
            occurred_at = %occurred_at,
            running_shell_emitted = true,
            running_shell_emit_elapsed = shell_started.elapsed().as_millis() as u64,
            "admitted proxy request emitted running shell before route context"
        ),
        Err(err) => warn!(
            ?err,
            proxy_request_id,
            invoke_id,
            "failed to broadcast admitted running proxy capture snapshot"
        ),
    }
    Some(AdmittedProxyRuntimeSnapshot { occurred_at })
}

fn reject_missing_proxy_bearer(
    state: &AppState,
    invoke_id: &str,
    admitted_runtime_snapshot: Option<&AdmittedProxyRuntimeSnapshot>,
    headers: &HeaderMap,
) -> Option<Response> {
    extract_bearer_token(headers).is_none().then(|| {
        let err = ProxyErrorResponse {
            status: StatusCode::UNAUTHORIZED,
            message: PROXY_POOL_ROUTE_KEY_MISSING_OR_INVALID_MESSAGE.to_string(),
            cvm_id: None,
            retry_after_secs: None,
            code: None,
            blocked_binding: None,
        };
        if let Some(runtime_snapshot) = admitted_runtime_snapshot {
            terminalize_proxy_runtime_snapshot_with_error(
                state,
                invoke_id,
                &runtime_snapshot.occurred_at,
                err.status,
                PROXY_FAILURE_POOL_ROUTING_BLOCKED,
                &err.message,
                "missing_bearer_token",
            );
            schedule_dashboard_activity_live_snapshot(state);
        }
        build_proxy_error_response(err, invoke_id)
    })
}

struct RouteContextRequest<'a> {
    state: &'a AppState,
    proxy_request_id: u64,
    method: &'a Method,
    uri: &'a Uri,
    headers: &'a HeaderMap,
    invoke_id: &'a str,
    started_at: Instant,
    route_context_started: Instant,
    admitted_runtime_snapshot: Option<&'a AdmittedProxyRuntimeSnapshot>,
}

async fn resolve_proxy_route_context(
    request: RouteContextRequest<'_>,
) -> Result<PoolRoutingTimeoutSettingsResolved, Response> {
    match resolve_proxy_route_context_for_request(
        request.state,
        request.proxy_request_id,
        request.method,
        request.uri,
        request.headers,
    )
    .await
    {
        Ok(route_context) => Ok(route_context),
        Err(err) => {
            warn!(
                proxy_request_id = request.proxy_request_id,
                method = %request.method,
                uri = %request.uri,
                status = %err.status,
                error = %err.message,
                route_context_elapsed = request.route_context_started.elapsed().as_millis() as u64,
                elapsed_ms = request.started_at.elapsed().as_millis(),
                "openai proxy request failed during route validation"
            );
            if let Some(runtime_snapshot) = request.admitted_runtime_snapshot {
                terminalize_proxy_runtime_snapshot_with_error(
                    request.state,
                    request.invoke_id,
                    &runtime_snapshot.occurred_at,
                    err.status,
                    PROXY_FAILURE_POOL_ROUTING_BLOCKED,
                    &err.message,
                    "route_validation_failed",
                );
                schedule_dashboard_activity_live_snapshot(request.state);
            }
            Err(build_proxy_error_response(err, request.invoke_id))
        }
    }
}

fn finish_proxy_openai_v1_common(
    result: Result<Response, ProxyErrorResponse>,
    proxy_request_id: u64,
    method: &Method,
    uri: &Uri,
    started_at: Instant,
    invoke_id: &str,
) -> Response {
    match result {
        Ok(response) => {
            info!(
                proxy_request_id,
                method = %method,
                uri = %uri,
                status = %response.status(),
                elapsed_ms = started_at.elapsed().as_millis(),
                "openai proxy response headers ready"
            );
            response
        }
        Err(err) => {
            warn!(
                proxy_request_id,
                method = %method,
                uri = %uri,
                status = %err.status,
                error = %err.message,
                elapsed_ms = started_at.elapsed().as_millis(),
                "openai proxy request failed"
            );
            build_proxy_error_response(err, invoke_id)
        }
    }
}

async fn acquire_proxy_request_permit_for_request(
    state: &AppState,
    proxy_request_id: u64,
    method: &Method,
    uri: &Uri,
) -> Option<ProxyRequestConcurrencyPermit> {
    Some(acquire_proxy_request_concurrency_permit(state, proxy_request_id, method, uri).await)
}

#[derive(Debug, Clone)]
pub(crate) struct ProxyErrorResponse {
    pub(crate) status: StatusCode,
    pub(crate) message: String,
    pub(crate) cvm_id: Option<String>,
    pub(crate) retry_after_secs: Option<u64>,
    pub(crate) code: Option<String>,
    pub(crate) blocked_binding: Option<BlockedBindingDiagnostic>,
}

#[derive(Debug, Clone)]
pub(crate) struct ProxyErrorResponseEnvelope {
    pub(crate) status: StatusCode,
    pub(crate) body_text: String,
    pub(crate) retry_after: Option<String>,
    pub(crate) cvm_invoke_id: Option<String>,
}

impl ProxyErrorResponseEnvelope {
    pub(crate) fn into_response(self) -> Response {
        let mut response = Response::new(axum::body::Body::from(self.body_text));
        *response.status_mut() = self.status;
        response.headers_mut().insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/json"),
        );
        if let Some(cvm_invoke_id) = self.cvm_invoke_id
            && let Ok(header_value) = HeaderValue::from_str(&cvm_invoke_id)
        {
            response
                .headers_mut()
                .insert(HeaderName::from_static(CVM_INVOKE_ID_HEADER), header_value);
        }
        if let Some(retry_after) = self.retry_after
            && let Ok(header_value) = HeaderValue::from_str(&retry_after)
        {
            response
                .headers_mut()
                .insert(header::RETRY_AFTER, header_value);
        }
        response
    }
}

pub(crate) fn build_proxy_error_response_envelope(
    err: &ProxyErrorResponse,
    invoke_id: &str,
) -> ProxyErrorResponseEnvelope {
    let code = (err.status == StatusCode::GATEWAY_TIMEOUT
        && err
            .message
            .contains(PROXY_FAILURE_UPSTREAM_HANDSHAKE_TIMEOUT))
    .then_some(PROXY_FAILURE_UPSTREAM_HANDSHAKE_TIMEOUT.to_string())
    .or_else(|| err.code.clone());
    let mut payload = json!({ "error": err.message });
    if let Some(cvm_id) = err.cvm_id.as_ref() {
        payload["cvmId"] = json!(cvm_id);
    }
    if let Some(code) = code.as_ref() {
        payload["code"] = json!(code);
    }
    if let Some(blocked_binding) = err.blocked_binding.as_ref() {
        payload["blockedBinding"] = json!(blocked_binding);
    }
    ProxyErrorResponseEnvelope {
        status: err.status,
        body_text: serde_json::to_string(&payload)
            .expect("proxy error response payload should serialize"),
        retry_after: err.retry_after_secs.map(|value| value.to_string()),
        cvm_invoke_id: err.cvm_id.as_ref().map(|_| invoke_id.to_string()),
    }
}

pub(crate) const PROXY_POOL_ROUTE_KEY_MISSING_OR_INVALID_MESSAGE: &str =
    "pool route key missing or invalid";
pub(crate) fn build_proxy_error_response(err: ProxyErrorResponse, invoke_id: &str) -> Response {
    build_proxy_error_response_envelope(&err, invoke_id).into_response()
}

pub(crate) fn build_blocked_binding_diagnostic(
    constraint_source: BlockedBindingConstraintSource,
    upstream_account_id: i64,
    upstream_account_label: Option<&str>,
    prompt_cache_key: Option<&str>,
) -> BlockedBindingDiagnostic {
    BlockedBindingDiagnostic {
        constraint_source,
        upstream_account_id,
        upstream_account_label: blocked_binding_account_label(
            upstream_account_label,
            upstream_account_id,
        ),
        prompt_cache_key: prompt_cache_key
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned),
        recovery_action: BlockedBindingRecoveryAction::ClearAndResetAffinity,
    }
}

pub(crate) fn single_account_binding_constraint_source(
    binding_constraint: Option<&PromptCacheConversationBindingConstraint>,
    owner_auto_guard_active: bool,
) -> Option<(BlockedBindingConstraintSource, i64)> {
    let PromptCacheConversationBindingConstraint::UpstreamAccount(upstream_account_id) =
        binding_constraint?
    else {
        return None;
    };
    Some((
        if owner_auto_guard_active {
            BlockedBindingConstraintSource::EncryptedSessionOwner
        } else {
            BlockedBindingConstraintSource::UpstreamAccountBinding
        },
        *upstream_account_id,
    ))
}

pub(crate) struct SingleAccountBindingBlockedErrorRequest<'a> {
    pub(crate) state: &'a AppState,
    pub(crate) binding_constraint: Option<&'a PromptCacheConversationBindingConstraint>,
    pub(crate) owner_auto_guard_active: bool,
    pub(crate) account: Option<PoolResolvedAccount>,
    pub(crate) prompt_cache_key: Option<&'a str>,
    pub(crate) message: Option<String>,
    pub(crate) attempt_count: usize,
    pub(crate) distinct_account_count: usize,
}

pub(crate) async fn build_single_account_binding_blocked_error(
    request: SingleAccountBindingBlockedErrorRequest<'_>,
) -> Option<PoolUpstreamError> {
    let SingleAccountBindingBlockedErrorRequest {
        state,
        binding_constraint,
        owner_auto_guard_active,
        account,
        prompt_cache_key,
        message,
        attempt_count,
        distinct_account_count,
    } = request;
    let prompt_cache_key = prompt_cache_key
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let explicit_constraint =
        single_account_binding_constraint_source(binding_constraint, owner_auto_guard_active);
    let mut owner_row = None;
    let (constraint_source, upstream_account_id) =
        if let Some(explicit_constraint) = explicit_constraint {
            explicit_constraint
        } else {
            if owner_auto_guard_active && let Some(key) = prompt_cache_key {
                owner_row = load_prompt_cache_encrypted_session_owner_row(&state.pool, key)
                    .await
                    .ok()
                    .flatten();
            }
            owner_row.as_ref().map(|row| {
                (
                    BlockedBindingConstraintSource::EncryptedSessionOwner,
                    row.owner_upstream_account_id,
                )
            })?
        };
    let upstream_account_label = account
        .as_ref()
        .map(|value| value.display_name.trim())
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .or_else(|| {
            owner_row
                .as_ref()
                .and_then(|row| row.owner_upstream_account_name.clone())
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
        });
    let blocked_binding = build_blocked_binding_diagnostic(
        constraint_source,
        upstream_account_id,
        upstream_account_label.as_deref(),
        prompt_cache_key,
    );
    let default_message = match constraint_source {
        BlockedBindingConstraintSource::UpstreamAccountBinding => format!(
            "prompt cache conversation is bound to upstream account {} but that account is currently unavailable",
            blocked_binding.upstream_account_label
        ),
        BlockedBindingConstraintSource::EncryptedSessionOwner => format!(
            "encrypted session owner routing is constrained to upstream account {} but that account is currently unavailable",
            blocked_binding.upstream_account_label
        ),
    };
    Some(build_pool_assigned_binding_blocked_error(
        account,
        message.unwrap_or(default_message),
        PROXY_FAILURE_POOL_ASSIGNED_ACCOUNT_BLOCKED,
        Some(blocked_binding),
        attempt_count,
        distinct_account_count,
    ))
}

pub(crate) fn proxy_error_response_from_pool_upstream_error(
    err: PoolUpstreamError,
    cvm_id: Option<String>,
) -> ProxyErrorResponse {
    ProxyErrorResponse {
        retry_after_secs: retry_after_secs_for_proxy_error(err.status, &err.message),
        status: err.status,
        message: err.message,
        cvm_id,
        code: Some(err.failure_kind.to_string()),
        blocked_binding: err.blocked_binding,
    }
}

#[derive(Debug)]
pub(crate) struct ProxyRequestConcurrencyPermit {
    in_flight: Arc<AtomicUsize>,
}

#[derive(Debug, Clone)]
pub(crate) struct AdmittedProxyRuntimeSnapshot {
    pub(crate) occurred_at: String,
}

impl Drop for ProxyRequestConcurrencyPermit {
    fn drop(&mut self) {
        let _ = self
            .in_flight
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |current| {
                Some(current.saturating_sub(1))
            });
    }
}

pub(crate) async fn acquire_proxy_request_concurrency_permit(
    state: &AppState,
    proxy_request_id: u64,
    method: &Method,
    original_uri: &Uri,
) -> ProxyRequestConcurrencyPermit {
    let in_flight = state
        .proxy_request_in_flight
        .fetch_add(1, Ordering::AcqRel)
        .saturating_add(1);
    info!(
        proxy_request_id,
        method = %method,
        uri = %original_uri,
        in_flight,
        proxy_request_admitted_observed = true,
        max_proxy_in_flight_observed = in_flight,
        "proxy request admitted"
    );

    ProxyRequestConcurrencyPermit {
        in_flight: state.proxy_request_in_flight.clone(),
    }
}

pub(crate) async fn take_or_acquire_proxy_request_concurrency_permit(
    permit: &mut Option<ProxyRequestConcurrencyPermit>,
    state: &AppState,
    proxy_request_id: u64,
    method: &Method,
    original_uri: &Uri,
) -> ProxyRequestConcurrencyPermit {
    match permit.take() {
        Some(permit) => permit,
        None => {
            acquire_proxy_request_concurrency_permit(state, proxy_request_id, method, original_uri)
                .await
        }
    }
}

pub(crate) async fn resolve_proxy_route_context_for_request(
    state: &AppState,
    proxy_request_id: u64,
    method: &Method,
    original_uri: &Uri,
    headers: &HeaderMap,
) -> Result<PoolRoutingTimeoutSettingsResolved, ProxyErrorResponse> {
    let pool_route_active = match request_matches_pool_route(state, headers).await {
        Ok(active) => active,
        Err(err) => {
            warn!(
                proxy_request_id,
                method = %method,
                uri = %original_uri,
                error = %err,
                "failed to resolve pool route"
            );
            return Err(ProxyErrorResponse {
                status: StatusCode::INTERNAL_SERVER_ERROR,
                message: format!("failed to resolve pool routing settings: {err}"),
                cvm_id: None,
                retry_after_secs: None,
                code: None,
                blocked_binding: None,
            });
        }
    };

    if !pool_route_active {
        return Err(ProxyErrorResponse {
            status: StatusCode::UNAUTHORIZED,
            message: PROXY_POOL_ROUTE_KEY_MISSING_OR_INVALID_MESSAGE.to_string(),
            cvm_id: None,
            retry_after_secs: None,
            code: None,
            blocked_binding: None,
        });
    }

    match resolve_proxy_request_timeouts(state, true).await {
        Ok(timeouts) => Ok(timeouts),
        Err(err) => {
            warn!(
                proxy_request_id,
                method = %method,
                uri = %original_uri,
                error = %err,
                "failed to resolve pool routing timeouts"
            );
            Err(ProxyErrorResponse {
                status: StatusCode::INTERNAL_SERVER_ERROR,
                message: format!("failed to resolve pool routing timeouts: {err}"),
                cvm_id: None,
                retry_after_secs: None,
                code: None,
                blocked_binding: None,
            })
        }
    }
}

pub(crate) async fn resolve_proxy_request_timeouts(
    state: &AppState,
    pool_route_active: bool,
) -> Result<PoolRoutingTimeoutSettingsResolved> {
    if pool_route_active {
        Ok(load_pool_routing_runtime_cache(state).await?.timeouts)
    } else {
        Ok(pool_routing_timeouts_from_config(&state.config))
    }
}

#[derive(Debug)]
pub(crate) struct ForwardProxyUpstreamResponse {
    pub(crate) selected_proxy: SelectedForwardProxy,
    pub(crate) response: ProxyUpstreamResponseBody,
    pub(crate) transport_bytes_live_counted: bool,
    pub(crate) connect_latency_ms: f64,
    /// `Instant` captured right before sending the upstream request for the final attempt.
    /// Used to record end-to-end latency once streaming finishes.
    pub(crate) attempt_started_at: Instant,
    pub(crate) attempt_recorded: bool,
    pub(crate) attempt_update: Option<ForwardProxyAttemptUpdate>,
    pub(crate) http_approx: ForwardProxyHttpApproxObservation,
}

#[derive(Debug)]
pub(crate) struct ForwardProxyUpstreamError {
    pub(crate) selected_proxy: SelectedForwardProxy,
    pub(crate) status: StatusCode,
    pub(crate) message: String,
    pub(crate) failure_kind: &'static str,
    pub(crate) attempt_failure_kind: &'static str,
    pub(crate) connect_latency_ms: f64,
    pub(crate) http_approx: ForwardProxyHttpApproxObservation,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct ForwardProxyHttpApproxObservation {
    pub(crate) approx_upload_bytes: usize,
    pub(crate) approx_download_bytes_before_response_body: usize,
    pub(crate) final_response_header_bytes_approx: usize,
    pub(crate) request_compression: Option<RequestCompressionObservation>,
    pub(crate) request_transmission_complete: bool,
}

pub(crate) enum ProxyUpstreamResponseBody {
    Reqwest(reqwest::Response),
    Axum(Response),
}

impl fmt::Debug for ProxyUpstreamResponseBody {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Reqwest(response) => f
                .debug_struct("ProxyUpstreamResponseBody::Reqwest")
                .field("status", &response.status())
                .finish(),
            Self::Axum(response) => f
                .debug_struct("ProxyUpstreamResponseBody::Axum")
                .field("status", &response.status())
                .finish(),
        }
    }
}

impl ProxyUpstreamResponseBody {
    pub(crate) fn status(&self) -> StatusCode {
        match self {
            Self::Reqwest(response) => response.status(),
            Self::Axum(response) => response.status(),
        }
    }

    pub(crate) fn headers(&self) -> &HeaderMap {
        match self {
            Self::Reqwest(response) => response.headers(),
            Self::Axum(response) => response.headers(),
        }
    }

    pub(crate) async fn into_bytes(self) -> Result<Bytes, String> {
        match self {
            Self::Reqwest(response) => response.bytes().await.map_err(|err| err.to_string()),
            Self::Axum(response) => axum::body::to_bytes(response.into_body(), usize::MAX)
                .await
                .map_err(|err| err.to_string()),
        }
    }

    async fn into_first_chunk(self) -> Result<(Self, Option<Bytes>, Option<Instant>), String> {
        match self {
            Self::Reqwest(mut response) => {
                let first_chunk = response.chunk().await.map_err(|err| err.to_string())?;
                let received_at = first_chunk.as_ref().map(|_| Instant::now());
                Ok((Self::Reqwest(response), first_chunk, received_at))
            }
            Self::Axum(response) => {
                let (parts, body) = response.into_parts();
                let mut stream = body.into_data_stream();
                let first_chunk = match stream.next().await {
                    Some(Ok(chunk)) => Some(chunk),
                    Some(Err(err)) => return Err(err.to_string()),
                    None => None,
                };
                let received_at = first_chunk.as_ref().map(|_| Instant::now());
                let response = Response::from_parts(parts, Body::from_stream(stream));
                Ok((Self::Axum(response), first_chunk, received_at))
            }
        }
    }

    pub(crate) fn into_bytes_stream(
        self,
    ) -> Pin<Box<dyn futures_util::Stream<Item = Result<Bytes, io::Error>> + Send>> {
        match self {
            Self::Reqwest(response) => Box::pin(response.bytes_stream().map_err(io::Error::other)),
            Self::Axum(response) => Box::pin(
                response
                    .into_body()
                    .into_data_stream()
                    .map_err(|err| io::Error::other(err.to_string())),
            ),
        }
    }
}

pub(crate) const ASYNC_STREAMING_RAW_WRITER_QUEUE_CAPACITY: usize = 8;

pub(crate) fn pool_upstream_timeout_message(total_timeout: Duration, phase: &str) -> String {
    format!(
        "request timed out after {}ms while {phase}",
        total_timeout.as_millis()
    )
}

pub(crate) fn proxy_request_send_timeout_message(
    capture_target: Option<ProxyCaptureTarget>,
    total_timeout: Duration,
) -> String {
    match capture_target {
        Some(ProxyCaptureTarget::Responses | ProxyCaptureTarget::ResponsesCompact) => {
            pool_upstream_timeout_message(total_timeout, "waiting for first upstream chunk")
        }
        _ => format!(
            "{PROXY_UPSTREAM_HANDSHAKE_TIMEOUT} after {}ms",
            total_timeout.as_millis()
        ),
    }
}

pub(crate) async fn read_pool_upstream_bytes_with_timeout(
    response: ProxyUpstreamResponseBody,
    total_timeout: Duration,
    started: Instant,
    phase: &str,
) -> Result<Bytes, String> {
    let Some(timeout_budget) = remaining_timeout_budget(total_timeout, started.elapsed()) else {
        return Err(pool_upstream_timeout_message(total_timeout, phase));
    };

    match timeout(timeout_budget, response.into_bytes()).await {
        Ok(result) => result,
        Err(_) => Err(pool_upstream_timeout_message(total_timeout, phase)),
    }
}

pub(crate) async fn read_pool_upstream_first_chunk_with_timeout(
    response: ProxyUpstreamResponseBody,
    total_timeout: Duration,
    started: Instant,
) -> Result<(ProxyUpstreamResponseBody, Option<Bytes>, Option<Instant>), String> {
    let Some(timeout_budget) = remaining_timeout_budget(total_timeout, started.elapsed()) else {
        return Err(pool_upstream_timeout_message(
            total_timeout,
            "waiting for first upstream chunk",
        ));
    };

    match timeout(timeout_budget, response.into_first_chunk()).await {
        Ok(result) => result,
        Err(_) => Err(pool_upstream_timeout_message(
            total_timeout,
            "waiting for first upstream chunk",
        )),
    }
}

#[derive(Debug)]
pub(crate) struct PoolUpstreamResponse {
    pub(crate) account: PoolResolvedAccount,
    pub(crate) response: ProxyUpstreamResponseBody,
    pub(crate) transport_bytes_live_counted: bool,
    pub(crate) stream_timeout: Option<Duration>,
    pub(crate) oauth_responses_debug: Option<oauth_bridge::OauthResponsesDebugInfo>,
    pub(crate) connect_latency_ms: f64,
    pub(crate) attempt_started_at_utc: DateTime<Utc>,
    pub(crate) first_byte_latency_ms: f64,
    pub(crate) first_chunk: Option<Bytes>,
    pub(crate) first_chunk_received_at: Option<Instant>,
    pub(crate) first_stream_chunk_received_at: Option<Instant>,
    pub(crate) pending_attempt_record: Option<PendingPoolAttemptRecord>,
    pub(crate) deferred_early_phase_cleanup_guard: Option<PoolEarlyPhaseOrphanCleanupGuard>,
    pub(crate) live_attempt_activity_lease: Option<PoolLiveAttemptActivityLease>,
    pub(crate) attempt_summary: PoolAttemptSummary,
    pub(crate) requested_service_tier: Option<String>,
    pub(crate) request_body_for_capture: Option<Bytes>,
    pub(crate) codex_imagegen_rewrite: Option<Value>,
    pub(crate) reservation_guard: Option<PoolRoutingReservationDropGuard>,
}
