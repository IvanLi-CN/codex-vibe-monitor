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

pub(crate) async fn proxy_openai_v1_with_connect_info(
    State(state): State<Arc<AppState>>,
    connect_info: Option<ConnectInfo<SocketAddr>>,
    downstream_transport: Option<Extension<DownstreamTransportObserver>>,
    ws: Option<WebSocketUpgrade>,
    OriginalUri(original_uri): OriginalUri,
    method: Method,
    headers: HeaderMap,
    body: Body,
) -> Response {
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
    let request_content_length = headers
        .get(header::CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<usize>().ok());
    let request_may_have_body = request_may_have_body(&method, &headers);
    let method_for_log = method.clone();
    let uri_for_log = original_uri.clone();

    info!(
        proxy_request_id,
        method = %method_for_log,
        uri = %uri_for_log,
        proxy_request_started = true,
        has_body = request_may_have_body,
        content_length = ?request_content_length,
        peer_ip = ?peer_ip,
        "openai proxy request started"
    );

    let target_url =
        match build_proxy_upstream_url(&state.config.openai_upstream_base_url, &original_uri) {
            Ok(url) => url,
            Err(err) => {
                let status = if err.to_string().contains(PROXY_DOT_SEGMENT_PATH_NOT_ALLOWED)
                    || err.to_string().contains(PROXY_INVALID_REQUEST_TARGET)
                    || err
                        .to_string()
                        .contains("failed to parse proxy upstream url")
                {
                    StatusCode::BAD_REQUEST
                } else {
                    StatusCode::INTERNAL_SERVER_ERROR
                };
                return build_proxy_error_response(
                    ProxyErrorResponse {
                        status,
                        message: format!("failed to build upstream url: {err}"),
                        cvm_id: None,
                        retry_after_secs: None,
                    },
                    &invoke_id,
                );
            }
        };

    let proxy_request_permit = Some(
        acquire_proxy_request_concurrency_permit(
            state.as_ref(),
            proxy_request_id,
            &method_for_log,
            &uri_for_log,
        )
        .await,
    );
    let capture_target = capture_target_for_request(original_uri.path(), &method);
    let transport_request_observer = downstream_transport
        .as_ref()
        .map(DownstreamTransportObserver::begin_request);
    let downstream_request_observer = capture_target
        .is_some()
        .then_some(transport_request_observer)
        .flatten();
    let admitted_runtime_snapshot = match capture_target {
        Some(target) => {
            let occurred_at = format_naive(Utc::now().with_timezone(&Shanghai).naive_local());
            let requester_ip = extract_requester_ip(&headers, peer_ip);
            let header_sticky_key = extract_sticky_key_from_headers(&headers);
            let header_prompt_cache_key = extract_prompt_cache_key_from_headers(&headers);
            let shell_started = Instant::now();
            let admitted_record = build_admitted_proxy_capture_runtime_snapshot(
                &invoke_id,
                &occurred_at,
                target,
                requester_ip.as_deref(),
                header_sticky_key.as_deref(),
                header_prompt_cache_key.as_deref(),
            );
            if let Err(err) = persist_and_broadcast_proxy_capture_runtime_snapshot(
                state.as_ref(),
                admitted_record,
            )
            .await
            {
                warn!(
                    ?err,
                    proxy_request_id,
                    invoke_id = %invoke_id,
                    "failed to broadcast admitted running proxy capture snapshot"
                );
            } else {
                debug!(
                    proxy_request_id,
                    invoke_id = %invoke_id,
                    occurred_at = %occurred_at,
                    running_shell_emitted = true,
                    running_shell_emit_elapsed = shell_started.elapsed().as_millis() as u64,
                    "admitted proxy request emitted running shell before route context"
                );
            }
            Some(AdmittedProxyRuntimeSnapshot { occurred_at })
        }
        None => None,
    };

    if extract_bearer_token(&headers).is_none() {
        let err = ProxyErrorResponse {
            status: StatusCode::UNAUTHORIZED,
            message: PROXY_POOL_ROUTE_KEY_MISSING_OR_INVALID_MESSAGE.to_string(),
            cvm_id: None,
            retry_after_secs: None,
        };
        if let Some(runtime_snapshot) = admitted_runtime_snapshot.as_ref() {
            terminalize_proxy_runtime_snapshot_with_error(
                state.as_ref(),
                &invoke_id,
                &runtime_snapshot.occurred_at,
                err.status,
                PROXY_FAILURE_POOL_ROUTING_BLOCKED,
                &err.message,
                "missing_bearer_token",
            );
            schedule_dashboard_activity_live_snapshot(state.as_ref());
        }
        return build_proxy_error_response(err, &invoke_id);
    }

    let route_context_started = Instant::now();
    let runtime_timeouts = match resolve_proxy_route_context_for_request(
        state.as_ref(),
        proxy_request_id,
        &method_for_log,
        &uri_for_log,
        &headers,
    )
    .await
    {
        Ok(route_context) => route_context,
        Err(err) => {
            warn!(
                proxy_request_id,
                method = %method_for_log,
                uri = %uri_for_log,
                status = %err.status,
                error = %err.message,
                route_context_elapsed = route_context_started.elapsed().as_millis() as u64,
                elapsed_ms = started_at.elapsed().as_millis(),
                "openai proxy request failed during route validation"
            );
            if let Some(runtime_snapshot) = admitted_runtime_snapshot.as_ref() {
                terminalize_proxy_runtime_snapshot_with_error(
                    state.as_ref(),
                    &invoke_id,
                    &runtime_snapshot.occurred_at,
                    err.status,
                    PROXY_FAILURE_POOL_ROUTING_BLOCKED,
                    &err.message,
                    "route_validation_failed",
                );
                schedule_dashboard_activity_live_snapshot(state.as_ref());
            }
            return build_proxy_error_response(err, &invoke_id);
        }
    };
    debug!(
        proxy_request_id,
        route_context_elapsed = route_context_started.elapsed().as_millis() as u64,
        "proxy route context resolved"
    );
    let pool_route_active = true;

    match Box::pin(proxy_openai_v1_inner(
        state,
        proxy_request_id,
        invoke_id.clone(),
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
    ))
    .await
    {
        Ok(response) => {
            let status = response.status();
            info!(
                proxy_request_id,
                method = %method_for_log,
                uri = %uri_for_log,
                status = %status,
                elapsed_ms = started_at.elapsed().as_millis(),
                "openai proxy response headers ready"
            );
            response
        }
        Err(err) => {
            warn!(
                proxy_request_id,
                method = %method_for_log,
                uri = %uri_for_log,
                status = %err.status,
                error = %err.message,
                elapsed_ms = started_at.elapsed().as_millis(),
                "openai proxy request failed"
            );
            build_proxy_error_response(err, &invoke_id)
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct ProxyErrorResponse {
    pub(crate) status: StatusCode,
    pub(crate) message: String,
    pub(crate) cvm_id: Option<String>,
    pub(crate) retry_after_secs: Option<u64>,
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
    .then_some(PROXY_FAILURE_UPSTREAM_HANDSHAKE_TIMEOUT);
    let mut payload = json!({ "error": err.message });
    if let Some(cvm_id) = err.cvm_id.as_ref() {
        payload["cvmId"] = json!(cvm_id);
    }
    if let Some(code) = code {
        payload["code"] = json!(code);
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
            });
        }
    };

    if !pool_route_active {
        return Err(ProxyErrorResponse {
            status: StatusCode::UNAUTHORIZED,
            message: PROXY_POOL_ROUTE_KEY_MISSING_OR_INVALID_MESSAGE.to_string(),
            cvm_id: None,
            retry_after_secs: None,
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

    async fn into_first_chunk(self) -> Result<(Self, Option<Bytes>), String> {
        match self {
            Self::Reqwest(mut response) => {
                let first_chunk = response.chunk().await.map_err(|err| err.to_string())?;
                Ok((Self::Reqwest(response), first_chunk))
            }
            Self::Axum(response) => {
                let (parts, body) = response.into_parts();
                let mut stream = body.into_data_stream();
                let first_chunk = match stream.next().await {
                    Some(Ok(chunk)) => Some(chunk),
                    Some(Err(err)) => return Err(err.to_string()),
                    None => None,
                };
                let response = Response::from_parts(parts, Body::from_stream(stream));
                Ok((Self::Axum(response), first_chunk))
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

pub(crate) const RAW_PAYLOAD_TRUNCATED_REASON_ASYNC_BACKPRESSURE_DROPPED: &str =
    "async_backpressure_dropped";
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
) -> Result<(ProxyUpstreamResponseBody, Option<Bytes>), String> {
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
    pub(crate) stream_timeout: Option<Duration>,
    pub(crate) oauth_responses_debug: Option<oauth_bridge::OauthResponsesDebugInfo>,
    pub(crate) connect_latency_ms: f64,
    pub(crate) attempt_started_at_utc: DateTime<Utc>,
    pub(crate) first_byte_latency_ms: f64,
    pub(crate) first_chunk: Option<Bytes>,
    pub(crate) pending_attempt_record: Option<PendingPoolAttemptRecord>,
    pub(crate) deferred_early_phase_cleanup_guard: Option<PoolEarlyPhaseOrphanCleanupGuard>,
    pub(crate) live_attempt_activity_lease: Option<PoolLiveAttemptActivityLease>,
    pub(crate) attempt_summary: PoolAttemptSummary,
    pub(crate) requested_service_tier: Option<String>,
    pub(crate) request_body_for_capture: Option<Bytes>,
}

#[derive(Debug)]
pub(crate) struct PoolUpstreamError {
    pub(crate) account: Option<PoolResolvedAccount>,
    pub(crate) status: StatusCode,
    pub(crate) message: String,
    pub(crate) canonical_error_message: Option<String>,
    pub(crate) failure_kind: &'static str,
    pub(crate) connect_latency_ms: f64,
    pub(crate) upstream_error_code: Option<String>,
    pub(crate) upstream_error_message: Option<String>,
    pub(crate) downstream_error_message: Option<String>,
    pub(crate) upstream_request_id: Option<String>,
    pub(crate) proxy_binding_key_snapshot: Option<String>,
    pub(crate) oauth_responses_debug: Option<oauth_bridge::OauthResponsesDebugInfo>,
    pub(crate) attempt_summary: PoolAttemptSummary,
    pub(crate) requested_service_tier: Option<String>,
    pub(crate) request_body_for_capture: Option<Bytes>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct PoolAttemptSummary {
    pub(crate) pool_attempt_count: usize,
    pub(crate) pool_distinct_account_count: usize,
    pub(crate) pool_attempt_terminal_reason: Option<String>,
}

pub(crate) fn pool_attempt_summary(
    pool_attempt_count: usize,
    pool_distinct_account_count: usize,
    pool_attempt_terminal_reason: Option<String>,
) -> PoolAttemptSummary {
    PoolAttemptSummary {
        pool_attempt_count,
        pool_distinct_account_count,
        pool_attempt_terminal_reason,
    }
}

pub(crate) fn pool_upstream_error_is_rate_limited(err: &PoolUpstreamError) -> bool {
    err.status == StatusCode::TOO_MANY_REQUESTS
        || matches!(
            err.failure_kind,
            FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429
                | FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED
                | PROXY_FAILURE_POOL_ALL_ACCOUNTS_RATE_LIMITED
        )
}

pub(crate) fn build_pool_rate_limited_error(
    attempt_count: usize,
    distinct_account_count: usize,
    failure_kind: &'static str,
) -> PoolUpstreamError {
    PoolUpstreamError {
        account: None,
        status: StatusCode::TOO_MANY_REQUESTS,
        message: POOL_ALL_ACCOUNTS_RATE_LIMITED_MESSAGE.to_string(),
        canonical_error_message: None,
        failure_kind,
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
            Some(failure_kind.to_string()),
        ),
        requested_service_tier: None,
        request_body_for_capture: None,
    }
}

pub(crate) fn build_pool_no_available_account_error(
    attempt_count: usize,
    distinct_account_count: usize,
    _retry_after_secs: u64,
) -> PoolUpstreamError {
    PoolUpstreamError {
        account: None,
        status: StatusCode::SERVICE_UNAVAILABLE,
        message: POOL_NO_AVAILABLE_ACCOUNT_MESSAGE.to_string(),
        canonical_error_message: None,
        failure_kind: PROXY_FAILURE_POOL_NO_AVAILABLE_ACCOUNT,
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

pub(crate) const PROXY_FAILURE_ENCRYPTED_SESSION_OWNER_UNAVAILABLE: &str =
    "encrypted_session_owner_unavailable";
pub(crate) const ENCRYPTED_SESSION_OWNER_UNAVAILABLE_MESSAGE: &str = "encrypted session owner unavailable; automatic routing cannot move this encrypted conversation";

pub(crate) fn build_encrypted_session_owner_unavailable_error(
    account: Option<PoolResolvedAccount>,
    attempt_count: usize,
    distinct_account_count: usize,
) -> PoolUpstreamError {
    PoolUpstreamError {
        account,
        status: StatusCode::SERVICE_UNAVAILABLE,
        message: ENCRYPTED_SESSION_OWNER_UNAVAILABLE_MESSAGE.to_string(),
        canonical_error_message: None,
        failure_kind: PROXY_FAILURE_ENCRYPTED_SESSION_OWNER_UNAVAILABLE,
        connect_latency_ms: 0.0,
        upstream_error_code: Some(PROXY_FAILURE_ENCRYPTED_SESSION_OWNER_UNAVAILABLE.to_string()),
        upstream_error_message: Some(ENCRYPTED_SESSION_OWNER_UNAVAILABLE_MESSAGE.to_string()),
        downstream_error_message: None,
        upstream_request_id: None,
        proxy_binding_key_snapshot: None,
        oauth_responses_debug: None,
        attempt_summary: pool_attempt_summary(
            attempt_count,
            distinct_account_count,
            Some(PROXY_FAILURE_ENCRYPTED_SESSION_OWNER_UNAVAILABLE.to_string()),
        ),
        requested_service_tier: None,
        request_body_for_capture: None,
    }
}

pub(crate) fn build_pool_assigned_account_blocked_error(
    account: PoolResolvedAccount,
    message: String,
    failure_kind: &'static str,
    attempt_count: usize,
    distinct_account_count: usize,
) -> PoolUpstreamError {
    PoolUpstreamError {
        account: Some(account),
        status: StatusCode::SERVICE_UNAVAILABLE,
        message,
        canonical_error_message: None,
        failure_kind,
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
            Some(failure_kind.to_string()),
        ),
        requested_service_tier: None,
        request_body_for_capture: None,
    }
}

pub(crate) fn retry_after_secs_for_proxy_error(status: StatusCode, message: &str) -> Option<u64> {
    if status != StatusCode::SERVICE_UNAVAILABLE {
        return None;
    }
    if message == POOL_NO_AVAILABLE_ACCOUNT_MESSAGE
        || message == ENCRYPTED_SESSION_OWNER_UNAVAILABLE_MESSAGE
    {
        return Some(DEFAULT_POOL_NO_AVAILABLE_ACCOUNT_RETRY_AFTER_SECS);
    }
    None
}

pub(crate) fn build_pool_degraded_only_error(
    attempt_count: usize,
    distinct_account_count: usize,
) -> PoolUpstreamError {
    PoolUpstreamError {
        account: None,
        status: StatusCode::SERVICE_UNAVAILABLE,
        message: POOL_ALL_ACCOUNTS_DEGRADED_MESSAGE.to_string(),
        canonical_error_message: None,
        failure_kind: PROXY_FAILURE_POOL_ALL_ACCOUNTS_DEGRADED,
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
            Some(PROXY_FAILURE_POOL_ALL_ACCOUNTS_DEGRADED.to_string()),
        ),
        requested_service_tier: None,
        request_body_for_capture: None,
    }
}

pub(crate) fn pool_upstream_error_preserves_existing_sticky_owner(
    err: Option<&PoolUpstreamError>,
) -> bool {
    err.and_then(|value| value.account.as_ref())
        .is_some_and(|account| account.routing_source == PoolRoutingSelectionSource::StickyReuse)
        && matches!(
            err.map(|value| value.failure_kind),
            Some(
                FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429
                    | FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_5XX
                    | PROXY_FAILURE_FAILED_CONTACT_UPSTREAM
                    | PROXY_FAILURE_UPSTREAM_HANDSHAKE_TIMEOUT
                    | PROXY_FAILURE_UPSTREAM_STREAM_ERROR
                    | PROXY_FAILURE_UPSTREAM_RESPONSE_FAILED
            )
        )
}

pub(crate) fn pool_upstream_error_has_concrete_account_context(
    err: Option<&PoolUpstreamError>,
) -> bool {
    err.and_then(|value| value.account.as_ref()).is_some()
}

pub(crate) fn sticky_owner_terminal_error_preservation_is_active(
    preserve_sticky_owner_terminal_error: bool,
    err: Option<&PoolUpstreamError>,
) -> bool {
    preserve_sticky_owner_terminal_error && pool_upstream_error_has_concrete_account_context(err)
}

pub(crate) fn take_sticky_owner_terminal_error(
    preserve_sticky_owner_terminal_error: bool,
    last_error: &mut Option<PoolUpstreamError>,
    attempt_count: usize,
    distinct_account_count: usize,
) -> Option<PoolUpstreamError> {
    if !sticky_owner_terminal_error_preservation_is_active(
        preserve_sticky_owner_terminal_error,
        last_error.as_ref(),
    ) {
        return None;
    }
    let mut err = last_error.take()?;
    if err.status.is_success() {
        err.status = StatusCode::SERVICE_UNAVAILABLE;
    }
    err.attempt_summary = pool_attempt_summary(
        attempt_count,
        distinct_account_count,
        Some(err.failure_kind.to_string()),
    );
    Some(err)
}

pub(crate) async fn take_and_record_sticky_owner_terminal_error(
    state: &AppState,
    trace_context: Option<&PoolUpstreamAttemptTraceContext>,
    preserve_sticky_owner_terminal_error: bool,
    last_error: &mut Option<PoolUpstreamError>,
    attempt_count: usize,
    distinct_account_count: usize,
) -> Option<PoolUpstreamError> {
    let err = take_sticky_owner_terminal_error(
        preserve_sticky_owner_terminal_error,
        last_error,
        attempt_count,
        distinct_account_count,
    )?;
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
            "failed to persist preserved sticky-owner terminal attempt"
        );
    }
    Some(err)
}

pub(crate) fn store_pool_failover_error(
    last_error: &mut Option<PoolUpstreamError>,
    preserve_sticky_owner_terminal_error: &mut bool,
    err: PoolUpstreamError,
) {
    *preserve_sticky_owner_terminal_error |=
        pool_upstream_error_preserves_existing_sticky_owner(Some(&err));
    *last_error = Some(err);
}

#[derive(Debug, Clone)]
pub(crate) struct PendingPoolAttemptRecord {
    pub(crate) attempt_id: Option<i64>,
    pub(crate) invoke_id: String,
    pub(crate) occurred_at: String,
    pub(crate) endpoint: String,
    pub(crate) sticky_key: Option<String>,
    pub(crate) requester_ip: Option<String>,
    pub(crate) group_name_snapshot: Option<String>,
    pub(crate) proxy_binding_key_snapshot: Option<String>,
    pub(crate) upstream_account_id: i64,
    pub(crate) upstream_route_key: String,
    pub(crate) attempt_index: i64,
    pub(crate) distinct_account_index: i64,
    pub(crate) same_account_retry_index: i64,
    pub(crate) started_at: String,
    pub(crate) connect_latency_ms: f64,
    pub(crate) first_byte_latency_ms: f64,
    pub(crate) compact_support_status: Option<String>,
    pub(crate) compact_support_reason: Option<String>,
    pub(crate) upstream_request_compression_algorithm: Option<String>,
    pub(crate) upstream_request_compression_mode: Option<String>,
    pub(crate) upstream_request_logical_body_bytes: Option<i64>,
    pub(crate) upstream_request_transmitted_body_bytes: Option<i64>,
    pub(crate) upstream_request_header_bytes_approx: Option<i64>,
    pub(crate) upstream_response_body_bytes: Option<i64>,
    pub(crate) upstream_response_header_bytes_approx: Option<i64>,
}

#[derive(Debug, Default)]
pub(crate) struct PoolFailoverProgress {
    pub(crate) excluded_account_ids: Vec<i64>,
    pub(crate) excluded_upstream_route_keys: HashSet<String>,
    pub(crate) attempt_count: usize,
    pub(crate) last_error: Option<PoolUpstreamError>,
    pub(crate) preserve_sticky_owner_terminal_error: bool,
    pub(crate) overload_required_upstream_route_key: Option<String>,
    pub(crate) timeout_route_failover_pending: bool,
    pub(crate) responses_total_timeout_started_at: Option<Instant>,
    pub(crate) no_available_wait_deadline: Option<Instant>,
}

#[derive(Debug, Clone)]
pub(crate) struct PoolUpstreamAttemptTraceContext {
    pub(crate) invoke_id: String,
    pub(crate) occurred_at: String,
    pub(crate) endpoint: String,
    pub(crate) sticky_key: Option<String>,
    pub(crate) requester_ip: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct PoolAttemptRuntimeSnapshotContext {
    pub(crate) capture_target: ProxyCaptureTarget,
    pub(crate) request_info: RequestCaptureInfo,
    pub(crate) prompt_cache_key: Option<String>,
    pub(crate) owner_auto_guard_active: bool,
    pub(crate) t_req_read_ms: f64,
    pub(crate) t_req_parse_ms: f64,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct InvocationRecoverySelector {
    pub(crate) invoke_id: String,
    pub(crate) occurred_at: String,
}

impl InvocationRecoverySelector {
    pub(crate) fn new(invoke_id: impl Into<String>, occurred_at: impl Into<String>) -> Self {
        Self {
            invoke_id: invoke_id.into(),
            occurred_at: occurred_at.into(),
        }
    }
}

impl From<&PendingPoolAttemptRecord> for InvocationRecoverySelector {
    fn from(value: &PendingPoolAttemptRecord) -> Self {
        Self::new(value.invoke_id.clone(), value.occurred_at.clone())
    }
}

#[derive(Debug, Clone, FromRow)]
#[allow(dead_code)]
pub(crate) struct RecoveredPoolAttemptRow {
    pub(crate) id: i64,
    pub(crate) invoke_id: String,
    pub(crate) occurred_at: String,
    pub(crate) sticky_key: Option<String>,
    pub(crate) upstream_account_id: Option<i64>,
}

#[derive(Debug, Clone, FromRow)]
pub(crate) struct RecoveredInvocationRow {
    pub(crate) id: i64,
    pub(crate) invoke_id: String,
    pub(crate) occurred_at: String,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct PoolOrphanRecoveryOutcome {
    pub(crate) recovered_attempts: usize,
    pub(crate) recovered_invocations: usize,
}

pub(crate) const POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_PENDING: &str = "pending";
pub(crate) struct CompactSupportObservation {
    pub(crate) status: &'static str,
    pub(crate) reason: Option<String>,
}
pub(crate) const POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS: &str = "success";
pub(crate) const POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_HTTP_FAILURE: &str = "http_failure";
pub(crate) const POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_TRANSPORT_FAILURE: &str = "transport_failure";
pub(crate) const POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_BUDGET_EXHAUSTED_FINAL: &str =
    "budget_exhausted_final";
pub(crate) const POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_CONNECTING: &str = "connecting";
pub(crate) const POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_SENDING_REQUEST: &str = "sending_request";
pub(crate) const POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_WAITING_FIRST_BYTE: &str =
    "waiting_first_byte";
pub(crate) const POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_STREAMING_RESPONSE: &str =
    "streaming_response";
pub(crate) const POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_COMPLETED: &str = "completed";
pub(crate) const POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_FAILED: &str = "failed";
pub(crate) const POOL_VIA_INVOKE_ID_PREFIX: &str = "pool-via-";
pub(crate) const POOL_EARLY_PHASE_ORPHAN_RECOVERY_GRACE: Duration = Duration::from_secs(30);
pub(crate) const POOL_ATTEMPT_RECOVERY_SELECTOR_BATCH_SIZE: usize = 400;
pub(crate) const PROXY_INVOCATION_RECOVERY_SELECTOR_BATCH_SIZE: usize = 400;

pub(crate) struct PoolEarlyPhaseOrphanCleanupGuard {
    state: Arc<AppState>,
    pending_attempt_record: PendingPoolAttemptRecord,
    pub(crate) first_byte_observed: bool,
    pub(crate) terminal_outcome_observed: bool,
    pub(crate) armed: bool,
}

pub(crate) struct PoolViaRuntimeSnapshotCleanupGuard {
    state: Arc<AppState>,
    invoke_id: String,
}

impl PoolViaRuntimeSnapshotCleanupGuard {
    pub(crate) fn new(state: Arc<AppState>, proxy_request_id: u64) -> Self {
        Self {
            state,
            invoke_id: format!("{POOL_VIA_INVOKE_ID_PREFIX}{proxy_request_id}"),
        }
    }
}

impl Drop for PoolViaRuntimeSnapshotCleanupGuard {
    fn drop(&mut self) {
        let removed_count = self
            .state
            .proxy_runtime_invocations
            .remove_non_terminal_by_invoke_id(&self.invoke_id);
        debug!(
            invoke_id = %self.invoke_id,
            removed_count,
            "request-scoped via-pool runtime snapshots cleaned up"
        );
    }
}

impl std::fmt::Debug for PoolEarlyPhaseOrphanCleanupGuard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PoolEarlyPhaseOrphanCleanupGuard")
            .field("pending_attempt_record", &self.pending_attempt_record)
            .field("armed", &self.armed)
            .finish()
    }
}

impl PoolEarlyPhaseOrphanCleanupGuard {
    pub(crate) fn new(
        state: Arc<AppState>,
        pending_attempt_record: PendingPoolAttemptRecord,
    ) -> Self {
        Self {
            state,
            pending_attempt_record,
            first_byte_observed: false,
            terminal_outcome_observed: false,
            armed: true,
        }
    }

    pub(crate) fn disarm(&mut self) {
        self.armed = false;
    }

    pub(crate) fn mark_first_byte_observed(&mut self, first_byte_latency_ms: f64) {
        self.first_byte_observed = true;
        self.pending_attempt_record.first_byte_latency_ms = self
            .pending_attempt_record
            .first_byte_latency_ms
            .max(first_byte_latency_ms);
    }

    pub(crate) fn mark_terminal_outcome_observed(&mut self) {
        self.terminal_outcome_observed = true;
    }
}

impl Drop for PoolEarlyPhaseOrphanCleanupGuard {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }

        let state = self.state.clone();
        let pending_attempt_record = self.pending_attempt_record.clone();
        let first_byte_observed = self.first_byte_observed;
        let terminal_outcome_observed = self.terminal_outcome_observed;
        tokio::spawn(async move {
            if let Err(err) = recover_guard_dropped_pool_early_phase_orphan(
                state.as_ref(),
                pending_attempt_record,
                first_byte_observed,
                terminal_outcome_observed,
            )
            .await
            {
                warn!(error = %err, "failed to recover dropped pool early-phase orphan");
            }
        });
    }
}

pub(crate) struct PoolInvocationCleanupGuard {
    state: Arc<AppState>,
    selector: InvocationRecoverySelector,
    recovery_trigger: &'static str,
    armed: bool,
}

impl std::fmt::Debug for PoolInvocationCleanupGuard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PoolInvocationCleanupGuard")
            .field("selector", &self.selector)
            .field("recovery_trigger", &self.recovery_trigger)
            .field("armed", &self.armed)
            .finish()
    }
}

impl PoolInvocationCleanupGuard {
    pub(crate) fn new(
        state: Arc<AppState>,
        selector: InvocationRecoverySelector,
        recovery_trigger: &'static str,
    ) -> Self {
        Self {
            state,
            selector,
            recovery_trigger,
            armed: true,
        }
    }

    pub(crate) fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for PoolInvocationCleanupGuard {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }

        let state = self.state.clone();
        let selector = self.selector.clone();
        let recovery_trigger = self.recovery_trigger;
        tokio::spawn(async move {
            if let Err(err) = recover_guard_dropped_pool_invocation_orphan(
                state.as_ref(),
                selector,
                recovery_trigger,
            )
            .await
            {
                warn!(error = %err, recovery_trigger, "failed to recover dropped pool invocation orphan");
            }
        });
    }
}

