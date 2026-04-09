async fn health_check(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    if state.startup_ready.load(Ordering::Acquire) {
        (StatusCode::OK, "ok")
    } else {
        (StatusCode::SERVICE_UNAVAILABLE, "starting")
    }
}

#[cfg(test)]
async fn proxy_openai_v1(
    State(state): State<Arc<AppState>>,
    OriginalUri(original_uri): OriginalUri,
    method: Method,
    headers: HeaderMap,
    body: Body,
) -> Response {
    proxy_openai_v1_common(state, original_uri, method, headers, body, None).await
}

async fn proxy_openai_v1_with_connect_info(
    State(state): State<Arc<AppState>>,
    connect_info: Option<ConnectInfo<SocketAddr>>,
    OriginalUri(original_uri): OriginalUri,
    method: Method,
    headers: HeaderMap,
    body: Body,
) -> Response {
    proxy_openai_v1_common(
        state,
        original_uri,
        method,
        headers,
        body,
        connect_info.map(|info| info.0.ip()),
    )
    .await
}

async fn proxy_openai_v1_common(
    state: Arc<AppState>,
    original_uri: Uri,
    method: Method,
    headers: HeaderMap,
    body: Body,
    peer_ip: Option<IpAddr>,
) -> Response {
    let proxy_request_id = next_proxy_request_id();
    let invoke_id = format!("proxy-{proxy_request_id}-{}", Utc::now().timestamp_millis());
    let started_at = Instant::now();
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

    if extract_bearer_token(&headers).is_none() {
        return build_proxy_error_response(
            ProxyErrorResponse {
                status: StatusCode::UNAUTHORIZED,
                message: PROXY_POOL_ROUTE_KEY_MISSING_OR_INVALID_MESSAGE.to_string(),
                cvm_id: None,
                retry_after_secs: None,
            },
            &invoke_id,
        );
    }

    let proxy_request_permit = Some(
        acquire_proxy_request_concurrency_permit(
            state.as_ref(),
            proxy_request_id,
            &method_for_log,
            &uri_for_log,
        )
        .await,
    );

    let (pool_route_active, runtime_timeouts) = match resolve_proxy_route_context_after_admission(
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
                elapsed_ms = started_at.elapsed().as_millis(),
                "openai proxy request failed during route admission"
            );
            return build_proxy_error_response(err, &invoke_id);
        }
    };

    match proxy_openai_v1_inner(
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
    )
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
struct ProxyErrorResponse {
    status: StatusCode,
    message: String,
    cvm_id: Option<String>,
    retry_after_secs: Option<u64>,
}

const PROXY_POOL_ROUTE_KEY_MISSING_OR_INVALID_MESSAGE: &str = "pool route key missing or invalid";
const RAW_PAYLOAD_TRUNCATED_REASON_ASYNC_BACKPRESSURE_DROPPED: &str =
    "async_backpressure_dropped";
const ASYNC_STREAMING_RAW_WRITER_QUEUE_CAPACITY: usize = 8;

fn build_proxy_error_response(err: ProxyErrorResponse, invoke_id: &str) -> Response {
    match err.cvm_id {
        Some(cvm_id) => {
            let mut response = (
                err.status,
                Json(json!({ "error": err.message, "cvmId": cvm_id })),
            )
                .into_response();
            if let Ok(header_value) = HeaderValue::from_str(invoke_id) {
                response
                    .headers_mut()
                    .insert(HeaderName::from_static(CVM_INVOKE_ID_HEADER), header_value);
            }
            if let Some(retry_after_secs) = err.retry_after_secs
                && let Ok(header_value) = HeaderValue::from_str(&retry_after_secs.to_string())
            {
                response
                    .headers_mut()
                    .insert(header::RETRY_AFTER, header_value);
            }
            response
        }
        None => {
            let mut response = (err.status, Json(json!({ "error": err.message }))).into_response();
            if let Some(retry_after_secs) = err.retry_after_secs
                && let Ok(header_value) = HeaderValue::from_str(&retry_after_secs.to_string())
            {
                response
                    .headers_mut()
                    .insert(header::RETRY_AFTER, header_value);
            }
            response
        }
    }
}