pub(crate) struct PoolLiveAttemptActivityLease {
    live_attempt_ids: Arc<std::sync::Mutex<HashSet<i64>>>,
    attempt_id: i64,
}

impl std::fmt::Debug for PoolLiveAttemptActivityLease {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PoolLiveAttemptActivityLease")
            .field("attempt_id", &self.attempt_id)
            .finish()
    }
}

impl PoolLiveAttemptActivityLease {
    pub(crate) fn new(state: Arc<AppState>, attempt_id: i64) -> Self {
        {
            let mut live_attempt_ids = state
                .pool_live_attempt_ids
                .lock()
                .unwrap_or_else(|err| err.into_inner());
            live_attempt_ids.insert(attempt_id);
        }
        Self {
            live_attempt_ids: state.pool_live_attempt_ids.clone(),
            attempt_id,
        }
    }
}

impl Drop for PoolLiveAttemptActivityLease {
    fn drop(&mut self) {
        let mut live_attempt_ids = self
            .live_attempt_ids
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        live_attempt_ids.remove(&self.attempt_id);
    }
}

pub(crate) fn disarm_pool_early_phase_cleanup_guard(
    guard: &mut Option<PoolEarlyPhaseOrphanCleanupGuard>,
) {
    if let Some(guard) = guard.as_mut() {
        guard.disarm();
    }
}

pub(crate) fn complete_deferred_pool_early_phase_cleanup_guard(
    guard: &mut Option<PoolEarlyPhaseOrphanCleanupGuard>,
) {
    if let Some(guard) = guard.as_mut() {
        guard.mark_terminal_outcome_observed();
    }
    disarm_pool_early_phase_cleanup_guard(guard);
}

pub(crate) fn finalize_deferred_pool_early_phase_cleanup_guard_after_terminal_invocation(
    guard: &mut Option<PoolEarlyPhaseOrphanCleanupGuard>,
    terminal_invocation_persisted: bool,
) {
    if !terminal_invocation_persisted || guard.is_none() {
        return;
    }
    complete_deferred_pool_early_phase_cleanup_guard(guard);
}

pub(crate) fn disarm_pool_invocation_cleanup_guard(guard: &mut Option<PoolInvocationCleanupGuard>) {
    if let Some(guard) = guard.as_mut() {
        guard.disarm();
    }
}
pub(crate) const POOL_UPSTREAM_MAX_DISTINCT_ACCOUNTS: usize = 3;
pub(crate) const POOL_UPSTREAM_RESPONSES_MAX_TIMEOUT_ROUTE_KEYS: usize = 3;

#[derive(Debug)]
pub(crate) struct PoolReplayTempFile {
    pub(crate) path: PathBuf,
}

impl Drop for PoolReplayTempFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

#[derive(Debug, Clone)]
pub(crate) enum PoolReplayBodySnapshot {
    Empty,
    Memory(Bytes),
    File {
        temp_file: Arc<PoolReplayTempFile>,
        size: usize,
    },
}

#[derive(Debug, Clone)]
pub(crate) enum PoolReplayBodyStatus {
    Reading,
    Complete(PoolReplayBodySnapshot),
    ReadError(RequestBodyReadError),
    InternalError(String),
    Incomplete,
}

#[derive(Debug, Clone)]
pub(crate) enum PoolReplayBodyStickyKeyProbeStatus {
    Pending,
    Ready(PoolReplayBodyKeyProbe),
}

#[derive(Debug, Clone, Default)]
pub(crate) struct PoolReplayBodyKeyProbe {
    pub(crate) sticky_key: Option<String>,
    pub(crate) prompt_cache_key: Option<String>,
    pub(crate) model: Option<String>,
    pub(crate) contains_encrypted_content: bool,
}

pub(crate) struct PoolReplayBodyBuffer {
    proxy_request_id: u64,
    len: usize,
    memory: Vec<u8>,
    file: Option<(Arc<PoolReplayTempFile>, tokio::fs::File)>,
    sticky_key_prefix_probe: Vec<u8>,
}

pub(crate) struct PoolReplayableRequestBody {
    pub(crate) body: reqwest::Body,
    pub(crate) status_rx: watch::Receiver<PoolReplayBodyStatus>,
    pub(crate) sticky_key_probe_rx: watch::Receiver<PoolReplayBodyStickyKeyProbeStatus>,
    pub(crate) cancel: CancellationToken,
}

pub(crate) fn proxy_forward_response_status_is_success(
    status: StatusCode,
    stream_error: bool,
) -> bool {
    !stream_error && status != StatusCode::TOO_MANY_REQUESTS && !status.is_server_error()
}

pub(crate) fn proxy_capture_response_status_is_success(
    status: StatusCode,
    stream_error: bool,
    logical_stream_failure: bool,
) -> bool {
    !logical_stream_failure && proxy_forward_response_status_is_success(status, stream_error)
}

pub(crate) fn proxy_capture_is_pure_downstream_close(
    status: StatusCode,
    stream_error: bool,
    logical_stream_failure: bool,
    downstream_closed: bool,
) -> bool {
    downstream_closed && status.is_success() && !stream_error && !logical_stream_failure
}

pub(crate) fn proxy_capture_invocation_failure_kind(
    status: StatusCode,
    stream_error: bool,
    logical_stream_failure: bool,
    pure_downstream_closed: bool,
) -> Option<&'static str> {
    if stream_error {
        Some(PROXY_FAILURE_UPSTREAM_STREAM_ERROR)
    } else if logical_stream_failure {
        Some(PROXY_FAILURE_UPSTREAM_RESPONSE_FAILED)
    } else if pure_downstream_closed {
        Some(PROXY_STREAM_TERMINAL_DOWNSTREAM_CLOSED)
    } else if status == StatusCode::TOO_MANY_REQUESTS {
        Some(FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429)
    } else if status.is_server_error() {
        Some(FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_5XX)
    } else {
        None
    }
}

pub(crate) fn proxy_capture_invocation_status(
    status: StatusCode,
    has_error_message: bool,
    pure_downstream_closed: bool,
) -> String {
    if pure_downstream_closed {
        INVOCATION_STATUS_WARNING_SUCCESS.to_string()
    } else if status.is_success() && !has_error_message {
        "success".to_string()
    } else {
        format!("http_{}", status.as_u16())
    }
}

pub(crate) fn pool_capture_attempt_status(
    status: StatusCode,
    stream_error: bool,
    logical_stream_failure: bool,
    pure_downstream_closed: bool,
) -> &'static str {
    if pure_downstream_closed {
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS
    } else if stream_error {
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_TRANSPORT_FAILURE
    } else if !status.is_success() || logical_stream_failure {
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_HTTP_FAILURE
    } else {
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS
    }
}

pub(crate) fn proxy_forward_response_failure_kind(
    status: StatusCode,
    stream_error: bool,
) -> Option<&'static str> {
    if stream_error {
        Some(FORWARD_PROXY_FAILURE_STREAM_ERROR)
    } else if status == StatusCode::TOO_MANY_REQUESTS {
        Some(FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429)
    } else if status.is_server_error() {
        Some(FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_5XX)
    } else {
        None
    }
}