#[derive(Debug)]
struct ProxyRequestConcurrencyPermit {
    in_flight: Arc<AtomicUsize>,
}

impl Drop for ProxyRequestConcurrencyPermit {
    fn drop(&mut self) {
        let _ = self.in_flight.fetch_update(Ordering::AcqRel, Ordering::Acquire, |current| {
            Some(current.saturating_sub(1))
        });
    }
}

async fn acquire_proxy_request_concurrency_permit(
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
        "proxy request admitted"
    );
    ProxyRequestConcurrencyPermit {
        in_flight: state.proxy_request_in_flight.clone(),
    }
}

async fn take_or_acquire_proxy_request_concurrency_permit(
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

async fn resolve_proxy_route_context_after_admission(
    state: &AppState,
    proxy_request_id: u64,
    method: &Method,
    original_uri: &Uri,
    headers: &HeaderMap,
) -> Result<(bool, PoolRoutingTimeoutSettingsResolved), ProxyErrorResponse> {
    let direct_timeouts = pool_routing_timeouts_from_config(&state.config);
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
        return Ok((false, direct_timeouts));
    }

    match resolve_proxy_request_timeouts(state, true).await {
        Ok(timeouts) => Ok((true, timeouts)),
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

async fn resolve_proxy_request_timeouts(
    state: &AppState,
    pool_route_active: bool,
) -> Result<PoolRoutingTimeoutSettingsResolved> {
    if pool_route_active {
        resolve_pool_routing_timeouts(&state.pool, &state.config).await
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
}

#[derive(Debug)]
pub(crate) struct ForwardProxyUpstreamError {
    pub(crate) selected_proxy: SelectedForwardProxy,
    pub(crate) status: StatusCode,
    pub(crate) message: String,
    pub(crate) failure_kind: &'static str,
    pub(crate) attempt_failure_kind: &'static str,
    pub(crate) connect_latency_ms: f64,
}

enum ProxyUpstreamResponseBody {
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
    fn status(&self) -> StatusCode {
        match self {
            Self::Reqwest(response) => response.status(),
            Self::Axum(response) => response.status(),
        }
    }

    fn headers(&self) -> &HeaderMap {
        match self {
            Self::Reqwest(response) => response.headers(),
            Self::Axum(response) => response.headers(),
        }
    }

    async fn into_bytes(self) -> Result<Bytes, String> {
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

    fn into_bytes_stream(
        self,
    ) -> Pin<Box<dyn futures_util::Stream<Item = Result<Bytes, io::Error>> + Send>> {
        match self {
            Self::Reqwest(response) => Box::pin(
                response
                    .bytes_stream()
                    .map_err(|err| io::Error::other(err.to_string())),
            ),
            Self::Axum(response) => Box::pin(
                response
                    .into_body()
                    .into_data_stream()
                    .map_err(|err| io::Error::other(err.to_string())),
            ),
        }
    }
}

fn pool_upstream_timeout_message(total_timeout: Duration, phase: &str) -> String {
    format!(
        "request timed out after {}ms while {phase}",
        total_timeout.as_millis()
    )
}

fn proxy_request_send_timeout_message(
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

async fn read_pool_upstream_bytes_with_timeout(
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

async fn read_pool_upstream_first_chunk_with_timeout(
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
    pub(crate) oauth_responses_debug: Option<oauth_bridge::OauthResponsesDebugInfo>,
    pub(crate) connect_latency_ms: f64,
    pub(crate) attempt_started_at_utc: DateTime<Utc>,
    pub(crate) first_byte_latency_ms: f64,
    pub(crate) first_chunk: Option<Bytes>,
    pub(crate) pending_attempt_record: Option<PendingPoolAttemptRecord>,
    pub(crate) attempt_summary: PoolAttemptSummary,
    pub(crate) requested_service_tier: Option<String>,
    pub(crate) request_body_for_capture: Option<Bytes>,
}

#[derive(Debug)]
pub(crate) struct PoolUpstreamError {
    pub(crate) account: Option<PoolResolvedAccount>,
    pub(crate) status: StatusCode,
    pub(crate) message: String,
    pub(crate) failure_kind: &'static str,
    pub(crate) connect_latency_ms: f64,
    pub(crate) upstream_error_code: Option<String>,
    pub(crate) upstream_error_message: Option<String>,
    pub(crate) upstream_request_id: Option<String>,
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

fn pool_attempt_summary(
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

fn pool_upstream_error_is_rate_limited(err: &PoolUpstreamError) -> bool {
    err.status == StatusCode::TOO_MANY_REQUESTS
        || matches!(
            err.failure_kind,
            FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429
                | FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED
                | PROXY_FAILURE_POOL_ALL_ACCOUNTS_RATE_LIMITED
        )
}

fn build_pool_rate_limited_error(
    attempt_count: usize,
    distinct_account_count: usize,
    failure_kind: &'static str,
) -> PoolUpstreamError {
    PoolUpstreamError {
        account: None,
        status: StatusCode::TOO_MANY_REQUESTS,
        message: POOL_ALL_ACCOUNTS_RATE_LIMITED_MESSAGE.to_string(),
        failure_kind,
        connect_latency_ms: 0.0,
        upstream_error_code: None,
        upstream_error_message: None,
        upstream_request_id: None,
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

fn build_pool_no_available_account_error(
    attempt_count: usize,
    distinct_account_count: usize,
    _retry_after_secs: u64,
) -> PoolUpstreamError {
    PoolUpstreamError {
        account: None,
        status: StatusCode::SERVICE_UNAVAILABLE,
        message: POOL_NO_AVAILABLE_ACCOUNT_MESSAGE.to_string(),
        failure_kind: PROXY_FAILURE_POOL_NO_AVAILABLE_ACCOUNT,
        connect_latency_ms: 0.0,
        upstream_error_code: None,
        upstream_error_message: None,
        upstream_request_id: None,
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

fn retry_after_secs_for_proxy_error(
    status: StatusCode,
    message: &str,
    _proxy_request_concurrency_wait_timeout: Option<Duration>,
) -> Option<u64> {
    if status != StatusCode::SERVICE_UNAVAILABLE {
        return None;
    }
    if message == POOL_NO_AVAILABLE_ACCOUNT_MESSAGE {
        return Some(DEFAULT_POOL_NO_AVAILABLE_ACCOUNT_RETRY_AFTER_SECS);
    }
    None
}

fn build_pool_degraded_only_error(
    attempt_count: usize,
    distinct_account_count: usize,
) -> PoolUpstreamError {
    PoolUpstreamError {
        account: None,
        status: StatusCode::SERVICE_UNAVAILABLE,
        message: POOL_ALL_ACCOUNTS_DEGRADED_MESSAGE.to_string(),
        failure_kind: PROXY_FAILURE_POOL_ALL_ACCOUNTS_DEGRADED,
        connect_latency_ms: 0.0,
        upstream_error_code: None,
        upstream_error_message: None,
        upstream_request_id: None,
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

fn pool_upstream_error_preserves_existing_sticky_owner(
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

fn pool_upstream_error_has_concrete_account_context(err: Option<&PoolUpstreamError>) -> bool {
    err.and_then(|value| value.account.as_ref()).is_some()
}

fn sticky_owner_terminal_error_preservation_is_active(
    preserve_sticky_owner_terminal_error: bool,
    err: Option<&PoolUpstreamError>,
) -> bool {
    preserve_sticky_owner_terminal_error && pool_upstream_error_has_concrete_account_context(err)
}

fn take_sticky_owner_terminal_error(
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

async fn take_and_record_sticky_owner_terminal_error(
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

fn store_pool_failover_error(
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
}

#[derive(Debug, Default)]
struct PoolFailoverProgress {
    excluded_account_ids: Vec<i64>,
    excluded_upstream_route_keys: HashSet<String>,
    attempt_count: usize,
    last_error: Option<PoolUpstreamError>,
    preserve_sticky_owner_terminal_error: bool,
    overload_required_upstream_route_key: Option<String>,
    timeout_route_failover_pending: bool,
    responses_total_timeout_started_at: Option<Instant>,
    no_available_wait_deadline: Option<Instant>,
}

#[derive(Debug, Clone)]
struct PoolUpstreamAttemptTraceContext {
    invoke_id: String,
    occurred_at: String,
    endpoint: String,
    sticky_key: Option<String>,
    requester_ip: Option<String>,
}

#[derive(Debug, Clone)]
struct PoolAttemptRuntimeSnapshotContext {
    capture_target: ProxyCaptureTarget,
    request_info: RequestCaptureInfo,
    prompt_cache_key: Option<String>,
    t_req_read_ms: f64,
    t_req_parse_ms: f64,
}

const POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_PENDING: &str = "pending";
struct CompactSupportObservation {
    status: &'static str,
    reason: Option<String>,
}
const POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS: &str = "success";
const POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_HTTP_FAILURE: &str = "http_failure";
const POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_TRANSPORT_FAILURE: &str = "transport_failure";
const POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_BUDGET_EXHAUSTED_FINAL: &str = "budget_exhausted_final";
const POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_CONNECTING: &str = "connecting";
const POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_SENDING_REQUEST: &str = "sending_request";
const POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_WAITING_FIRST_BYTE: &str = "waiting_first_byte";
const POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_STREAMING_RESPONSE: &str = "streaming_response";
const POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_COMPLETED: &str = "completed";
const POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_FAILED: &str = "failed";
const POOL_UPSTREAM_MAX_DISTINCT_ACCOUNTS: usize = 3;
const POOL_UPSTREAM_RESPONSES_MAX_TIMEOUT_ROUTE_KEYS: usize = 3;

#[derive(Debug)]
struct PoolReplayTempFile {
    path: PathBuf,
}

impl Drop for PoolReplayTempFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

#[derive(Debug, Clone)]
enum PoolReplayBodySnapshot {
    Empty,
    Memory(Bytes),
    File {
        temp_file: Arc<PoolReplayTempFile>,
        size: usize,
        sticky_key: Option<String>,
    },
}

#[derive(Debug, Clone)]
enum PoolReplayBodyStatus {
    Reading,
    Complete(PoolReplayBodySnapshot),
    ReadError(RequestBodyReadError),
    InternalError(String),
    Incomplete,
}

#[derive(Debug, Clone)]
enum PoolReplayBodyStickyKeyProbeStatus {
    Pending,
    Ready(Option<String>),
}

struct PoolReplayBodyBuffer {
    proxy_request_id: u64,
    len: usize,
    memory: Vec<u8>,
    file: Option<(Arc<PoolReplayTempFile>, tokio::fs::File)>,
    sticky_key_prefix_probe: Vec<u8>,
}

struct PoolReplayableRequestBody {
    body: reqwest::Body,
    status_rx: watch::Receiver<PoolReplayBodyStatus>,
    sticky_key_probe_rx: watch::Receiver<PoolReplayBodyStickyKeyProbeStatus>,
    cancel: CancellationToken,
}

fn proxy_forward_response_status_is_success(status: StatusCode, stream_error: bool) -> bool {
    !stream_error && status != StatusCode::TOO_MANY_REQUESTS && !status.is_server_error()
}

fn proxy_capture_response_status_is_success(
    status: StatusCode,
    stream_error: bool,
    logical_stream_failure: bool,
) -> bool {
    !logical_stream_failure && proxy_forward_response_status_is_success(status, stream_error)
}

fn proxy_forward_response_failure_kind(
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

fn proxy_capture_response_failure_kind(
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

fn upstream_error_code_is_server_overloaded(code: Option<&str>) -> bool {
    code.is_some_and(|value| value.eq_ignore_ascii_case(UPSTREAM_ERROR_CODE_SERVER_IS_OVERLOADED))
}

fn route_http_failure_is_retryable_server_overloaded(
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

fn response_info_is_retryable_server_overloaded(
    status: StatusCode,
    response_info: &ResponseCaptureInfo,
) -> bool {
    status == StatusCode::OK
        && response_info.stream_terminal_event.is_some()
        && upstream_error_code_is_server_overloaded(response_info.upstream_error_code.as_deref())
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
            disposition: UpstreamAccountFailureDisposition::HardUnavailable,
            failure_kind: FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED,
            reason_code: "upstream_http_429_quota_exhausted",
            next_account_status: Some("error"),
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

fn compact_support_negative_signal(message: &str) -> bool {
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

fn classify_compact_support_observation(
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

fn fallback_proxy_429_retry_delay(retry_index: u32) -> Duration {
    let exponent = retry_index.saturating_sub(1).min(16);
    let multiplier = 1_u64 << exponent;
    Duration::from_millis(500_u64.saturating_mul(multiplier)).min(Duration::from_secs(5))
}

fn pool_group_upstream_429_retry_delay(state: &AppState) -> Duration {
    if let Some(delay) = state.pool_group_429_retry_delay_override {
        return delay;
    }
    Duration::from_secs(rand::thread_rng().gen_range(
        MIN_POOL_GROUP_UPSTREAM_429_RETRY_DELAY_SECS..=MAX_POOL_GROUP_UPSTREAM_429_RETRY_DELAY_SECS,
    ))
}

const DEFAULT_POOL_NO_AVAILABLE_ACCOUNT_WAIT_TIMEOUT_SECS: u64 = 10;
const DEFAULT_POOL_NO_AVAILABLE_ACCOUNT_WAIT_POLL_INTERVAL_MS: u64 = 250;
const DEFAULT_POOL_NO_AVAILABLE_ACCOUNT_RETRY_AFTER_SECS: u64 = 10;
const POOL_NO_AVAILABLE_ACCOUNT_MESSAGE: &str = "no healthy pool account is available";

#[derive(Debug, Clone, Copy)]
struct PoolNoAvailableWaitSettings {
    timeout: Duration,
    poll_interval: Duration,
    retry_after_secs: u64,
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
    fn normalized_poll_interval(self) -> Duration {
        if self.poll_interval.is_zero() {
            Duration::from_millis(1)
        } else {
            self.poll_interval
        }
    }
}

#[derive(Debug)]
enum PoolAccountResolutionWithWait {
    Resolution(PoolAccountResolution),
    TotalTimeoutExpired,
}

const POOL_UPSTREAM_SAME_ACCOUNT_MAX_ATTEMPTS: u8 = 3;

impl PoolReplayBodyBuffer {
    fn new(proxy_request_id: u64) -> Self {
        Self {
            proxy_request_id,
            len: 0,
            memory: Vec::new(),
            file: None,
            sticky_key_prefix_probe: Vec::new(),
        }
    }

    async fn append(&mut self, chunk: &[u8]) -> io::Result<()> {
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

    async fn finish(mut self) -> io::Result<PoolReplayBodySnapshot> {
        if let Some((temp_file, mut file)) = self.file.take() {
            file.flush().await?;
            return Ok(PoolReplayBodySnapshot::File {
                temp_file,
                size: self.len,
                sticky_key: best_effort_extract_sticky_key_from_request_body_prefix(
                    &self.sticky_key_prefix_probe,
                ),
            });
        }

        if self.memory.is_empty() {
            Ok(PoolReplayBodySnapshot::Empty)
        } else {
            Ok(PoolReplayBodySnapshot::Memory(Bytes::from(self.memory)))
        }
    }
}

impl PoolReplayBodySnapshot {
    fn to_reqwest_body(&self) -> reqwest::Body {
        match self {
            Self::Empty => reqwest::Body::from(Bytes::new()),
            Self::Memory(bytes) => reqwest::Body::from(bytes.clone()),
            Self::File {
                temp_file, size, ..
            } => {
                let path = temp_file.path.clone();
                let expected_size = *size;
                let stream = stream::unfold(
                    Some((path, expected_size, None::<tokio::fs::File>)),
                    |state| async move {
                        let Some((path, remaining, file)) = state else {
                            return None;
                        };
                        if remaining == 0 {
                            return None;
                        }
                        let mut file = match file {
                            Some(file) => file,
                            None => match tokio::fs::File::open(&path).await {
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
                                    Some((path, remaining - read_len, Some(file))),
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

    async fn to_bytes(&self) -> io::Result<Bytes> {
        match self {
            Self::Empty => Ok(Bytes::new()),
            Self::Memory(bytes) => Ok(bytes.clone()),
            Self::File { temp_file, .. } => tokio::fs::read(&temp_file.path).await.map(Bytes::from),
        }
    }

    async fn to_prefix_bytes(&self, limit: usize) -> io::Result<Bytes> {
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

    async fn extract_request_stream_flag(&self, content_encoding: Option<&str>) -> Option<bool> {
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
                    let reader = open_decoded_response_reader(&path, content_encoding.as_deref())
                        .ok()?;
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
struct PreparedPoolRequestBody {
    snapshot: PoolReplayBodySnapshot,
    request_body_for_capture: Option<Bytes>,
    requested_service_tier: Option<String>,
}

fn pool_request_snapshot_preserves_content_length(
    snapshot: &PoolReplayBodySnapshot,
    forwarded_content_length: Option<usize>,
) -> bool {
    matches!(
        snapshot,
        PoolReplayBodySnapshot::File { size, .. } if forwarded_content_length == Some(*size)
    )
}

fn pool_request_snapshot_kind(snapshot: &PoolReplayBodySnapshot) -> &'static str {
    match snapshot {
        PoolReplayBodySnapshot::Empty => "empty",
        PoolReplayBodySnapshot::Memory(_) => "memory",
        PoolReplayBodySnapshot::File { .. } => "file",
    }
}

fn pool_request_snapshot_body_bytes(snapshot: &PoolReplayBodySnapshot) -> usize {
    match snapshot {
        PoolReplayBodySnapshot::Empty => 0,
        PoolReplayBodySnapshot::Memory(bytes) => bytes.len(),
        PoolReplayBodySnapshot::File { size, .. } => *size,
    }
}

async fn prepare_pool_request_body_for_account(
    body: Option<&PoolReplayBodySnapshot>,
    original_uri: &Uri,
    method: &Method,
    fast_mode_rewrite_mode: TagFastModeRewriteMode,
    proxy_request_id: u64,
) -> Result<PreparedPoolRequestBody, String> {
    let capture_target = capture_target_for_request(original_uri.path(), method);
    let rewrite_required = capture_target.is_some_and(|target| target.allows_fast_mode_rewrite())
        && fast_mode_rewrite_mode != TagFastModeRewriteMode::KeepOriginal;

    let Some(snapshot) = body.cloned() else {
        return Ok(PreparedPoolRequestBody {
            snapshot: PoolReplayBodySnapshot::Empty,
            request_body_for_capture: Some(Bytes::new()),
            requested_service_tier: None,
        });
    };

    if !rewrite_required {
        let (request_body_for_capture, requested_service_tier) = match &snapshot {
            PoolReplayBodySnapshot::Empty => (Some(Bytes::new()), None),
            PoolReplayBodySnapshot::Memory(bytes) => {
                let requested_service_tier = serde_json::from_slice::<Value>(bytes)
                    .ok()
                    .and_then(|value| extract_requested_service_tier_from_request_body(&value));
                (Some(bytes.clone()), requested_service_tier)
            }
            PoolReplayBodySnapshot::File { .. } => (None, None),
        };
        return Ok(PreparedPoolRequestBody {
            snapshot,
            request_body_for_capture,
            requested_service_tier,
        });
    }

    let original_bytes = snapshot
        .to_bytes()
        .await
        .map_err(|err| format!("failed to materialize pool request body for rewrite: {err}"))?;
    let Some(target) = capture_target else {
        return Ok(PreparedPoolRequestBody {
            snapshot: PoolReplayBodySnapshot::Memory(original_bytes.clone()),
            request_body_for_capture: Some(original_bytes),
            requested_service_tier: None,
        });
    };
    let mut value = match serde_json::from_slice::<Value>(&original_bytes) {
        Ok(value) => value,
        Err(_) => {
            return Ok(PreparedPoolRequestBody {
                snapshot: PoolReplayBodySnapshot::Memory(original_bytes.clone()),
                request_body_for_capture: Some(original_bytes),
                requested_service_tier: None,
            });
        }
    };

    let rewritten = if target.allows_fast_mode_rewrite() {
        rewrite_request_service_tier_for_fast_mode(&mut value, fast_mode_rewrite_mode)
    } else {
        false
    };
    let requested_service_tier = extract_requested_service_tier_from_request_body(&value);
    if !rewritten {
        return Ok(PreparedPoolRequestBody {
            snapshot: PoolReplayBodySnapshot::Memory(original_bytes.clone()),
            request_body_for_capture: Some(original_bytes),
            requested_service_tier,
        });
    }

    let rewritten_bytes = serde_json::to_vec(&value)
        .map(Bytes::from)
        .map_err(|err| format!("failed to serialize rewritten pool request body: {err}"))?;
    let rewritten_snapshot = pool_request_snapshot_from_bytes(
        rewritten_bytes.clone(),
        match snapshot {
            PoolReplayBodySnapshot::File { sticky_key, .. } => sticky_key,
            _ => None,
        },
        proxy_request_id,
    )
    .await?;
    Ok(PreparedPoolRequestBody {
        snapshot: rewritten_snapshot,
        request_body_for_capture: Some(rewritten_bytes.clone()),
        requested_service_tier,
    })
}

async fn pool_request_snapshot_from_bytes(
    bytes: Bytes,
    sticky_key: Option<String>,
    proxy_request_id: u64,
) -> Result<PoolReplayBodySnapshot, String> {
    if bytes.len() <= POOL_REQUEST_REPLAY_MEMORY_THRESHOLD_BYTES {
        return Ok(PoolReplayBodySnapshot::Memory(bytes));
    }

    let temp_file = Arc::new(PoolReplayTempFile {
        path: build_pool_replay_temp_path(proxy_request_id),
    });
    tokio::fs::write(&temp_file.path, &bytes)
        .await
        .map_err(|err| format!("failed to persist rewritten pool request body: {err}"))?;
    Ok(PoolReplayBodySnapshot::File {
        temp_file,
        size: bytes.len(),
        sticky_key,
    })
}

fn build_pool_replay_temp_path(proxy_request_id: u64) -> PathBuf {
    let mut path = env::temp_dir();
    path.push(format!(
        "cvm-pool-replay-{proxy_request_id}-{}.bin",
        Utc::now().timestamp_nanos_opt().unwrap_or_default()
    ));
    path
}

fn spawn_pool_replayable_request_body(
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
                    let _ = sticky_key_probe_tx.send(PoolReplayBodyStickyKeyProbeStatus::Ready(None));
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
                    let _ = sticky_key_probe_tx.send(PoolReplayBodyStickyKeyProbeStatus::Ready(None));
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
                        let _ = sticky_key_probe_tx.send(PoolReplayBodyStickyKeyProbeStatus::Ready(None));
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
                                let _ = sticky_key_probe_tx.send(PoolReplayBodyStickyKeyProbeStatus::Ready(None));
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
                        best_effort_extract_sticky_key_from_request_body_prefix(&sticky_key_probe),
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
                        let _ = sticky_key_probe_tx.send(PoolReplayBodyStickyKeyProbeStatus::Ready(None));
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
                    let _ = sticky_key_probe_tx.send(PoolReplayBodyStickyKeyProbeStatus::Ready(None));
                }
                let _ = status_tx.send(PoolReplayBodyStatus::ReadError(read_error.clone()));
                let _ = tx.send(Err(io::Error::other(read_error.message))).await;
                return;
            }
            data_len = data_len.saturating_add(chunk.len());

            if let Err(err) = buffer.append(&chunk).await {
                let msg = format!("failed to cache replayable request body: {err}");
                if !sticky_key_probe_ready {
                    let _ = sticky_key_probe_tx.send(PoolReplayBodyStickyKeyProbeStatus::Ready(None));
                }
                let _ = tx.send(Err(io::Error::other(msg.clone()))).await;
                let _ = status_tx.send(PoolReplayBodyStatus::InternalError(msg));
                return;
            }

            if !sticky_key_probe_ready && sticky_key_probe.len() < HEADER_STICKY_EARLY_STICKY_SCAN_BYTES
            {
                let probe_remaining =
                    HEADER_STICKY_EARLY_STICKY_SCAN_BYTES.saturating_sub(sticky_key_probe.len());
                sticky_key_probe.extend_from_slice(&chunk[..chunk.len().min(probe_remaining)]);
                if let Some(sticky_key) =
                    best_effort_extract_sticky_key_from_request_body_prefix(&sticky_key_probe)
                {
                    sticky_key_probe_ready = true;
                    let _ = sticky_key_probe_tx
                        .send(PoolReplayBodyStickyKeyProbeStatus::Ready(Some(sticky_key)));
                } else if sticky_key_probe.len() >= HEADER_STICKY_EARLY_STICKY_SCAN_BYTES {
                    sticky_key_probe_ready = true;
                    let _ = sticky_key_probe_tx.send(PoolReplayBodyStickyKeyProbeStatus::Ready(None));
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

async fn wait_for_replay_body_sticky_key_probe(
    sticky_key_probe_rx: &watch::Receiver<PoolReplayBodyStickyKeyProbeStatus>,
    max_wait: Duration,
) -> Option<String> {
    let mut sticky_key_probe_rx = sticky_key_probe_rx.clone();
    let wait_deadline = Instant::now() + max_wait;
    loop {
        match sticky_key_probe_rx.borrow().clone() {
            PoolReplayBodyStickyKeyProbeStatus::Ready(sticky_key) => return sticky_key,
            PoolReplayBodyStickyKeyProbeStatus::Pending => {}
        }
        let remaining = wait_deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return None;
        }
        match timeout(remaining, sticky_key_probe_rx.changed()).await {
            Ok(Ok(())) => {}
            Ok(Err(_)) | Err(_) => return None,
        }
    }
}

fn live_body_sticky_key_probe_wait_timeout(
    request_read_timeout: Duration,
    pre_attempt_total_timeout_deadline: Option<Instant>,
) -> Duration {
    match pre_attempt_total_timeout_deadline {
        Some(deadline) => request_read_timeout.min(deadline.saturating_duration_since(Instant::now())),
        None => request_read_timeout,
    }
}

async fn wait_for_replay_body_snapshot(
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