pub(crate) fn proxy_capture_response_failure_kind(
    status: StatusCode,
    stream_error: bool,
    logical_stream_failure: bool,
) -> Option<&'static str> {
    if logical_stream_failure {
        Some(PROXY_FAILURE_UPSTREAM_RESPONSE_FAILED)
    } else {
        proxy_forward_response_failure_kind(status, stream_error)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum UpstreamAccountFailureDisposition {
    HardUnavailable,
    RateLimited,
    Retryable,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct UpstreamAccountHttpFailureClassification {
    pub(crate) disposition: UpstreamAccountFailureDisposition,
    pub(crate) failure_kind: &'static str,
    pub(crate) reason_code: &'static str,
    pub(crate) next_account_status: Option<&'static str>,
}

pub(crate) fn upstream_error_indicates_quota_exhausted(message: &str) -> bool {
    let normalized = message.to_ascii_lowercase();
    [
        "insufficient_quota",
        "quota exhausted",
        "quota_exhausted",
        "the usage limit has been reached",
        "usage limit has been reached",
        "usage limit reached",
        "billing",
        "payment required",
        "subscription required",
        "weekly cap",
        "weekly limit",
        "plan limit",
        "plan quota",
        "check your plan",
    ]
    .iter()
    .any(|needle| normalized.contains(needle))
}

pub(crate) fn upstream_error_code_is_server_overloaded(code: Option<&str>) -> bool {
    code.is_some_and(|value| value.eq_ignore_ascii_case(UPSTREAM_ERROR_CODE_SERVER_IS_OVERLOADED))
}

pub(crate) fn route_http_failure_is_retryable_server_overloaded(
    status: StatusCode,
    error_message: &str,
) -> bool {
    if status != StatusCode::OK {
        return false;
    }

    let normalized = error_message.to_ascii_lowercase();
    normalized.contains(PROXY_FAILURE_UPSTREAM_RESPONSE_FAILED)
        && normalized.contains(UPSTREAM_ERROR_CODE_SERVER_IS_OVERLOADED)
}

pub(crate) fn response_info_is_retryable_server_overloaded(
    status: StatusCode,
    response_info: &ResponseCaptureInfo,
) -> bool {
    status == StatusCode::OK
        && response_info.stream_terminal_event.is_some()
        && upstream_error_code_is_server_overloaded(response_info.upstream_error_code.as_deref())
}

pub(crate) fn extract_unsupported_model_from_route_error(
    status: StatusCode,
    error_message: &str,
) -> Option<String> {
    static UNSUPPORTED_MODEL_CONTEXT_REGEX: Lazy<Regex> = Lazy::new(|| {
        Regex::new(
            r#"(?xi)
            unsupported[_\s]+model\s*[:=]\s*['"`]?([a-z0-9][a-z0-9._-]{0,127})['"`]?
            |
            model(?:\s+id)?\s+['"`]?([a-z0-9][a-z0-9._-]{0,127})['"`]?\s+is\s+not\s+supported\b
            |
            model\s+is\s+not\s+supported\s*[:=]\s*['"`]?([a-z0-9][a-z0-9._-]{0,127})['"`]?
            "#,
        )
        .expect("valid unsupported model context regex")
    });
    if status != StatusCode::BAD_REQUEST {
        return None;
    }
    let normalized = error_message.to_ascii_lowercase();
    if !(normalized.contains("unsupported_model")
        || normalized.contains("unsupported model")
        || normalized.contains("model is not supported")
        || normalized.contains("is not supported")
        || normalized.contains("unsupported model"))
    {
        return None;
    }
    if normalized.contains("for model")
        && !normalized.contains("model is not supported")
        && !normalized.contains("unsupported model")
    {
        return None;
    }
    UNSUPPORTED_MODEL_CONTEXT_REGEX
        .captures_iter(error_message)
        .filter_map(|captures| (1..=3).find_map(|index| captures.get(index)))
        .map(|value| value.as_str().trim().to_string())
        .filter(|value| {
            !value.is_empty()
                && value.len() <= 128
                && value
                    .bytes()
                    .any(|byte| byte.is_ascii_digit() || matches!(byte, b'-' | b'.'))
        })
        .last()
}

pub(crate) fn classify_pool_account_http_failure(
    account_kind: &str,
    status: StatusCode,
    error_message: &str,
) -> UpstreamAccountHttpFailureClassification {
    if status == StatusCode::TOO_MANY_REQUESTS
        && upstream_error_indicates_quota_exhausted(error_message)
    {
        return UpstreamAccountHttpFailureClassification {
            disposition: UpstreamAccountFailureDisposition::RateLimited,
            failure_kind: FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED,
            reason_code: "upstream_http_429_quota_exhausted",
            next_account_status: None,
        };
    }
    if status == StatusCode::TOO_MANY_REQUESTS {
        return UpstreamAccountHttpFailureClassification {
            disposition: UpstreamAccountFailureDisposition::RateLimited,
            failure_kind: FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429,
            reason_code: "upstream_http_429_rate_limit",
            next_account_status: None,
        };
    }
    if status == StatusCode::PAYMENT_REQUIRED {
        return UpstreamAccountHttpFailureClassification {
            disposition: UpstreamAccountFailureDisposition::HardUnavailable,
            failure_kind: PROXY_FAILURE_UPSTREAM_HTTP_402,
            reason_code: "upstream_http_402",
            next_account_status: Some("error"),
        };
    }
    if status == StatusCode::PAYLOAD_TOO_LARGE {
        return UpstreamAccountHttpFailureClassification {
            disposition: UpstreamAccountFailureDisposition::Retryable,
            failure_kind: PROXY_FAILURE_UPSTREAM_HTTP_413,
            reason_code: "upstream_http_413",
            next_account_status: None,
        };
    }
    if matches!(status, StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN) {
        let next_account_status = if account_kind == "oauth_codex"
            && is_explicit_reauth_error_message(error_message)
            && !is_scope_permission_error_message(error_message)
            && !is_bridge_error_message(error_message)
        {
            Some("needs_reauth")
        } else {
            Some("error")
        };
        return UpstreamAccountHttpFailureClassification {
            disposition: UpstreamAccountFailureDisposition::HardUnavailable,
            failure_kind: PROXY_FAILURE_UPSTREAM_HTTP_AUTH,
            reason_code: if status == StatusCode::UNAUTHORIZED {
                "upstream_http_401"
            } else {
                "upstream_http_403"
            },
            next_account_status,
        };
    }
    if status.is_server_error() {
        return UpstreamAccountHttpFailureClassification {
            disposition: UpstreamAccountFailureDisposition::Retryable,
            failure_kind: FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_5XX,
            reason_code: "upstream_http_5xx",
            next_account_status: None,
        };
    }
    UpstreamAccountHttpFailureClassification {
        disposition: UpstreamAccountFailureDisposition::Retryable,
        failure_kind: PROXY_FAILURE_POOL_NO_AVAILABLE_ACCOUNT,
        reason_code: "sync_error",
        next_account_status: None,
    }
}

pub(crate) fn compact_support_negative_signal(message: &str) -> bool {
    let normalized = message.to_ascii_lowercase();
    let has_compact_signal = normalized.contains("compact")
        || normalized.contains("responses/compact")
        || normalized.contains("gpt-5.4-openai-compact");
    if normalized.contains("no available channel for model") && has_compact_signal {
        return true;
    }
    has_compact_signal
        && [
            "unsupported model",
            "unsupported endpoint",
            "unsupported path",
            "unsupported route",
            "not support",
            "does not support",
            "is not supported",
            "unknown model",
            "model not found",
            "no channel",
        ]
        .iter()
        .any(|needle| normalized.contains(needle))
}

pub(crate) fn classify_compact_support_observation(
    original_uri: &Uri,
    status: Option<StatusCode>,
    message: Option<&str>,
) -> Option<CompactSupportObservation> {
    if original_uri.path() != "/v1/responses/compact" {
        return None;
    }
    match status {
        Some(code) if code.is_success() => Some(CompactSupportObservation {
            status: COMPACT_SUPPORT_STATUS_SUPPORTED,
            reason: Some("compact request succeeded".to_string()),
        }),
        _ => {
            let message = message
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(|value| value.to_string());
            if message
                .as_deref()
                .is_some_and(compact_support_negative_signal)
            {
                Some(CompactSupportObservation {
                    status: COMPACT_SUPPORT_STATUS_UNSUPPORTED,
                    reason: message,
                })
            } else {
                Some(CompactSupportObservation {
                    status: COMPACT_SUPPORT_STATUS_UNKNOWN,
                    reason: message,
                })
            }
        }
    }
}

fn capability_support_failure_signal(normalized: &str) -> bool {
    [
        "unsupported endpoint",
        "unsupported path",
        "unsupported route",
        "unsupported tool",
        "unsupported model",
        "does not support",
        "is not supported",
        "not support",
        "unknown model",
        "model not found",
        "no available channel for model",
        "no channel",
    ]
    .iter()
    .any(|needle| normalized.contains(needle))
}

pub(crate) fn response_endpoint_capability_negative_signal(message: &str) -> bool {
    let normalized = message.to_ascii_lowercase();
    capability_support_failure_signal(&normalized)
        && [
            "/v1/responses",
            "responses/compact",
            "/v1/chat/completions",
            "chat/completions",
        ]
        .iter()
        .any(|needle| normalized.contains(needle))
}

pub(crate) fn response_image_tool_capability_negative_signal(message: &str) -> bool {
    let normalized = message.to_ascii_lowercase();
    if !capability_support_failure_signal(&normalized) {
        return false;
    }
    normalized.contains("image_generation")
        || normalized.contains("image generation")
        || normalized.contains("gpt-image-")
}

pub(crate) fn image_endpoint_capability_negative_signal(message: &str) -> bool {
    let normalized = message.to_ascii_lowercase();
    if !capability_support_failure_signal(&normalized) {
        return false;
    }
    normalized.contains("gpt-image-")
        || normalized.contains("/v1/images/")
        || normalized.contains("images/generations")
        || normalized.contains("images/edits")
}

pub(crate) fn classify_response_endpoint_capability_observation(
    status: StatusCode,
    message: Option<&str>,
) -> CapabilitySupport {
    if status.is_success() {
        return CapabilitySupport::Supported;
    }
    let normalized_message = message
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    if matches!(
        status,
        StatusCode::BAD_REQUEST | StatusCode::NOT_FOUND | StatusCode::METHOD_NOT_ALLOWED
    ) && normalized_message
        .as_deref()
        .is_some_and(response_endpoint_capability_negative_signal)
    {
        CapabilitySupport::Unsupported
    } else {
        CapabilitySupport::Unknown
    }
}

pub(crate) fn classify_response_image_tool_capability_observation(
    status: StatusCode,
    message: Option<&str>,
) -> CapabilitySupport {
    if status.is_success() {
        return CapabilitySupport::Supported;
    }
    let normalized_message = message
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    if status == StatusCode::BAD_REQUEST
        && normalized_message
            .as_deref()
            .is_some_and(response_image_tool_capability_negative_signal)
    {
        CapabilitySupport::Unsupported
    } else {
        CapabilitySupport::Unknown
    }
}

pub(crate) fn classify_image_endpoint_capability_observation(
    status: StatusCode,
    message: Option<&str>,
) -> CapabilitySupport {
    if status.is_success() {
        return CapabilitySupport::Supported;
    }
    let normalized_message = message
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    if status == StatusCode::BAD_REQUEST
        && normalized_message
            .as_deref()
            .is_some_and(image_endpoint_capability_negative_signal)
    {
        CapabilitySupport::Unsupported
    } else {
        CapabilitySupport::Unknown
    }
}

pub(crate) fn fallback_proxy_429_retry_delay(retry_index: u32) -> Duration {
    let exponent = retry_index.saturating_sub(1).min(16);
    let multiplier = 1_u64 << exponent;
    Duration::from_millis(500_u64.saturating_mul(multiplier)).min(Duration::from_secs(5))
}

pub(crate) fn pool_group_upstream_429_retry_delay(state: &AppState) -> Duration {
    if let Some(delay) = state.pool_group_429_retry_delay_override {
        return delay;
    }
    Duration::from_secs(rand::thread_rng().gen_range(
        MIN_POOL_GROUP_UPSTREAM_429_RETRY_DELAY_SECS..=MAX_POOL_GROUP_UPSTREAM_429_RETRY_DELAY_SECS,
    ))
}

pub(crate) const DEFAULT_POOL_NO_AVAILABLE_ACCOUNT_WAIT_TIMEOUT_SECS: u64 = 10;
pub(crate) const DEFAULT_POOL_NO_AVAILABLE_ACCOUNT_WAIT_POLL_INTERVAL_MS: u64 = 250;
pub(crate) const DEFAULT_POOL_NO_AVAILABLE_ACCOUNT_RETRY_AFTER_SECS: u64 = 10;
pub(crate) const POOL_NO_AVAILABLE_ACCOUNT_MESSAGE: &str = "no healthy pool account is available";

#[derive(Debug, Clone, Copy)]
pub(crate) struct PoolNoAvailableWaitSettings {
    pub(crate) timeout: Duration,
    pub(crate) poll_interval: Duration,
    pub(crate) retry_after_secs: u64,
}

impl Default for PoolNoAvailableWaitSettings {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(DEFAULT_POOL_NO_AVAILABLE_ACCOUNT_WAIT_TIMEOUT_SECS),
            poll_interval: Duration::from_millis(
                DEFAULT_POOL_NO_AVAILABLE_ACCOUNT_WAIT_POLL_INTERVAL_MS,
            ),
            retry_after_secs: DEFAULT_POOL_NO_AVAILABLE_ACCOUNT_RETRY_AFTER_SECS,
        }
    }
}

impl PoolNoAvailableWaitSettings {
    pub(crate) fn normalized_poll_interval(self) -> Duration {
        if self.poll_interval.is_zero() {
            Duration::from_millis(1)
        } else {
            self.poll_interval
        }
    }
}

#[derive(Debug)]
pub(crate) enum PoolAccountResolutionWithWait {
    Resolution(PoolAccountResolution),
    TotalTimeoutExpired,
}

pub(crate) const POOL_UPSTREAM_SAME_ACCOUNT_MAX_ATTEMPTS: u8 = 3;
pub(crate) const OAUTH_RESPONSES_MAX_REWRITE_BODY_BYTES: usize = 8 * 1024 * 1024;
pub(crate) static NEXT_POOL_REPLAY_TEMP_FILE_ID: AtomicU64 = AtomicU64::new(1);

impl PoolReplayBodyBuffer {
    pub(crate) fn new(proxy_request_id: u64) -> Self {
        Self {
            proxy_request_id,
            len: 0,
            memory: Vec::new(),
            file: None,
            sticky_key_prefix_probe: Vec::new(),
        }
    }

    pub(crate) async fn append(&mut self, chunk: &[u8]) -> io::Result<()> {
        self.len = self.len.saturating_add(chunk.len());
        if self.sticky_key_prefix_probe.len() < HEADER_STICKY_EARLY_STICKY_SCAN_BYTES {
            let probe_remaining = HEADER_STICKY_EARLY_STICKY_SCAN_BYTES
                .saturating_sub(self.sticky_key_prefix_probe.len());
            self.sticky_key_prefix_probe
                .extend_from_slice(&chunk[..chunk.len().min(probe_remaining)]);
        }
        if let Some((_, file)) = self.file.as_mut() {
            file.write_all(chunk).await?;
            return Ok(());
        }

        if self.memory.len().saturating_add(chunk.len())
            <= POOL_REQUEST_REPLAY_MEMORY_THRESHOLD_BYTES
        {
            self.memory.extend_from_slice(chunk);
            return Ok(());
        }

        let temp_file = Arc::new(PoolReplayTempFile {
            path: build_pool_replay_temp_path(self.proxy_request_id),
        });
        let mut file = tokio::fs::File::create(&temp_file.path).await?;
        if !self.memory.is_empty() {
            file.write_all(&self.memory).await?;
            self.memory.clear();
        }
        file.write_all(chunk).await?;
        self.file = Some((temp_file, file));
        Ok(())
    }

    pub(crate) async fn finish(mut self) -> io::Result<PoolReplayBodySnapshot> {
        if let Some((temp_file, mut file)) = self.file.take() {
            file.flush().await?;
            return Ok(PoolReplayBodySnapshot::File {
                temp_file,
                size: self.len,
            });
        }

        if self.memory.is_empty() {
            Ok(PoolReplayBodySnapshot::Empty)
        } else {
            Ok(PoolReplayBodySnapshot::Memory(Bytes::from(self.memory)))
        }
    }
}

pub(crate) async fn pool_replay_snapshot_from_bytes(
    proxy_request_id: u64,
    bytes: Bytes,
) -> PoolReplayBodySnapshot {
    if bytes.is_empty() {
        return PoolReplayBodySnapshot::Empty;
    }
    if bytes.len() <= POOL_REQUEST_REPLAY_MEMORY_THRESHOLD_BYTES {
        return PoolReplayBodySnapshot::Memory(bytes);
    }

    let temp_file = Arc::new(PoolReplayTempFile {
        path: build_pool_replay_temp_path(proxy_request_id),
    });
    match tokio::fs::File::create(&temp_file.path).await {
        Ok(mut file) => {
            if let Err(err) = file.write_all(&bytes).await {
                warn!(
                    proxy_request_id,
                    bytes = bytes.len(),
                    error = %err,
                    "failed to write large replay snapshot; falling back to memory"
                );
                return PoolReplayBodySnapshot::Memory(bytes);
            }
            if let Err(err) = file.flush().await {
                warn!(
                    proxy_request_id,
                    bytes = bytes.len(),
                    error = %err,
                    "failed to flush large replay snapshot; falling back to memory"
                );
                return PoolReplayBodySnapshot::Memory(bytes);
            }
            PoolReplayBodySnapshot::File {
                temp_file,
                size: bytes.len(),
            }
        }
        Err(err) => {
            warn!(
                proxy_request_id,
                bytes = bytes.len(),
                error = %err,
                "failed to create large replay snapshot; falling back to memory"
            );
            PoolReplayBodySnapshot::Memory(bytes)
        }
    }
}

pub(crate) async fn pool_replay_snapshot_from_vec(
    proxy_request_id: u64,
    bytes: Vec<u8>,
) -> PoolReplayBodySnapshot {
    pool_replay_snapshot_from_bytes(proxy_request_id, Bytes::from(bytes)).await
}

impl PoolReplayBodySnapshot {
    pub(crate) fn to_reqwest_body(&self) -> reqwest::Body {
        match self {
            Self::Empty => reqwest::Body::from(Bytes::new()),
            Self::Memory(bytes) => reqwest::Body::from(bytes.clone()),
            Self::File { temp_file, size } => {
                let temp_file = temp_file.clone();
                let expected_size = *size;
                let stream = stream::unfold(
                    Some((temp_file, expected_size, None::<tokio::fs::File>)),
                    |state| async move {
                        let (temp_file, remaining, file) = state?;
                        if remaining == 0 {
                            return None;
                        }
                        let mut file = match file {
                            Some(file) => file,
                            None => match tokio::fs::File::open(&temp_file.path).await {
                                Ok(file) => file,
                                Err(err) => {
                                    return Some((Err(io::Error::other(err.to_string())), None));
                                }
                            },
                        };
                        let mut buf = vec![0_u8; remaining.min(64 * 1024)];
                        match file.read(&mut buf).await {
                            Ok(0) => None,
                            Ok(read_len) => {
                                buf.truncate(read_len);
                                Some((
                                    Ok(Bytes::from(buf)),
                                    Some((temp_file, remaining - read_len, Some(file))),
                                ))
                            }
                            Err(err) => Some((Err(io::Error::other(err.to_string())), None)),
                        }
                    },
                );
                reqwest::Body::wrap_stream(stream)
            }
        }
    }

    pub(crate) async fn to_bytes(&self) -> io::Result<Bytes> {
        match self {
            Self::Empty => Ok(Bytes::new()),
            Self::Memory(bytes) => Ok(bytes.clone()),
            Self::File { temp_file, .. } => tokio::fs::read(&temp_file.path).await.map(Bytes::from),
        }
    }

    pub(crate) async fn into_vec(self) -> io::Result<Vec<u8>> {
        match self {
            Self::Empty => Ok(Vec::new()),
            Self::Memory(bytes) => Ok(bytes.to_vec()),
            Self::File { temp_file, .. } => tokio::fs::read(&temp_file.path).await,
        }
    }

    pub(crate) async fn to_prefix_bytes(&self, limit: usize) -> io::Result<Bytes> {
        match self {
            Self::Empty => Ok(Bytes::new()),
            Self::Memory(bytes) => Ok(bytes.slice(..bytes.len().min(limit))),
            Self::File { temp_file, .. } => {
                let mut file = tokio::fs::File::open(&temp_file.path).await?;
                let mut buf = vec![0_u8; limit];
                let read_len = file.read(&mut buf).await?;
                buf.truncate(read_len);
                Ok(Bytes::from(buf))
            }
        }
    }

    pub(crate) async fn extract_request_stream_flag(
        &self,
        content_encoding: Option<&str>,
    ) -> Option<bool> {
        #[derive(serde::Deserialize)]
        struct StreamFlagProjection {
            #[serde(default)]
            stream: Option<bool>,
        }

        fn parse_stream_flag_from_bytes(bytes: &[u8]) -> Option<bool> {
            serde_json::from_slice::<StreamFlagProjection>(bytes)
                .ok()
                .and_then(|projection| projection.stream)
        }

        fn parse_stream_flag_from_reader<R: std::io::Read>(reader: R) -> Option<bool> {
            serde_json::from_reader::<R, StreamFlagProjection>(reader)
                .ok()
                .and_then(|projection| projection.stream)
        }

        match self {
            Self::Empty => None,
            Self::Memory(bytes) => {
                let (decoded, _) = decode_response_payload(bytes.as_ref(), content_encoding, true);
                parse_stream_flag_from_bytes(decoded.as_ref())
            }
            Self::File { temp_file, .. } => {
                let path = temp_file.path.clone();
                let content_encoding = content_encoding.map(str::to_string);
                tokio::task::spawn_blocking(move || {
                    let reader =
                        open_decoded_response_reader(&path, content_encoding.as_deref()).ok()?;
                    parse_stream_flag_from_reader(std::io::BufReader::new(reader))
                })
                .await
                .ok()
                .flatten()
            }
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct PreparedPoolRequestBody {
    pub(crate) snapshot: PoolReplayBodySnapshot,
    pub(crate) request_body_for_capture: Option<Bytes>,
    pub(crate) requested_service_tier: Option<String>,
    pub(crate) requested_image_intent: ImageIntent,
    pub(crate) snapshot_is_decoded: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct PoolRequestBodyPreparationError {
    pub(crate) status: StatusCode,
    pub(crate) message: String,
}

impl PoolRequestBodyPreparationError {
    fn bad_request(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message: message.into(),
        }
    }

    fn bad_gateway(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_GATEWAY,
            message: message.into(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PoolRequestBodyCompressionMode {
    Identity,
    Passthrough,
    Recompressed,
}

impl PoolRequestBodyCompressionMode {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Identity => "identity",
            Self::Passthrough => "passthrough",
            Self::Recompressed => "recompressed",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RequestBodyContentEncoding {
    Identity,
    Gzip,
    Deflate { zlib_wrapper: bool },
    Zstd,
}

impl RequestBodyContentEncoding {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Identity => "identity",
            Self::Gzip => "gzip",
            Self::Deflate { .. } => "deflate",
            Self::Zstd => "zstd",
        }
    }

    pub(crate) fn header_value(self) -> Option<&'static str> {
        match self {
            Self::Identity => None,
            Self::Gzip => Some("gzip"),
            Self::Deflate { .. } => Some("deflate"),
            Self::Zstd => Some("zstd"),
        }
    }

    pub(crate) fn algorithm(self) -> RequestCompressionAlgorithm {
        match self {
            Self::Identity => RequestCompressionAlgorithm::Identity,
            Self::Gzip => RequestCompressionAlgorithm::Gzip,
            Self::Deflate { .. } => RequestCompressionAlgorithm::Deflate,
            Self::Zstd => RequestCompressionAlgorithm::Zstd,
        }
    }
}

#[derive(Debug)]
pub(crate) struct PreparedPoolUpstreamRequestBody {
    pub(crate) body: reqwest::Body,
    pub(crate) content_length: Option<usize>,
    pub(crate) content_encoding: RequestBodyContentEncoding,
    pub(crate) compression_mode: PoolRequestBodyCompressionMode,
    pub(crate) byte_observation: PreparedPoolUpstreamRequestBodyObservation,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct ObservedByteCounter {
    inner: Arc<AtomicU64>,
}

impl ObservedByteCounter {
    pub(crate) fn add(&self, bytes: usize) {
        let Ok(bytes) = u64::try_from(bytes) else {
            return;
        };
        self.inner.fetch_add(bytes, Ordering::Relaxed);
    }

    pub(crate) fn load(&self) -> usize {
        usize::try_from(self.inner.load(Ordering::Relaxed)).unwrap_or(usize::MAX)
    }
}

#[derive(Debug, Clone)]
pub(crate) enum ObservedBodyBytes {
    Fixed(usize),
    Counter(ObservedByteCounter),
}

impl ObservedBodyBytes {
    pub(crate) fn load(&self) -> usize {
        match self {
            Self::Fixed(bytes) => *bytes,
            Self::Counter(counter) => counter.load(),
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct PreparedPoolUpstreamRequestBodyObservation {
    pub(crate) logical_body_bytes: ObservedBodyBytes,
    pub(crate) transmitted_body_bytes: ObservedByteCounter,
}

#[derive(Debug, Clone)]
pub(crate) struct RequestCompressionObservation {
    pub(crate) algorithm: String,
    pub(crate) mode: String,
    pub(crate) logical_body_bytes: usize,
    pub(crate) transmitted_body_bytes: usize,
}

#[derive(Debug)]
struct CountingAsyncRead<R> {
    inner: R,
    counter: ObservedByteCounter,
}

impl<R> CountingAsyncRead<R> {
    fn new(inner: R, counter: ObservedByteCounter) -> Self {
        Self { inner, counter }
    }
}

impl<R> AsyncRead for CountingAsyncRead<R>
where
    R: AsyncRead + Unpin,
{
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<io::Result<()>> {
        let filled_before = buf.filled().len();
        let result = Pin::new(&mut self.inner).poll_read(cx, buf);
        if let std::task::Poll::Ready(Ok(())) = &result {
            let filled_after = buf.filled().len();
            if filled_after > filled_before {
                self.counter.add(filled_after - filled_before);
            }
        }
        result
    }
}

pub(crate) fn http_visible_header_bytes_approx(headers: &HeaderMap) -> usize {
    headers
        .iter()
        .map(|(name, value)| name.as_str().len() + 2 + value.as_bytes().len() + 2)
        .sum()
}

pub(crate) fn counted_reqwest_body_from_bytes(
    bytes: Bytes,
    counter: ObservedByteCounter,
) -> reqwest::Body {
    if bytes.is_empty() {
        return reqwest::Body::from(Bytes::new());
    }
    let stream = stream::once(async move {
        counter.add(bytes.len());
        Ok::<Bytes, io::Error>(bytes)
    });
    reqwest::Body::wrap_stream(stream)
}

fn counted_reqwest_body_from_reader<R>(reader: R, counter: ObservedByteCounter) -> reqwest::Body
where
    R: AsyncRead + Send + 'static,
{
    let stream = ReaderStream::new(reader).map(move |chunk| {
        if let Ok(bytes) = &chunk {
            counter.add(bytes.len());
        }
        chunk
    });
    reqwest::Body::wrap_stream(stream)
}

fn counted_reqwest_body_from_snapshot(
    snapshot: &PoolReplayBodySnapshot,
    counter: ObservedByteCounter,
) -> reqwest::Body {
    match snapshot {
        PoolReplayBodySnapshot::Empty => reqwest::Body::from(Bytes::new()),
        PoolReplayBodySnapshot::Memory(bytes) => {
            counted_reqwest_body_from_bytes(bytes.clone(), counter)
        }
        PoolReplayBodySnapshot::File { temp_file, size } => {
            let temp_file = temp_file.clone();
            let expected_size = *size;
            let stream = stream::unfold(
                Some((temp_file, expected_size, None::<tokio::fs::File>, counter)),
                |state| async move {
                    let (temp_file, remaining, file, counter) = state?;
                    if remaining == 0 {
                        return None;
                    }
                    let mut file = match file {
                        Some(file) => file,
                        None => match tokio::fs::File::open(&temp_file.path).await {
                            Ok(file) => file,
                            Err(err) => {
                                return Some((Err(io::Error::other(err.to_string())), None));
                            }
                        },
                    };
                    let mut buf = vec![0_u8; remaining.min(64 * 1024)];
                    match file.read(&mut buf).await {
                        Ok(0) => None,
                        Ok(read_len) => {
                            buf.truncate(read_len);
                            counter.add(read_len);
                            Some((
                                Ok(Bytes::from(buf)),
                                Some((temp_file, remaining - read_len, Some(file), counter)),
                            ))
                        }
                        Err(err) => Some((Err(io::Error::other(err.to_string())), None)),
                    }
                },
            );
            reqwest::Body::wrap_stream(stream)
        }
    }
}

type BoxedPoolRequestReader = Pin<Box<dyn AsyncRead + Send>>;

fn request_compression_preset_to_async_level(
    preset: RequestCompressionLevelPreset,
) -> AsyncCompressionLevel {
    match preset {
        RequestCompressionLevelPreset::Fast => AsyncCompressionLevel::Fastest,
        RequestCompressionLevelPreset::Balanced => AsyncCompressionLevel::Default,
        RequestCompressionLevelPreset::Best => AsyncCompressionLevel::Best,
    }
}

fn resolve_request_body_content_encoding_from_prefix(
    prefix: Option<&[u8]>,
    content_encoding: Option<&str>,
) -> Result<RequestBodyContentEncoding, PoolRequestBodyPreparationError> {
    let encodings = parse_content_encodings(content_encoding);
    if encodings.is_empty() || encodings.iter().all(|encoding| encoding == "identity") {
        return Ok(RequestBodyContentEncoding::Identity);
    }
    if encodings.len() != 1 {
        return Err(PoolRequestBodyPreparationError::bad_request(format!(
            "unsupported request Content-Encoding chain: {}",
            encodings.join(", ")
        )));
    }

    match encodings[0].as_str() {
        "gzip" | "x-gzip" => Ok(RequestBodyContentEncoding::Gzip),
        "deflate" => Ok(RequestBodyContentEncoding::Deflate {
            zlib_wrapper: deflate_stream_uses_zlib_wrapper(prefix.unwrap_or_default()),
        }),
        "zstd" => Ok(RequestBodyContentEncoding::Zstd),
        other => Err(PoolRequestBodyPreparationError::bad_request(format!(
            "unsupported request Content-Encoding: {other}"
        ))),
    }
}

async fn resolve_request_body_content_encoding(
    snapshot: &PoolReplayBodySnapshot,
    content_encoding: Option<&str>,
) -> Result<RequestBodyContentEncoding, PoolRequestBodyPreparationError> {
    let prefix = if parse_content_encodings(content_encoding)
        .iter()
        .any(|encoding| encoding == "deflate")
    {
        Some(snapshot.to_prefix_bytes(2).await.map_err(|err| {
            PoolRequestBodyPreparationError::bad_gateway(format!(
                "failed to inspect deflate request body header: {err}"
            ))
        })?)
    } else {
        None
    };
    resolve_request_body_content_encoding_from_prefix(
        prefix.as_ref().map(Bytes::as_ref),
        content_encoding,
    )
}

pub(crate) fn observe_request_compression_from_bytes(
    bytes: &[u8],
    content_encoding: Option<&str>,
) -> Option<RequestCompressionObservation> {
    let needs_prefix = parse_content_encodings(content_encoding)
        .iter()
        .any(|encoding| encoding == "deflate");
    let prefix = needs_prefix.then(|| &bytes[..bytes.len().min(2)]);
    let encoding =
        resolve_request_body_content_encoding_from_prefix(prefix, content_encoding).ok()?;
    let logical_body_bytes = decode_request_payload_bytes(bytes, encoding).ok()?.len();
    Some(RequestCompressionObservation {
        algorithm: encoding.algorithm().as_str().to_string(),
        mode: if matches!(encoding, RequestBodyContentEncoding::Identity) {
            PoolRequestBodyCompressionMode::Identity
                .as_str()
                .to_string()
        } else {
            PoolRequestBodyCompressionMode::Passthrough
                .as_str()
                .to_string()
        },
        logical_body_bytes,
        transmitted_body_bytes: bytes.len(),
    })
}

async fn count_decoded_request_snapshot_bytes(
    snapshot: &PoolReplayBodySnapshot,
    encoding: RequestBodyContentEncoding,
) -> Result<usize, PoolRequestBodyPreparationError> {
    if matches!(encoding, RequestBodyContentEncoding::Identity) {
        return Ok(pool_request_snapshot_body_bytes(snapshot));
    }
    match snapshot {
        PoolReplayBodySnapshot::Empty => Ok(0),
        PoolReplayBodySnapshot::Memory(bytes) => {
            Ok(decode_request_payload_bytes(bytes, encoding)?.len())
        }
        PoolReplayBodySnapshot::File { .. } => {
            let raw_reader = open_pool_request_snapshot_reader(snapshot).await?;
            let mut decoded_reader = decode_pool_request_reader(raw_reader, encoding).await?;
            let mut total = 0usize;
            let mut buf = [0_u8; 64 * 1024];
            loop {
                let read_len = decoded_reader.read(&mut buf).await.map_err(|err| {
                    PoolRequestBodyPreparationError::bad_gateway(format!(
                        "failed to count decoded request body bytes: {err}"
                    ))
                })?;
                if read_len == 0 {
                    break;
                }
                total = total.saturating_add(read_len);
            }
            Ok(total)
        }
    }
}

fn decode_request_payload_bytes(
    bytes: &[u8],
    encoding: RequestBodyContentEncoding,
) -> Result<Bytes, PoolRequestBodyPreparationError> {
    match encoding {
        RequestBodyContentEncoding::Identity => Ok(Bytes::copy_from_slice(bytes)),
        RequestBodyContentEncoding::Gzip => {
            let mut decoder = GzDecoder::new(bytes);
            let mut decoded = Vec::new();
            decoder.read_to_end(&mut decoded).map_err(|err| {
                PoolRequestBodyPreparationError::bad_request(format!(
                    "failed to decode gzip request body: {err}"
                ))
            })?;
            Ok(Bytes::from(decoded))
        }
        RequestBodyContentEncoding::Deflate { zlib_wrapper } => {
            let mut decoded = Vec::new();
            if zlib_wrapper {
                let mut decoder = ZlibDecoder::new(bytes);
                decoder.read_to_end(&mut decoded).map_err(|err| {
                    PoolRequestBodyPreparationError::bad_request(format!(
                        "failed to decode deflate request body: {err}"
                    ))
                })?;
            } else {
                let mut decoder = DeflateDecoder::new(bytes);
                decoder.read_to_end(&mut decoded).map_err(|err| {
                    PoolRequestBodyPreparationError::bad_request(format!(
                        "failed to decode deflate request body: {err}"
                    ))
                })?;
            }
            Ok(Bytes::from(decoded))
        }
        RequestBodyContentEncoding::Zstd => {
            zstd::decode_all(bytes).map(Bytes::from).map_err(|err| {
                PoolRequestBodyPreparationError::bad_request(format!(
                    "failed to decode zstd request body: {err}"
                ))
            })
        }
    }
}

async fn open_pool_request_snapshot_reader(
    snapshot: &PoolReplayBodySnapshot,
) -> Result<BoxedPoolRequestReader, PoolRequestBodyPreparationError> {
    match snapshot {
        PoolReplayBodySnapshot::Empty => Ok(Box::pin(tokio::io::empty())),
        PoolReplayBodySnapshot::Memory(bytes) => {
            let bytes = bytes.clone();
            let stream = stream::once(async move { Ok::<Bytes, io::Error>(bytes) });
            Ok(Box::pin(StreamReader::new(stream)))
        }
        PoolReplayBodySnapshot::File { temp_file, .. } => {
            let file = tokio::fs::File::open(&temp_file.path)
                .await
                .map_err(|err| {
                    PoolRequestBodyPreparationError::bad_gateway(format!(
                        "failed to open request replay body: {err}"
                    ))
                })?;
            Ok(Box::pin(file))
        }
    }
}

async fn decode_pool_request_reader(
    reader: BoxedPoolRequestReader,
    encoding: RequestBodyContentEncoding,
) -> Result<BoxedPoolRequestReader, PoolRequestBodyPreparationError> {
    match encoding {
        RequestBodyContentEncoding::Identity => Ok(reader),
        RequestBodyContentEncoding::Gzip => Ok(Box::pin(AsyncGzipDecoder::new(
            tokio::io::BufReader::new(reader),
        ))),
        RequestBodyContentEncoding::Deflate { zlib_wrapper } => {
            let mut buffered = tokio::io::BufReader::new(reader);
            let _ = buffered.fill_buf().await.map_err(|err| {
                PoolRequestBodyPreparationError::bad_request(format!(
                    "failed to read deflate request body header: {err}"
                ))
            })?;
            if zlib_wrapper {
                Ok(Box::pin(AsyncZlibDecoder::new(buffered)))
            } else {
                Ok(Box::pin(AsyncDeflateDecoder::new(buffered)))
            }
        }
        RequestBodyContentEncoding::Zstd => Ok(Box::pin(AsyncZstdDecoder::new(
            tokio::io::BufReader::new(reader),
        ))),
    }
}

fn encode_pool_request_reader(
    reader: BoxedPoolRequestReader,
    encoding: RequestBodyContentEncoding,
    level: AsyncCompressionLevel,
) -> BoxedPoolRequestReader {
    let buffered = tokio::io::BufReader::new(reader);
    match encoding {
        RequestBodyContentEncoding::Identity => Box::pin(buffered),
        RequestBodyContentEncoding::Gzip => {
            Box::pin(AsyncGzipEncoder::with_quality(buffered, level))
        }
        RequestBodyContentEncoding::Deflate { zlib_wrapper } => {
            if zlib_wrapper {
                Box::pin(AsyncZlibEncoder::with_quality(buffered, level))
            } else {
                Box::pin(
                    async_compression::tokio::bufread::DeflateEncoder::with_quality(
                        buffered, level,
                    ),
                )
            }
        }
        RequestBodyContentEncoding::Zstd => {
            Box::pin(AsyncZstdEncoder::with_quality(buffered, level))
        }
    }
}

pub(crate) async fn build_pool_upstream_request_body(
    prepared: &PreparedPoolRequestBody,
    request_compression_algorithm: RequestCompressionAlgorithm,
    request_compression_level_preset: RequestCompressionLevelPreset,
    downstream_content_encoding: Option<&str>,
) -> Result<PreparedPoolUpstreamRequestBody, PoolRequestBodyPreparationError> {
    if matches!(prepared.snapshot, PoolReplayBodySnapshot::Empty) {
        let transmitted_body_bytes = ObservedByteCounter::default();
        return Ok(PreparedPoolUpstreamRequestBody {
            body: reqwest::Body::from(Bytes::new()),
            content_length: Some(0),
            content_encoding: RequestBodyContentEncoding::Identity,
            compression_mode: PoolRequestBodyCompressionMode::Identity,
            byte_observation: PreparedPoolUpstreamRequestBodyObservation {
                logical_body_bytes: ObservedBodyBytes::Fixed(0),
                transmitted_body_bytes,
            },
        });
    }

    let downstream_encoding =
        resolve_request_body_content_encoding(&prepared.snapshot, downstream_content_encoding)
            .await?;
    let target_encoding = match request_compression_algorithm {
        RequestCompressionAlgorithm::Follow => downstream_encoding,
        RequestCompressionAlgorithm::Identity => RequestBodyContentEncoding::Identity,
        RequestCompressionAlgorithm::Gzip => RequestBodyContentEncoding::Gzip,
        RequestCompressionAlgorithm::Deflate => {
            RequestBodyContentEncoding::Deflate { zlib_wrapper: true }
        }
        RequestCompressionAlgorithm::Zstd => RequestBodyContentEncoding::Zstd,
    };

    if prepared.snapshot_is_decoded
        && matches!(target_encoding, RequestBodyContentEncoding::Identity)
    {
        let transmitted_body_bytes = ObservedByteCounter::default();
        return Ok(PreparedPoolUpstreamRequestBody {
            body: counted_reqwest_body_from_snapshot(
                &prepared.snapshot,
                transmitted_body_bytes.clone(),
            ),
            content_length: Some(pool_request_snapshot_body_bytes(&prepared.snapshot)),
            content_encoding: RequestBodyContentEncoding::Identity,
            compression_mode: PoolRequestBodyCompressionMode::Identity,
            byte_observation: PreparedPoolUpstreamRequestBodyObservation {
                logical_body_bytes: ObservedBodyBytes::Fixed(pool_request_snapshot_body_bytes(
                    &prepared.snapshot,
                )),
                transmitted_body_bytes,
            },
        });
    }

    if !prepared.snapshot_is_decoded && target_encoding == downstream_encoding {
        let compression_mode = if matches!(target_encoding, RequestBodyContentEncoding::Identity) {
            PoolRequestBodyCompressionMode::Identity
        } else {
            PoolRequestBodyCompressionMode::Passthrough
        };
        let transmitted_body_bytes = ObservedByteCounter::default();
        let logical_body_bytes = if matches!(target_encoding, RequestBodyContentEncoding::Identity)
        {
            ObservedBodyBytes::Fixed(pool_request_snapshot_body_bytes(&prepared.snapshot))
        } else {
            ObservedBodyBytes::Fixed(
                count_decoded_request_snapshot_bytes(&prepared.snapshot, target_encoding).await?,
            )
        };
        return Ok(PreparedPoolUpstreamRequestBody {
            body: counted_reqwest_body_from_snapshot(
                &prepared.snapshot,
                transmitted_body_bytes.clone(),
            ),
            content_length: Some(pool_request_snapshot_body_bytes(&prepared.snapshot)),
            content_encoding: target_encoding,
            compression_mode,
            byte_observation: PreparedPoolUpstreamRequestBodyObservation {
                logical_body_bytes,
                transmitted_body_bytes,
            },
        });
    }

    let raw_reader = open_pool_request_snapshot_reader(&prepared.snapshot).await?;
    let decoded_reader = if prepared.snapshot_is_decoded {
        raw_reader
    } else {
        decode_pool_request_reader(raw_reader, downstream_encoding).await?
    };

    if matches!(target_encoding, RequestBodyContentEncoding::Identity) {
        let transmitted_body_bytes = ObservedByteCounter::default();
        return Ok(PreparedPoolUpstreamRequestBody {
            body: counted_reqwest_body_from_reader(decoded_reader, transmitted_body_bytes.clone()),
            content_length: None,
            content_encoding: RequestBodyContentEncoding::Identity,
            compression_mode: PoolRequestBodyCompressionMode::Identity,
            byte_observation: PreparedPoolUpstreamRequestBodyObservation {
                logical_body_bytes: if prepared.snapshot_is_decoded {
                    ObservedBodyBytes::Fixed(pool_request_snapshot_body_bytes(&prepared.snapshot))
                } else {
                    ObservedBodyBytes::Counter(transmitted_body_bytes.clone())
                },
                transmitted_body_bytes,
            },
        });
    }

    let level = request_compression_preset_to_async_level(request_compression_level_preset);
    let logical_body_bytes = if prepared.snapshot_is_decoded {
        ObservedBodyBytes::Fixed(pool_request_snapshot_body_bytes(&prepared.snapshot))
    } else {
        ObservedBodyBytes::Counter(ObservedByteCounter::default())
    };
    let decoded_reader = match &logical_body_bytes {
        ObservedBodyBytes::Fixed(_) => decoded_reader,
        ObservedBodyBytes::Counter(counter) => {
            Box::pin(CountingAsyncRead::new(decoded_reader, counter.clone()))
        }
    };
    let encoded_reader = encode_pool_request_reader(decoded_reader, target_encoding, level);
    let transmitted_body_bytes = ObservedByteCounter::default();
    Ok(PreparedPoolUpstreamRequestBody {
        body: counted_reqwest_body_from_reader(encoded_reader, transmitted_body_bytes.clone()),
        content_length: None,
        content_encoding: target_encoding,
        compression_mode: PoolRequestBodyCompressionMode::Recompressed,
        byte_observation: PreparedPoolUpstreamRequestBodyObservation {
            logical_body_bytes,
            transmitted_body_bytes,
        },
    })
}

pub(crate) fn pool_request_snapshot_preserves_content_length(
    snapshot: &PoolReplayBodySnapshot,
) -> bool {
    matches!(snapshot, PoolReplayBodySnapshot::File { .. })
}

pub(crate) fn pool_request_snapshot_kind(snapshot: &PoolReplayBodySnapshot) -> &'static str {
    match snapshot {
        PoolReplayBodySnapshot::Empty => "empty",
        PoolReplayBodySnapshot::Memory(_) => "memory",
        PoolReplayBodySnapshot::File { .. } => "file",
    }
}

pub(crate) fn pool_request_snapshot_body_bytes(snapshot: &PoolReplayBodySnapshot) -> usize {
    match snapshot {
        PoolReplayBodySnapshot::Empty => 0,
        PoolReplayBodySnapshot::Memory(bytes) => bytes.len(),
        PoolReplayBodySnapshot::File { size, .. } => *size,
    }
}

pub(crate) fn request_entry_openai_json_tools_contain_image_generation(value: &Value) -> bool {
    value
        .get("tools")
        .and_then(Value::as_array)
        .is_some_and(|tools| {
            tools.iter().any(|tool| {
                tool.get("type")
                    .and_then(Value::as_str)
                    .is_some_and(|tool_type| tool_type.trim() == "image_generation")
            })
        })
}

pub(crate) fn request_entry_openai_json_tool_choice_selects_image_generation(
    value: &Value,
) -> bool {
    let Some(tool_choice) = value.get("tool_choice") else {
        return false;
    };
    match tool_choice {
        Value::String(choice) => choice.trim() == "image_generation",
        Value::Object(choice) => {
            choice
                .get("type")
                .and_then(Value::as_str)
                .is_some_and(|tool_type| tool_type.trim() == "image_generation")
                || choice
                    .get("tool")
                    .and_then(Value::as_object)
                    .and_then(|tool| tool.get("type"))
                    .and_then(Value::as_str)
                    .is_some_and(|tool_type| tool_type.trim() == "image_generation")
                || choice
                    .get("function")
                    .and_then(Value::as_object)
                    .and_then(|function| function.get("name"))
                    .and_then(Value::as_str)
                    .is_some_and(|name| name.trim() == "image_generation")
        }
        _ => false,
    }
}

pub(crate) fn rewrite_openai_responses_image_tools(
    value: &mut Value,
    rewrite_mode: crate::ImageToolRewriteMode,
    image_intent: crate::ImageIntent,
) -> bool {
    use crate::ImageToolRewriteMode::*;

    let has_image_tool = request_entry_openai_json_tools_contain_image_generation(value);
    let tool_choice_selects_image_generation =
        request_entry_openai_json_tool_choice_selects_image_generation(value);
    let Some(obj) = value.as_object_mut() else {
        return false;
    };

    match rewrite_mode {
        KeepOriginal => false,
        ForceRemove => {
            let mut modified = false;
            if let Some(tools) = obj.get_mut("tools").and_then(Value::as_array_mut) {
                let original_len = tools.len();
                tools.retain(|tool| {
                    tool.get("type")
                        .and_then(Value::as_str)
                        .is_none_or(|tool_type| tool_type.trim() != "image_generation")
                });
                modified |= tools.len() != original_len;
            }
            if tool_choice_selects_image_generation {
                obj.remove("tool_choice");
                modified = true;
            }
            modified
        }
        FillMissing | ForceAdd => {
            if matches!(rewrite_mode, FillMissing) && image_intent != crate::ImageIntent::Yes {
                return false;
            }

            let mut modified = false;
            if !has_image_tool {
                let tool = serde_json::json!({
                    "type": "image_generation",
                    "output_format": "png",
                });
                match obj.get_mut("tools") {
                    Some(Value::Array(tools)) => {
                        tools.push(tool);
                    }
                    Some(_) => {
                        obj.insert("tools".to_string(), Value::Array(vec![tool]));
                    }
                    None => {
                        obj.insert("tools".to_string(), Value::Array(vec![tool]));
                    }
                }
                modified = true;
            }
            if !obj.contains_key("tool_choice") {
                obj.insert(
                    "tool_choice".to_string(),
                    serde_json::json!({"type": "image_generation"}),
                );
                modified = true;
            }
            modified
        }
    }
}

pub(crate) async fn prepare_pool_request_body_for_account(
    proxy_request_id: u64,
    body: Option<&PoolReplayBodySnapshot>,
    original_uri: &Uri,
    method: &Method,
    content_encoding: Option<&str>,
    fast_mode_rewrite_mode: TagFastModeRewriteMode,
    image_tool_rewrite_mode: crate::ImageToolRewriteMode,
) -> Result<PreparedPoolRequestBody, PoolRequestBodyPreparationError> {
    let capture_target = capture_target_for_request(original_uri.path(), method);
    let default_image_intent = match capture_target {
        Some(ProxyCaptureTarget::ImageGenerations | ProxyCaptureTarget::ImageEdits) => {
            ImageIntent::DirectImage
        }
        _ => ImageIntent::Unknown,
    };
    let fast_mode_rewrite_required = capture_target
        .is_some_and(|target| target.allows_fast_mode_rewrite())
        && fast_mode_rewrite_mode != TagFastModeRewriteMode::KeepOriginal;
    let image_tool_rewrite_required = capture_target.is_some_and(|target| {
        matches!(
            target,
            ProxyCaptureTarget::Responses | ProxyCaptureTarget::ResponsesCompact
        )
    }) && image_tool_rewrite_mode
        != crate::ImageToolRewriteMode::KeepOriginal;
    let rewrite_required = fast_mode_rewrite_required || image_tool_rewrite_required;

    let Some(snapshot) = body.cloned() else {
        return Ok(PreparedPoolRequestBody {
            snapshot: PoolReplayBodySnapshot::Empty,
            request_body_for_capture: Some(Bytes::new()),
            requested_service_tier: None,
            requested_image_intent: default_image_intent,
            snapshot_is_decoded: false,
        });
    };

    if !rewrite_required {
        let (request_body_for_capture, requested_service_tier, requested_image_intent) =
            match &snapshot {
                PoolReplayBodySnapshot::Empty => (Some(Bytes::new()), None, default_image_intent),
                PoolReplayBodySnapshot::Memory(bytes) => {
                    let (requested_service_tier, requested_image_intent) =
                        serde_json::from_slice::<Value>(bytes)
                            .ok()
                            .map(|value| {
                                (
                                    extract_requested_service_tier_from_request_body(&value),
                                    capture_target
                                        .map(|target| {
                                            infer_image_intent_from_request_body(target, &value)
                                        })
                                        .unwrap_or(ImageIntent::Unknown),
                                )
                            })
                            .unwrap_or((None, default_image_intent));
                    (
                        Some(bytes.clone()),
                        requested_service_tier,
                        requested_image_intent,
                    )
                }
                PoolReplayBodySnapshot::File { .. } => (None, None, default_image_intent),
            };
        return Ok(PreparedPoolRequestBody {
            snapshot,
            request_body_for_capture,
            requested_service_tier,
            requested_image_intent,
            snapshot_is_decoded: false,
        });
    }

    let original_bytes = snapshot.to_bytes().await.map_err(|err| {
        PoolRequestBodyPreparationError::bad_gateway(format!(
            "failed to materialize pool request body for rewrite: {err}"
        ))
    })?;
    let downstream_encoding =
        resolve_request_body_content_encoding(&snapshot, content_encoding).await?;
    let decoded_original_bytes =
        decode_request_payload_bytes(&original_bytes, downstream_encoding)?;
    let Some(target) = capture_target else {
        return Ok(PreparedPoolRequestBody {
            snapshot,
            request_body_for_capture: Some(original_bytes),
            requested_service_tier: None,
            requested_image_intent: default_image_intent,
            snapshot_is_decoded: false,
        });
    };
    let mut value = match serde_json::from_slice::<Value>(&decoded_original_bytes) {
        Ok(value) => value,
        Err(_) => {
            return Ok(PreparedPoolRequestBody {
                snapshot,
                request_body_for_capture: Some(original_bytes),
                requested_service_tier: None,
                requested_image_intent: default_image_intent,
                snapshot_is_decoded: false,
            });
        }
    };

    let rewritten = if target.allows_fast_mode_rewrite() {
        rewrite_request_service_tier_for_fast_mode(&mut value, fast_mode_rewrite_mode)
    } else {
        false
    };
    let original_image_intent = infer_image_intent_from_request_body(target, &value);
    let image_rewritten = if matches!(
        target,
        ProxyCaptureTarget::Responses | ProxyCaptureTarget::ResponsesCompact
    ) {
        rewrite_openai_responses_image_tools(
            &mut value,
            image_tool_rewrite_mode,
            original_image_intent,
        )
    } else {
        false
    };
    let requested_service_tier = extract_requested_service_tier_from_request_body(&value);
    let upstream_image_intent = infer_image_intent_from_request_body(target, &value);
    if !rewritten && !image_rewritten {
        return Ok(PreparedPoolRequestBody {
            snapshot,
            request_body_for_capture: Some(original_bytes),
            requested_service_tier,
            requested_image_intent: upstream_image_intent,
            snapshot_is_decoded: false,
        });
    }

    let rewritten_bytes = serde_json::to_vec(&value).map(Bytes::from).map_err(|err| {
        PoolRequestBodyPreparationError::bad_gateway(format!(
            "failed to serialize rewritten pool request body: {err}"
        ))
    })?;
    let rewritten_snapshot =
        pool_replay_snapshot_from_bytes(proxy_request_id, rewritten_bytes.clone()).await;
    Ok(PreparedPoolRequestBody {
        snapshot: rewritten_snapshot,
        request_body_for_capture: Some(rewritten_bytes.clone()),
        requested_service_tier,
        requested_image_intent: upstream_image_intent,
        snapshot_is_decoded: true,
    })
}

pub(crate) fn build_pool_replay_temp_path(proxy_request_id: u64) -> PathBuf {
    let mut path = env::temp_dir();
    let unique_id = NEXT_POOL_REPLAY_TEMP_FILE_ID.fetch_add(1, Ordering::Relaxed);
    path.push(format!(
        "cvm-pool-replay-{proxy_request_id}-{}-{unique_id}.bin",
        Utc::now().timestamp_nanos_opt().unwrap_or_default(),
    ));
    path
}

pub(crate) fn spawn_pool_replayable_request_body(
    body: Body,
    body_limit: usize,
    request_read_timeout: Duration,
    proxy_request_id: u64,
) -> PoolReplayableRequestBody {
    let (tx, rx) = mpsc::channel::<Result<Bytes, io::Error>>(16);
    let (status_tx, status_rx) = watch::channel(PoolReplayBodyStatus::Reading);
    let (sticky_key_probe_tx, sticky_key_probe_rx) =
        watch::channel(PoolReplayBodyStickyKeyProbeStatus::Pending);
    let cancel = CancellationToken::new();
    let cancel_for_task = cancel.clone();

    tokio::spawn(async move {
        let mut buffer = PoolReplayBodyBuffer::new(proxy_request_id);
        let mut data_len = 0usize;
        let mut stream = body.into_data_stream();
        let read_deadline = Instant::now() + request_read_timeout;
        let mut live_consumer_open = true;
        let mut sticky_key_probe = Vec::new();
        let mut sticky_key_probe_ready = false;

        loop {
            if cancel_for_task.is_cancelled() {
                if !sticky_key_probe_ready {
                    let _ = sticky_key_probe_tx.send(PoolReplayBodyStickyKeyProbeStatus::Ready(
                        PoolReplayBodyKeyProbe::default(),
                    ));
                }
                let _ = status_tx.send(PoolReplayBodyStatus::Incomplete);
                return;
            }

            let remaining = read_deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                let read_error = RequestBodyReadError {
                    status: StatusCode::REQUEST_TIMEOUT,
                    message: format!(
                        "request body read timed out after {}ms",
                        request_read_timeout.as_millis()
                    ),
                    failure_kind: PROXY_FAILURE_REQUEST_BODY_READ_TIMEOUT,
                    partial_body: Vec::new(),
                };
                warn!(
                    proxy_request_id,
                    timeout_ms = request_read_timeout.as_millis(),
                    read_bytes = data_len,
                    "openai proxy request body read timed out"
                );
                if !sticky_key_probe_ready {
                    let _ = sticky_key_probe_tx.send(PoolReplayBodyStickyKeyProbeStatus::Ready(
                        PoolReplayBodyKeyProbe::default(),
                    ));
                }
                let _ = status_tx.send(PoolReplayBodyStatus::ReadError(read_error.clone()));
                let _ = tx
                    .send(Err(io::Error::new(
                        io::ErrorKind::TimedOut,
                        read_error.message,
                    )))
                    .await;
                return;
            }

            let next_chunk = tokio::select! {
                _ = cancel_for_task.cancelled() => {
                    if !sticky_key_probe_ready {
                        let _ = sticky_key_probe_tx.send(PoolReplayBodyStickyKeyProbeStatus::Ready(
                            PoolReplayBodyKeyProbe::default(),
                        ));
                    }
                    let _ = status_tx.send(PoolReplayBodyStatus::Incomplete);
                    return;
                }
                chunk = timeout(remaining, stream.next()) => {
                    match chunk {
                        Ok(chunk) => chunk,
                        Err(_) => {
                            let read_error = RequestBodyReadError {
                                status: StatusCode::REQUEST_TIMEOUT,
                                message: format!(
                                    "request body read timed out after {}ms",
                                    request_read_timeout.as_millis()
                                ),
                                failure_kind: PROXY_FAILURE_REQUEST_BODY_READ_TIMEOUT,
                                partial_body: Vec::new(),
                            };
                            warn!(
                                proxy_request_id,
                                timeout_ms = request_read_timeout.as_millis(),
                                read_bytes = data_len,
                                "openai proxy request body read timed out"
                            );
                            if !sticky_key_probe_ready {
                                let _ = sticky_key_probe_tx.send(PoolReplayBodyStickyKeyProbeStatus::Ready(
                                    PoolReplayBodyKeyProbe::default(),
                                ));
                            }
                            let _ = status_tx.send(PoolReplayBodyStatus::ReadError(read_error.clone()));
                            let _ = tx
                                .send(Err(io::Error::new(
                                    io::ErrorKind::TimedOut,
                                    read_error.message,
                                )))
                                .await;
                            return;
                        }
                    }
                }
            };

            let Some(chunk) = next_chunk else {
                if !sticky_key_probe_ready {
                    let _ = sticky_key_probe_tx.send(PoolReplayBodyStickyKeyProbeStatus::Ready(
                        PoolReplayBodyKeyProbe {
                            sticky_key: best_effort_extract_sticky_key_from_request_body_prefix(
                                &sticky_key_probe,
                            ),
                            prompt_cache_key:
                                best_effort_extract_prompt_cache_key_from_request_body_prefix(
                                    &sticky_key_probe,
                                ),
                            model: best_effort_extract_model_from_request_body_prefix(
                                &sticky_key_probe,
                            ),
                            contains_encrypted_content:
                                best_effort_extract_encrypted_content_from_request_body_prefix(
                                    &sticky_key_probe,
                                ),
                        },
                    ));
                }
                match buffer.finish().await {
                    Ok(snapshot) => {
                        let _ = status_tx.send(PoolReplayBodyStatus::Complete(snapshot));
                    }
                    Err(err) => {
                        let _ = status_tx.send(PoolReplayBodyStatus::InternalError(format!(
                            "failed to finalize replay body cache: {err}"
                        )));
                    }
                }
                return;
            };

            let chunk = match chunk {
                Ok(chunk) => chunk,
                Err(err) => {
                    let msg = format!("failed to read request body stream: {err}");
                    let read_error = RequestBodyReadError {
                        status: StatusCode::BAD_REQUEST,
                        message: msg,
                        failure_kind: PROXY_FAILURE_REQUEST_BODY_STREAM_ERROR_CLIENT_CLOSED,
                        partial_body: Vec::new(),
                    };
                    if !sticky_key_probe_ready {
                        let _ =
                            sticky_key_probe_tx.send(PoolReplayBodyStickyKeyProbeStatus::Ready(
                                PoolReplayBodyKeyProbe::default(),
                            ));
                    }
                    let _ = status_tx.send(PoolReplayBodyStatus::ReadError(read_error.clone()));
                    let _ = tx.send(Err(io::Error::other(read_error.message))).await;
                    return;
                }
            };

            if data_len.saturating_add(chunk.len()) > body_limit {
                let read_error = RequestBodyReadError {
                    status: StatusCode::PAYLOAD_TOO_LARGE,
                    message: format!("request body exceeds {body_limit} bytes"),
                    failure_kind: PROXY_FAILURE_BODY_TOO_LARGE,
                    partial_body: Vec::new(),
                };
                if !sticky_key_probe_ready {
                    let _ = sticky_key_probe_tx.send(PoolReplayBodyStickyKeyProbeStatus::Ready(
                        PoolReplayBodyKeyProbe::default(),
                    ));
                }
                let _ = status_tx.send(PoolReplayBodyStatus::ReadError(read_error.clone()));
                let _ = tx.send(Err(io::Error::other(read_error.message))).await;
                return;
            }
            data_len = data_len.saturating_add(chunk.len());

            if let Err(err) = buffer.append(&chunk).await {
                let msg = format!("failed to cache replayable request body: {err}");
                if !sticky_key_probe_ready {
                    let _ = sticky_key_probe_tx.send(PoolReplayBodyStickyKeyProbeStatus::Ready(
                        PoolReplayBodyKeyProbe::default(),
                    ));
                }
                let _ = tx.send(Err(io::Error::other(msg.clone()))).await;
                let _ = status_tx.send(PoolReplayBodyStatus::InternalError(msg));
                return;
            }

            if !sticky_key_probe_ready
                && sticky_key_probe.len() < HEADER_STICKY_EARLY_STICKY_SCAN_BYTES
            {
                let probe_remaining =
                    HEADER_STICKY_EARLY_STICKY_SCAN_BYTES.saturating_sub(sticky_key_probe.len());
                sticky_key_probe.extend_from_slice(&chunk[..chunk.len().min(probe_remaining)]);
                let key_probe = PoolReplayBodyKeyProbe {
                    sticky_key: best_effort_extract_sticky_key_from_request_body_prefix(
                        &sticky_key_probe,
                    ),
                    prompt_cache_key: best_effort_extract_prompt_cache_key_from_request_body_prefix(
                        &sticky_key_probe,
                    ),
                    model: best_effort_extract_model_from_request_body_prefix(&sticky_key_probe),
                    contains_encrypted_content:
                        best_effort_extract_encrypted_content_from_request_body_prefix(
                            &sticky_key_probe,
                        ),
                };
                if key_probe.sticky_key.is_some()
                    || key_probe.prompt_cache_key.is_some()
                    || key_probe.contains_encrypted_content
                    || sticky_key_probe.len() >= HEADER_STICKY_EARLY_STICKY_SCAN_BYTES
                {
                    sticky_key_probe_ready = true;
                    let _ = sticky_key_probe_tx
                        .send(PoolReplayBodyStickyKeyProbeStatus::Ready(key_probe));
                }
            }

            if live_consumer_open && tx.send(Ok(chunk)).await.is_err() {
                live_consumer_open = false;
            }
        }
    });

    PoolReplayableRequestBody {
        body: reqwest::Body::wrap_stream(ReceiverStream::new(rx)),
        status_rx,
        sticky_key_probe_rx,
        cancel,
    }
}

pub(crate) async fn wait_for_replay_body_sticky_key_probe(
    sticky_key_probe_rx: &watch::Receiver<PoolReplayBodyStickyKeyProbeStatus>,
    max_wait: Duration,
) -> PoolReplayBodyKeyProbe {
    let mut sticky_key_probe_rx = sticky_key_probe_rx.clone();
    let wait_deadline = Instant::now() + max_wait;
    loop {
        match sticky_key_probe_rx.borrow().clone() {
            PoolReplayBodyStickyKeyProbeStatus::Ready(key_probe) => return key_probe,
            PoolReplayBodyStickyKeyProbeStatus::Pending => {}
        }
        let remaining = wait_deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return PoolReplayBodyKeyProbe::default();
        }
        match timeout(remaining, sticky_key_probe_rx.changed()).await {
            Ok(Ok(())) => {}
            Ok(Err(_)) | Err(_) => return PoolReplayBodyKeyProbe::default(),
        }
    }
}

pub(crate) fn live_body_sticky_key_probe_wait_timeout(
    request_read_timeout: Duration,
    pre_attempt_total_timeout_deadline: Option<Instant>,
) -> Duration {
    match pre_attempt_total_timeout_deadline {
        Some(deadline) => {
            request_read_timeout.min(deadline.saturating_duration_since(Instant::now()))
        }
        None => request_read_timeout,
    }
}

pub(crate) async fn wait_for_replay_body_snapshot(
    state: &AppState,
    original_uri: &Uri,
    method: &Method,
    replay_status_rx: &watch::Receiver<PoolReplayBodyStatus>,
    replay_cancel: &CancellationToken,
    replay_wait_timeout: Duration,
    responses_total_timeout_started_at: Option<Instant>,
) -> Result<PoolReplayBodySnapshot, (StatusCode, String)> {
    let mut replay_status_rx = replay_status_rx.clone();
    let responses_total_timeout =
        pool_upstream_responses_total_timeout(&state.config, original_uri, method);
    let wait_deadline = Instant::now() + replay_wait_timeout;

    let replay_status = loop {
        let current = replay_status_rx.borrow().clone();
        if !matches!(current, PoolReplayBodyStatus::Reading) {
            break current;
        }

        let replay_wait_remaining = wait_deadline.saturating_duration_since(Instant::now());
        if replay_wait_remaining.is_zero() {
            replay_cancel.cancel();
            return Err((
                StatusCode::REQUEST_TIMEOUT,
                format!(
                    "request body read timed out after {}ms",
                    replay_wait_timeout.as_millis()
                ),
            ));
        }

        let wait_budget = if let (Some(total_timeout), Some(started_at)) =
            (responses_total_timeout, responses_total_timeout_started_at)
        {
            let Some(total_wait_remaining) =
                remaining_timeout_budget(total_timeout, started_at.elapsed())
            else {
                replay_cancel.cancel();
                return Err((
                    StatusCode::GATEWAY_TIMEOUT,
                    pool_total_timeout_exhausted_message(total_timeout),
                ));
            };
            replay_wait_remaining.min(total_wait_remaining)
        } else {
            replay_wait_remaining
        };

        match timeout(wait_budget, replay_status_rx.changed()).await {
            Ok(Ok(())) => {}
            Ok(Err(_)) => break PoolReplayBodyStatus::Incomplete,
            Err(_) => {
                replay_cancel.cancel();
                return if let (Some(total_timeout), Some(started_at)) =
                    (responses_total_timeout, responses_total_timeout_started_at)
                {
                    if pool_total_timeout_exhausted(total_timeout, started_at) {
                        Err((
                            StatusCode::GATEWAY_TIMEOUT,
                            pool_total_timeout_exhausted_message(total_timeout),
                        ))
                    } else {
                        Err((
                            StatusCode::REQUEST_TIMEOUT,
                            format!(
                                "request body read timed out after {}ms",
                                replay_wait_timeout.as_millis()
                            ),
                        ))
                    }
                } else {
                    Err((
                        StatusCode::REQUEST_TIMEOUT,
                        format!(
                            "request body read timed out after {}ms",
                            replay_wait_timeout.as_millis()
                        ),
                    ))
                };
            }
        }
    };

    match replay_status {
        PoolReplayBodyStatus::Complete(snapshot) => Ok(snapshot),
        PoolReplayBodyStatus::ReadError(err) => Err((err.status, err.message)),
        PoolReplayBodyStatus::InternalError(message) => Err((StatusCode::BAD_GATEWAY, message)),
        PoolReplayBodyStatus::Reading | PoolReplayBodyStatus::Incomplete => Err((
            StatusCode::BAD_GATEWAY,
            "failed to cache replayable request body".to_string(),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_unsupported_model_from_route_error_supports_short_and_hyphenated_ids() {
        assert_eq!(
            extract_unsupported_model_from_route_error(
                StatusCode::BAD_REQUEST,
                "model o3 is not supported by this account",
            )
            .as_deref(),
            Some("o3")
        );
        assert_eq!(
            extract_unsupported_model_from_route_error(
                StatusCode::BAD_REQUEST,
                "unsupported model: 'o4-mini'",
            )
            .as_deref(),
            Some("o4-mini")
        );
        assert_eq!(
            extract_unsupported_model_from_route_error(
                StatusCode::BAD_REQUEST,
                "model id `computer-use-preview` is not supported",
            )
            .as_deref(),
            Some("computer-use-preview")
        );
        assert_eq!(
            extract_unsupported_model_from_route_error(
                StatusCode::BAD_REQUEST,
                "unsupported_model: pool upstream responded with 400: unsupported model: gpt-5.5",
            )
            .as_deref(),
            Some("gpt-5.5")
        );
    }

    #[test]
    fn extract_unsupported_model_from_route_error_ignores_non_model_bad_requests() {
        assert_eq!(
            extract_unsupported_model_from_route_error(
                StatusCode::BAD_REQUEST,
                "request body is not supported for this endpoint",
            ),
            None
        );
        assert_eq!(
            extract_unsupported_model_from_route_error(
                StatusCode::TOO_MANY_REQUESTS,
                "unsupported model: gpt-5.5",
            ),
            None
        );
        assert_eq!(
            extract_unsupported_model_from_route_error(
                StatusCode::BAD_REQUEST,
                "model is not supported",
            ),
            None
        );
        assert_eq!(
            extract_unsupported_model_from_route_error(
                StatusCode::BAD_REQUEST,
                "response_format is not supported for model gpt-4o",
            ),
            None
        );
        assert_eq!(
            extract_unsupported_model_from_route_error(
                StatusCode::BAD_REQUEST,
                "unsupported_model: pool",
            ),
            None
        );
        assert_eq!(
            extract_unsupported_model_from_route_error(
                StatusCode::BAD_REQUEST,
                "unsupported_model: response_format is not supported for model gpt-4o",
            ),
            None
        );
    }

    #[test]
    fn classify_response_endpoint_capability_observation_is_conservative() {
        assert_eq!(
            classify_response_endpoint_capability_observation(StatusCode::OK, None),
            CapabilitySupport::Supported
        );
        assert_eq!(
            classify_response_endpoint_capability_observation(
                StatusCode::BAD_REQUEST,
                Some("unsupported endpoint: /v1/responses is not supported by this account"),
            ),
            CapabilitySupport::Unsupported
        );
        assert_eq!(
            classify_response_endpoint_capability_observation(
                StatusCode::BAD_REQUEST,
                Some("unsupported tool: image_generation is not supported by this account"),
            ),
            CapabilitySupport::Unknown
        );
    }

    #[test]
    fn classify_response_image_tool_capability_observation_learns_tool_failures_only() {
        assert_eq!(
            classify_response_image_tool_capability_observation(StatusCode::OK, None),
            CapabilitySupport::Supported
        );
        assert_eq!(
            classify_response_image_tool_capability_observation(
                StatusCode::BAD_REQUEST,
                Some("unsupported tool: image_generation is not supported by this account"),
            ),
            CapabilitySupport::Unsupported
        );
        assert_eq!(
            classify_response_image_tool_capability_observation(
                StatusCode::BAD_REQUEST,
                Some("request body is invalid"),
            ),
            CapabilitySupport::Unknown
        );
        assert_eq!(
            classify_response_image_tool_capability_observation(
                StatusCode::BAD_REQUEST,
                Some("invalid image size: width must be divisible by 64"),
            ),
            CapabilitySupport::Unknown
        );
    }

    #[test]
    fn classify_image_endpoint_capability_observation_learns_direct_image_failures() {
        assert_eq!(
            classify_image_endpoint_capability_observation(StatusCode::OK, None),
            CapabilitySupport::Supported
        );
        assert_eq!(
            classify_image_endpoint_capability_observation(
                StatusCode::BAD_REQUEST,
                Some("No available channel for model gpt-image-1 under group default"),
            ),
            CapabilitySupport::Unsupported
        );
    }
}
