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

    match proxy_openai_v1_inner(
        state,
        proxy_request_id,
        invoke_id.clone(),
        original_uri,
        method,
        headers,
        body,
        peer_ip,
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
            match err.cvm_id {
                Some(cvm_id) => {
                    let mut response = (
                        err.status,
                        Json(json!({ "error": err.message, "cvmId": cvm_id })),
                    )
                        .into_response();
                    if let Ok(header_value) = HeaderValue::from_str(&invoke_id) {
                        response
                            .headers_mut()
                            .insert(HeaderName::from_static(CVM_INVOKE_ID_HEADER), header_value);
                    }
                    if let Some(retry_after_secs) = err.retry_after_secs
                        && let Ok(header_value) =
                            HeaderValue::from_str(&retry_after_secs.to_string())
                    {
                        response
                            .headers_mut()
                            .insert(header::RETRY_AFTER, header_value);
                    }
                    response
                }
                None => {
                    let mut response =
                        (err.status, Json(json!({ "error": err.message }))).into_response();
                    if let Some(retry_after_secs) = err.retry_after_secs
                        && let Ok(header_value) =
                            HeaderValue::from_str(&retry_after_secs.to_string())
                    {
                        response
                            .headers_mut()
                            .insert(header::RETRY_AFTER, header_value);
                    }
                    response
                }
            }
        }
    }
}

#[derive(Debug)]
struct ProxyErrorResponse {
    status: StatusCode,
    message: String,
    cvm_id: Option<String>,
    retry_after_secs: Option<u64>,
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
    pub(crate) deferred_early_phase_cleanup_guard: Option<PoolEarlyPhaseOrphanCleanupGuard>,
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

fn retry_after_secs_for_proxy_error(status: StatusCode, message: &str) -> Option<u64> {
    (status == StatusCode::SERVICE_UNAVAILABLE && message == POOL_NO_AVAILABLE_ACCOUNT_MESSAGE)
        .then_some(DEFAULT_POOL_NO_AVAILABLE_ACCOUNT_RETRY_AFTER_SECS)
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

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
struct InvocationRecoverySelector {
    invoke_id: String,
    occurred_at: String,
}

impl InvocationRecoverySelector {
    fn new(invoke_id: impl Into<String>, occurred_at: impl Into<String>) -> Self {
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
struct RecoveredPoolAttemptRow {
    id: i64,
    invoke_id: String,
    occurred_at: String,
}

#[derive(Debug, Clone, FromRow)]
struct RecoveredInvocationRow {
    id: i64,
    invoke_id: String,
    occurred_at: String,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct PoolOrphanRecoveryOutcome {
    pub(crate) recovered_attempts: usize,
    pub(crate) recovered_invocations: usize,
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
const POOL_EARLY_PHASE_ORPHAN_RECOVERY_GRACE: Duration = Duration::from_secs(30);
const PROXY_INVOCATION_RECOVERY_SELECTOR_BATCH_SIZE: usize = 400;

struct PoolEarlyPhaseOrphanCleanupGuard {
    state: Arc<AppState>,
    pending_attempt_record: PendingPoolAttemptRecord,
    armed: bool,
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
    fn new(state: Arc<AppState>, pending_attempt_record: PendingPoolAttemptRecord) -> Self {
        Self {
            state,
            pending_attempt_record,
            armed: true,
        }
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for PoolEarlyPhaseOrphanCleanupGuard {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }

        let state = self.state.clone();
        let pending_attempt_record = self.pending_attempt_record.clone();
        tokio::spawn(async move {
            if let Err(err) =
                recover_guard_dropped_pool_early_phase_orphan(state.as_ref(), pending_attempt_record)
                    .await
            {
                warn!(error = %err, "failed to recover dropped pool early-phase orphan");
            }
        });
    }
}

fn disarm_pool_early_phase_cleanup_guard(
    guard: &mut Option<PoolEarlyPhaseOrphanCleanupGuard>,
) {
    if let Some(guard) = guard.as_mut() {
        guard.disarm();
    }
}
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

struct PoolReplayBodyBuffer {
    proxy_request_id: u64,
    len: usize,
    memory: Vec<u8>,
    file: Option<(Arc<PoolReplayTempFile>, tokio::fs::File)>,
}

struct PoolReplayableRequestBody {
    body: reqwest::Body,
    status_rx: watch::Receiver<PoolReplayBodyStatus>,
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
const OAUTH_RESPONSES_MAX_REWRITE_BODY_BYTES: usize = 8 * 1024 * 1024;

impl PoolReplayBodyBuffer {
    fn new(proxy_request_id: u64) -> Self {
        Self {
            proxy_request_id,
            len: 0,
            memory: Vec::new(),
            file: None,
        }
    }

    async fn append(&mut self, chunk: &[u8]) -> io::Result<()> {
        self.len = self.len.saturating_add(chunk.len());
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
            Self::File { temp_file, size } => {
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
}

#[derive(Debug, Clone)]
struct PreparedPoolRequestBody {
    snapshot: PoolReplayBodySnapshot,
    request_body_for_capture: Option<Bytes>,
    requested_service_tier: Option<String>,
}

fn pool_request_snapshot_preserves_content_length(snapshot: &PoolReplayBodySnapshot) -> bool {
    matches!(snapshot, PoolReplayBodySnapshot::File { .. })
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
    Ok(PreparedPoolRequestBody {
        snapshot: PoolReplayBodySnapshot::Memory(rewritten_bytes.clone()),
        request_body_for_capture: Some(rewritten_bytes.clone()),
        requested_service_tier,
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
    let cancel = CancellationToken::new();
    let cancel_for_task = cancel.clone();

    tokio::spawn(async move {
        let mut buffer = PoolReplayBodyBuffer::new(proxy_request_id);
        let mut data_len = 0usize;
        let mut stream = body.into_data_stream();
        let read_deadline = Instant::now() + request_read_timeout;

        loop {
            if cancel_for_task.is_cancelled() {
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
                let _ = status_tx.send(PoolReplayBodyStatus::ReadError(read_error.clone()));
                let _ = tx.send(Err(io::Error::other(read_error.message))).await;
                return;
            }
            data_len = data_len.saturating_add(chunk.len());

            if let Err(err) = buffer.append(&chunk).await {
                let msg = format!("failed to cache replayable request body: {err}");
                let _ = tx.send(Err(io::Error::other(msg.clone()))).await;
                let _ = status_tx.send(PoolReplayBodyStatus::InternalError(msg));
                return;
            }

            if tx.send(Ok(chunk)).await.is_err() {
                let _ = status_tx.send(PoolReplayBodyStatus::Incomplete);
                return;
            }
        }
    });

    PoolReplayableRequestBody {
        body: reqwest::Body::wrap_stream(ReceiverStream::new(rx)),
        status_rx,
        cancel,
    }
}

fn parse_retry_after_delay(value: &HeaderValue) -> Option<Duration> {
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

async fn resolve_pool_account_for_request_with_wait(
    state: &AppState,
    sticky_key: Option<&str>,
    excluded_ids: &[i64],
    excluded_upstream_route_keys: &HashSet<String>,
    required_upstream_route_key: Option<&str>,
    wait_for_no_available: bool,
    wait_deadline: &mut Option<Instant>,
    total_timeout_deadline: Option<Instant>,
) -> Result<PoolAccountResolutionWithWait> {
    let poll_interval = state.pool_no_available_wait.normalized_poll_interval();

    loop {
        if total_timeout_deadline.is_some_and(|deadline| Instant::now() >= deadline) {
            return Ok(PoolAccountResolutionWithWait::TotalTimeoutExpired);
        }
        let resolution = resolve_pool_account_for_request_with_route_requirement(
            state,
            sticky_key,
            excluded_ids,
            excluded_upstream_route_keys,
            required_upstream_route_key,
        )
        .await?;
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
                tokio::time::sleep(
                    poll_interval.min(effective_deadline.saturating_duration_since(now)),
                )
                .await;
            }
            _ => return Ok(PoolAccountResolutionWithWait::Resolution(resolution)),
        }
    }
}

async fn send_pool_request_with_failover(
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
    let request_connection_scoped = connection_scoped_header_names(headers);
    let reservation_key = build_pool_routing_reservation_key(proxy_request_id);
    let mut reservation_guard =
        PoolRoutingReservationDropGuard::new(state.clone(), reservation_key.clone());
    let runtime_timeouts = resolve_pool_routing_timeouts(&state.pool, &state.config)
        .await
        .map_err(|err| PoolUpstreamError {
            account: preferred_account.clone(),
            status: StatusCode::BAD_GATEWAY,
            message: format!("failed to resolve pool routing timeouts: {err}"),
            failure_kind: PROXY_FAILURE_POOL_NO_AVAILABLE_ACCOUNT,
            connect_latency_ms: 0.0,
            upstream_error_code: None,
            upstream_error_message: None,
            upstream_request_id: None,
            oauth_responses_debug: None,
            attempt_summary: pool_attempt_summary(
                failover_progress.attempt_count,
                failover_progress.excluded_account_ids.len(),
                Some(PROXY_FAILURE_POOL_NO_AVAILABLE_ACCOUNT.to_string()),
            ),
            requested_service_tier: None,
            request_body_for_capture: None,
        })?;
    let pre_first_byte_timeout =
        pool_upstream_first_chunk_timeout(&runtime_timeouts, original_uri, &method);
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
    let send_timeout = pool_upstream_send_timeout(
        original_uri,
        &method,
        handshake_timeout,
        pre_first_byte_timeout,
    );
    let mut excluded_ids = failover_progress.excluded_account_ids;
    let mut excluded_upstream_route_keys = failover_progress.excluded_upstream_route_keys;
    let mut last_error = failover_progress.last_error;
    let mut preserve_sticky_owner_terminal_error =
        failover_progress.preserve_sticky_owner_terminal_error
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
        .filter(|account| !excluded_upstream_route_keys.contains(&account.upstream_route_key()));
    let initial_same_account_attempts = same_account_attempts.max(1);
    let mut attempt_count = failover_progress.attempt_count;
    let mut timeout_route_failover_pending = failover_progress.timeout_route_failover_pending;
    let mut exhausted_accounts_all_rate_limited = initial_errors_all_rate_limited;
    let mut no_available_wait_deadline = failover_progress.no_available_wait_deadline;

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
                account: None,
                status: StatusCode::BAD_GATEWAY,
                message: terminal_message.clone(),
                failure_kind: terminal_failure_kind,
                connect_latency_ms: 0.0,
                upstream_error_code: None,
                upstream_error_message: None,
                upstream_request_id: None,
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
            {
                final_error.status = StatusCode::BAD_GATEWAY;
                final_error.message = terminal_message;
                final_error.failure_kind = terminal_failure_kind;
                final_error.upstream_error_code = None;
                final_error.upstream_error_message = None;
                final_error.upstream_request_id = None;
            } else if final_error.status != StatusCode::TOO_MANY_REQUESTS {
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
            let wait_for_no_available = if attempt_count == 0 {
                true
            } else if uses_timeout_route_failover {
                !timeout_route_failover_pending
            } else if sticky_owner_terminal_error_preservation_is_active(
                preserve_sticky_owner_terminal_error,
                last_error.as_ref(),
            ) {
                false
            } else {
                false
            };
            let total_timeout_deadline = responses_total_timeout_started_at
                .zip(responses_total_timeout)
                .map(|(started_at, total_timeout)| started_at + total_timeout)
                .or_else(|| {
                    if attempt_count == 0 {
                        pre_attempt_total_timeout_deadline
                    } else {
                        None
                    }
                });
            let route_scoped_overload_selection =
                overload_required_upstream_route_key.clone();
            match resolve_pool_account_for_request_with_wait(
                state.as_ref(),
                sticky_key,
                &excluded_ids,
                &excluded_upstream_route_keys,
                route_scoped_overload_selection.as_deref(),
                wait_for_no_available,
                &mut no_available_wait_deadline,
                total_timeout_deadline,
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
                            account: None,
                            status: StatusCode::BAD_GATEWAY,
                            message: "no alternate upstream route is available after timeout"
                                .to_string(),
                            failure_kind: terminal_failure_kind,
                            connect_latency_ms: 0.0,
                            upstream_error_code: None,
                            upstream_error_message: None,
                            upstream_request_id: None,
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
                    if uses_timeout_route_failover && timeout_route_failover_pending {
                        let mut err = last_error.unwrap_or(PoolUpstreamError {
                            account: None,
                            status: StatusCode::BAD_GATEWAY,
                            message: "no alternate upstream route is available after timeout"
                                .to_string(),
                            failure_kind: PROXY_FAILURE_POOL_NO_ALTERNATE_UPSTREAM_AFTER_TIMEOUT,
                            connect_latency_ms: 0.0,
                            upstream_error_code: None,
                            upstream_error_message: None,
                            upstream_request_id: None,
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
                    PoolAccountResolution::BlockedByPolicy(message),
                )) => {
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
                    let terminal_failure_kind = PROXY_FAILURE_POOL_NO_AVAILABLE_ACCOUNT;
                    let mut err = PoolUpstreamError {
                        account: None,
                        status: StatusCode::SERVICE_UNAVAILABLE,
                        message,
                        failure_kind: terminal_failure_kind,
                        connect_latency_ms: 0.0,
                        upstream_error_code: None,
                        upstream_error_message: None,
                        upstream_request_id: None,
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
                        account: None,
                        status: StatusCode::BAD_GATEWAY,
                        message: format!("failed to resolve pool account: {err}"),
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
                    });
                }
            }
        };
        if responses_total_timeout_started_at.is_none() && no_available_wait_deadline.is_some() {
            responses_total_timeout_started_at = pre_attempt_total_timeout_started_at;
        }
        reserve_pool_routing_account(state.as_ref(), &reservation_key, &account);
        timeout_route_failover_pending = false;

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
                            account: Some(account),
                            status: StatusCode::BAD_GATEWAY,
                            message: format!("failed to build pool upstream url: {err}"),
                            failure_kind: PROXY_FAILURE_FAILED_CONTACT_UPSTREAM,
                            connect_latency_ms: 0.0,
                            upstream_error_code: None,
                            upstream_error_message: None,
                            upstream_request_id: None,
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
        let group_upstream_429_max_retries = account.effective_group_upstream_429_max_retries();
        let same_account_attempt_loop_budget = overload_same_account_attempt_budget
            .saturating_add(group_upstream_429_max_retries);
        let mut group_upstream_429_retry_count = 0_u8;
        let mut first_response_attempt_started_at = None;

        for same_account_attempt in 0..same_account_attempt_loop_budget {
            if original_uri.path() == "/v1/responses" && first_response_attempt_started_at.is_none()
            {
                first_response_attempt_started_at = Some(Instant::now());
            }
            let attempt_total_timeout_started_at = if original_uri.path() == "/v1/responses" {
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
            let pending_attempt_record: Option<PendingPoolAttemptRecord>;
            let mut early_phase_cleanup_guard: Option<PoolEarlyPhaseOrphanCleanupGuard>;
            let prepared_request_body = match prepare_pool_request_body_for_account(
                body.as_ref(),
                original_uri,
                &method,
                account.fast_mode_rewrite_mode,
            )
            .await
            {
                Ok(prepared) => prepared,
                Err(message) => {
                    store_pool_failover_error(
                        &mut last_error,
                        &mut preserve_sticky_owner_terminal_error,
                        PoolUpstreamError {
                        account: Some(account.clone()),
                        status: StatusCode::BAD_GATEWAY,
                        message,
                        failure_kind: PROXY_FAILURE_FAILED_CONTACT_UPSTREAM,
                        connect_latency_ms: 0.0,
                        upstream_error_code: None,
                        upstream_error_message: None,
                        upstream_request_id: None,
                        oauth_responses_debug: None,
                        attempt_summary: PoolAttemptSummary::default(),
                        requested_service_tier: None,
                        request_body_for_capture: None,
                        },
                    );
                    exhausted_accounts_all_rate_limited = false;
                    continue 'account_loop;
                }
            };
            let attempted_requested_service_tier =
                prepared_request_body.requested_service_tier.clone();
            let attempted_request_body_for_capture =
                prepared_request_body.request_body_for_capture.clone();
            let (response, oauth_responses_debug, forward_proxy_selection) = match &account.auth {
                PoolResolvedAuth::ApiKey { authorization } => {
                    let (forward_proxy_scope, selected_proxy, client) =
                        match select_pool_account_forward_proxy_client(state.as_ref(), &account)
                            .await
                        {
                            Ok(selection) => selection,
                            Err(message) => {
                                if let Err(route_err) = record_pool_route_transport_failure(
                                    &state.pool,
                                    account.account_id,
                                    sticky_key,
                                    &message,
                                    trace_context.as_ref().map(|trace| trace.invoke_id.as_str()),
                                )
                                .await
                                {
                                    warn!(account_id = account.account_id, error = %route_err, "failed to record pool forward proxy selection failure");
                                }
                                store_pool_failover_error(
                                    &mut last_error,
                                    &mut preserve_sticky_owner_terminal_error,
                                    PoolUpstreamError {
                                    account: Some(account.clone()),
                                    status: StatusCode::BAD_GATEWAY,
                                    message: message.clone(),
                                    failure_kind: PROXY_FAILURE_FAILED_CONTACT_UPSTREAM,
                                    connect_latency_ms: 0.0,
                                    upstream_error_code: None,
                                    upstream_error_message: None,
                                    upstream_request_id: None,
                                    oauth_responses_debug: None,
                                    attempt_summary: PoolAttemptSummary::default(),
                                    requested_service_tier: attempted_requested_service_tier
                                        .clone(),
                                    request_body_for_capture: attempted_request_body_for_capture
                                        .clone(),
                                    },
                                );
                                exhausted_accounts_all_rate_limited = false;
                                continue 'account_loop;
                            }
                        };
                    attempt_count += 1;
                    attempt_index = attempt_count as i64;
                    attempt_started_at = shanghai_now_string();
                    let mut request = client.request(
                        method.clone(),
                        api_key_target_url
                            .clone()
                            .expect("api key pool route should always have an upstream url"),
                    );
                    let preserve_content_length = pool_request_snapshot_preserves_content_length(
                        &prepared_request_body.snapshot,
                    );
                    let forwarded_content_length = headers
                        .get(header::CONTENT_LENGTH)
                        .and_then(|value| value.to_str().ok())
                        .map(str::to_string);
                    let outbound_snapshot_kind =
                        pool_request_snapshot_kind(&prepared_request_body.snapshot);
                    let outbound_body_bytes =
                        pool_request_snapshot_body_bytes(&prepared_request_body.snapshot);
                    for (name, value) in headers {
                        if *name == header::AUTHORIZATION {
                            continue;
                        }
                        if *name == header::CONTENT_LENGTH && !preserve_content_length {
                            continue;
                        }
                        if should_forward_proxy_header(name, &request_connection_scoped) {
                            request = request.header(name, value);
                        }
                    }
                    request = request.header(header::AUTHORIZATION, authorization.clone());
                    request = request.body(prepared_request_body.snapshot.to_reqwest_body());
                    if let Err(route_err) =
                        record_account_selected(&state.pool, account.account_id).await
                    {
                        warn!(
                            account_id = account.account_id,
                            error = %route_err,
                            "failed to record selected pool account"
                        );
                    }
                    pending_attempt_record = if let Some(trace) = trace_context.as_ref() {
                        Some(
                            begin_pool_upstream_request_attempt(
                                &state.pool,
                                trace,
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
                    let attempt_runtime_snapshot = runtime_snapshot_context.as_ref().map(|ctx| {
                        let mut ctx = ctx.clone();
                        ctx.request_info.requested_service_tier =
                            attempted_requested_service_tier.clone();
                        ctx
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
                        )
                        .await;
                    }
                    early_phase_cleanup_guard = pending_attempt_record
                        .as_ref()
                        .map(|pending| PoolEarlyPhaseOrphanCleanupGuard::new(state.clone(), pending.clone()));
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
                    match timeout(attempt_send_timeout, request.send()).await {
                        Ok(Ok(response)) => (
                            ProxyUpstreamResponseBody::Reqwest(response),
                            None,
                            Some((forward_proxy_scope, selected_proxy)),
                        ),
                        Ok(Err(err)) => {
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
                                snapshot_kind = outbound_snapshot_kind,
                                outbound_body_bytes,
                                forwarded_content_length = forwarded_content_length.as_deref(),
                                preserved_content_length = preserve_content_length,
                                error = %err,
                                "pool upstream request send failed before response"
                            );
                            let message = format!("failed to contact upstream: {err}");
                            let compact_support_observation = classify_compact_support_observation(
                                original_uri,
                                None,
                                Some(message.as_str()),
                            );
                            let timeout_shaped_failure = pool_failure_is_timeout_shaped(
                                PROXY_FAILURE_FAILED_CONTACT_UPSTREAM,
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
                                    Some(PROXY_FAILURE_FAILED_CONTACT_UPSTREAM),
                                    Some(message.as_str()),
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
                            let has_retry_budget =
                                same_account_attempt + 1 < same_account_attempt_budget;
                            if has_retry_budget && !should_timeout_route_failover {
                                let retry_delay = fallback_proxy_429_retry_delay(
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
                            if let Err(route_err) = record_pool_route_transport_failure(
                                &state.pool,
                                account.account_id,
                                sticky_key,
                                &message,
                                trace_context.as_ref().map(|trace| trace.invoke_id.as_str()),
                            )
                            .await
                            {
                                warn!(account_id = account.account_id, error = %route_err, "failed to record pool transport failure");
                            }
                            store_pool_failover_error(
                                &mut last_error,
                                &mut preserve_sticky_owner_terminal_error,
                                PoolUpstreamError {
                                account: Some(account.clone()),
                                status: StatusCode::BAD_GATEWAY,
                                message: message.clone(),
                                failure_kind: PROXY_FAILURE_FAILED_CONTACT_UPSTREAM,
                                connect_latency_ms: elapsed_ms(connect_started),
                                upstream_error_code: None,
                                upstream_error_message: None,
                                upstream_request_id: None,
                                oauth_responses_debug: None,
                                attempt_summary: PoolAttemptSummary::default(),
                                requested_service_tier: attempted_requested_service_tier.clone(),
                                request_body_for_capture: attempted_request_body_for_capture
                                    .clone(),
                                },
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
                        Err(_) => {
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
                                snapshot_kind = outbound_snapshot_kind,
                                outbound_body_bytes,
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
                                    Some(PROXY_FAILURE_UPSTREAM_HANDSHAKE_TIMEOUT),
                                    Some(message.as_str()),
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
                                        account: Some(account.clone()),
                                        status: StatusCode::BAD_GATEWAY,
                                        message: message.clone(),
                                        failure_kind: PROXY_FAILURE_UPSTREAM_HANDSHAKE_TIMEOUT,
                                        connect_latency_ms: elapsed_ms(connect_started),
                                        upstream_error_code: None,
                                        upstream_error_message: None,
                                        upstream_request_id: None,
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
                            let has_retry_budget =
                                same_account_attempt + 1 < same_account_attempt_budget;
                            if has_retry_budget && !should_timeout_route_failover {
                                let retry_delay = fallback_proxy_429_retry_delay(
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
                            if let Err(route_err) = record_pool_route_transport_failure(
                                &state.pool,
                                account.account_id,
                                sticky_key,
                                &message,
                                trace_context.as_ref().map(|trace| trace.invoke_id.as_str()),
                            )
                            .await
                            {
                                warn!(account_id = account.account_id, error = %route_err, "failed to record pool handshake timeout");
                            }
                            store_pool_failover_error(
                                &mut last_error,
                                &mut preserve_sticky_owner_terminal_error,
                                PoolUpstreamError {
                                account: Some(account.clone()),
                                status: StatusCode::BAD_GATEWAY,
                                message: message.clone(),
                                failure_kind: PROXY_FAILURE_UPSTREAM_HANDSHAKE_TIMEOUT,
                                connect_latency_ms: elapsed_ms(connect_started),
                                upstream_error_code: None,
                                upstream_error_message: None,
                                upstream_request_id: None,
                                oauth_responses_debug: None,
                                attempt_summary: PoolAttemptSummary::default(),
                                requested_service_tier: attempted_requested_service_tier.clone(),
                                request_body_for_capture: attempted_request_body_for_capture
                                    .clone(),
                                },
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
                    }
                }
                PoolResolvedAuth::Oauth {
                    access_token,
                    chatgpt_account_id,
                } => {
                    let (forward_proxy_scope, selected_proxy, client) =
                        match select_pool_account_forward_proxy_client(state.as_ref(), &account)
                            .await
                        {
                            Ok(selection) => selection,
                            Err(message) => {
                                if let Err(route_err) = record_pool_route_transport_failure(
                                    &state.pool,
                                    account.account_id,
                                    sticky_key,
                                    &message,
                                    trace_context.as_ref().map(|trace| trace.invoke_id.as_str()),
                                )
                                .await
                                {
                                    warn!(account_id = account.account_id, error = %route_err, "failed to record pool oauth forward proxy selection failure");
                                }
                                store_pool_failover_error(
                                    &mut last_error,
                                    &mut preserve_sticky_owner_terminal_error,
                                    PoolUpstreamError {
                                    account: Some(account.clone()),
                                    status: StatusCode::BAD_GATEWAY,
                                    message: message.clone(),
                                    failure_kind: PROXY_FAILURE_FAILED_CONTACT_UPSTREAM,
                                    connect_latency_ms: 0.0,
                                    upstream_error_code: None,
                                    upstream_error_message: None,
                                    upstream_request_id: None,
                                    oauth_responses_debug: None,
                                    attempt_summary: PoolAttemptSummary::default(),
                                    requested_service_tier: attempted_requested_service_tier
                                        .clone(),
                                    request_body_for_capture: attempted_request_body_for_capture
                                        .clone(),
                                    },
                                );
                                exhausted_accounts_all_rate_limited = false;
                                continue 'account_loop;
                            }
                        };
                    let oauth_body = match &prepared_request_body.snapshot {
                        PoolReplayBodySnapshot::File { size, .. }
                            if original_uri.path() == "/v1/responses"
                                && *size > OAUTH_RESPONSES_MAX_REWRITE_BODY_BYTES =>
                        {
                            store_pool_failover_error(
                                &mut last_error,
                                &mut preserve_sticky_owner_terminal_error,
                                PoolUpstreamError {
                                account: Some(account.clone()),
                                status: StatusCode::PAYLOAD_TOO_LARGE,
                                message: format!(
                                    "oauth /v1/responses request body exceeds {} bytes rewrite limit",
                                    OAUTH_RESPONSES_MAX_REWRITE_BODY_BYTES
                                ),
                                failure_kind: PROXY_FAILURE_BODY_TOO_LARGE,
                                connect_latency_ms: 0.0,
                                upstream_error_code: None,
                                upstream_error_message: None,
                                upstream_request_id: None,
                                oauth_responses_debug: None,
                                attempt_summary: pool_attempt_summary(
                                    attempt_count,
                                    distinct_account_count,
                                    Some(PROXY_FAILURE_BODY_TOO_LARGE.to_string()),
                                ),
                                requested_service_tier: attempted_requested_service_tier.clone(),
                                request_body_for_capture:
                                    attempted_request_body_for_capture.clone(),
                                },
                            );
                            exhausted_accounts_all_rate_limited = false;
                            continue 'account_loop;
                        }
                        snapshot if original_uri.path() == "/v1/responses" => {
                            oauth_bridge::OauthUpstreamRequestBody::Bytes(
                                snapshot.to_bytes().await.map_err(|err| PoolUpstreamError {
                                    account: Some(account.clone()),
                                    status: StatusCode::BAD_GATEWAY,
                                    message: format!("failed to replay oauth request body: {err}"),
                                    failure_kind: PROXY_FAILURE_FAILED_CONTACT_UPSTREAM,
                                    connect_latency_ms: 0.0,
                                    upstream_error_code: None,
                                    upstream_error_message: None,
                                    upstream_request_id: None,
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
                        snapshot => oauth_bridge::OauthUpstreamRequestBody::Stream {
                            debug_body_prefix: Some(
                                snapshot
                                    .to_prefix_bytes(
                                        oauth_bridge::OAUTH_REQUEST_BODY_PREFIX_FINGERPRINT_MAX_BYTES,
                                    )
                                    .await
                                    .map_err(|err| PoolUpstreamError {
                                        account: Some(account.clone()),
                                        status: StatusCode::BAD_GATEWAY,
                                        message: format!(
                                            "failed to replay oauth request body prefix: {err}"
                                        ),
                                        failure_kind: PROXY_FAILURE_FAILED_CONTACT_UPSTREAM,
                                        connect_latency_ms: 0.0,
                                        upstream_error_code: None,
                                        upstream_error_message: None,
                                        upstream_request_id: None,
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
                            body: snapshot.to_reqwest_body(),
                        },
                    };
                    attempt_count += 1;
                    attempt_index = attempt_count as i64;
                    attempt_started_at = shanghai_now_string();
                    if let Err(route_err) =
                        record_account_selected(&state.pool, account.account_id).await
                    {
                        warn!(
                            account_id = account.account_id,
                            error = %route_err,
                            "failed to record selected pool account"
                        );
                    }
                    pending_attempt_record = if let Some(trace) = trace_context.as_ref() {
                        Some(
                            begin_pool_upstream_request_attempt(
                                &state.pool,
                                trace,
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
                    let attempt_runtime_snapshot = runtime_snapshot_context.as_ref().map(|ctx| {
                        let mut ctx = ctx.clone();
                        ctx.request_info.requested_service_tier =
                            attempted_requested_service_tier.clone();
                        ctx
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
                        )
                        .await;
                    }
                    early_phase_cleanup_guard = pending_attempt_record
                        .as_ref()
                        .map(|pending| PoolEarlyPhaseOrphanCleanupGuard::new(state.clone(), pending.clone()));
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
                        let oauth_response = oauth_bridge::send_oauth_upstream_request(
                            &client,
                            method.clone(),
                            original_uri,
                            headers,
                            oauth_body,
                            attempt_send_timeout,
                            attempt_pre_first_byte_timeout,
                            Some(account.account_id),
                            access_token,
                            chatgpt_account_id.as_deref(),
                            state.upstream_accounts.crypto_key.as_ref(),
                        )
                        .await;
                        (
                            ProxyUpstreamResponseBody::Axum(oauth_response.response),
                            oauth_response.request_debug,
                            Some((forward_proxy_scope, selected_proxy)),
                        )
                    }
                }
            };

            let connect_latency_ms = elapsed_ms(connect_started);
            let status = response.status();
            if status == StatusCode::TOO_MANY_REQUESTS
                || status.is_server_error()
                || matches!(
                    status,
                    StatusCode::UNAUTHORIZED | StatusCode::PAYMENT_REQUIRED | StatusCode::FORBIDDEN
                )
            {
                let has_retry_budget = same_account_attempt + 1 < same_account_attempt_budget;
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
                let (upstream_error_code, upstream_error_message, upstream_request_id, message) =
                    match read_pool_upstream_bytes_with_timeout(
                        response,
                        attempt_pre_first_byte_timeout,
                        connect_started,
                        "reading upstream error body",
                    )
                    .await
                    {
                        Ok(body_bytes) => summarize_pool_upstream_http_failure(
                            status,
                            upstream_request_id_header.as_deref(),
                            &body_bytes,
                        ),
                        Err(err) => (
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
                let http_failure_classification =
                    classify_pool_account_http_failure(&account.kind, status, &route_error_message);
                let failure_kind = oauth_transport_failure_kind
                    .unwrap_or(http_failure_classification.failure_kind);
                let compact_support_observation = classify_compact_support_observation(
                    original_uri,
                    Some(status),
                    Some(route_error_message.as_str()),
                );
                let timeout_shaped_failure = status.is_server_error()
                    && pool_failure_is_timeout_shaped(failure_kind, &message);
                let should_timeout_route_failover =
                    uses_timeout_route_failover && timeout_shaped_failure;
                let retry_delay = (has_retry_budget
                    && !should_timeout_route_failover
                    && status.is_server_error()
                    && status != StatusCode::TOO_MANY_REQUESTS)
                    .then(|| {
                        retry_after_header
                            .as_ref()
                            .and_then(parse_retry_after_delay)
                            .unwrap_or_else(|| {
                                fallback_proxy_429_retry_delay(u32::from(same_account_attempt) + 1)
                            })
                    });
                let finished_at = shanghai_now_string();
                if let Some(pending_attempt_record) = pending_attempt_record.as_ref()
                    && let Err(record_err) = finalize_pool_upstream_request_attempt(
                        &state.pool,
                        pending_attempt_record,
                        finished_at.as_str(),
                        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_HTTP_FAILURE,
                        Some(status),
                        Some(failure_kind),
                        Some(message.as_str()),
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
                    if has_group_upstream_429_retry_budget {
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
                    if let Some(retry_delay) = retry_delay {
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
                if let Err(route_err) = record_pool_route_http_failure(
                    &state.pool,
                    account.account_id,
                    &account.kind,
                    sticky_key,
                    status,
                    &route_error_message,
                    trace_context.as_ref().map(|trace| trace.invoke_id.as_str()),
                )
                .await
                {
                    warn!(account_id = account.account_id, error = %route_err, "failed to record pool upstream http failure");
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
                    account: Some(account.clone()),
                    status,
                    message: message.clone(),
                    failure_kind,
                    connect_latency_ms,
                    upstream_error_code,
                    upstream_error_message,
                    upstream_request_id,
                    oauth_responses_debug: oauth_responses_debug.clone(),
                    attempt_summary: PoolAttemptSummary::default(),
                    requested_service_tier: attempted_requested_service_tier.clone(),
                    request_body_for_capture: attempted_request_body_for_capture.clone(),
                    },
                );
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
            let (response, first_chunk) = match read_pool_upstream_first_chunk_with_timeout(
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
                            Some(PROXY_FAILURE_UPSTREAM_STREAM_ERROR),
                            Some(message.as_str()),
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
                                account: Some(account.clone()),
                                status: StatusCode::BAD_GATEWAY,
                                message: message.clone(),
                                failure_kind: PROXY_FAILURE_UPSTREAM_STREAM_ERROR,
                                connect_latency_ms,
                                upstream_error_code: None,
                                upstream_error_message: None,
                                upstream_request_id: None,
                                oauth_responses_debug: oauth_responses_debug.clone(),
                                attempt_summary: PoolAttemptSummary::default(),
                                requested_service_tier: attempted_requested_service_tier.clone(),
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
                    let has_retry_budget = same_account_attempt + 1 < same_account_attempt_budget;
                    if has_retry_budget && !should_timeout_route_failover {
                        let retry_delay =
                            fallback_proxy_429_retry_delay(u32::from(same_account_attempt) + 1);
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
                    if let Err(route_err) = record_pool_route_transport_failure(
                        &state.pool,
                        account.account_id,
                        sticky_key,
                        &message,
                        trace_context.as_ref().map(|trace| trace.invoke_id.as_str()),
                    )
                    .await
                    {
                        warn!(account_id = account.account_id, error = %route_err, "failed to record pool first chunk failure");
                    }
                    store_pool_failover_error(
                        &mut last_error,
                        &mut preserve_sticky_owner_terminal_error,
                        PoolUpstreamError {
                        account: Some(account.clone()),
                        status: StatusCode::BAD_GATEWAY,
                        message: message.clone(),
                        failure_kind: PROXY_FAILURE_UPSTREAM_STREAM_ERROR,
                        connect_latency_ms,
                        upstream_error_code: None,
                        upstream_error_message: None,
                        upstream_request_id: None,
                        oauth_responses_debug: oauth_responses_debug.clone(),
                        attempt_summary: PoolAttemptSummary::default(),
                        requested_service_tier: attempted_requested_service_tier.clone(),
                        request_body_for_capture: attempted_request_body_for_capture.clone(),
                        },
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
            let response_is_event_stream = response
                .headers()
                .get(header::CONTENT_TYPE)
                .and_then(|value| value.to_str().ok())
                .is_some_and(|value| value.starts_with("text/event-stream"));
            let initial_gate_outcome = if original_uri.path() == "/v1/responses"
                && status == StatusCode::OK
                && response_is_event_stream
            {
                gate_pool_initial_response_stream(
                    response,
                    first_chunk,
                    attempt_pre_first_byte_timeout,
                    connect_started,
                )
                .await
            } else if original_uri.path() == "/v1/responses/compact" && status == StatusCode::OK {
                Ok(gate_pool_initial_compact_response(status, response.headers(), first_chunk.as_ref())
                    .unwrap_or(PoolInitialResponseGateOutcome::Forward {
                        response,
                        prefetched_bytes: first_chunk,
                    }))
            } else {
                Ok(PoolInitialResponseGateOutcome::Forward {
                    response,
                    prefetched_bytes: first_chunk,
                })
            };
            let (response, first_chunk) = match initial_gate_outcome {
                Ok(PoolInitialResponseGateOutcome::Forward {
                    response,
                    prefetched_bytes,
                }) => (response, prefetched_bytes),
                Ok(PoolInitialResponseGateOutcome::RetrySameAccount {
                    message,
                    upstream_error_code,
                    upstream_error_message,
                    upstream_request_id,
                }) => {
                    let finished_at = shanghai_now_string();
                    if let Some(pending_attempt_record) = pending_attempt_record.as_ref()
                        && let Err(record_err) = finalize_pool_upstream_request_attempt(
                            &state.pool,
                            pending_attempt_record,
                            finished_at.as_str(),
                            POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_HTTP_FAILURE,
                            Some(status),
                            Some(PROXY_FAILURE_UPSTREAM_RESPONSE_FAILED),
                            Some(message.as_str()),
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
                        let retry_delay =
                            fallback_proxy_429_retry_delay(u32::from(same_account_attempt) + 1);
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

                    if let Err(route_err) = record_pool_route_retryable_overload_failure(
                        &state.pool,
                        account.account_id,
                        sticky_key,
                        &message,
                        trace_context.as_ref().map(|trace| trace.invoke_id.as_str()),
                    )
                    .await
                    {
                        warn!(account_id = account.account_id, error = %route_err, "failed to record retryable response.failed route state");
                    }
                    store_pool_failover_error(
                        &mut last_error,
                        &mut preserve_sticky_owner_terminal_error,
                        PoolUpstreamError {
                        account: Some(account.clone()),
                        status,
                        message: message.clone(),
                        failure_kind: PROXY_FAILURE_UPSTREAM_RESPONSE_FAILED,
                        connect_latency_ms,
                        upstream_error_code,
                        upstream_error_message,
                        upstream_request_id,
                        oauth_responses_debug: oauth_responses_debug.clone(),
                        attempt_summary: PoolAttemptSummary::default(),
                        requested_service_tier: attempted_requested_service_tier.clone(),
                        request_body_for_capture: attempted_request_body_for_capture.clone(),
                        },
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
                            Some(PROXY_FAILURE_UPSTREAM_STREAM_ERROR),
                            Some(message.as_str()),
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
                    if let Err(route_err) = record_pool_route_transport_failure(
                        &state.pool,
                        account.account_id,
                        sticky_key,
                        &message,
                        trace_context.as_ref().map(|trace| trace.invoke_id.as_str()),
                    )
                    .await
                    {
                        warn!(account_id = account.account_id, error = %route_err, "failed to record first-event gate transport failure");
                    }
                    store_pool_failover_error(
                        &mut last_error,
                        &mut preserve_sticky_owner_terminal_error,
                        PoolUpstreamError {
                        account: Some(account.clone()),
                        status: StatusCode::BAD_GATEWAY,
                        message: message.clone(),
                        failure_kind: PROXY_FAILURE_UPSTREAM_STREAM_ERROR,
                        connect_latency_ms,
                        upstream_error_code: None,
                        upstream_error_message: None,
                        upstream_request_id: None,
                        oauth_responses_debug: oauth_responses_debug.clone(),
                        attempt_summary: PoolAttemptSummary::default(),
                        requested_service_tier: attempted_requested_service_tier.clone(),
                        request_body_for_capture: attempted_request_body_for_capture.clone(),
                        },
                    );
                    exhausted_accounts_all_rate_limited = false;
                    disarm_pool_early_phase_cleanup_guard(&mut early_phase_cleanup_guard);
                    continue 'account_loop;
                }
            };

            let mut deferred_early_phase_cleanup_guard = None;
            if let Some(pending_attempt_record) = pending_attempt_record.as_ref() {
                match update_pool_upstream_request_attempt_phase(
                    &state.pool,
                    pending_attempt_record,
                    POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_STREAMING_RESPONSE,
                )
                .await
                {
                    Ok(phase_persisted) => {
                        if phase_persisted
                            && let Err(err) = broadcast_pool_upstream_attempts_snapshot(
                                state.as_ref(),
                                &pending_attempt_record.invoke_id,
                            )
                            .await
                        {
                            warn!(
                                invoke_id = %pending_attempt_record.invoke_id,
                                error = %err,
                                "failed to broadcast pool attempt streaming phase snapshot"
                            );
                        }
                        disarm_pool_early_phase_cleanup_guard(&mut early_phase_cleanup_guard);
                    }
                    Err(err) => {
                        warn!(
                            invoke_id = %pending_attempt_record.invoke_id,
                            error = %err,
                            "failed to persist pool attempt streaming phase; deferring early-phase cleanup guard to terminal stream handling"
                        );
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
                oauth_responses_debug,
                connect_latency_ms,
                attempt_started_at_utc,
                first_byte_latency_ms,
                first_chunk,
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
                attempt_summary: pool_attempt_summary(attempt_count, distinct_account_count, None),
                requested_service_tier: attempted_requested_service_tier,
                request_body_for_capture: attempted_request_body_for_capture,
            });
        }
    }
}

fn extract_bearer_token(headers: &HeaderMap) -> Option<String> {
    let authorization = headers.get(header::AUTHORIZATION)?.to_str().ok()?.trim();
    let (scheme, token) = authorization.split_once(' ')?;
    if !scheme.eq_ignore_ascii_case("bearer") {
        return None;
    }
    let normalized = token.trim();
    if normalized.is_empty() {
        None
    } else {
        Some(normalized.to_string())
    }
}

async fn request_matches_pool_route(state: &AppState, headers: &HeaderMap) -> Result<bool> {
    let Some(api_key) = extract_bearer_token(headers) else {
        return Ok(false);
    };
    pool_api_key_matches(state, &api_key).await
}

async fn extract_sticky_key_from_replay_snapshot(
    snapshot: &PoolReplayBodySnapshot,
) -> Option<String> {
    let bytes = match snapshot {
        PoolReplayBodySnapshot::Empty => return None,
        PoolReplayBodySnapshot::Memory(bytes) => bytes.to_vec(),
        PoolReplayBodySnapshot::File { temp_file, .. } => {
            tokio::fs::read(&temp_file.path).await.ok()?
        }
    };

    serde_json::from_slice::<Value>(&bytes)
        .ok()
        .and_then(|value| extract_sticky_key_from_request_body(&value))
}

async fn continue_or_retry_pool_live_request(
    state: Arc<AppState>,
    proxy_request_id: u64,
    method: Method,
    original_uri: &Uri,
    headers: &HeaderMap,
    handshake_timeout: Duration,
    initial_account: PoolResolvedAccount,
    sticky_key: Option<String>,
    responses_total_timeout_started_at: Option<Instant>,
    replay_status_rx: &watch::Receiver<PoolReplayBodyStatus>,
    replay_cancel: &CancellationToken,
    first_error: PoolUpstreamError,
) -> Result<PoolUpstreamResponse, PoolUpstreamError> {
    let reservation_key = build_pool_routing_reservation_key(proxy_request_id);
    let replay_status = { replay_status_rx.borrow().clone() };
    match replay_status {
        PoolReplayBodyStatus::Complete(snapshot) => {
            let replay_sticky_key = extract_sticky_key_from_replay_snapshot(&snapshot)
                .await
                .or(sticky_key);
            let uses_timeout_route_failover =
                pool_uses_responses_timeout_failover_policy(original_uri, &method);
            let first_error_is_timeout_shaped = uses_timeout_route_failover
                && pool_failure_is_timeout_shaped(first_error.failure_kind, &first_error.message);
            let preserve_sticky_owner_terminal_error =
                pool_upstream_error_preserves_existing_sticky_owner(Some(&first_error));
            let (preferred_account, failover_progress, same_account_attempts) =
                if first_error_is_timeout_shaped {
                    let mut excluded_upstream_route_keys = HashSet::new();
                    excluded_upstream_route_keys.insert(initial_account.upstream_route_key());
                    (
                        None,
                        PoolFailoverProgress {
                            excluded_account_ids: vec![initial_account.account_id],
                            excluded_upstream_route_keys,
                            attempt_count: 1,
                            last_error: Some(first_error),
                            preserve_sticky_owner_terminal_error,
                            overload_required_upstream_route_key: None,
                            timeout_route_failover_pending: true,
                            responses_total_timeout_started_at,
                            no_available_wait_deadline: None,
                        },
                        POOL_UPSTREAM_SAME_ACCOUNT_MAX_ATTEMPTS,
                    )
                } else if pool_upstream_error_is_rate_limited(&first_error) {
                    (
                        None,
                        PoolFailoverProgress {
                            excluded_account_ids: vec![initial_account.account_id],
                            attempt_count: 1,
                            last_error: Some(first_error),
                            preserve_sticky_owner_terminal_error,
                            responses_total_timeout_started_at,
                            ..PoolFailoverProgress::default()
                        },
                        POOL_UPSTREAM_SAME_ACCOUNT_MAX_ATTEMPTS,
                    )
                } else {
                    (
                        Some(initial_account.clone()),
                        PoolFailoverProgress {
                            attempt_count: 1,
                            responses_total_timeout_started_at,
                            ..PoolFailoverProgress::default()
                        },
                        POOL_UPSTREAM_SAME_ACCOUNT_MAX_ATTEMPTS.saturating_sub(1),
                    )
                };
            send_pool_request_with_failover(
                state,
                proxy_request_id,
                method,
                original_uri,
                headers,
                Some(snapshot),
                handshake_timeout,
                None,
                None,
                replay_sticky_key.as_deref(),
                preferred_account,
                failover_progress,
                same_account_attempts,
            )
            .await
        }
        PoolReplayBodyStatus::ReadError(err) => {
            release_pool_routing_reservation(state.as_ref(), &reservation_key);
            Err(PoolUpstreamError {
                account: Some(initial_account),
                status: err.status,
                message: err.message,
                failure_kind: err.failure_kind,
                connect_latency_ms: first_error.connect_latency_ms,
                upstream_error_code: None,
                upstream_error_message: None,
                upstream_request_id: None,
                oauth_responses_debug: None,
                attempt_summary: first_error.attempt_summary.clone(),
                requested_service_tier: first_error.requested_service_tier.clone(),
                request_body_for_capture: first_error.request_body_for_capture.clone(),
            })
        }
        PoolReplayBodyStatus::InternalError(message) => {
            release_pool_routing_reservation(state.as_ref(), &reservation_key);
            Err(PoolUpstreamError {
                account: Some(initial_account),
                status: StatusCode::INTERNAL_SERVER_ERROR,
                message,
                failure_kind: PROXY_FAILURE_FAILED_CONTACT_UPSTREAM,
                connect_latency_ms: first_error.connect_latency_ms,
                upstream_error_code: None,
                upstream_error_message: None,
                upstream_request_id: None,
                oauth_responses_debug: None,
                attempt_summary: first_error.attempt_summary.clone(),
                requested_service_tier: first_error.requested_service_tier.clone(),
                request_body_for_capture: first_error.request_body_for_capture.clone(),
            })
        }
        PoolReplayBodyStatus::Reading | PoolReplayBodyStatus::Incomplete => {
            replay_cancel.cancel();
            release_pool_routing_reservation(state.as_ref(), &reservation_key);
            Err(first_error)
        }
    }
}

async fn maybe_backfill_oauth_request_debug_from_replay_status(
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

async fn proxy_openai_v1_via_pool(
    state: Arc<AppState>,
    proxy_request_id: u64,
    original_uri: &Uri,
    method: Method,
    headers: HeaderMap,
    body: Body,
    runtime_timeouts: PoolRoutingTimeoutSettingsResolved,
) -> Result<Response, (StatusCode, String)> {
    let request_started_at = Instant::now();
    let body_limit = state.config.openai_proxy_max_request_body_bytes;
    let pool_routing_reservation_key = build_pool_routing_reservation_key(proxy_request_id);
    let capture_target = capture_target_for_request(original_uri.path(), &method);
    let handshake_timeout =
        proxy_upstream_send_timeout_for_capture_target(&runtime_timeouts, capture_target);
    let _pre_first_byte_timeout =
        pool_upstream_first_chunk_timeout(&runtime_timeouts, original_uri, &method);
    let responses_total_timeout =
        pool_upstream_responses_total_timeout(&state.config, original_uri, &method);
    let pre_attempt_total_timeout_deadline =
        responses_total_timeout.map(|total_timeout| request_started_at + total_timeout);
    let responses_total_timeout_started_at_from_request =
        responses_total_timeout.map(|_| request_started_at);
    let responses_total_timeout_started_at = None;
    let pre_attempt_total_timeout_exceeded =
        || pre_attempt_total_timeout_deadline.is_some_and(|deadline| Instant::now() >= deadline);
    let pre_attempt_total_timeout_error = || {
        pool_pre_attempt_total_timeout_error(
            responses_total_timeout.expect("pre-attempt total-timeout requires responses timeout"),
        )
    };
    let header_sticky_key = extract_sticky_key_from_headers(&headers);
    let body_size_hint_exact = body
        .size_hint()
        .exact()
        .and_then(|value| usize::try_from(value).ok());
    let (upstream, sticky_key) = if request_may_have_body(&method, &headers) {
        let should_prebuffer_for_body_sticky = header_sticky_key.is_none()
            && headers
                .get(header::CONTENT_TYPE)
                .and_then(|value| value.to_str().ok())
                .is_some_and(|value| value.to_ascii_lowercase().starts_with("application/json"))
            && headers
                .get(header::CONTENT_LENGTH)
                .and_then(|value| value.to_str().ok())
                .and_then(|value| value.parse::<usize>().ok())
                .or(body_size_hint_exact)
                .is_some_and(|value| value <= body_limit);

        if should_prebuffer_for_body_sticky {
            let request_body_bytes = read_request_body_with_limit(
                body,
                body_limit,
                runtime_timeouts.request_read_timeout,
                proxy_request_id,
            )
            .await
            .map_err(|err| (err.status, err.message))?;
            if pre_attempt_total_timeout_exceeded() {
                return Err(pre_attempt_total_timeout_error());
            }
            let request_body_bytes = Bytes::from(request_body_bytes);
            let body_sticky_key = serde_json::from_slice::<Value>(&request_body_bytes)
                .ok()
                .and_then(|value| extract_sticky_key_from_request_body(&value));
            (
                send_pool_request_with_failover(
                    state.clone(),
                    proxy_request_id,
                    method,
                    original_uri,
                    &headers,
                    Some(PoolReplayBodySnapshot::Memory(request_body_bytes)),
                    handshake_timeout,
                    None,
                    None,
                    body_sticky_key.as_deref(),
                    None,
                    PoolFailoverProgress {
                        responses_total_timeout_started_at:
                            responses_total_timeout_started_at_from_request,
                        ..PoolFailoverProgress::default()
                    },
                    POOL_UPSTREAM_SAME_ACCOUNT_MAX_ATTEMPTS,
                )
                .await
                .map_err(|err| (err.status, err.message))?,
                body_sticky_key,
            )
        } else {
            let unwrap_initial_pool_account = |resolution,
                                               no_available_wait_deadline|
             -> Result<
                (PoolResolvedAccount, Option<Instant>),
                (StatusCode, String),
            > {
                let initial_account = match resolution {
                    Ok(PoolAccountResolutionWithWait::Resolution(
                        PoolAccountResolution::Resolved(account),
                    )) => account,
                    Ok(PoolAccountResolutionWithWait::TotalTimeoutExpired) => {
                        let total_timeout = responses_total_timeout
                            .expect("pre-attempt total-timeout expiry requires responses timeout");
                        return Err((
                            StatusCode::GATEWAY_TIMEOUT,
                            pool_total_timeout_exhausted_message(total_timeout),
                        ));
                    }
                    Ok(PoolAccountResolutionWithWait::Resolution(
                        PoolAccountResolution::Unavailable,
                    ))
                    | Ok(PoolAccountResolutionWithWait::Resolution(
                        PoolAccountResolution::NoCandidate,
                    )) => {
                        return Err((
                            StatusCode::SERVICE_UNAVAILABLE,
                            POOL_NO_AVAILABLE_ACCOUNT_MESSAGE.to_string(),
                        ));
                    }
                    Ok(PoolAccountResolutionWithWait::Resolution(
                        PoolAccountResolution::RateLimited,
                    )) => {
                        return Err((
                            StatusCode::TOO_MANY_REQUESTS,
                            POOL_ALL_ACCOUNTS_RATE_LIMITED_MESSAGE.to_string(),
                        ));
                    }
                    Ok(PoolAccountResolutionWithWait::Resolution(
                        PoolAccountResolution::DegradedOnly,
                    )) => {
                        return Err((
                            StatusCode::SERVICE_UNAVAILABLE,
                            POOL_ALL_ACCOUNTS_DEGRADED_MESSAGE.to_string(),
                        ));
                    }
                    Ok(PoolAccountResolutionWithWait::Resolution(
                        PoolAccountResolution::BlockedByPolicy(message),
                    )) => {
                        return Err((StatusCode::SERVICE_UNAVAILABLE, message));
                    }
                    Err(err) => {
                        return Err((
                            StatusCode::BAD_GATEWAY,
                            format!("failed to resolve pool account: {err}"),
                        ));
                    }
                };
                Ok((initial_account, no_available_wait_deadline))
            };
            let (
                request_body_snapshot,
                body_sticky_key,
                initial_account,
                no_available_wait_deadline,
            ) = if let Some(sticky_key) = header_sticky_key.clone() {
                let state_for_wait = state.clone();
                let wait_task_sticky_key = sticky_key.clone();
                let shared_wait_deadline = Arc::new(std::sync::Mutex::new(None));
                let shared_wait_deadline_for_task = shared_wait_deadline.clone();
                let header_sticky_resolution = async move {
                    let excluded_ids = Vec::new();
                    let excluded_upstream_route_keys = HashSet::new();
                    let mut no_available_wait_deadline = None;
                    let poll_interval = state_for_wait
                        .pool_no_available_wait
                        .normalized_poll_interval();
                    loop {
                        if pre_attempt_total_timeout_deadline
                            .is_some_and(|deadline| Instant::now() >= deadline)
                        {
                            break (
                                Ok(PoolAccountResolutionWithWait::TotalTimeoutExpired),
                                no_available_wait_deadline,
                            );
                        }
                        let resolution = resolve_pool_account_for_request(
                            state_for_wait.as_ref(),
                            Some(wait_task_sticky_key.as_str()),
                            &excluded_ids,
                            &excluded_upstream_route_keys,
                        )
                        .await;
                        if pre_attempt_total_timeout_deadline
                            .is_some_and(|deadline| Instant::now() >= deadline)
                        {
                            break (
                                Ok(PoolAccountResolutionWithWait::TotalTimeoutExpired),
                                no_available_wait_deadline,
                            );
                        }
                        match resolution {
                            Ok(
                                resolution @ (PoolAccountResolution::Unavailable
                                | PoolAccountResolution::NoCandidate),
                            ) => {
                                let wait_deadline =
                                    if let Some(deadline) = no_available_wait_deadline {
                                        deadline
                                    } else {
                                        let deadline = Instant::now()
                                            + state_for_wait.pool_no_available_wait.timeout;
                                        no_available_wait_deadline = Some(deadline);
                                        *shared_wait_deadline_for_task
                                            .lock()
                                            .expect("lock shared header wait deadline") =
                                            Some(deadline);
                                        deadline
                                    };
                                let effective_deadline = pre_attempt_total_timeout_deadline
                                    .map(|deadline| std::cmp::min(wait_deadline, deadline))
                                    .unwrap_or(wait_deadline);
                                let now = Instant::now();
                                if now >= effective_deadline {
                                    if pre_attempt_total_timeout_deadline
                                        .is_some_and(|deadline| deadline <= wait_deadline)
                                    {
                                        break (
                                            Ok(PoolAccountResolutionWithWait::TotalTimeoutExpired),
                                            no_available_wait_deadline,
                                        );
                                    }
                                    break (
                                        Ok(PoolAccountResolutionWithWait::Resolution(resolution)),
                                        no_available_wait_deadline,
                                    );
                                }
                                tokio::time::sleep(
                                    poll_interval
                                        .min(effective_deadline.saturating_duration_since(now)),
                                )
                                .await;
                            }
                            Ok(resolution) => {
                                break (
                                    Ok(PoolAccountResolutionWithWait::Resolution(resolution)),
                                    no_available_wait_deadline,
                                );
                            }
                            Err(err) => break (Err(err), no_available_wait_deadline),
                        }
                    }
                };
                tokio::pin!(header_sticky_resolution);
                let mut header_sticky_resolution_finished = false;
                let mut pending_header_sticky_terminal_error: Option<(StatusCode, String)> = None;
                let mut resolved_header_sticky_account: Option<PoolResolvedAccount> = None;
                let mut header_sticky_wait_deadline = None;
                let mut request_body_buffer = PoolReplayBodyBuffer::new(proxy_request_id);
                let mut request_body_stream = body.into_data_stream();
                let request_body_deadline = Instant::now() + runtime_timeouts.request_read_timeout;
                let mut request_body_len = 0usize;
                let mut observed_body_sticky_key = None;
                let mut sticky_key_probe = Vec::new();
                let mut sticky_key_probe_exhausted = false;
                let request_body_timeout_error = || RequestBodyReadError {
                    status: StatusCode::REQUEST_TIMEOUT,
                    message: format!(
                        "request body read timed out after {}ms",
                        runtime_timeouts.request_read_timeout.as_millis()
                    ),
                    failure_kind: PROXY_FAILURE_REQUEST_BODY_READ_TIMEOUT,
                    partial_body: Vec::new(),
                };
                let request_body_snapshot = loop {
                    let remaining = request_body_deadline.saturating_duration_since(Instant::now());
                    if remaining.is_zero() {
                        return Err((
                            request_body_timeout_error().status,
                            request_body_timeout_error().message,
                        ));
                    }
                    tokio::select! {
                        resolution_result = &mut header_sticky_resolution, if !header_sticky_resolution_finished => {
                            header_sticky_resolution_finished = true;
                            let (resolution, no_available_wait_deadline) = resolution_result;
                            header_sticky_wait_deadline = no_available_wait_deadline;
                            match resolution {
                                Ok(PoolAccountResolutionWithWait::Resolution(
                                    PoolAccountResolution::Resolved(account),
                                )) => {
                                    resolved_header_sticky_account = Some(account);
                                }
                                Ok(PoolAccountResolutionWithWait::TotalTimeoutExpired) => {
                                    let total_timeout =
                                        responses_total_timeout.expect(
                                            "pre-attempt total-timeout expiry requires responses timeout",
                                        );
                                    return Err((
                                        StatusCode::GATEWAY_TIMEOUT,
                                        pool_total_timeout_exhausted_message(total_timeout),
                                    ));
                                }
                                Ok(PoolAccountResolutionWithWait::Resolution(
                                    PoolAccountResolution::RateLimited,
                                )) => {
                                    pending_header_sticky_terminal_error = Some((
                                        StatusCode::TOO_MANY_REQUESTS,
                                        POOL_ALL_ACCOUNTS_RATE_LIMITED_MESSAGE.to_string(),
                                    ));
                                }
                                Ok(PoolAccountResolutionWithWait::Resolution(
                                    PoolAccountResolution::DegradedOnly,
                                )) => {
                                    pending_header_sticky_terminal_error = Some((
                                        StatusCode::SERVICE_UNAVAILABLE,
                                        POOL_ALL_ACCOUNTS_DEGRADED_MESSAGE.to_string(),
                                    ));
                                }
                                Ok(PoolAccountResolutionWithWait::Resolution(
                                    PoolAccountResolution::BlockedByPolicy(message),
                                )) => {
                                    pending_header_sticky_terminal_error =
                                        Some((StatusCode::SERVICE_UNAVAILABLE, message));
                                }
                                Err(err) => {
                                    pending_header_sticky_terminal_error = Some((
                                        StatusCode::BAD_GATEWAY,
                                        format!("failed to resolve pool account: {err}"),
                                    ));
                                }
                                Ok(PoolAccountResolutionWithWait::Resolution(
                                    PoolAccountResolution::Unavailable,
                                ))
                                | Ok(PoolAccountResolutionWithWait::Resolution(
                                    PoolAccountResolution::NoCandidate,
                                )) => {}
                            }
                            if observed_body_sticky_key.as_deref() == Some(sticky_key.as_str())
                                && let Some((status, message)) =
                                    pending_header_sticky_terminal_error.as_ref()
                            {
                                return Err((*status, message.clone()));
                            }
                        }
                        next_chunk = timeout(remaining, request_body_stream.next()) => {
                            let next_chunk = match next_chunk {
                                Ok(chunk) => chunk,
                                Err(_) => {
                                    let err = request_body_timeout_error();
                                    return Err((err.status, err.message));
                                }
                            };
                            let Some(chunk) = next_chunk else {
                                break request_body_buffer.finish().await.map_err(|err| {
                                    (
                                        StatusCode::BAD_GATEWAY,
                                        format!("failed to cache request body for oauth replay: {err}"),
                                    )
                                })?;
                            };
                            let chunk = match chunk {
                                Ok(chunk) => chunk,
                                Err(err) => {
                                    return Err((
                                        StatusCode::BAD_REQUEST,
                                        format!("failed to read request body stream: {err}"),
                                    ));
                                }
                            };
                            if request_body_len.saturating_add(chunk.len()) > body_limit {
                                return Err((
                                    StatusCode::PAYLOAD_TOO_LARGE,
                                    format!("request body exceeds {body_limit} bytes"),
                                ));
                            }
                            request_body_len = request_body_len.saturating_add(chunk.len());
                            request_body_buffer.append(&chunk).await.map_err(|err| {
                                (
                                    StatusCode::BAD_GATEWAY,
                                    format!("failed to cache request body for oauth replay: {err}"),
                                )
                            })?;
                            if !sticky_key_probe_exhausted
                                && observed_body_sticky_key.is_none()
                                && sticky_key_probe.len() < HEADER_STICKY_EARLY_STICKY_SCAN_BYTES
                            {
                                let probe_remaining = HEADER_STICKY_EARLY_STICKY_SCAN_BYTES
                                    .saturating_sub(sticky_key_probe.len());
                                sticky_key_probe.extend_from_slice(&chunk[..chunk.len().min(probe_remaining)]);
                                observed_body_sticky_key =
                                    best_effort_extract_sticky_key_from_request_body_prefix(
                                        &sticky_key_probe,
                                    );
                                sticky_key_probe_exhausted =
                                    observed_body_sticky_key.is_some()
                                        || sticky_key_probe.len()
                                            >= HEADER_STICKY_EARLY_STICKY_SCAN_BYTES;
                            }
                            if header_sticky_resolution_finished
                                && observed_body_sticky_key.as_deref() == Some(sticky_key.as_str())
                                && let Some((status, message)) =
                                    pending_header_sticky_terminal_error.as_ref()
                            {
                                return Err((*status, message.clone()));
                            }
                        }
                    }
                };
                let body_sticky_key =
                    if let Some(observed_body_sticky_key) = observed_body_sticky_key {
                        Some(observed_body_sticky_key)
                    } else {
                        extract_sticky_key_from_replay_snapshot(&request_body_snapshot)
                            .await
                            .or(Some(sticky_key.clone()))
                    };
                if !header_sticky_resolution_finished {
                    if body_sticky_key.as_deref() == Some(sticky_key.as_str()) {
                        let (resolution, no_available_wait_deadline) =
                            header_sticky_resolution.await;
                        header_sticky_wait_deadline = no_available_wait_deadline;
                        match resolution {
                            Ok(PoolAccountResolutionWithWait::Resolution(
                                PoolAccountResolution::Resolved(account),
                            )) => {
                                resolved_header_sticky_account = Some(account);
                            }
                            Ok(PoolAccountResolutionWithWait::TotalTimeoutExpired) => {
                                let total_timeout = responses_total_timeout.expect(
                                    "pre-attempt total-timeout expiry requires responses timeout",
                                );
                                return Err((
                                    StatusCode::GATEWAY_TIMEOUT,
                                    pool_total_timeout_exhausted_message(total_timeout),
                                ));
                            }
                            Ok(PoolAccountResolutionWithWait::Resolution(
                                PoolAccountResolution::RateLimited,
                            )) => {
                                pending_header_sticky_terminal_error = Some((
                                    StatusCode::TOO_MANY_REQUESTS,
                                    POOL_ALL_ACCOUNTS_RATE_LIMITED_MESSAGE.to_string(),
                                ));
                            }
                            Ok(PoolAccountResolutionWithWait::Resolution(
                                PoolAccountResolution::DegradedOnly,
                            )) => {
                                pending_header_sticky_terminal_error = Some((
                                    StatusCode::SERVICE_UNAVAILABLE,
                                    POOL_ALL_ACCOUNTS_DEGRADED_MESSAGE.to_string(),
                                ));
                            }
                            Ok(PoolAccountResolutionWithWait::Resolution(
                                PoolAccountResolution::BlockedByPolicy(message),
                            )) => {
                                pending_header_sticky_terminal_error =
                                    Some((StatusCode::SERVICE_UNAVAILABLE, message));
                            }
                            Err(err) => {
                                pending_header_sticky_terminal_error = Some((
                                    StatusCode::BAD_GATEWAY,
                                    format!("failed to resolve pool account: {err}"),
                                ));
                            }
                            Ok(PoolAccountResolutionWithWait::Resolution(
                                PoolAccountResolution::Unavailable,
                            ))
                            | Ok(PoolAccountResolutionWithWait::Resolution(
                                PoolAccountResolution::NoCandidate,
                            )) => {}
                        }
                    }
                }
                if body_sticky_key.as_deref() == Some(sticky_key.as_str())
                    && let Some((status, message)) = pending_header_sticky_terminal_error
                {
                    return Err((status, message));
                }
                let mut no_available_wait_deadline =
                    header_sticky_wait_deadline.or(*shared_wait_deadline
                        .lock()
                        .expect("lock shared header wait deadline"));
                let initial_account = if body_sticky_key.as_deref() == Some(sticky_key.as_str()) {
                    if let Some(account) = resolved_header_sticky_account {
                        account
                    } else {
                        let resolution = resolve_pool_account_for_request_with_wait(
                            state.as_ref(),
                            body_sticky_key.as_deref(),
                            &[],
                            &HashSet::new(),
                            None,
                            true,
                            &mut no_available_wait_deadline,
                            pre_attempt_total_timeout_deadline,
                        )
                        .await;
                        let (initial_account, updated_no_available_wait_deadline) =
                            unwrap_initial_pool_account(resolution, no_available_wait_deadline)?;
                        no_available_wait_deadline = updated_no_available_wait_deadline;
                        initial_account
                    }
                } else {
                    let resolution = resolve_pool_account_for_request_with_wait(
                        state.as_ref(),
                        body_sticky_key.as_deref(),
                        &[],
                        &HashSet::new(),
                        None,
                        true,
                        &mut no_available_wait_deadline,
                        pre_attempt_total_timeout_deadline,
                    )
                    .await;
                    let (initial_account, updated_no_available_wait_deadline) =
                        unwrap_initial_pool_account(resolution, no_available_wait_deadline)?;
                    no_available_wait_deadline = updated_no_available_wait_deadline;
                    initial_account
                };
                if pre_attempt_total_timeout_exceeded() {
                    return Err(pre_attempt_total_timeout_error());
                }
                (
                    request_body_snapshot,
                    body_sticky_key,
                    initial_account,
                    no_available_wait_deadline,
                )
            } else {
                let request_body_snapshot = read_request_body_snapshot_with_limit(
                    body,
                    body_limit,
                    runtime_timeouts.request_read_timeout,
                    proxy_request_id,
                )
                .await
                .map_err(|err| (err.status, err.message))?;
                let body_sticky_key =
                    extract_sticky_key_from_replay_snapshot(&request_body_snapshot).await;
                let mut no_available_wait_deadline = None;
                let resolution = resolve_pool_account_for_request_with_wait(
                    state.as_ref(),
                    body_sticky_key.as_deref(),
                    &[],
                    &HashSet::new(),
                    None,
                    true,
                    &mut no_available_wait_deadline,
                    pre_attempt_total_timeout_deadline,
                )
                .await;
                let (initial_account, no_available_wait_deadline) =
                    unwrap_initial_pool_account(resolution, no_available_wait_deadline)?;
                (
                    request_body_snapshot,
                    body_sticky_key,
                    initial_account,
                    no_available_wait_deadline,
                )
            };
            (
                send_pool_request_with_failover(
                    state.clone(),
                    proxy_request_id,
                    method,
                    original_uri,
                    &headers,
                    Some(request_body_snapshot),
                    handshake_timeout,
                    None,
                    None,
                    body_sticky_key.as_deref(),
                    Some(initial_account),
                    PoolFailoverProgress {
                        responses_total_timeout_started_at: if no_available_wait_deadline.is_some()
                        {
                            responses_total_timeout_started_at_from_request
                        } else {
                            None
                        },
                        no_available_wait_deadline,
                        ..PoolFailoverProgress::default()
                    },
                    POOL_UPSTREAM_SAME_ACCOUNT_MAX_ATTEMPTS,
                )
                .await
                .map_err(|err| (err.status, err.message))?,
                body_sticky_key,
            )
        }
    } else {
        let same_account_attempts = if method == Method::GET {
            1
        } else {
            POOL_UPSTREAM_SAME_ACCOUNT_MAX_ATTEMPTS
        };
        (
            send_pool_request_with_failover(
                state.clone(),
                proxy_request_id,
                method,
                original_uri,
                &headers,
                None,
                handshake_timeout,
                None,
                None,
                header_sticky_key.as_deref(),
                None,
                PoolFailoverProgress {
                    responses_total_timeout_started_at,
                    ..PoolFailoverProgress::default()
                },
                same_account_attempts,
            )
            .await
            .map_err(|err| (err.status, err.message))?,
            header_sticky_key,
        )
    };

    let account = upstream.account;
    let upstream_attempt_started_at_utc = upstream.attempt_started_at_utc;
    let upstream_invoke_id = upstream
        .pending_attempt_record
        .as_ref()
        .map(|record| record.invoke_id.clone());
    let t_upstream_connect_ms = upstream.connect_latency_ms;
    let t_upstream_ttfb_ms = upstream.first_byte_latency_ms;
    let upstream_response = upstream.response;
    let rewritten_location = normalize_proxy_location_header(
        upstream_response.status(),
        upstream_response.headers(),
        &account.upstream_base_url,
    )
    .map_err(|err| {
        (
            StatusCode::BAD_GATEWAY,
            format!("failed to process upstream redirect: {err}"),
        )
    })?;

    let upstream_status = upstream_response.status();
    let upstream_connection_scoped = connection_scoped_header_names(upstream_response.headers());
    let mut response_builder = Response::builder().status(upstream_status);
    for (name, value) in upstream_response.headers() {
        if should_forward_proxy_header(name, &upstream_connection_scoped) {
            if name == header::LOCATION {
                if let Some(rewritten) = rewritten_location.as_deref() {
                    response_builder = response_builder.header(name, rewritten);
                }
            } else {
                response_builder = response_builder.header(name, value);
            }
        }
    }

    let mut upstream_stream = upstream_response.into_bytes_stream();
    let first_chunk = upstream.first_chunk;
    if let Some(chunk) = first_chunk.as_ref() {
        info!(
            proxy_request_id,
            account_id = account.account_id,
            ttfb_ms = t_upstream_ttfb_ms,
            first_chunk_bytes = chunk.len(),
            "pool upstream response first chunk ready"
        );
    } else {
        consume_pool_routing_reservation(state.as_ref(), &pool_routing_reservation_key);
        if let Err(route_err) = record_pool_route_success(
            &state.pool,
            account.account_id,
            upstream_attempt_started_at_utc,
            sticky_key.as_deref(),
            upstream_invoke_id.as_deref(),
        )
        .await
        {
            warn!(account_id = account.account_id, error = %route_err, "failed to record pool route success");
        }
        return response_builder.body(Body::empty()).map_err(|err| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("failed to build proxy response: {err}"),
            )
        });
    }

    let (tx, rx) = mpsc::channel::<Result<Bytes, io::Error>>(16);
    let state_for_record = state.clone();
    let reservation_key_for_record = pool_routing_reservation_key.clone();
    let sticky_key_for_record = sticky_key.clone();
    let invoke_id_for_record = upstream_invoke_id.clone();
    let upstream_attempt_started_at_utc_for_record = upstream_attempt_started_at_utc;
    tokio::spawn(async move {
        let mut forwarded_chunks = 0usize;
        let mut forwarded_bytes = 0usize;
        let stream_started_at = Instant::now();
        let mut stream_error_message: Option<String> = None;
        let mut downstream_closed = false;

        if let Some(chunk) = first_chunk {
            forwarded_chunks = forwarded_chunks.saturating_add(1);
            forwarded_bytes = forwarded_bytes.saturating_add(chunk.len());
            if tx.send(Ok(chunk)).await.is_err() {
                downstream_closed = true;
            }
        }

        loop {
            if downstream_closed {
                break;
            }
            let Some(next_chunk) = upstream_stream.next().await else {
                break;
            };
            match next_chunk {
                Ok(chunk) => {
                    forwarded_chunks = forwarded_chunks.saturating_add(1);
                    forwarded_bytes = forwarded_bytes.saturating_add(chunk.len());
                    if tx.send(Ok(chunk)).await.is_err() {
                        break;
                    }
                }
                Err(err) => {
                    let message = format!("upstream stream error: {err}");
                    stream_error_message = Some(message.clone());
                    let _ = tx.send(Err(io::Error::other(message))).await;
                    break;
                }
            }
        }

        if let Some(message) = stream_error_message.as_deref() {
            release_pool_routing_reservation(
                state_for_record.as_ref(),
                &reservation_key_for_record,
            );
            if let Err(route_err) = record_pool_route_transport_failure(
                &state_for_record.pool,
                account.account_id,
                sticky_key_for_record.as_deref(),
                message,
                invoke_id_for_record.as_deref(),
            )
            .await
            {
                warn!(account_id = account.account_id, error = %route_err, "failed to record pool stream error");
            }
        } else {
            consume_pool_routing_reservation(
                state_for_record.as_ref(),
                &reservation_key_for_record,
            );
            if let Err(route_err) = record_pool_route_success(
                &state_for_record.pool,
                account.account_id,
                upstream_attempt_started_at_utc_for_record,
                sticky_key_for_record.as_deref(),
                invoke_id_for_record.as_deref(),
            )
            .await
            {
                warn!(account_id = account.account_id, error = %route_err, "failed to record pool route success");
            }
        }

        info!(
            proxy_request_id,
            account_id = account.account_id,
            t_upstream_connect_ms,
            forwarded_chunks,
            forwarded_bytes,
            elapsed_ms = stream_started_at.elapsed().as_millis(),
            "pool upstream response stream completed"
        );
    });

    response_builder
        .body(Body::from_stream(ReceiverStream::new(rx)))
        .map_err(|err| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("failed to build proxy response: {err}"),
            )
        })
}

pub(crate) async fn send_forward_proxy_request_with_429_retry(
    state: Arc<AppState>,
    method: Method,
    target_url: Url,
    headers: &HeaderMap,
    body: Option<Bytes>,
    handshake_timeout: Duration,
    capture_target: Option<ProxyCaptureTarget>,
    upstream_429_max_retries: u8,
) -> Result<ForwardProxyUpstreamResponse, ForwardProxyUpstreamError> {
    let request_connection_scoped = connection_scoped_header_names(headers);

    for attempt in 0..=upstream_429_max_retries {
        let selected_proxy = match select_forward_proxy_for_request(state.as_ref()).await {
            Ok(selected_proxy) => selected_proxy,
            Err(err) => {
                return Err(ForwardProxyUpstreamError {
                    selected_proxy: SelectedForwardProxy::from_endpoint(
                        &ForwardProxyEndpoint::direct(),
                    ),
                    status: StatusCode::BAD_GATEWAY,
                    message: format!("failed to select forward proxy node: {err}"),
                    failure_kind: PROXY_FAILURE_FAILED_CONTACT_UPSTREAM,
                    attempt_failure_kind: FORWARD_PROXY_FAILURE_SEND_ERROR,
                    connect_latency_ms: 0.0,
                });
            }
        };
        let client = match state
            .http_clients
            .client_for_forward_proxy(selected_proxy.endpoint_url.as_ref())
        {
            Ok(client) => client,
            Err(err) => {
                return Err(ForwardProxyUpstreamError {
                    selected_proxy,
                    status: StatusCode::BAD_GATEWAY,
                    message: format!("failed to initialize forward proxy client: {err}"),
                    failure_kind: PROXY_FAILURE_FAILED_CONTACT_UPSTREAM,
                    attempt_failure_kind: FORWARD_PROXY_FAILURE_SEND_ERROR,
                    connect_latency_ms: 0.0,
                });
            }
        };

        let mut request = client.request(method.clone(), target_url.clone());
        for (name, value) in headers {
            if should_forward_proxy_header(name, &request_connection_scoped) {
                request = request.header(name, value);
            }
        }
        if let Some(body_bytes) = body.clone() {
            request = request.body(body_bytes);
        }

        let connect_started = Instant::now();
        let response = match timeout(handshake_timeout, request.send()).await {
            Ok(Ok(response)) => response,
            Ok(Err(err)) => {
                return Err(ForwardProxyUpstreamError {
                    selected_proxy,
                    status: StatusCode::BAD_GATEWAY,
                    message: format!("failed to contact upstream: {err}"),
                    failure_kind: PROXY_FAILURE_FAILED_CONTACT_UPSTREAM,
                    attempt_failure_kind: FORWARD_PROXY_FAILURE_SEND_ERROR,
                    connect_latency_ms: elapsed_ms(connect_started),
                });
            }
            Err(_) => {
                return Err(ForwardProxyUpstreamError {
                    selected_proxy,
                    status: StatusCode::BAD_GATEWAY,
                    message: proxy_request_send_timeout_message(capture_target, handshake_timeout),
                    failure_kind: PROXY_FAILURE_UPSTREAM_HANDSHAKE_TIMEOUT,
                    attempt_failure_kind: FORWARD_PROXY_FAILURE_HANDSHAKE_TIMEOUT,
                    connect_latency_ms: elapsed_ms(connect_started),
                });
            }
        };

        let connect_latency_ms = elapsed_ms(connect_started);
        if response.status() != StatusCode::TOO_MANY_REQUESTS {
            return Ok(ForwardProxyUpstreamResponse {
                selected_proxy,
                response: ProxyUpstreamResponseBody::Reqwest(response),
                connect_latency_ms,
                attempt_started_at: connect_started,
                attempt_recorded: false,
                attempt_update: None,
            });
        }

        if attempt < upstream_429_max_retries {
            record_forward_proxy_attempt(
                state.clone(),
                selected_proxy.clone(),
                false,
                Some(connect_latency_ms),
                Some(FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429),
                false,
            )
            .await;

            let retry_delay = response
                .headers()
                .get(header::RETRY_AFTER)
                .and_then(parse_retry_after_delay)
                .unwrap_or_else(|| fallback_proxy_429_retry_delay(u32::from(attempt) + 1));
            info!(
                proxy_key_ref = %forward_proxy_log_ref(&selected_proxy.key),
                proxy_source = selected_proxy.source,
                proxy_label = selected_proxy.display_name,
                proxy_url_ref = %forward_proxy_log_ref_option(selected_proxy.endpoint_url_raw.as_deref()),
                retry_index = attempt + 1,
                max_429_retries = upstream_429_max_retries,
                retry_after_ms = retry_delay.as_millis(),
                "upstream responded 429; retrying forward proxy request"
            );
            sleep(retry_delay).await;
            continue;
        }

        // Final 429: defer attempt recording until the caller finishes consuming / forwarding
        // the response body, so a later stream error can override this classification.
        return Ok(ForwardProxyUpstreamResponse {
            selected_proxy,
            response: ProxyUpstreamResponseBody::Reqwest(response),
            connect_latency_ms,
            attempt_started_at: connect_started,
            attempt_recorded: false,
            attempt_update: None,
        });
    }

    unreachable!("429 retry loop should always return a response or error")
}

async fn proxy_openai_v1_inner(
    state: Arc<AppState>,
    proxy_request_id: u64,
    invoke_id: String,
    original_uri: Uri,
    method: Method,
    headers: HeaderMap,
    body: Body,
    peer_ip: Option<IpAddr>,
) -> Result<Response, ProxyErrorResponse> {
    let pool_route_active = request_matches_pool_route(state.as_ref(), &headers)
        .await
        .map_err(|err| ProxyErrorResponse {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message: format!("failed to resolve pool routing settings: {err}"),
            cvm_id: None,
            retry_after_secs: None,
        })?;
    let runtime_timeouts = resolve_proxy_request_timeouts(state.as_ref(), pool_route_active)
        .await
        .map_err(|err| ProxyErrorResponse {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message: format!("failed to resolve pool routing timeouts: {err}"),
            cvm_id: None,
            retry_after_secs: None,
        })?;
    if !pool_route_active {
        return Err(ProxyErrorResponse {
            status: StatusCode::UNAUTHORIZED,
            message: "pool route key missing or invalid".to_string(),
            cvm_id: None,
            retry_after_secs: None,
        });
    }
    let target_url =
        build_proxy_upstream_url(&state.config.openai_upstream_base_url, &original_uri).map_err(
            |err| {
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
                ProxyErrorResponse {
                    status,
                    message: format!("failed to build upstream url: {err}"),
                    cvm_id: None,
                    retry_after_secs: None,
                }
            },
        )?;

    if method == Method::GET && is_models_list_path(original_uri.path()) {
        return proxy_openai_v1_via_pool(
            state,
            proxy_request_id,
            &original_uri,
            method,
            headers,
            body,
            runtime_timeouts,
        )
        .await
        .map_err(|(status, message)| ProxyErrorResponse {
            retry_after_secs: retry_after_secs_for_proxy_error(status, &message),
            status,
            message,
            cvm_id: None,
        });
    }

    if let Some(target) = capture_target_for_request(original_uri.path(), &method) {
        let tracked_invoke_id = invoke_id.clone();
        return proxy_openai_v1_capture_target(
            state,
            proxy_request_id,
            invoke_id,
            &original_uri,
            headers,
            body,
            target,
            target_url,
            peer_ip,
            pool_route_active,
            runtime_timeouts,
        )
        .await
        .map_err(|(status, message)| ProxyErrorResponse {
            retry_after_secs: retry_after_secs_for_proxy_error(status, &message),
            status,
            message,
            cvm_id: Some(tracked_invoke_id),
        });
    }

    return proxy_openai_v1_via_pool(
        state,
        proxy_request_id,
        &original_uri,
        method,
        headers,
        body,
        runtime_timeouts,
    )
    .await
    .map_err(|(status, message)| ProxyErrorResponse {
        retry_after_secs: retry_after_secs_for_proxy_error(status, &message),
        status,
        message,
        cvm_id: None,
    });
}

fn capture_target_for_request(path: &str, method: &Method) -> Option<ProxyCaptureTarget> {
    if *method != Method::POST {
        return None;
    }
    match path {
        "/v1/chat/completions" => Some(ProxyCaptureTarget::ChatCompletions),
        "/v1/responses" => Some(ProxyCaptureTarget::Responses),
        "/v1/responses/compact" => Some(ProxyCaptureTarget::ResponsesCompact),
        _ => None,
    }
}

#[allow(clippy::too_many_arguments)]
async fn proxy_openai_v1_capture_target(
    state: Arc<AppState>,
    proxy_request_id: u64,
    invoke_id: String,
    original_uri: &Uri,
    headers: HeaderMap,
    body: Body,
    capture_target: ProxyCaptureTarget,
    target_url: Url,
    peer_ip: Option<IpAddr>,
    pool_route_active: bool,
    runtime_timeouts: PoolRoutingTimeoutSettingsResolved,
) -> Result<Response, (StatusCode, String)> {
    let capture_started = Instant::now();
    let pool_routing_reservation_key = build_pool_routing_reservation_key(proxy_request_id);
    let occurred_at_utc = Utc::now();
    let occurred_at = format_naive(occurred_at_utc.with_timezone(&Shanghai).naive_local());
    let body_limit = state.config.openai_proxy_max_request_body_bytes;
    let requester_ip = extract_requester_ip(&headers, peer_ip);
    let header_sticky_key = extract_sticky_key_from_headers(&headers);
    let header_prompt_cache_key = extract_prompt_cache_key_from_headers(&headers);

    let req_read_started = Instant::now();
    let request_body_bytes = match read_request_body_with_limit(
        body,
        body_limit,
        runtime_timeouts.request_read_timeout,
        proxy_request_id,
    )
    .await
    {
        Ok(bytes) => bytes,
        Err(read_err) => {
            let t_req_read_ms = elapsed_ms(req_read_started);
            let request_info = RequestCaptureInfo::default();
            let req_raw = store_raw_payload_file(
                &state.config,
                &invoke_id,
                "request",
                &read_err.partial_body,
            );
            let usage = ParsedUsage::default();
            let (cost, cost_estimated, price_version) = estimate_proxy_cost_from_shared_catalog(
                &state.pricing_catalog,
                None,
                &usage,
                None,
                ProxyPricingMode::ResponseTier,
            )
            .await;
            let error_message = format!("[{}] {}", read_err.failure_kind, read_err.message);

            warn!(
                proxy_request_id,
                status = %read_err.status,
                failure_kind = read_err.failure_kind,
                error = %read_err.message,
                elapsed_ms = t_req_read_ms,
                "openai proxy request body read failed"
            );

            let record = ProxyCaptureRecord {
                invoke_id,
                occurred_at,
                model: None,
                usage,
                cost,
                cost_estimated,
                price_version,
                status: if read_err.status.is_server_error() {
                    format!("http_{}", read_err.status.as_u16())
                } else {
                    "failed".to_string()
                },
                error_message: Some(error_message),
                failure_kind: Some(read_err.failure_kind.to_string()),
                payload: Some(build_proxy_payload_summary(ProxyPayloadSummary {
                    target: capture_target,
                    status: read_err.status,
                    is_stream: request_info.is_stream,
                    request_model: None,
                    requested_service_tier: request_info.requested_service_tier.as_deref(),
                    billing_service_tier: None,
                    reasoning_effort: request_info.reasoning_effort.as_deref(),
                    response_model: None,
                    usage_missing_reason: None,
                    request_parse_error: request_info.parse_error.as_deref(),
                    failure_kind: Some(read_err.failure_kind),
                    requester_ip: requester_ip.as_deref(),
                    upstream_scope: if pool_route_active {
                        INVOCATION_UPSTREAM_SCOPE_INTERNAL
                    } else {
                        INVOCATION_UPSTREAM_SCOPE_EXTERNAL
                    },
                    route_mode: if pool_route_active {
                        INVOCATION_ROUTE_MODE_POOL
                    } else {
                        INVOCATION_ROUTE_MODE_FORWARD_PROXY
                    },
                    sticky_key: header_sticky_key.as_deref(),
                    prompt_cache_key: header_prompt_cache_key.as_deref(),
                    upstream_account_id: None,
                    upstream_account_name: None,
                    upstream_account_kind: None,
                    upstream_base_url_host: None,
                    oauth_account_header_attached: None,
                    oauth_account_id_shape: None,
                    oauth_forwarded_header_count: None,
                    oauth_forwarded_header_names: None,
                    oauth_fingerprint_version: None,
                    oauth_forwarded_header_fingerprints: None,
                    oauth_prompt_cache_header_forwarded: None,
                    oauth_request_body_prefix_fingerprint: None,
                    oauth_request_body_prefix_bytes: None,
                    oauth_responses_rewrite: None,
                    service_tier: None,
                    stream_terminal_event: None,
                    upstream_error_code: None,
                    upstream_error_message: None,
                    upstream_request_id: None,
                    response_content_encoding: None,
                    proxy_display_name: None,
                    proxy_weight_delta: None,
                    pool_attempt_count: None,
                    pool_distinct_account_count: None,
                    pool_attempt_terminal_reason: None,
                })),
                raw_response: "{}".to_string(),
                req_raw,
                resp_raw: RawPayloadMeta::default(),
                timings: StageTimings {
                    t_total_ms: 0.0,
                    t_req_read_ms,
                    t_req_parse_ms: 0.0,
                    t_upstream_connect_ms: 0.0,
                    t_upstream_ttfb_ms: 0.0,
                    t_upstream_stream_ms: 0.0,
                    t_resp_parse_ms: 0.0,
                    t_persist_ms: 0.0,
                },
            };
            if let Err(err) =
                persist_and_broadcast_proxy_capture(state.as_ref(), capture_started, record).await
            {
                warn!(proxy_request_id, error = %err, "failed to persist proxy capture record");
            }
            return Err((read_err.status, read_err.message));
        }
    };
    let t_req_read_ms = elapsed_ms(req_read_started);

    let proxy_settings = state.proxy_model_settings.read().await.clone();
    let req_parse_started = Instant::now();
    let (upstream_body, mut request_info, body_rewritten) = prepare_target_request_body(
        capture_target,
        request_body_bytes,
        state.config.proxy_enforce_stream_include_usage,
    );
    let prompt_cache_key = request_info
        .prompt_cache_key
        .clone()
        .or_else(|| header_prompt_cache_key.clone());
    let sticky_key = request_info
        .sticky_key
        .clone()
        .or_else(|| header_sticky_key.clone());
    let pool_attempt_trace_context = pool_route_active.then(|| PoolUpstreamAttemptTraceContext {
        invoke_id: invoke_id.clone(),
        occurred_at: occurred_at.clone(),
        endpoint: capture_target.endpoint().to_string(),
        sticky_key: sticky_key.clone(),
        requester_ip: requester_ip.clone(),
    });
    let t_req_parse_ms = elapsed_ms(req_parse_started);
    let upstream_body_bytes = Bytes::from(upstream_body);
    let base_request_bytes_for_capture = upstream_body_bytes.clone();

    let initial_running_record = build_running_proxy_capture_record(
        &invoke_id,
        &occurred_at,
        capture_target,
        &request_info,
        requester_ip.as_deref(),
        sticky_key.as_deref(),
        prompt_cache_key.as_deref(),
        pool_route_active,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        t_req_read_ms,
        t_req_parse_ms,
        0.0,
        0.0,
    );
    if let Err(err) = persist_and_broadcast_proxy_capture_runtime_snapshot(
        state.as_ref(),
        initial_running_record,
    )
    .await
    {
        warn!(
            ?err,
            invoke_id = %invoke_id,
            "failed to broadcast initial running proxy capture snapshot"
        );
    }

    let mut upstream_headers = headers.clone();
    if body_rewritten {
        upstream_headers.remove(header::CONTENT_LENGTH);
    }
    let pool_attempt_runtime_snapshot =
        pool_route_active.then(|| PoolAttemptRuntimeSnapshotContext {
            capture_target,
            request_info: request_info.clone(),
            prompt_cache_key: prompt_cache_key.clone(),
            t_req_read_ms,
            t_req_parse_ms,
        });
    let handshake_timeout =
        proxy_upstream_send_timeout_for_capture_target(&runtime_timeouts, Some(capture_target));
    let first_byte_timeout =
        pool_upstream_first_chunk_timeout(&runtime_timeouts, &original_uri, &Method::POST);
    let stream_timeout = proxy_capture_target_stream_timeout(&runtime_timeouts, capture_target);
    let (
        selected_proxy,
        pool_account,
        t_upstream_connect_ms,
        prefetched_first_chunk,
        prefetched_ttfb_ms,
        oauth_responses_debug,
        attempt_already_recorded,
        final_attempt_update,
        pending_pool_attempt_record,
        deferred_pool_early_phase_cleanup_guard,
        pending_pool_attempt_summary,
        upstream_attempt_started_at,
        upstream_attempt_started_at_utc,
        final_request_body_for_capture,
        final_requested_service_tier,
        upstream_response,
    ) = if pool_route_active {
        match send_pool_request_with_failover(
            state.clone(),
            proxy_request_id,
            Method::POST,
            &original_uri,
            &upstream_headers,
            Some(PoolReplayBodySnapshot::Memory(upstream_body_bytes.clone())),
            handshake_timeout,
            pool_attempt_trace_context.clone(),
            pool_attempt_runtime_snapshot.clone(),
            sticky_key.as_deref(),
            None,
            PoolFailoverProgress::default(),
            POOL_UPSTREAM_SAME_ACCOUNT_MAX_ATTEMPTS,
        )
        .await
        {
            Ok(response) => (
                None,
                Some(response.account),
                response.connect_latency_ms,
                response.first_chunk,
                response.first_byte_latency_ms,
                response.oauth_responses_debug,
                true,
                None,
                response.pending_attempt_record,
                response.deferred_early_phase_cleanup_guard,
                response.attempt_summary,
                None,
                Some(response.attempt_started_at_utc),
                response.request_body_for_capture,
                response.requested_service_tier,
                response.response,
            ),
            Err(err) => {
                request_info.requested_service_tier = err
                    .requested_service_tier
                    .clone()
                    .or(request_info.requested_service_tier);
                let request_body_for_capture = err
                    .request_body_for_capture
                    .clone()
                    .unwrap_or_else(|| base_request_bytes_for_capture.clone());
                let req_raw = store_raw_payload_file(
                    &state.config,
                    &invoke_id,
                    "request",
                    request_body_for_capture.as_ref(),
                );
                let usage = ParsedUsage::default();
                let (billing_service_tier, pricing_mode) =
                    resolve_proxy_billing_service_tier_and_pricing_mode_for_account(
                        None,
                        request_info.requested_service_tier.as_deref(),
                        None,
                        err.account.as_ref(),
                    );
                let (cost, cost_estimated, price_version) =
                    estimate_proxy_cost_from_shared_catalog(
                        &state.pricing_catalog,
                        request_info.model.as_deref(),
                        &usage,
                        billing_service_tier.as_deref(),
                        pricing_mode,
                    )
                    .await;
                let error_message = format!("[{}] {}", err.failure_kind, err.message);
                let pool_proxy_display_name = resolve_invocation_proxy_display_name(None);
                let record = ProxyCaptureRecord {
                    invoke_id,
                    occurred_at,
                    model: request_info.model,
                    usage,
                    cost,
                    cost_estimated,
                    price_version,
                    status: if err.status.is_server_error() {
                        format!("http_{}", err.status.as_u16())
                    } else {
                        "failed".to_string()
                    },
                    error_message: Some(error_message),
                    failure_kind: Some(err.failure_kind.to_string()),
                    payload: Some(build_proxy_payload_summary(ProxyPayloadSummary {
                        target: capture_target,
                        status: err.status,
                        is_stream: request_info.is_stream,
                        request_model: None,
                        requested_service_tier: request_info.requested_service_tier.as_deref(),
                        billing_service_tier: billing_service_tier.as_deref(),
                        reasoning_effort: request_info.reasoning_effort.as_deref(),
                        response_model: None,
                        usage_missing_reason: None,
                        request_parse_error: request_info.parse_error.as_deref(),
                        failure_kind: Some(err.failure_kind),
                        requester_ip: requester_ip.as_deref(),
                        upstream_scope: INVOCATION_UPSTREAM_SCOPE_INTERNAL,
                        route_mode: INVOCATION_ROUTE_MODE_POOL,
                        sticky_key: sticky_key.as_deref(),
                        prompt_cache_key: prompt_cache_key.as_deref(),
                        upstream_account_id: err.account.as_ref().map(|account| account.account_id),
                        upstream_account_name: err
                            .account
                            .as_ref()
                            .map(|account| account.display_name.as_str()),
                        upstream_account_kind: payload_summary_upstream_account_kind(
                            err.account.as_ref(),
                        ),
                        upstream_base_url_host: payload_summary_upstream_base_url_host(
                            err.account.as_ref(),
                        ),
                        oauth_account_header_attached: oauth_account_header_attached_for_account(
                            err.account.as_ref(),
                        ),
                        oauth_account_id_shape: oauth_account_id_shape_for_account(
                            err.account.as_ref(),
                        ),
                        oauth_forwarded_header_count: err
                            .oauth_responses_debug
                            .as_ref()
                            .map(|debug| debug.forwarded_header_names.len()),
                        oauth_forwarded_header_names: err
                            .oauth_responses_debug
                            .as_ref()
                            .map(|debug| debug.forwarded_header_names.as_slice()),
                        oauth_fingerprint_version: err
                            .oauth_responses_debug
                            .as_ref()
                            .and_then(|debug| debug.fingerprint_version),
                        oauth_forwarded_header_fingerprints: err
                            .oauth_responses_debug
                            .as_ref()
                            .and_then(|debug| debug.forwarded_header_fingerprints.as_ref()),
                        oauth_prompt_cache_header_forwarded: err
                            .oauth_responses_debug
                            .as_ref()
                            .map(|debug| debug.prompt_cache_header_forwarded),
                        oauth_request_body_prefix_fingerprint: err
                            .oauth_responses_debug
                            .as_ref()
                            .and_then(|debug| debug.request_body_prefix_fingerprint.as_deref()),
                        oauth_request_body_prefix_bytes: err
                            .oauth_responses_debug
                            .as_ref()
                            .and_then(|debug| debug.request_body_prefix_bytes),
                        oauth_responses_rewrite: err
                            .oauth_responses_debug
                            .as_ref()
                            .map(|debug| &debug.rewrite),
                        service_tier: None,
                        stream_terminal_event: None,
                        upstream_error_code: err.upstream_error_code.as_deref(),
                        upstream_error_message: err.upstream_error_message.as_deref(),
                        upstream_request_id: err.upstream_request_id.as_deref(),
                        response_content_encoding: None,
                        proxy_display_name: pool_proxy_display_name.as_deref(),
                        proxy_weight_delta: None,
                        pool_attempt_count: Some(err.attempt_summary.pool_attempt_count),
                        pool_distinct_account_count: Some(
                            err.attempt_summary.pool_distinct_account_count,
                        ),
                        pool_attempt_terminal_reason: err
                            .attempt_summary
                            .pool_attempt_terminal_reason
                            .as_deref(),
                    })),
                    raw_response: "{}".to_string(),
                    req_raw,
                    resp_raw: RawPayloadMeta::default(),
                    timings: StageTimings {
                        t_total_ms: 0.0,
                        t_req_read_ms,
                        t_req_parse_ms,
                        t_upstream_connect_ms: err.connect_latency_ms,
                        t_upstream_ttfb_ms: 0.0,
                        t_upstream_stream_ms: 0.0,
                        t_resp_parse_ms: 0.0,
                        t_persist_ms: 0.0,
                    },
                };
                if let Err(err) =
                    persist_and_broadcast_proxy_capture(state.as_ref(), capture_started, record)
                        .await
                {
                    warn!(proxy_request_id, error = %err, "failed to persist proxy capture record");
                }
                return Err((err.status, err.message));
            }
        }
    } else {
        match send_forward_proxy_request_with_429_retry(
            state.clone(),
            Method::POST,
            target_url,
            &upstream_headers,
            Some(upstream_body_bytes.clone()),
            handshake_timeout,
            Some(capture_target),
            proxy_settings.upstream_429_max_retries,
        )
        .await
        {
            Ok(response) => (
                Some(response.selected_proxy),
                None,
                response.connect_latency_ms,
                None,
                0.0,
                None,
                response.attempt_recorded,
                response.attempt_update,
                None,
                None,
                PoolAttemptSummary::default(),
                Some(response.attempt_started_at),
                None,
                Some(base_request_bytes_for_capture.clone()),
                request_info.requested_service_tier.clone(),
                response.response,
            ),
            Err(err) => {
                let req_raw = store_raw_payload_file(
                    &state.config,
                    &invoke_id,
                    "request",
                    base_request_bytes_for_capture.as_ref(),
                );
                let proxy_attempt_update = record_forward_proxy_attempt(
                    state.clone(),
                    err.selected_proxy.clone(),
                    false,
                    Some(err.connect_latency_ms),
                    Some(err.attempt_failure_kind),
                    false,
                )
                .await;
                let usage = ParsedUsage::default();
                let (billing_service_tier, pricing_mode) =
                    resolve_proxy_billing_service_tier_and_pricing_mode(
                        None,
                        request_info.requested_service_tier.as_deref(),
                        None,
                        None,
                    );
                let (cost, cost_estimated, price_version) =
                    estimate_proxy_cost_from_shared_catalog(
                        &state.pricing_catalog,
                        request_info.model.as_deref(),
                        &usage,
                        billing_service_tier.as_deref(),
                        pricing_mode,
                    )
                    .await;
                let error_message = format!("[{}] {}", err.failure_kind, err.message);
                let record = ProxyCaptureRecord {
                    invoke_id,
                    occurred_at,
                    model: request_info.model,
                    usage,
                    cost,
                    cost_estimated,
                    price_version,
                    status: if err.status.is_server_error() {
                        format!("http_{}", err.status.as_u16())
                    } else {
                        "failed".to_string()
                    },
                    error_message: Some(error_message),
                    failure_kind: Some(err.failure_kind.to_string()),
                    payload: Some(build_proxy_payload_summary(ProxyPayloadSummary {
                        target: capture_target,
                        status: err.status,
                        is_stream: request_info.is_stream,
                        request_model: None,
                        requested_service_tier: request_info.requested_service_tier.as_deref(),
                        billing_service_tier: billing_service_tier.as_deref(),
                        reasoning_effort: request_info.reasoning_effort.as_deref(),
                        response_model: None,
                        usage_missing_reason: None,
                        request_parse_error: request_info.parse_error.as_deref(),
                        failure_kind: Some(err.failure_kind),
                        requester_ip: requester_ip.as_deref(),
                        upstream_scope: INVOCATION_UPSTREAM_SCOPE_EXTERNAL,
                        route_mode: INVOCATION_ROUTE_MODE_FORWARD_PROXY,
                        sticky_key: sticky_key.as_deref(),
                        prompt_cache_key: prompt_cache_key.as_deref(),
                        upstream_account_id: None,
                        upstream_account_name: None,
                        upstream_account_kind: None,
                        upstream_base_url_host: None,
                        oauth_account_header_attached: None,
                        oauth_account_id_shape: None,
                        oauth_forwarded_header_count: None,
                        oauth_forwarded_header_names: None,
                        oauth_fingerprint_version: None,
                        oauth_forwarded_header_fingerprints: None,
                        oauth_prompt_cache_header_forwarded: None,
                        oauth_request_body_prefix_fingerprint: None,
                        oauth_request_body_prefix_bytes: None,
                        oauth_responses_rewrite: None,
                        service_tier: None,
                        stream_terminal_event: None,
                        upstream_error_code: None,
                        upstream_error_message: None,
                        upstream_request_id: None,
                        response_content_encoding: None,
                        proxy_display_name: Some(err.selected_proxy.display_name.as_str()),
                        proxy_weight_delta: proxy_attempt_update.delta(),
                        pool_attempt_count: None,
                        pool_distinct_account_count: None,
                        pool_attempt_terminal_reason: None,
                    })),
                    raw_response: "{}".to_string(),
                    req_raw,
                    resp_raw: RawPayloadMeta::default(),
                    timings: StageTimings {
                        t_total_ms: 0.0,
                        t_req_read_ms,
                        t_req_parse_ms,
                        t_upstream_connect_ms: err.connect_latency_ms,
                        t_upstream_ttfb_ms: 0.0,
                        t_upstream_stream_ms: 0.0,
                        t_resp_parse_ms: 0.0,
                        t_persist_ms: 0.0,
                    },
                };
                if let Err(err) =
                    persist_and_broadcast_proxy_capture(state.as_ref(), capture_started, record)
                        .await
                {
                    warn!(proxy_request_id, error = %err, "failed to persist proxy capture record");
                }
                return Err((err.status, err.message));
            }
        }
    };
    request_info.requested_service_tier = final_requested_service_tier
        .clone()
        .or(request_info.requested_service_tier);
    let req_raw = store_raw_payload_file(
        &state.config,
        &invoke_id,
        "request",
        final_request_body_for_capture
            .as_ref()
            .unwrap_or(&base_request_bytes_for_capture)
            .as_ref(),
    );

    let upstream_status = upstream_response.status();
    let location_base_url = location_rewrite_upstream_base(
        pool_account.as_ref(),
        &state.config.openai_upstream_base_url,
    );
    let rewritten_location = match normalize_proxy_location_header(
        upstream_status,
        upstream_response.headers(),
        location_base_url,
    ) {
        Ok(location) => location,
        Err(err) => {
            let message = format!("failed to process upstream redirect: {err}");
            let proxy_attempt_update = if let Some(selected_proxy) = selected_proxy.as_ref() {
                record_forward_proxy_attempt(
                    state.clone(),
                    selected_proxy.clone(),
                    false,
                    Some(t_upstream_connect_ms),
                    Some(FORWARD_PROXY_FAILURE_SEND_ERROR),
                    false,
                )
                .await
            } else {
                ForwardProxyAttemptUpdate::default()
            };
            let usage = ParsedUsage::default();
            let (billing_service_tier, pricing_mode) =
                resolve_proxy_billing_service_tier_and_pricing_mode_for_account(
                    None,
                    request_info.requested_service_tier.as_deref(),
                    None,
                    pool_account.as_ref(),
                );
            let (cost, cost_estimated, price_version) = estimate_proxy_cost_from_shared_catalog(
                &state.pricing_catalog,
                request_info.model.as_deref(),
                &usage,
                billing_service_tier.as_deref(),
                pricing_mode,
            )
            .await;
            let proxy_display_name = resolve_invocation_proxy_display_name(selected_proxy.as_ref());
            let record = ProxyCaptureRecord {
                invoke_id,
                occurred_at,
                model: request_info.model,
                usage,
                cost,
                cost_estimated,
                price_version,
                status: "http_502".to_string(),
                error_message: Some(message.clone()),
                failure_kind: None,
                payload: Some(build_proxy_payload_summary(ProxyPayloadSummary {
                    target: capture_target,
                    status: StatusCode::BAD_GATEWAY,
                    is_stream: request_info.is_stream,
                    request_model: None,
                    requested_service_tier: request_info.requested_service_tier.as_deref(),
                    billing_service_tier: billing_service_tier.as_deref(),
                    reasoning_effort: request_info.reasoning_effort.as_deref(),
                    response_model: None,
                    usage_missing_reason: None,
                    request_parse_error: request_info.parse_error.as_deref(),
                    failure_kind: None,
                    requester_ip: requester_ip.as_deref(),
                    upstream_scope: if pool_route_active {
                        INVOCATION_UPSTREAM_SCOPE_INTERNAL
                    } else {
                        INVOCATION_UPSTREAM_SCOPE_EXTERNAL
                    },
                    route_mode: if pool_route_active {
                        INVOCATION_ROUTE_MODE_POOL
                    } else {
                        INVOCATION_ROUTE_MODE_FORWARD_PROXY
                    },
                    sticky_key: sticky_key.as_deref(),
                    prompt_cache_key: prompt_cache_key.as_deref(),
                    upstream_account_id: pool_account.as_ref().map(|account| account.account_id),
                    upstream_account_name: pool_account
                        .as_ref()
                        .map(|account| account.display_name.as_str()),
                    upstream_account_kind: payload_summary_upstream_account_kind(
                        pool_account.as_ref(),
                    ),
                    upstream_base_url_host: payload_summary_upstream_base_url_host(
                        pool_account.as_ref(),
                    ),
                    oauth_account_header_attached: oauth_account_header_attached_for_account(
                        pool_account.as_ref(),
                    ),
                    oauth_account_id_shape: oauth_account_id_shape_for_account(
                        pool_account.as_ref(),
                    ),
                    oauth_forwarded_header_count: None,
                    oauth_forwarded_header_names: None,
                    oauth_fingerprint_version: None,
                    oauth_forwarded_header_fingerprints: None,
                    oauth_prompt_cache_header_forwarded: None,
                    oauth_request_body_prefix_fingerprint: None,
                    oauth_request_body_prefix_bytes: None,
                    oauth_responses_rewrite: None,
                    service_tier: None,
                    stream_terminal_event: None,
                    upstream_error_code: None,
                    upstream_error_message: None,
                    upstream_request_id: None,
                    response_content_encoding: Some(
                        summarize_response_content_encoding(
                            upstream_response
                                .headers()
                                .get(header::CONTENT_ENCODING)
                                .and_then(|value| value.to_str().ok()),
                        )
                        .as_str(),
                    ),
                    proxy_display_name: proxy_display_name.as_deref(),
                    proxy_weight_delta: if selected_proxy.is_some() {
                        proxy_attempt_update.delta()
                    } else {
                        None
                    },
                    pool_attempt_count: None,
                    pool_distinct_account_count: None,
                    pool_attempt_terminal_reason: None,
                })),
                raw_response: "{}".to_string(),
                req_raw,
                resp_raw: RawPayloadMeta::default(),
                timings: StageTimings {
                    t_total_ms: 0.0,
                    t_req_read_ms,
                    t_req_parse_ms,
                    t_upstream_connect_ms,
                    t_upstream_ttfb_ms: 0.0,
                    t_upstream_stream_ms: 0.0,
                    t_resp_parse_ms: 0.0,
                    t_persist_ms: 0.0,
                },
            };
            if let Err(err) =
                persist_and_broadcast_proxy_capture(state.as_ref(), capture_started, record).await
            {
                warn!(proxy_request_id, error = %err, "failed to persist proxy capture record");
            }
            return Err((StatusCode::BAD_GATEWAY, message));
        }
    };

    let upstream_connection_scoped = connection_scoped_header_names(upstream_response.headers());
    let response_is_event_stream = upstream_response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.starts_with("text/event-stream"));
    let upstream_content_encoding = upstream_response
        .headers()
        .get(header::CONTENT_ENCODING)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    let response_content_encoding =
        summarize_response_content_encoding(upstream_content_encoding.as_deref());
    let selected_proxy_display_name =
        resolve_invocation_proxy_display_name(selected_proxy.as_ref());
    let response_running_record = build_running_proxy_capture_record(
        &invoke_id,
        &occurred_at,
        capture_target,
        &request_info,
        requester_ip.as_deref(),
        sticky_key.as_deref(),
        prompt_cache_key.as_deref(),
        pool_route_active,
        pool_account.as_ref().map(|account| account.account_id),
        pool_account
            .as_ref()
            .map(|account| account.display_name.as_str()),
        payload_summary_upstream_account_kind(pool_account.as_ref()),
        payload_summary_upstream_base_url_host(pool_account.as_ref()),
        selected_proxy_display_name.as_deref(),
        pool_account
            .as_ref()
            .map(|_| pending_pool_attempt_summary.pool_attempt_count),
        pool_account
            .as_ref()
            .map(|_| pending_pool_attempt_summary.pool_distinct_account_count),
        None,
        Some(response_content_encoding.as_str()),
        t_req_read_ms,
        t_req_parse_ms,
        t_upstream_connect_ms,
        prefetched_ttfb_ms,
    );
    if let Err(err) = persist_and_broadcast_proxy_capture_runtime_snapshot(
        state.as_ref(),
        response_running_record,
    )
    .await
    {
        warn!(
            ?err,
            invoke_id = %invoke_id,
            "failed to broadcast response-ready proxy capture snapshot"
        );
    }
    let mut response_builder = Response::builder().status(upstream_status);
    for (name, value) in upstream_response.headers() {
        if should_forward_proxy_header(name, &upstream_connection_scoped) {
            if name == header::LOCATION {
                if let Some(rewritten) = rewritten_location.as_deref() {
                    response_builder = response_builder.header(name, rewritten);
                }
            } else {
                response_builder = response_builder.header(name, value);
            }
        }
    }

    let state_for_task = state.clone();
    let request_info_for_task = request_info.clone();
    let req_raw_for_task = req_raw.clone();
    let invoke_id_for_task = invoke_id.clone();
    let occurred_at_for_task = occurred_at.clone();
    let upstream_content_encoding_for_task = upstream_content_encoding.clone();
    let requester_ip_for_task = requester_ip.clone();
    let sticky_key_for_task = sticky_key.clone();
    let reservation_key_for_task = pool_routing_reservation_key.clone();
    let prompt_cache_key_for_task = prompt_cache_key.clone();
    let selected_proxy_for_task = selected_proxy.clone();
    let selected_proxy_display_name_for_task = selected_proxy_display_name.clone();
    let pool_account_for_task = pool_account.clone();
    let oauth_responses_debug_for_task = oauth_responses_debug.clone();
    let attempt_already_recorded_for_task = attempt_already_recorded;
    let final_attempt_update_for_task = final_attempt_update;
    let pending_pool_attempt_record_for_task = pending_pool_attempt_record.clone();
    let mut deferred_pool_early_phase_cleanup_guard_for_task =
        deferred_pool_early_phase_cleanup_guard;
    let pending_pool_attempt_summary_for_task = pending_pool_attempt_summary.clone();
    let prefetched_first_chunk_for_task = prefetched_first_chunk;
    let prefetched_ttfb_ms_for_task = prefetched_ttfb_ms;
    let upstream_attempt_started_at_for_task = upstream_attempt_started_at;
    let upstream_attempt_started_at_utc_for_task = upstream_attempt_started_at_utc;
    let first_byte_timeout_for_task = first_byte_timeout;
    let stream_timeout_for_task = stream_timeout;
    let response_is_event_stream_for_task = response_is_event_stream;
    let (tx, rx) = mpsc::channel::<Result<Bytes, io::Error>>(16);

    tokio::spawn(async move {
        let mut stream = upstream_response.into_bytes_stream();
        let ttfb_started = Instant::now();
        let stream_started = Instant::now();
        let mut t_upstream_ttfb_ms = prefetched_ttfb_ms_for_task;
        let mut stream_started_at: Option<Instant> = None;
        let mut response_preview = RawResponsePreviewBuffer::default();
        let mut response_raw_writer =
            StreamingRawPayloadWriter::new(&state_for_task.config, &invoke_id_for_task, "response");
        let mut stream_response_parser = StreamResponsePayloadChunkParser::default();
        let mut nonstream_parse_buffer = (!response_is_event_stream_for_task).then(|| {
            BoundedResponseParseBuffer::new(BOUNDED_NON_STREAM_RESPONSE_PARSE_LIMIT_BYTES)
        });
        let mut stream_error: Option<String> = None;
        let mut downstream_closed = false;
        let mut forwarded_chunks = 0usize;
        let mut forwarded_bytes = 0usize;

        if let Some(chunk) = prefetched_first_chunk_for_task {
            response_preview.append(&chunk);
            response_raw_writer.append(&chunk).await;
            stream_response_parser.ingest_bytes(&chunk);
            if let Some(buffer) = nonstream_parse_buffer.as_mut() {
                buffer.append(&chunk);
            }
            forwarded_chunks = forwarded_chunks.saturating_add(1);
            forwarded_bytes = forwarded_bytes.saturating_add(chunk.len());
            stream_started_at = Some(Instant::now());
            if !downstream_closed && tx.send(Ok(chunk)).await.is_err() {
                downstream_closed = true;
            }
        }

        loop {
            let next_chunk = if let Some(stream_started_at) = stream_started_at {
                if let Some(stream_timeout) = stream_timeout_for_task {
                    let Some(timeout_budget) =
                        remaining_timeout_budget(stream_timeout, stream_started_at.elapsed())
                    else {
                        let message = pool_upstream_timeout_message(
                            stream_timeout,
                            "waiting for upstream stream completion",
                        );
                        stream_error = Some(message.clone());
                        if !downstream_closed
                            && tx.send(Err(io::Error::other(message))).await.is_err()
                        {
                            downstream_closed = true;
                        }
                        break;
                    };
                    match timeout(timeout_budget, stream.next()).await {
                        Ok(next_chunk) => next_chunk,
                        Err(_) => {
                            let message = pool_upstream_timeout_message(
                                stream_timeout,
                                "waiting for upstream stream completion",
                            );
                            stream_error = Some(message.clone());
                            if !downstream_closed
                                && tx.send(Err(io::Error::other(message))).await.is_err()
                            {
                                downstream_closed = true;
                            }
                            break;
                        }
                    }
                } else {
                    stream.next().await
                }
            } else if let Some(attempt_started_at) = upstream_attempt_started_at_for_task {
                let Some(timeout_budget) = remaining_timeout_budget(
                    first_byte_timeout_for_task,
                    attempt_started_at.elapsed(),
                ) else {
                    let message = pool_upstream_timeout_message(
                        first_byte_timeout_for_task,
                        "waiting for first upstream chunk",
                    );
                    stream_error = Some(message.clone());
                    if !downstream_closed && tx.send(Err(io::Error::other(message))).await.is_err()
                    {
                        downstream_closed = true;
                    }
                    break;
                };
                match timeout(timeout_budget, stream.next()).await {
                    Ok(next_chunk) => next_chunk,
                    Err(_) => {
                        let message = pool_upstream_timeout_message(
                            first_byte_timeout_for_task,
                            "waiting for first upstream chunk",
                        );
                        stream_error = Some(message.clone());
                        if !downstream_closed
                            && tx.send(Err(io::Error::other(message))).await.is_err()
                        {
                            downstream_closed = true;
                        }
                        break;
                    }
                }
            } else {
                stream.next().await
            };
            let Some(next_chunk) = next_chunk else {
                break;
            };
            match next_chunk {
                Ok(chunk) => {
                    if stream_started_at.is_none() {
                        t_upstream_ttfb_ms = upstream_attempt_started_at_for_task
                            .map(elapsed_ms)
                            .unwrap_or_else(|| elapsed_ms(ttfb_started));
                        stream_started_at = Some(Instant::now());
                        let running_record = build_running_proxy_capture_record(
                            &invoke_id_for_task,
                            &occurred_at_for_task,
                            capture_target,
                            &request_info_for_task,
                            requester_ip_for_task.as_deref(),
                            sticky_key_for_task.as_deref(),
                            prompt_cache_key_for_task.as_deref(),
                            pool_account_for_task.is_some(),
                            pool_account_for_task
                                .as_ref()
                                .map(|account| account.account_id),
                            pool_account_for_task
                                .as_ref()
                                .map(|account| account.display_name.as_str()),
                            payload_summary_upstream_account_kind(
                                pool_account_for_task.as_ref(),
                            ),
                            payload_summary_upstream_base_url_host(
                                pool_account_for_task.as_ref(),
                            ),
                            selected_proxy_display_name_for_task.as_deref(),
                            pool_account_for_task
                                .as_ref()
                                .map(|_| pending_pool_attempt_summary_for_task.pool_attempt_count),
                            pool_account_for_task.as_ref().map(|_| {
                                pending_pool_attempt_summary_for_task.pool_distinct_account_count
                            }),
                            None,
                            Some(response_content_encoding.as_str()),
                            t_req_read_ms,
                            t_req_parse_ms,
                            t_upstream_connect_ms,
                            t_upstream_ttfb_ms,
                        );
                        if let Err(err) = persist_and_broadcast_proxy_capture_runtime_snapshot(
                            state_for_task.as_ref(),
                            running_record,
                        )
                        .await
                        {
                            warn!(
                                ?err,
                                invoke_id = %invoke_id_for_task,
                                "failed to broadcast first-byte proxy capture snapshot"
                            );
                        }
                    }
                    response_preview.append(&chunk);
                    response_raw_writer.append(&chunk).await;
                    stream_response_parser.ingest_bytes(&chunk);
                    if let Some(buffer) = nonstream_parse_buffer.as_mut() {
                        buffer.append(&chunk);
                    }
                    forwarded_chunks = forwarded_chunks.saturating_add(1);
                    forwarded_bytes = forwarded_bytes.saturating_add(chunk.len());
                    if !downstream_closed && tx.send(Ok(chunk)).await.is_err() {
                        downstream_closed = true;
                    }
                }
                Err(err) => {
                    let msg = format!("upstream stream error: {err}");
                    stream_error = Some(msg.clone());
                    if !downstream_closed {
                        let _ = tx.send(Err(io::Error::other(msg))).await;
                    }
                    break;
                }
            }
        }
        drop(tx);

        let terminal_state = if stream_error.is_some() {
            PROXY_STREAM_TERMINAL_ERROR
        } else if downstream_closed {
            PROXY_STREAM_TERMINAL_DOWNSTREAM_CLOSED
        } else {
            PROXY_STREAM_TERMINAL_COMPLETED
        };
        if let Some(err) = stream_error.as_deref() {
            warn!(
                proxy_request_id,
                terminal_state,
                error = err,
                forwarded_chunks,
                forwarded_bytes,
                elapsed_ms = stream_started.elapsed().as_millis(),
                "openai proxy capture stream finished with upstream error"
            );
        } else {
            info!(
                proxy_request_id,
                terminal_state,
                forwarded_chunks,
                forwarded_bytes,
                elapsed_ms = stream_started.elapsed().as_millis(),
                "openai proxy capture stream finished"
            );
        }

        let t_upstream_stream_ms = stream_started_at.map(elapsed_ms).unwrap_or(0.0);
        let resp_raw = response_raw_writer.finish().await;
        let preview_bytes = response_preview.as_slice().to_vec();
        let raw_response_preview = response_preview.into_preview();
        let streamed_response_outcome = stream_response_parser.finish();
        let preview_looks_like_sse = response_payload_looks_like_sse_after_decode(
            &preview_bytes,
            upstream_content_encoding_for_task.as_deref(),
        );
        let response_is_stream_hint = response_is_event_stream_for_task
            || streamed_response_outcome.saw_stream_fields
            || preview_looks_like_sse;
        let resp_parse_started = Instant::now();
        let mut response_info = if response_is_stream_hint {
            if streamed_response_outcome.saw_stream_fields {
                streamed_response_outcome.response_info
            } else {
                parse_target_response_preview_payload(
                    capture_target,
                    &preview_bytes,
                    true,
                    upstream_content_encoding_for_task.as_deref(),
                )
            }
        } else {
            nonstream_parse_buffer
                .take()
                .map(|buffer| {
                    buffer.into_response_info(
                        capture_target,
                        upstream_content_encoding_for_task.as_deref(),
                    )
                })
                .unwrap_or_else(|| {
                    parse_target_response_payload(
                        capture_target,
                        &preview_bytes,
                        false,
                        upstream_content_encoding_for_task.as_deref(),
                    )
                })
        };
        let t_resp_parse_ms = elapsed_ms(resp_parse_started);

        if response_info.model.is_none() {
            response_info.model = request_info_for_task.model.clone();
        }
        if response_info.usage_missing_reason.is_none() && stream_error.is_some() {
            response_info.usage_missing_reason = Some("upstream_stream_error".to_string());
        }

        let failure_kind = if stream_error.is_some() {
            Some(PROXY_FAILURE_UPSTREAM_STREAM_ERROR)
        } else if downstream_closed {
            Some(PROXY_STREAM_TERMINAL_DOWNSTREAM_CLOSED)
        } else if response_info.stream_terminal_event.is_some() {
            Some(PROXY_FAILURE_UPSTREAM_RESPONSE_FAILED)
        } else if upstream_status == StatusCode::TOO_MANY_REQUESTS {
            Some(FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429)
        } else if upstream_status.is_server_error() {
            Some(FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_5XX)
        } else {
            None
        };
        let had_stream_error = stream_error.is_some();
        let had_logical_stream_failure = response_info.stream_terminal_event.is_some();

        let error_message = if let Some(err) = stream_error {
            Some(format!("[{}] {err}", PROXY_FAILURE_UPSTREAM_STREAM_ERROR))
        } else if downstream_closed {
            Some(format!(
                "[{}] downstream closed while streaming upstream response",
                PROXY_STREAM_TERMINAL_DOWNSTREAM_CLOSED
            ))
        } else if response_info.stream_terminal_event.is_some() {
            Some(format_upstream_response_failed_message(&response_info))
        } else if !upstream_status.is_success() {
            response_info
                .upstream_error_message
                .clone()
                .or_else(|| extract_error_message_from_response_preview(&preview_bytes))
        } else {
            None
        };
        let status = if upstream_status.is_success() && error_message.is_none() {
            "success".to_string()
        } else {
            format!("http_{}", upstream_status.as_u16())
        };
        let pending_pool_attempt_terminal_reason = if pool_account_for_task.is_none() {
            None
        } else if had_stream_error {
            Some(PROXY_FAILURE_UPSTREAM_STREAM_ERROR.to_string())
        } else if downstream_closed {
            Some(PROXY_STREAM_TERMINAL_DOWNSTREAM_CLOSED.to_string())
        } else if response_info.stream_terminal_event.is_some() {
            Some(PROXY_FAILURE_UPSTREAM_RESPONSE_FAILED.to_string())
        } else if !upstream_status.is_success() {
            Some(
                failure_kind
                    .unwrap_or(PROXY_FAILURE_UPSTREAM_RESPONSE_FAILED)
                    .to_string(),
            )
        } else {
            None
        };
        let mut pending_pool_attempt_summary = pending_pool_attempt_summary_for_task.clone();
        pending_pool_attempt_summary.pool_attempt_terminal_reason =
            pending_pool_attempt_terminal_reason.clone();
        let selected_proxy_display_name =
            resolve_invocation_proxy_display_name(selected_proxy_for_task.as_ref());
        let proxy_attempt_update = if let Some(selected_proxy) = selected_proxy_for_task.as_ref() {
            let forward_proxy_success = proxy_capture_response_status_is_success(
                upstream_status,
                had_stream_error,
                had_logical_stream_failure,
            );
            if attempt_already_recorded_for_task {
                final_attempt_update_for_task.unwrap_or_default()
            } else {
                record_forward_proxy_attempt(
                    state_for_task.clone(),
                    selected_proxy.clone(),
                    forward_proxy_success,
                    Some(t_upstream_connect_ms + t_upstream_ttfb_ms + t_upstream_stream_ms),
                    proxy_capture_response_failure_kind(
                        upstream_status,
                        had_stream_error,
                        had_logical_stream_failure,
                    ),
                    false,
                )
                .await
            }
        } else {
            if let Some(account) = pool_account_for_task.as_ref() {
                let pool_route_success = proxy_capture_response_status_is_success(
                    upstream_status,
                    had_stream_error,
                    had_logical_stream_failure,
                );
                let route_result = if pool_route_success {
                    consume_pool_routing_reservation(
                        state_for_task.as_ref(),
                        &reservation_key_for_task,
                    );
                    record_pool_route_success(
                        &state_for_task.pool,
                        account.account_id,
                        upstream_attempt_started_at_utc_for_task.unwrap_or_else(Utc::now),
                        sticky_key_for_task.as_deref(),
                        None,
                    )
                    .await
                } else if had_stream_error {
                    let route_message = error_message
                        .as_deref()
                        .unwrap_or("upstream stream error")
                        .to_string();
                    release_pool_routing_reservation(
                        state_for_task.as_ref(),
                        &reservation_key_for_task,
                    );
                    record_pool_route_transport_failure(
                        &state_for_task.pool,
                        account.account_id,
                        sticky_key_for_task.as_deref(),
                        &route_message,
                        None,
                    )
                    .await
                } else {
                    let route_message = error_message
                        .as_deref()
                        .unwrap_or("upstream request failed")
                        .to_string();
                    release_pool_routing_reservation(
                        state_for_task.as_ref(),
                        &reservation_key_for_task,
                    );
                    if response_info_is_retryable_server_overloaded(upstream_status, &response_info)
                    {
                        record_pool_route_retryable_overload_failure(
                            &state_for_task.pool,
                            account.account_id,
                            sticky_key_for_task.as_deref(),
                            &route_message,
                            None,
                        )
                        .await
                    } else {
                        record_pool_route_http_failure(
                            &state_for_task.pool,
                            account.account_id,
                            &account.kind,
                            sticky_key_for_task.as_deref(),
                            upstream_status,
                            &route_message,
                            None,
                        )
                        .await
                    }
                };
                if let Err(err) = route_result {
                    warn!(account_id = account.account_id, error = %err, "failed to record pool capture route state");
                }
            }
            ForwardProxyAttemptUpdate::default()
        };
        if let Some(pending_attempt_record) = pending_pool_attempt_record_for_task.as_ref() {
            let finished_at = shanghai_now_string();
            let attempt_status = if had_stream_error || downstream_closed {
                POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_TRANSPORT_FAILURE
            } else if !upstream_status.is_success() || had_logical_stream_failure {
                POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_HTTP_FAILURE
            } else {
                POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS
            };
            if let Err(err) = finalize_pool_upstream_request_attempt(
                &state_for_task.pool,
                pending_attempt_record,
                finished_at.as_str(),
                attempt_status,
                Some(upstream_status),
                failure_kind,
                error_message.as_deref(),
                Some(t_upstream_connect_ms),
                Some(t_upstream_ttfb_ms),
                Some(t_upstream_stream_ms),
                response_info.upstream_request_id.as_deref(),
                None,
                None,
            )
            .await
            {
                warn!(
                    invoke_id = %pending_attempt_record.invoke_id,
                    error = %err,
                    "failed to persist final pool attempt"
                );
            } else {
                disarm_pool_early_phase_cleanup_guard(
                    &mut deferred_pool_early_phase_cleanup_guard_for_task,
                );
            }
            if let Err(err) = broadcast_pool_upstream_attempts_snapshot(
                state_for_task.as_ref(),
                &pending_attempt_record.invoke_id,
            )
            .await
            {
                warn!(
                    invoke_id = %pending_attempt_record.invoke_id,
                    error = %err,
                    "failed to broadcast final pool attempt snapshot"
                );
            }
        }
        let (billing_service_tier, pricing_mode) =
            resolve_proxy_billing_service_tier_and_pricing_mode_for_account(
                None,
                request_info_for_task.requested_service_tier.as_deref(),
                response_info.service_tier.as_deref(),
                pool_account_for_task.as_ref(),
            );
        let (cost, cost_estimated, price_version) = estimate_proxy_cost_from_shared_catalog(
            &state_for_task.pricing_catalog,
            response_info.model.as_deref(),
            &response_info.usage,
            billing_service_tier.as_deref(),
            pricing_mode,
        )
        .await;
        let payload = build_proxy_payload_summary(ProxyPayloadSummary {
            target: capture_target,
            status: upstream_status,
            is_stream: request_info_for_task.is_stream,
            request_model: request_info_for_task.model.as_deref(),
            requested_service_tier: request_info_for_task.requested_service_tier.as_deref(),
            billing_service_tier: billing_service_tier.as_deref(),
            reasoning_effort: request_info_for_task.reasoning_effort.as_deref(),
            response_model: response_info.model.as_deref(),
            usage_missing_reason: response_info.usage_missing_reason.as_deref(),
            request_parse_error: request_info_for_task.parse_error.as_deref(),
            failure_kind,
            requester_ip: requester_ip_for_task.as_deref(),
            upstream_scope: if pool_account_for_task.is_some() {
                INVOCATION_UPSTREAM_SCOPE_INTERNAL
            } else {
                INVOCATION_UPSTREAM_SCOPE_EXTERNAL
            },
            route_mode: if pool_account_for_task.is_some() {
                INVOCATION_ROUTE_MODE_POOL
            } else {
                INVOCATION_ROUTE_MODE_FORWARD_PROXY
            },
            sticky_key: sticky_key_for_task.as_deref(),
            prompt_cache_key: prompt_cache_key_for_task.as_deref(),
            upstream_account_id: pool_account_for_task
                .as_ref()
                .map(|account| account.account_id),
            upstream_account_name: pool_account_for_task
                .as_ref()
                .map(|account| account.display_name.as_str()),
            upstream_account_kind: payload_summary_upstream_account_kind(
                pool_account_for_task.as_ref(),
            ),
            upstream_base_url_host: payload_summary_upstream_base_url_host(
                pool_account_for_task.as_ref(),
            ),
            oauth_account_header_attached: oauth_account_header_attached_for_account(
                pool_account_for_task.as_ref(),
            ),
            oauth_account_id_shape: oauth_account_id_shape_for_account(
                pool_account_for_task.as_ref(),
            ),
            oauth_forwarded_header_count: oauth_responses_debug_for_task
                .as_ref()
                .map(|debug| debug.forwarded_header_names.len()),
            oauth_forwarded_header_names: oauth_responses_debug_for_task
                .as_ref()
                .map(|debug| debug.forwarded_header_names.as_slice()),
            oauth_fingerprint_version: oauth_responses_debug_for_task
                .as_ref()
                .and_then(|debug| debug.fingerprint_version),
            oauth_forwarded_header_fingerprints: oauth_responses_debug_for_task
                .as_ref()
                .and_then(|debug| debug.forwarded_header_fingerprints.as_ref()),
            oauth_prompt_cache_header_forwarded: oauth_responses_debug_for_task
                .as_ref()
                .map(|debug| debug.prompt_cache_header_forwarded),
            oauth_request_body_prefix_fingerprint: oauth_responses_debug_for_task
                .as_ref()
                .and_then(|debug| debug.request_body_prefix_fingerprint.as_deref()),
            oauth_request_body_prefix_bytes: oauth_responses_debug_for_task
                .as_ref()
                .and_then(|debug| debug.request_body_prefix_bytes),
            oauth_responses_rewrite: oauth_responses_debug_for_task
                .as_ref()
                .map(|debug| &debug.rewrite),
            service_tier: response_info.service_tier.as_deref(),
            stream_terminal_event: response_info.stream_terminal_event.as_deref(),
            upstream_error_code: response_info.upstream_error_code.as_deref(),
            upstream_error_message: response_info.upstream_error_message.as_deref(),
            upstream_request_id: response_info.upstream_request_id.as_deref(),
            response_content_encoding: Some(response_content_encoding.as_str()),
            proxy_display_name: selected_proxy_display_name.as_deref(),
            proxy_weight_delta: if selected_proxy_for_task.is_some() {
                proxy_attempt_update.delta()
            } else {
                None
            },
            pool_attempt_count: pool_account_for_task
                .as_ref()
                .map(|_| pending_pool_attempt_summary.pool_attempt_count),
            pool_distinct_account_count: pool_account_for_task
                .as_ref()
                .map(|_| pending_pool_attempt_summary.pool_distinct_account_count),
            pool_attempt_terminal_reason: pool_account_for_task
                .as_ref()
                .and_then(|_| pending_pool_attempt_terminal_reason.as_deref()),
        });

        let record = ProxyCaptureRecord {
            invoke_id: invoke_id_for_task,
            occurred_at: occurred_at_for_task,
            model: response_info.model,
            usage: response_info.usage,
            cost,
            cost_estimated,
            price_version,
            status,
            error_message,
            failure_kind: failure_kind.map(|kind| kind.to_string()),
            payload: Some(payload),
            raw_response: raw_response_preview,
            req_raw: req_raw_for_task,
            resp_raw,
            timings: StageTimings {
                t_total_ms: 0.0,
                t_req_read_ms,
                t_req_parse_ms,
                t_upstream_connect_ms,
                t_upstream_ttfb_ms,
                t_upstream_stream_ms,
                t_resp_parse_ms,
                t_persist_ms: 0.0,
            },
        };

        if let Err(err) =
            persist_and_broadcast_proxy_capture(state_for_task.as_ref(), capture_started, record)
                .await
        {
            warn!(proxy_request_id, error = %err, "failed to persist proxy capture record");
        }
    });

    response_builder
        .body(Body::from_stream(ReceiverStream::new(rx)))
        .map_err(|err| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("failed to build proxy response: {err}"),
            )
        })
}

async fn read_request_body_with_limit(
    body: Body,
    body_limit: usize,
    request_read_timeout: Duration,
    proxy_request_id: u64,
) -> Result<Vec<u8>, RequestBodyReadError> {
    let mut data = Vec::new();
    let mut stream = body.into_data_stream();
    let read_deadline = Instant::now() + request_read_timeout;

    loop {
        let remaining = read_deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            warn!(
                proxy_request_id,
                timeout_ms = request_read_timeout.as_millis(),
                read_bytes = data.len(),
                "openai proxy request body read timed out"
            );
            return Err(RequestBodyReadError {
                status: StatusCode::REQUEST_TIMEOUT,
                message: format!(
                    "request body read timed out after {}ms",
                    request_read_timeout.as_millis()
                ),
                failure_kind: PROXY_FAILURE_REQUEST_BODY_READ_TIMEOUT,
                partial_body: data,
            });
        }

        let next_chunk = match timeout(remaining, stream.next()).await {
            Ok(chunk) => chunk,
            Err(_) => {
                warn!(
                    proxy_request_id,
                    timeout_ms = request_read_timeout.as_millis(),
                    read_bytes = data.len(),
                    "openai proxy request body read timed out"
                );
                return Err(RequestBodyReadError {
                    status: StatusCode::REQUEST_TIMEOUT,
                    message: format!(
                        "request body read timed out after {}ms",
                        request_read_timeout.as_millis()
                    ),
                    failure_kind: PROXY_FAILURE_REQUEST_BODY_READ_TIMEOUT,
                    partial_body: data,
                });
            }
        };

        let Some(chunk) = next_chunk else {
            break;
        };
        let chunk = match chunk {
            Ok(chunk) => chunk,
            Err(err) => {
                warn!(
                    proxy_request_id,
                    error = %err,
                    read_bytes = data.len(),
                    "openai proxy request body stream error"
                );
                return Err(RequestBodyReadError {
                    status: StatusCode::BAD_REQUEST,
                    message: format!("failed to read request body stream: {err}"),
                    failure_kind: PROXY_FAILURE_REQUEST_BODY_STREAM_ERROR_CLIENT_CLOSED,
                    partial_body: data,
                });
            }
        };

        if data.len().saturating_add(chunk.len()) > body_limit {
            let allowed = body_limit.saturating_sub(data.len());
            if allowed > 0 {
                data.extend_from_slice(&chunk[..allowed.min(chunk.len())]);
            }
            return Err(RequestBodyReadError {
                status: StatusCode::PAYLOAD_TOO_LARGE,
                message: format!("request body exceeds {body_limit} bytes"),
                failure_kind: PROXY_FAILURE_BODY_TOO_LARGE,
                partial_body: data,
            });
        }

        data.extend_from_slice(&chunk);
    }

    Ok(data)
}

async fn read_request_body_snapshot_with_limit(
    body: Body,
    body_limit: usize,
    request_read_timeout: Duration,
    proxy_request_id: u64,
) -> Result<PoolReplayBodySnapshot, RequestBodyReadError> {
    let mut buffer = PoolReplayBodyBuffer::new(proxy_request_id);
    let mut stream = body.into_data_stream();
    let read_deadline = Instant::now() + request_read_timeout;
    let mut data_len = 0usize;

    loop {
        let remaining = read_deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            warn!(
                proxy_request_id,
                timeout_ms = request_read_timeout.as_millis(),
                read_bytes = data_len,
                "openai proxy request body read timed out"
            );
            return Err(RequestBodyReadError {
                status: StatusCode::REQUEST_TIMEOUT,
                message: format!(
                    "request body read timed out after {}ms",
                    request_read_timeout.as_millis()
                ),
                failure_kind: PROXY_FAILURE_REQUEST_BODY_READ_TIMEOUT,
                partial_body: Vec::new(),
            });
        }

        let next_chunk = match timeout(remaining, stream.next()).await {
            Ok(chunk) => chunk,
            Err(_) => {
                warn!(
                    proxy_request_id,
                    timeout_ms = request_read_timeout.as_millis(),
                    read_bytes = data_len,
                    "openai proxy request body read timed out"
                );
                return Err(RequestBodyReadError {
                    status: StatusCode::REQUEST_TIMEOUT,
                    message: format!(
                        "request body read timed out after {}ms",
                        request_read_timeout.as_millis()
                    ),
                    failure_kind: PROXY_FAILURE_REQUEST_BODY_READ_TIMEOUT,
                    partial_body: Vec::new(),
                });
            }
        };

        let Some(chunk) = next_chunk else {
            return buffer.finish().await.map_err(|err| RequestBodyReadError {
                status: StatusCode::BAD_GATEWAY,
                message: format!("failed to cache request body for oauth replay: {err}"),
                failure_kind: PROXY_FAILURE_FAILED_CONTACT_UPSTREAM,
                partial_body: Vec::new(),
            });
        };

        let chunk = match chunk {
            Ok(chunk) => chunk,
            Err(err) => {
                warn!(
                    proxy_request_id,
                    error = %err,
                    read_bytes = data_len,
                    "openai proxy request body stream error"
                );
                return Err(RequestBodyReadError {
                    status: StatusCode::BAD_REQUEST,
                    message: format!("failed to read request body stream: {err}"),
                    failure_kind: PROXY_FAILURE_REQUEST_BODY_STREAM_ERROR_CLIENT_CLOSED,
                    partial_body: Vec::new(),
                });
            }
        };

        if data_len.saturating_add(chunk.len()) > body_limit {
            return Err(RequestBodyReadError {
                status: StatusCode::PAYLOAD_TOO_LARGE,
                message: format!("request body exceeds {body_limit} bytes"),
                failure_kind: PROXY_FAILURE_BODY_TOO_LARGE,
                partial_body: Vec::new(),
            });
        }
        data_len = data_len.saturating_add(chunk.len());

        buffer
            .append(&chunk)
            .await
            .map_err(|err| RequestBodyReadError {
                status: StatusCode::BAD_GATEWAY,
                message: format!("failed to cache request body for oauth replay: {err}"),
                failure_kind: PROXY_FAILURE_FAILED_CONTACT_UPSTREAM,
                partial_body: Vec::new(),
            })?;
    }
}

const HEADER_STICKY_EARLY_STICKY_SCAN_BYTES: usize = 64 * 1024;

fn best_effort_extract_sticky_key_from_request_body_prefix(bytes: &[u8]) -> Option<String> {
    const STICKY_KEY_PATTERNS: &[&[u8]] = &[
        br#""sticky_key""#,
        br#""stickyKey""#,
        br#""prompt_cache_key""#,
        br#""promptCacheKey""#,
    ];

    fn find_subslice(haystack: &[u8], needle: &[u8], start: usize) -> Option<usize> {
        haystack[start..]
            .windows(needle.len())
            .position(|window| window == needle)
            .map(|offset| start + offset)
    }

    fn skip_ascii_whitespace(bytes: &[u8], mut index: usize) -> usize {
        while index < bytes.len() && bytes[index].is_ascii_whitespace() {
            index += 1;
        }
        index
    }

    fn parse_json_string(bytes: &[u8], index: usize) -> Option<(String, usize)> {
        if bytes.get(index) != Some(&b'"') {
            return None;
        }
        let mut parsed = String::new();
        let mut cursor = index + 1;
        while cursor < bytes.len() {
            match bytes[cursor] {
                b'"' => return Some((parsed, cursor + 1)),
                b'\\' => {
                    cursor += 1;
                    let escaped = *bytes.get(cursor)?;
                    match escaped {
                        b'"' | b'\\' | b'/' => parsed.push(escaped as char),
                        b'b' => parsed.push('\u{0008}'),
                        b'f' => parsed.push('\u{000C}'),
                        b'n' => parsed.push('\n'),
                        b'r' => parsed.push('\r'),
                        b't' => parsed.push('\t'),
                        // Stay conservative for unicode escapes; missing the key is better
                        // than returning a false early decision.
                        b'u' => return None,
                        _ => return None,
                    }
                }
                byte if byte.is_ascii_control() => return None,
                byte if byte.is_ascii() => parsed.push(byte as char),
                _ => return None,
            }
            cursor += 1;
        }
        None
    }

    for pattern in STICKY_KEY_PATTERNS {
        let mut cursor = 0usize;
        while let Some(key_start) = find_subslice(bytes, pattern, cursor) {
            let mut value_start = skip_ascii_whitespace(bytes, key_start + pattern.len());
            if bytes.get(value_start) != Some(&b':') {
                cursor = key_start + pattern.len();
                continue;
            }
            value_start = skip_ascii_whitespace(bytes, value_start + 1);
            let Some((value, next_index)) = parse_json_string(bytes, value_start) else {
                cursor = key_start + pattern.len();
                continue;
            };
            let normalized = value.trim();
            if !normalized.is_empty() {
                return Some(normalized.to_string());
            }
            cursor = next_index;
        }
    }
    None
}

fn prepare_target_request_body(
    target: ProxyCaptureTarget,
    body: Vec<u8>,
    auto_include_usage: bool,
) -> (Vec<u8>, RequestCaptureInfo, bool) {
    let mut info = RequestCaptureInfo {
        model: None,
        sticky_key: None,
        prompt_cache_key: None,
        requested_service_tier: None,
        reasoning_effort: None,
        is_stream: false,
        parse_error: None,
    };

    if body.is_empty() {
        return (body, info, false);
    }

    let mut value: Value = match serde_json::from_slice(&body) {
        Ok(value) => value,
        Err(err) => {
            info.parse_error = Some(format!("request_json_parse_error:{err}"));
            return (body, info, false);
        }
    };

    info.model = value
        .get("model")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    info.sticky_key = extract_sticky_key_from_request_body(&value);
    info.prompt_cache_key = extract_prompt_cache_key_from_request_body(&value);
    info.reasoning_effort = extract_reasoning_effort_from_request_body(target, &value);
    info.is_stream = value
        .get("stream")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let mut rewritten = false;
    if target.should_auto_include_usage()
        && info.is_stream
        && auto_include_usage
        && let Some(object) = value.as_object_mut()
    {
        let stream_options = object
            .entry("stream_options".to_string())
            .or_insert_with(|| json!({}));
        if let Some(stream_options_obj) = stream_options.as_object_mut() {
            stream_options_obj.insert("include_usage".to_string(), Value::Bool(true));
            rewritten = true;
        } else {
            object.insert(
                "stream_options".to_string(),
                json!({ "include_usage": true }),
            );
            rewritten = true;
        }
    }

    info.requested_service_tier = extract_requested_service_tier_from_request_body(&value);

    if rewritten {
        match serde_json::to_vec(&value) {
            Ok(rewritten_body) => (rewritten_body, info, true),
            Err(err) => {
                let mut fallback = info;
                fallback.parse_error = Some(format!("request_json_rewrite_error:{err}"));
                (body, fallback, false)
            }
        }
    } else {
        (body, info, false)
    }
}

fn proxy_upstream_send_timeout_for_capture_target(
    timeouts: &PoolRoutingTimeoutSettingsResolved,
    capture_target: Option<ProxyCaptureTarget>,
) -> Duration {
    match capture_target {
        Some(ProxyCaptureTarget::Responses) => timeouts.responses_first_byte_timeout,
        Some(ProxyCaptureTarget::ResponsesCompact) => timeouts.compact_first_byte_timeout,
        _ => timeouts.default_send_timeout,
    }
}

fn pool_upstream_first_chunk_timeout(
    timeouts: &PoolRoutingTimeoutSettingsResolved,
    original_uri: &Uri,
    method: &Method,
) -> Duration {
    match capture_target_for_request(original_uri.path(), method) {
        Some(ProxyCaptureTarget::Responses) => timeouts.responses_first_byte_timeout,
        Some(ProxyCaptureTarget::ResponsesCompact) => timeouts.compact_first_byte_timeout,
        _ => timeouts.default_first_byte_timeout,
    }
}

fn pool_upstream_responses_total_timeout(
    config: &AppConfig,
    original_uri: &Uri,
    method: &Method,
) -> Option<Duration> {
    pool_uses_responses_timeout_failover_policy(original_uri, method)
        .then_some(config.pool_upstream_responses_total_timeout)
}

fn proxy_capture_target_stream_timeout(
    timeouts: &PoolRoutingTimeoutSettingsResolved,
    capture_target: ProxyCaptureTarget,
) -> Option<Duration> {
    match capture_target {
        ProxyCaptureTarget::Responses => Some(timeouts.responses_stream_timeout),
        ProxyCaptureTarget::ResponsesCompact => Some(timeouts.compact_stream_timeout),
        ProxyCaptureTarget::ChatCompletions => None,
    }
}

fn pool_upstream_send_timeout(
    original_uri: &Uri,
    method: &Method,
    send_timeout: Duration,
    pre_first_byte_timeout: Duration,
) -> Duration {
    if pool_uses_responses_timeout_failover_policy(original_uri, method) {
        pre_first_byte_timeout
    } else {
        send_timeout.min(pre_first_byte_timeout)
    }
}

fn pool_uses_responses_timeout_failover_policy(original_uri: &Uri, method: &Method) -> bool {
    method == Method::POST
        && matches!(
            original_uri.path(),
            "/v1/responses" | "/v1/responses/compact"
        )
}

fn pool_timeout_budget_with_total_limit(
    timeout: Duration,
    total_timeout: Option<Duration>,
    total_timeout_started_at: Option<Instant>,
) -> Option<Duration> {
    match (total_timeout, total_timeout_started_at) {
        (Some(total_timeout), Some(started_at)) => {
            remaining_timeout_budget(total_timeout, started_at.elapsed())
                .map(|remaining| remaining.min(timeout))
        }
        // A running total-timeout only exists once we have an anchor instant.
        // Pre-attempt no-account waiting uses an explicit deadline earlier in the flow;
        // real upstream attempts without an anchor should keep their own per-phase budgets.
        (Some(_), None) => Some(timeout),
        (None, _) => Some(timeout),
    }
}

fn pool_total_timeout_exhausted(total_timeout: Duration, started_at: Instant) -> bool {
    timeout_budget_exhausted(total_timeout, started_at.elapsed())
}

fn pool_total_timeout_exhausted_message(total_timeout: Duration) -> String {
    format!(
        "pool upstream total timeout exhausted after {}ms",
        total_timeout.as_millis()
    )
}

fn build_pool_total_timeout_exhausted_error(
    total_timeout: Duration,
    last_error: Option<PoolUpstreamError>,
    attempt_count: usize,
    distinct_account_count: usize,
) -> PoolUpstreamError {
    let mut final_error = last_error.unwrap_or(PoolUpstreamError {
        account: None,
        status: StatusCode::GATEWAY_TIMEOUT,
        message: pool_total_timeout_exhausted_message(total_timeout),
        failure_kind: PROXY_FAILURE_POOL_TOTAL_TIMEOUT_EXHAUSTED,
        connect_latency_ms: 0.0,
        upstream_error_code: None,
        upstream_error_message: None,
        upstream_request_id: None,
        oauth_responses_debug: None,
        attempt_summary: PoolAttemptSummary::default(),
        requested_service_tier: None,
        request_body_for_capture: None,
    });
    final_error.status = StatusCode::GATEWAY_TIMEOUT;
    final_error.message = pool_total_timeout_exhausted_message(total_timeout);
    final_error.failure_kind = PROXY_FAILURE_POOL_TOTAL_TIMEOUT_EXHAUSTED;
    final_error.upstream_error_code = None;
    final_error.upstream_error_message = None;
    final_error.upstream_request_id = None;
    final_error.attempt_summary = pool_attempt_summary(
        attempt_count,
        distinct_account_count,
        Some(PROXY_FAILURE_POOL_TOTAL_TIMEOUT_EXHAUSTED.to_string()),
    );
    final_error
}

fn pool_pre_attempt_total_timeout_error(total_timeout: Duration) -> (StatusCode, String) {
    (
        StatusCode::GATEWAY_TIMEOUT,
        pool_total_timeout_exhausted_message(total_timeout),
    )
}

fn pool_uses_responses_family_retry_budget_policy(original_uri: &Uri, method: &Method) -> bool {
    method == Method::POST
        && matches!(
            original_uri.path(),
            "/v1/responses" | "/v1/responses/compact"
        )
}

fn pool_same_account_attempt_budget(
    original_uri: &Uri,
    method: &Method,
    distinct_account_count: usize,
    initial_same_account_attempts: u8,
) -> u8 {
    if pool_uses_responses_family_retry_budget_policy(original_uri, method) {
        if distinct_account_count <= 1 {
            initial_same_account_attempts.max(1)
        } else {
            1
        }
    } else if distinct_account_count <= 1 {
        initial_same_account_attempts.max(1)
    } else {
        POOL_UPSTREAM_SAME_ACCOUNT_MAX_ATTEMPTS
    }
}

const POOL_RESPONSES_FAMILY_INITIAL_OVERLOAD_ATTEMPT_BUDGET: u8 = 4;

fn pool_overload_same_account_attempt_budget(
    original_uri: &Uri,
    method: &Method,
    distinct_account_count: usize,
    same_account_attempt_budget: u8,
) -> u8 {
    if pool_uses_responses_family_retry_budget_policy(original_uri, method)
        && distinct_account_count <= 1
    {
        same_account_attempt_budget.max(POOL_RESPONSES_FAMILY_INITIAL_OVERLOAD_ATTEMPT_BUDGET)
    } else {
        same_account_attempt_budget
    }
}

fn pool_error_message_indicates_proxy_timeout(message: &str) -> bool {
    let message_lower = message.trim().to_ascii_lowercase();
    message_lower.contains("request timed out after")
        || message_lower.contains("upstream handshake timed out after")
}

fn pool_failure_is_timeout_shaped(failure_kind: &str, message: &str) -> bool {
    matches!(
        failure_kind,
        PROXY_FAILURE_UPSTREAM_HANDSHAKE_TIMEOUT
            | PROXY_FAILURE_FAILED_CONTACT_UPSTREAM
            | PROXY_FAILURE_UPSTREAM_STREAM_ERROR
            | PROXY_FAILURE_UPSTREAM_RESPONSE_FAILED
    ) && pool_error_message_indicates_proxy_timeout(message)
}

fn pool_account_forward_proxy_scope(
    account: &PoolResolvedAccount,
) -> std::result::Result<ForwardProxyRouteScope, String> {
    Ok(account.forward_proxy_scope.clone())
}

async fn select_pool_account_forward_proxy_client(
    state: &AppState,
    account: &PoolResolvedAccount,
) -> Result<(ForwardProxyRouteScope, SelectedForwardProxy, Client), String> {
    let scope = pool_account_forward_proxy_scope(account)?;
    let selected_proxy = select_forward_proxy_for_scope(state, &scope)
        .await
        .map_err(|err| match &scope {
            ForwardProxyRouteScope::BoundGroup { group_name, .. }
                if err
                    .to_string()
                    .contains("bound forward proxy group has no selectable nodes") =>
            {
                format!(
                    "upstream account group \"{group_name}\" has no selectable bound forward proxy nodes"
                )
            }
            _ => format!("failed to select forward proxy node: {err}"),
        })?;
    let client = match state
        .http_clients
        .client_for_forward_proxy(selected_proxy.endpoint_url.as_ref())
    {
        Ok(client) => client,
        Err(err) => {
            record_forward_proxy_scope_result(
                state,
                &scope,
                &selected_proxy.key,
                ForwardProxyRouteResultKind::NetworkFailure,
            )
            .await;
            return Err(format!("failed to initialize forward proxy client: {err}"));
        }
    };
    Ok((scope, selected_proxy, client))
}

async fn record_pool_account_forward_proxy_result(
    state: &AppState,
    scope: &ForwardProxyRouteScope,
    selected_proxy: &SelectedForwardProxy,
    result: ForwardProxyRouteResultKind,
) {
    record_forward_proxy_scope_result(state, scope, &selected_proxy.key, result).await;
}

fn extract_sticky_key_from_request_body(value: &Value) -> Option<String> {
    const STICKY_KEY_POINTERS: &[&str] = &[
        "/metadata/sticky_key",
        "/metadata/stickyKey",
        "/metadata/prompt_cache_key",
        "/metadata/promptCacheKey",
        "/sticky_key",
        "/stickyKey",
        "/prompt_cache_key",
        "/promptCacheKey",
    ];

    for pointer in STICKY_KEY_POINTERS {
        if let Some(sticky_key) = value.pointer(pointer).and_then(|v| v.as_str()) {
            let normalized = sticky_key.trim();
            if !normalized.is_empty() {
                return Some(normalized.to_string());
            }
        }
    }
    None
}

fn extract_prompt_cache_key_from_request_body(value: &Value) -> Option<String> {
    for pointer in [
        "/metadata/prompt_cache_key",
        "/metadata/promptCacheKey",
        "/prompt_cache_key",
        "/promptCacheKey",
    ] {
        if let Some(prompt_cache_key) = value.pointer(pointer).and_then(|v| v.as_str()) {
            let normalized = prompt_cache_key.trim();
            if !normalized.is_empty() {
                return Some(normalized.to_string());
            }
        }
    }
    None
}

fn extract_requested_service_tier_from_request_body(value: &Value) -> Option<String> {
    ["/service_tier", "/serviceTier"]
        .iter()
        .find_map(|pointer| value.pointer(pointer).and_then(|entry| entry.as_str()))
        .and_then(normalize_service_tier)
}

fn rewrite_request_service_tier_for_fast_mode(
    value: &mut Value,
    fast_mode_rewrite_mode: TagFastModeRewriteMode,
) -> bool {
    let Some(object) = value.as_object_mut() else {
        return false;
    };

    match fast_mode_rewrite_mode {
        TagFastModeRewriteMode::KeepOriginal => false,
        TagFastModeRewriteMode::ForceRemove => {
            let removed_snake = object.remove("service_tier").is_some();
            let removed_camel = object.remove("serviceTier").is_some();
            removed_snake || removed_camel
        }
        TagFastModeRewriteMode::FillMissing => {
            let has_existing_service_tier =
                object.contains_key("service_tier") || object.contains_key("serviceTier");
            if has_existing_service_tier {
                false
            } else {
                object.insert(
                    "service_tier".to_string(),
                    Value::String("priority".to_string()),
                );
                true
            }
        }
        TagFastModeRewriteMode::ForceAdd => {
            let mut rewritten = object.remove("serviceTier").is_some();
            if object.get("service_tier").and_then(|entry| entry.as_str()) != Some("priority") {
                object.insert(
                    "service_tier".to_string(),
                    Value::String("priority".to_string()),
                );
                rewritten = true;
            }
            rewritten
        }
    }
}

fn extract_reasoning_effort_from_request_body(
    target: ProxyCaptureTarget,
    value: &Value,
) -> Option<String> {
    let raw = match target {
        ProxyCaptureTarget::Responses | ProxyCaptureTarget::ResponsesCompact => {
            value.pointer("/reasoning/effort").and_then(|v| v.as_str())
        }
        ProxyCaptureTarget::ChatCompletions => {
            value.get("reasoning_effort").and_then(|v| v.as_str())
        }
    }?;

    let normalized = raw.trim();
    if normalized.is_empty() {
        None
    } else {
        Some(normalized.to_string())
    }
}

fn build_response_capture_info_from_bytes(
    bytes: &[u8],
    request_is_stream: bool,
    decode_failure_reason: Option<String>,
) -> ResponseCaptureInfo {
    if bytes.is_empty() {
        return ResponseCaptureInfo {
            model: None,
            usage: ParsedUsage::default(),
            usage_missing_reason: Some("empty_response".to_string()),
            service_tier: None,
            stream_terminal_event: None,
            upstream_error_code: None,
            upstream_error_message: None,
            upstream_request_id: None,
        };
    }

    let looks_like_stream = request_is_stream || response_payload_looks_like_sse(bytes);
    let mut response_info = if looks_like_stream {
        parse_stream_response_payload(bytes)
    } else {
        match serde_json::from_slice::<Value>(bytes) {
            Ok(value) => {
                let model = extract_model_from_payload(&value);
                let usage = extract_usage_from_payload(&value).unwrap_or_default();
                let service_tier = extract_service_tier_from_payload(&value);
                let usage_missing_reason = if usage.total_tokens.is_none()
                    && usage.input_tokens.is_none()
                    && usage.output_tokens.is_none()
                {
                    Some("usage_missing_in_response".to_string())
                } else {
                    None
                };
                ResponseCaptureInfo {
                    model,
                    usage,
                    usage_missing_reason,
                    service_tier,
                    stream_terminal_event: None,
                    upstream_error_code: extract_upstream_error_code(&value),
                    upstream_error_message: extract_upstream_error_message(&value),
                    upstream_request_id: extract_upstream_request_id(&value),
                }
            }
            Err(_) => {
                let model = extract_partial_json_model(bytes);
                let service_tier = extract_partial_json_service_tier(bytes);
                let upstream_error_code = extract_partial_json_string_field(bytes, &["code"]);
                let upstream_error_message = extract_partial_json_string_field(bytes, &["message"]);
                let upstream_request_id =
                    extract_partial_json_string_field(bytes, &["request_id", "requestId"]).or_else(
                        || {
                            upstream_error_message
                                .as_deref()
                                .and_then(extract_request_id_from_message)
                        },
                    );
                ResponseCaptureInfo {
                    model,
                    usage: ParsedUsage::default(),
                    usage_missing_reason: Some("response_not_json".to_string()),
                    service_tier,
                    stream_terminal_event: None,
                    upstream_error_code,
                    upstream_error_message,
                    upstream_request_id,
                }
            }
        }
    };

    if let Some(reason) = decode_failure_reason {
        let combined_reason = if let Some(existing) = response_info.usage_missing_reason.take() {
            format!("response_decode_failed:{reason};{existing}")
        } else {
            format!("response_decode_failed:{reason}")
        };
        response_info.usage_missing_reason = Some(combined_reason);
    }

    response_info
}

fn parse_target_response_payload(
    _target: ProxyCaptureTarget,
    bytes: &[u8],
    request_is_stream: bool,
    content_encoding: Option<&str>,
) -> ResponseCaptureInfo {
    let (decoded_bytes, decode_failure_reason) =
        decode_response_payload_for_parse(bytes, content_encoding);
    build_response_capture_info_from_bytes(
        decoded_bytes.as_ref(),
        request_is_stream,
        decode_failure_reason,
    )
}
fn parse_target_response_preview_payload(
    _target: ProxyCaptureTarget,
    bytes: &[u8],
    request_is_stream: bool,
    content_encoding: Option<&str>,
) -> ResponseCaptureInfo {
    let (decoded_bytes, decode_failure_reason) =
        decode_response_payload_for_preview_parse(bytes, content_encoding);
    build_response_capture_info_from_bytes(
        decoded_bytes.as_ref(),
        request_is_stream,
        decode_failure_reason,
    )
}

fn response_payload_looks_like_sse(bytes: &[u8]) -> bool {
    String::from_utf8_lossy(bytes)
        .lines()
        .find_map(|line| {
            let trimmed = line.trim_start();
            if trimmed.is_empty() {
                None
            } else {
                Some(
                    trimmed.starts_with("data:")
                        || trimmed.starts_with("event:")
                        || trimmed.starts_with("id:")
                        || trimmed.starts_with("retry:"),
                )
            }
        })
        .unwrap_or(false)
}

fn response_payload_looks_like_sse_after_decode(
    bytes: &[u8],
    content_encoding: Option<&str>,
) -> bool {
    let (decoded, _) = decode_response_payload_for_preview_parse(bytes, content_encoding);
    response_payload_looks_like_sse(decoded.as_ref())
}

#[cfg(test)]
static RESPONSE_CAPTURE_RAW_SSE_HINT_FALLBACK_CALLS: AtomicUsize = AtomicUsize::new(0);
#[cfg(test)]
static RESPONSE_CAPTURE_RAW_PARSE_FALLBACK_CALLS: AtomicUsize = AtomicUsize::new(0);

#[cfg(test)]
fn reset_response_capture_raw_fallback_counters() {
    RESPONSE_CAPTURE_RAW_SSE_HINT_FALLBACK_CALLS.store(0, Ordering::Relaxed);
    RESPONSE_CAPTURE_RAW_PARSE_FALLBACK_CALLS.store(0, Ordering::Relaxed);
}

#[cfg(test)]
fn response_capture_raw_fallback_counts() -> (usize, usize) {
    (
        RESPONSE_CAPTURE_RAW_SSE_HINT_FALLBACK_CALLS.load(Ordering::Relaxed),
        RESPONSE_CAPTURE_RAW_PARSE_FALLBACK_CALLS.load(Ordering::Relaxed),
    )
}

#[allow(dead_code)]
fn response_payload_looks_like_sse_from_raw_file(
    path: &Path,
    content_encoding: Option<&str>,
) -> std::result::Result<bool, String> {
    let mut reader = open_decoded_response_reader(path, content_encoding)?;
    let mut decoded_prefix = Vec::new();
    reader
        .by_ref()
        .take((RAW_RESPONSE_PREVIEW_LIMIT + 1) as u64)
        .read_to_end(&mut decoded_prefix)
        .map_err(|err| err.to_string())?;
    Ok(response_payload_looks_like_sse(&decoded_prefix))
}

#[allow(dead_code)]
fn response_payload_looks_like_sse_from_capture(
    resp_raw: &RawPayloadMeta,
    preview_bytes: &[u8],
    content_encoding: Option<&str>,
) -> bool {
    #[cfg(test)]
    RESPONSE_CAPTURE_RAW_SSE_HINT_FALLBACK_CALLS.fetch_add(1, Ordering::Relaxed);

    if response_payload_looks_like_sse_after_decode(preview_bytes, content_encoding) {
        return true;
    }

    if preview_bytes.len() < RAW_RESPONSE_PREVIEW_LIMIT && content_encoding.is_none() {
        return false;
    }

    let Some(path) = resp_raw.path.as_deref() else {
        return false;
    };

    response_payload_looks_like_sse_from_raw_file(&PathBuf::from(path), content_encoding)
        .unwrap_or(false)
}

fn decode_response_payload_for_parse<'a>(
    bytes: &'a [u8],
    content_encoding: Option<&str>,
) -> (Cow<'a, [u8]>, Option<String>) {
    decode_response_payload(bytes, content_encoding, false)
}

fn decode_response_payload_for_preview_parse<'a>(
    bytes: &'a [u8],
    content_encoding: Option<&str>,
) -> (Cow<'a, [u8]>, Option<String>) {
    let encodings = parse_content_encodings(content_encoding);
    if encodings.is_empty() {
        return (Cow::Borrowed(bytes), None);
    }

    let mut decoded = bytes.to_vec();
    for encoding in encodings.iter().rev() {
        match decode_single_content_encoding_lossy(decoded.as_slice(), encoding) {
            Ok((next, None)) => decoded = next,
            Ok((next, Some(err))) => return (Cow::Owned(next), Some(format!("{encoding}:{err}"))),
            Err(err) => return (Cow::Borrowed(bytes), Some(format!("{encoding}:{err}"))),
        }
    }

    (Cow::Owned(decoded), None)
}

fn read_decoder_lossy(
    mut reader: impl Read,
) -> std::result::Result<(Vec<u8>, Option<String>), String> {
    let mut decoded = Vec::new();
    match reader.read_to_end(&mut decoded) {
        Ok(_) => Ok((decoded, None)),
        Err(err) if !decoded.is_empty() => Ok((decoded, Some(err.to_string()))),
        Err(err) => Err(err.to_string()),
    }
}

fn decode_single_content_encoding_lossy(
    bytes: &[u8],
    encoding: &str,
) -> std::result::Result<(Vec<u8>, Option<String>), String> {
    match encoding {
        "identity" => Ok((bytes.to_vec(), None)),
        "gzip" | "x-gzip" => read_decoder_lossy(GzDecoder::new(bytes)),
        "br" => read_decoder_lossy(BrotliDecompressor::new(bytes, 4096)),
        "deflate" => {
            let mut zlib_decoder = ZlibDecoder::new(bytes);
            let mut decoded = Vec::new();
            match zlib_decoder.read_to_end(&mut decoded) {
                Ok(_) => Ok((decoded, None)),
                Err(zlib_err) if !decoded.is_empty() => Ok((decoded, Some(zlib_err.to_string()))),
                Err(zlib_err) => {
                    let mut raw_decoder = DeflateDecoder::new(bytes);
                    let mut raw_decoded = Vec::new();
                    match raw_decoder.read_to_end(&mut raw_decoded) {
                        Ok(_) => Ok((raw_decoded, None)),
                        Err(raw_err) if !raw_decoded.is_empty() => {
                            Ok((raw_decoded, Some(raw_err.to_string())))
                        }
                        Err(raw_err) => Err(format!("zlib={zlib_err}; raw={raw_err}")),
                    }
                }
            }
        }
        other => Err(format!("unsupported_content_encoding:{other}")),
    }
}

#[derive(Default)]
struct StreamResponsePayloadParser {
    model: Option<String>,
    usage: ParsedUsage,
    service_tier: Option<String>,
    service_tier_rank: u8,
    stream_terminal_event: Option<String>,
    upstream_error_code: Option<String>,
    upstream_error_message: Option<String>,
    upstream_request_id: Option<String>,
    usage_found: bool,
    parse_error_seen: bool,
    pending_event_name: Option<String>,
    saw_stream_fields: bool,
}

impl StreamResponsePayloadParser {
    fn ingest_line(&mut self, line: &str) {
        let trimmed = line.trim();
        if trimmed.starts_with("event:") {
            self.saw_stream_fields = true;
            self.pending_event_name = Some(trimmed.trim_start_matches("event:").trim().to_string());
            return;
        }
        if !trimmed.starts_with("data:") {
            return;
        }
        self.saw_stream_fields = true;
        let payload = trimmed.trim_start_matches("data:").trim();
        if payload.is_empty() || payload == "[DONE]" {
            self.pending_event_name = None;
            return;
        }
        match serde_json::from_str::<Value>(payload) {
            Ok(value) => {
                let event_name = self.pending_event_name.take();
                if self.model.is_none() {
                    self.model = extract_model_from_payload(&value);
                }
                if let Some(service_tier) = extract_service_tier_from_payload(&value) {
                    let rank = stream_payload_service_tier_rank(event_name.as_deref(), &value);
                    if should_overwrite_stream_service_tier(
                        self.service_tier.as_deref(),
                        self.service_tier_rank,
                        &service_tier,
                        rank,
                    ) {
                        self.service_tier = Some(service_tier);
                        self.service_tier_rank = rank;
                    }
                }
                if let Some(parsed_usage) = extract_usage_from_payload(&value) {
                    self.usage = parsed_usage;
                    self.usage_found = true;
                }
                if stream_payload_indicates_failure(event_name.as_deref(), &value) {
                    let candidate = event_name
                        .clone()
                        .or_else(|| extract_stream_payload_type(&value))
                        .unwrap_or_else(|| "response.failed".to_string());
                    if self.stream_terminal_event.is_none() || candidate == "response.failed" {
                        self.stream_terminal_event = Some(candidate);
                    }
                }
                if self.upstream_error_code.is_none() {
                    self.upstream_error_code = extract_upstream_error_code(&value);
                }
                if self.upstream_error_message.is_none() {
                    self.upstream_error_message = extract_upstream_error_message(&value);
                }
                if self.upstream_request_id.is_none() {
                    self.upstream_request_id = extract_upstream_request_id(&value);
                }
            }
            Err(_) => {
                self.pending_event_name = None;
                self.parse_error_seen = true;
            }
        }
    }

    fn finish(self) -> ResponseCaptureInfo {
        let usage_missing_reason = if self.usage_found {
            None
        } else if self.stream_terminal_event.is_some() {
            Some(PROXY_FAILURE_UPSTREAM_RESPONSE_FAILED.to_string())
        } else if self.parse_error_seen {
            Some("stream_event_parse_error".to_string())
        } else {
            Some("usage_missing_in_stream".to_string())
        };

        ResponseCaptureInfo {
            model: self.model,
            usage: self.usage,
            usage_missing_reason,
            service_tier: self.service_tier,
            stream_terminal_event: self.stream_terminal_event,
            upstream_error_code: self.upstream_error_code,
            upstream_error_message: self.upstream_error_message,
            upstream_request_id: self.upstream_request_id,
        }
    }
}

struct StreamResponsePayloadParseOutcome {
    response_info: ResponseCaptureInfo,
    saw_stream_fields: bool,
}

struct StreamResponsePayloadChunkParser {
    parser: StreamResponsePayloadParser,
    line_buffer: Vec<u8>,
    discarding_oversized_line: bool,
    line_buffer_limit: usize,
    discarded_oversized_line: bool,
}

impl Default for StreamResponsePayloadChunkParser {
    fn default() -> Self {
        Self::with_line_buffer_limit(STREAM_RESPONSE_LINE_BUFFER_LIMIT)
    }
}

impl StreamResponsePayloadChunkParser {
    fn with_line_buffer_limit(line_buffer_limit: usize) -> Self {
        Self {
            parser: StreamResponsePayloadParser::default(),
            line_buffer: Vec::new(),
            discarding_oversized_line: false,
            line_buffer_limit,
            discarded_oversized_line: false,
        }
    }

    fn line_bytes_look_like_stream_field(line: &[u8]) -> bool {
        let decoded = String::from_utf8_lossy(line);
        let trimmed = decoded.trim_start();
        trimmed.starts_with("data:")
            || trimmed.starts_with("event:")
            || trimmed.starts_with("id:")
            || trimmed.starts_with("retry:")
    }

    fn flush_line(&mut self) {
        if self.line_buffer.is_empty() {
            return;
        }
        let decoded = String::from_utf8_lossy(&self.line_buffer);
        self.parser.ingest_line(decoded.as_ref());
        self.line_buffer.clear();
    }

    fn start_discarding_oversized_line(&mut self) {
        if Self::line_bytes_look_like_stream_field(&self.line_buffer) {
            self.parser.saw_stream_fields = true;
        }
        self.parser.parse_error_seen = true;
        self.discarded_oversized_line = true;
        self.line_buffer.clear();
        self.discarding_oversized_line = true;
    }

    fn append_segment(&mut self, segment: &[u8], ends_line: bool) {
        if self.discarding_oversized_line {
            if ends_line {
                self.discarding_oversized_line = false;
            }
            return;
        }

        if self.line_buffer.len().saturating_add(segment.len()) > self.line_buffer_limit {
            self.start_discarding_oversized_line();
            if ends_line {
                self.discarding_oversized_line = false;
            }
            return;
        }

        self.line_buffer.extend_from_slice(segment);
        if ends_line {
            self.flush_line();
        }
    }

    fn ingest_bytes(&mut self, bytes: &[u8]) {
        if bytes.is_empty() {
            return;
        }

        let mut line_start = 0usize;
        for (idx, byte) in bytes.iter().enumerate() {
            if *byte == b'\n' {
                self.append_segment(&bytes[line_start..=idx], true);
                line_start = idx + 1;
            }
        }
        if line_start < bytes.len() {
            self.append_segment(&bytes[line_start..], false);
        }
    }

    fn finish(mut self) -> StreamResponsePayloadParseOutcome {
        if self.discarding_oversized_line {
            self.parser.parse_error_seen = true;
        } else {
            self.flush_line();
        }
        StreamResponsePayloadParseOutcome {
            saw_stream_fields: self.parser.saw_stream_fields,
            response_info: self.parser.finish(),
        }
    }
}

fn parse_stream_response_payload(bytes: &[u8]) -> ResponseCaptureInfo {
    let mut parser = StreamResponsePayloadChunkParser::default();
    parser.ingest_bytes(bytes);
    parser.finish().response_info
}

#[allow(dead_code)]
fn parse_stream_response_payload_from_reader<R: Read>(
    reader: R,
) -> io::Result<ResponseCaptureInfo> {
    let mut parser = StreamResponsePayloadChunkParser::with_line_buffer_limit(
        RAW_FILE_STREAM_RESPONSE_LINE_BUFFER_LIMIT,
    );
    let mut reader = io::BufReader::new(reader);
    let mut chunk = [0_u8; 8192];
    loop {
        let read = reader.read(&mut chunk)?;
        if read == 0 {
            break;
        }
        parser.ingest_bytes(&chunk[..read]);
    }
    Ok(parser.finish().response_info)
}

fn extract_stream_payload_type(value: &Value) -> Option<String> {
    value
        .get("type")
        .and_then(|entry| entry.as_str())
        .map(|entry| entry.to_string())
}

fn stream_payload_service_tier_rank(event_name: Option<&str>, value: &Value) -> u8 {
    match event_name.or_else(|| value.get("type").and_then(|entry| entry.as_str())) {
        Some("response.completed" | "response.failed") => 2,
        Some("response.created" | "response.in_progress") => 1,
        _ => 0,
    }
}

fn should_overwrite_stream_service_tier(
    current_tier: Option<&str>,
    current_rank: u8,
    next_tier: &str,
    next_rank: u8,
) -> bool {
    let Some(current_tier) = current_tier else {
        return true;
    };

    if next_rank > current_rank {
        return true;
    }
    if next_rank < current_rank {
        return false;
    }

    match (current_tier, next_tier) {
        (AUTO_SERVICE_TIER, AUTO_SERVICE_TIER) => true,
        (AUTO_SERVICE_TIER, _) => true,
        (_, AUTO_SERVICE_TIER) => false,
        _ => true,
    }
}

fn stream_payload_indicates_failure(event_name: Option<&str>, value: &Value) -> bool {
    matches!(event_name, Some("response.failed") | Some("error"))
        || value
            .get("type")
            .and_then(|entry| entry.as_str())
            .is_some_and(|kind| kind == "response.failed" || kind == "error")
        || value
            .pointer("/response/status")
            .and_then(|entry| entry.as_str())
            .is_some_and(|status| status.eq_ignore_ascii_case("failed"))
}

fn extract_upstream_error_object(value: &Value) -> Option<&Value> {
    value
        .get("error")
        .filter(|entry| entry.is_object())
        .or_else(|| {
            value
                .pointer("/response/error")
                .filter(|entry| entry.is_object())
        })
}

fn extract_upstream_error_code(value: &Value) -> Option<String> {
    extract_upstream_error_object(value)
        .and_then(|entry| entry.get("code"))
        .and_then(|entry| entry.as_str())
        .map(|entry| entry.to_string())
        .or_else(|| {
            value
                .get("code")
                .and_then(|entry| entry.as_str())
                .map(|entry| entry.to_string())
        })
}

fn extract_upstream_error_message(value: &Value) -> Option<String> {
    extract_upstream_error_object(value)
        .and_then(|entry| entry.get("message"))
        .and_then(|entry| entry.as_str())
        .map(|entry| entry.to_string())
        .or_else(|| {
            value
                .get("message")
                .and_then(|entry| entry.as_str())
                .map(|entry| entry.to_string())
        })
}

fn extract_upstream_request_id(value: &Value) -> Option<String> {
    extract_upstream_error_object(value)
        .and_then(|entry| {
            entry
                .get("request_id")
                .or_else(|| entry.get("requestId"))
                .and_then(|value| value.as_str())
        })
        .map(|entry| entry.to_string())
        .or_else(|| {
            value
                .get("request_id")
                .or_else(|| value.get("requestId"))
                .and_then(|entry| entry.as_str())
                .map(|entry| entry.to_string())
        })
        .or_else(|| {
            extract_upstream_error_message(value)
                .and_then(|message| extract_request_id_from_message(&message))
        })
}

fn find_first_sse_event_boundary(bytes: &[u8]) -> Option<usize> {
    let mut index = 0usize;
    while index + 1 < bytes.len() {
        if bytes[index] == b'\n' && bytes[index + 1] == b'\n' {
            return Some(index + 2);
        }
        if index + 3 < bytes.len()
            && bytes[index] == b'\r'
            && bytes[index + 1] == b'\n'
            && bytes[index + 2] == b'\r'
            && bytes[index + 3] == b'\n'
        {
            return Some(index + 4);
        }
        index += 1;
    }
    None
}

fn initial_sse_event_kind(bytes: &[u8]) -> Option<String> {
    let mut event_name = None;
    let mut data_lines = Vec::new();
    for line in String::from_utf8_lossy(bytes).lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with(':') {
            continue;
        }
        if trimmed.starts_with("event:") {
            let candidate = trimmed.trim_start_matches("event:").trim();
            if !candidate.is_empty() {
                event_name = Some(candidate.to_string());
            }
            continue;
        }
        if trimmed.starts_with("data:") {
            let payload = trimmed.trim_start_matches("data:").trim();
            if !payload.is_empty() {
                data_lines.push(payload.to_string());
            }
        }
    }

    if data_lines.is_empty() {
        return event_name;
    }

    let payload = data_lines.join("\n");
    if payload == "[DONE]" {
        return Some("[DONE]".to_string());
    }

    serde_json::from_str::<Value>(&payload)
        .ok()
        .and_then(|value| event_name.clone().or_else(|| extract_stream_payload_type(&value)))
        .or(event_name)
}

fn build_retryable_overload_gate_outcome(
    upstream_error_code: Option<String>,
    upstream_error_message: Option<String>,
    upstream_request_id: Option<String>,
) -> PoolInitialResponseGateOutcome {
    let response_info = ResponseCaptureInfo {
        model: None,
        usage: ParsedUsage::default(),
        usage_missing_reason: Some(PROXY_FAILURE_UPSTREAM_RESPONSE_FAILED.to_string()),
        service_tier: None,
        stream_terminal_event: Some("response.failed".to_string()),
        upstream_error_code: upstream_error_code.clone(),
        upstream_error_message: upstream_error_message.clone(),
        upstream_request_id: upstream_request_id.clone(),
    };
    PoolInitialResponseGateOutcome::RetrySameAccount {
        message: format_upstream_response_failed_message(&response_info),
        upstream_error_code,
        upstream_error_message,
        upstream_request_id,
    }
}

enum PoolInitialResponsesSseEventDecision {
    ContinueMetadata,
    Forward,
    RetrySameAccount {
        upstream_error_code: Option<String>,
        upstream_error_message: Option<String>,
        upstream_request_id: Option<String>,
    },
}

fn classify_pool_initial_responses_sse_event(
    status: StatusCode,
    event_bytes: &[u8],
) -> PoolInitialResponsesSseEventDecision {
    let response_info = parse_stream_response_payload(event_bytes);
    if response_info_is_retryable_server_overloaded(status, &response_info) {
        return PoolInitialResponsesSseEventDecision::RetrySameAccount {
            upstream_error_code: response_info.upstream_error_code,
            upstream_error_message: response_info.upstream_error_message,
            upstream_request_id: response_info.upstream_request_id,
        };
    }

    match initial_sse_event_kind(event_bytes).as_deref() {
        None | Some("response.created" | "response.in_progress") => {
            PoolInitialResponsesSseEventDecision::ContinueMetadata
        }
        _ => PoolInitialResponsesSseEventDecision::Forward,
    }
}

fn rebuild_proxy_upstream_response_stream(
    status: StatusCode,
    headers: &HeaderMap,
    stream: Pin<Box<dyn futures_util::Stream<Item = Result<Bytes, io::Error>> + Send>>,
) -> Result<ProxyUpstreamResponseBody, String> {
    let mut response_builder = Response::builder().status(status);
    for (name, value) in headers {
        response_builder = response_builder.header(name, value);
    }
    response_builder
        .body(Body::from_stream(stream))
        .map(ProxyUpstreamResponseBody::Axum)
        .map_err(|err| format!("failed to rebuild upstream response stream: {err}"))
}

enum PoolInitialResponseGateOutcome {
    Forward {
        response: ProxyUpstreamResponseBody,
        prefetched_bytes: Option<Bytes>,
    },
    RetrySameAccount {
        message: String,
        upstream_error_code: Option<String>,
        upstream_error_message: Option<String>,
        upstream_request_id: Option<String>,
    },
}

async fn gate_pool_initial_response_stream(
    response: ProxyUpstreamResponseBody,
    prefetched_first_chunk: Option<Bytes>,
    total_timeout: Duration,
    started: Instant,
) -> Result<PoolInitialResponseGateOutcome, String> {
    let status = response.status();
    let headers = response.headers().clone();
    let mut stream = response.into_bytes_stream();
    let mut buffered = Vec::new();
    let mut scanned_bytes = 0usize;
    let mut saw_non_metadata_event = false;
    if let Some(chunk) = prefetched_first_chunk {
        buffered.extend_from_slice(&chunk);
    }

    let mut gate_stream_error: Option<io::Error> = None;
    loop {
        while let Some(relative_event_end) = find_first_sse_event_boundary(&buffered[scanned_bytes..]) {
            let event_end = scanned_bytes + relative_event_end;
            match classify_pool_initial_responses_sse_event(status, &buffered[scanned_bytes..event_end]) {
                PoolInitialResponsesSseEventDecision::ContinueMetadata => {
                    scanned_bytes = event_end;
                }
                PoolInitialResponsesSseEventDecision::Forward => {
                    scanned_bytes = event_end;
                    saw_non_metadata_event = true;
                    break;
                }
                PoolInitialResponsesSseEventDecision::RetrySameAccount {
                    upstream_error_code,
                    upstream_error_message,
                    upstream_request_id,
                } => {
                    return Ok(build_retryable_overload_gate_outcome(
                        upstream_error_code,
                        upstream_error_message,
                        upstream_request_id,
                    ));
                }
            }
        }
        if saw_non_metadata_event {
            break;
        }
        if buffered.len() >= RAW_RESPONSE_PREVIEW_LIMIT {
            break;
        }

        let Some(timeout_budget) = remaining_timeout_budget(total_timeout, started.elapsed())
        else {
            break;
        };
        let next_chunk = match timeout(timeout_budget, stream.next()).await {
            Ok(next_chunk) => next_chunk,
            Err(_) => break,
        };
        let Some(next_chunk) = next_chunk else {
            break;
        };
        match next_chunk {
            Ok(chunk) => buffered.extend_from_slice(&chunk),
            Err(err) => {
                gate_stream_error = Some(io::Error::other(err.to_string()));
                break;
            }
        }
    }

    let remaining_stream: Pin<
        Box<dyn futures_util::Stream<Item = Result<Bytes, io::Error>> + Send>,
    > = if let Some(err) = gate_stream_error {
        Box::pin(stream::once(async move { Err(err) }))
    } else {
        stream
    };
    let rebuilt_response =
        rebuild_proxy_upstream_response_stream(status, &headers, remaining_stream)?;
    Ok(PoolInitialResponseGateOutcome::Forward {
        response: rebuilt_response,
        prefetched_bytes: (!buffered.is_empty()).then_some(Bytes::from(buffered)),
    })
}

fn gate_pool_initial_compact_response(
    status: StatusCode,
    headers: &HeaderMap,
    prefetched_first_chunk: Option<&Bytes>,
) -> Option<PoolInitialResponseGateOutcome> {
    if status != StatusCode::OK {
        return None;
    }

    let first_chunk = prefetched_first_chunk?;
    if first_chunk.is_empty() {
        return None;
    }

    let content_encoding = headers
        .get(header::CONTENT_ENCODING)
        .and_then(|value| value.to_str().ok());
    let (decoded_first_chunk, _) =
        decode_response_payload_for_preview_parse(first_chunk.as_ref(), content_encoding);
    let value = serde_json::from_slice::<Value>(decoded_first_chunk.as_ref()).ok()?;
    let error_object = extract_upstream_error_object(&value)?;
    let upstream_error_code = error_object
        .get("code")
        .and_then(|entry| entry.as_str())
        .map(str::to_string);
    if !upstream_error_code_is_server_overloaded(upstream_error_code.as_deref()) {
        return None;
    }

    Some(build_retryable_overload_gate_outcome(
        upstream_error_code,
        extract_upstream_error_message(&value),
        extract_upstream_request_id(&value),
    ))
}

fn extract_request_id_from_message(message: &str) -> Option<String> {
    let lower_message = message.to_ascii_lowercase();
    let start = lower_message
        .find("request id ")
        .map(|index| index + "request id ".len())
        .or_else(|| {
            lower_message
                .find("request_id=")
                .map(|index| index + "request_id=".len())
        })
        .or_else(|| {
            lower_message
                .find("x-request-id: ")
                .map(|index| index + "x-request-id: ".len())
        })?;
    let tail = &message[start..];
    let request_id: String = tail
        .chars()
        .take_while(|ch| ch.is_ascii_alphanumeric() || matches!(*ch, '-' | '_'))
        .collect();
    if request_id.is_empty() {
        None
    } else {
        Some(request_id)
    }
}

fn decode_response_payload_for_usage<'a>(
    bytes: &'a [u8],
    content_encoding: Option<&str>,
) -> (Cow<'a, [u8]>, Option<String>) {
    decode_response_payload(bytes, content_encoding, true)
}

fn decode_response_payload<'a>(
    bytes: &'a [u8],
    content_encoding: Option<&str>,
    allow_gzip_magic_fallback: bool,
) -> (Cow<'a, [u8]>, Option<String>) {
    let encodings = parse_content_encodings(content_encoding);
    if encodings.is_empty() {
        if allow_gzip_magic_fallback && response_payload_looks_like_gzip_magic(bytes) {
            return decode_single_content_encoding(bytes, "gzip")
                .map(|decoded| (decoded, None))
                .unwrap_or_else(|err| {
                    (
                        Cow::Borrowed(bytes),
                        Some(format!("response_gzip_decode_error:{err}")),
                    )
                });
        }
        return (Cow::Borrowed(bytes), None);
    }

    let mut encodings = encodings.iter().rev();
    let first_encoding = encodings.next().expect("non-empty encodings checked above");
    let mut decoded = match decode_single_content_encoding(bytes, first_encoding) {
        Ok(next) => next.into_owned(),
        Err(err) => {
            return (
                Cow::Borrowed(bytes),
                Some(format!("{first_encoding}:{err}")),
            );
        }
    };
    for encoding in encodings {
        match decode_single_content_encoding(decoded.as_slice(), encoding) {
            Ok(next) => decoded = next.into_owned(),
            Err(err) => {
                return (Cow::Borrowed(bytes), Some(format!("{encoding}:{err}")));
            }
        }
    }
    (Cow::Owned(decoded), None)
}

fn parse_content_encodings(content_encoding: Option<&str>) -> Vec<String> {
    content_encoding
        .into_iter()
        .flat_map(|raw| raw.split(','))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_ascii_lowercase())
        .collect()
}

fn decode_single_content_encoding<'a>(
    bytes: &'a [u8],
    encoding: &str,
) -> std::result::Result<Cow<'a, [u8]>, String> {
    match encoding {
        "identity" => Ok(Cow::Borrowed(bytes)),
        "gzip" | "x-gzip" => decode_gzip_payload(bytes),
        "br" => decode_brotli_payload(bytes),
        "deflate" => decode_deflate_payload(bytes),
        other => Err(format!("unsupported_content_encoding:{other}")),
    }
}

fn decode_gzip_payload<'a>(bytes: &'a [u8]) -> std::result::Result<Cow<'a, [u8]>, String> {
    let mut decoder = GzDecoder::new(bytes);
    let mut decoded = Vec::new();
    decoder
        .read_to_end(&mut decoded)
        .map_err(|err| err.to_string())?;
    Ok(Cow::Owned(decoded))
}

fn decode_brotli_payload<'a>(bytes: &'a [u8]) -> std::result::Result<Cow<'a, [u8]>, String> {
    let mut decoder = BrotliDecompressor::new(bytes, 4096);
    let mut decoded = Vec::new();
    decoder
        .read_to_end(&mut decoded)
        .map_err(|err| err.to_string())?;
    Ok(Cow::Owned(decoded))
}

fn decode_deflate_payload<'a>(bytes: &'a [u8]) -> std::result::Result<Cow<'a, [u8]>, String> {
    let mut zlib_decoder = ZlibDecoder::new(bytes);
    let mut decoded = Vec::new();
    match zlib_decoder.read_to_end(&mut decoded) {
        Ok(_) => Ok(Cow::Owned(decoded)),
        Err(zlib_err) => {
            let mut raw_decoder = DeflateDecoder::new(bytes);
            let mut raw_decoded = Vec::new();
            raw_decoder
                .read_to_end(&mut raw_decoded)
                .map_err(|raw_err| format!("zlib={zlib_err}; raw={raw_err}"))?;
            Ok(Cow::Owned(raw_decoded))
        }
    }
}

fn response_payload_looks_like_gzip_magic(bytes: &[u8]) -> bool {
    bytes.len() >= 2 && bytes[0] == 0x1f && bytes[1] == 0x8b
}

fn extract_model_from_payload(value: &Value) -> Option<String> {
    value
        .get("model")
        .and_then(|v| v.as_str())
        .map(|v| v.to_string())
        .or_else(|| {
            value
                .pointer("/response/model")
                .and_then(|v| v.as_str())
                .map(|v| v.to_string())
        })
}

fn extract_partial_json_string_field(bytes: &[u8], keys: &[&str]) -> Option<String> {
    let text = std::str::from_utf8(bytes).ok()?;
    keys.iter().find_map(|key| {
        let pattern = format!(r#""{}"\s*:\s*"((?:\\.|[^"\\])*)""#, regex::escape(key));
        let regex = Regex::new(&pattern).ok()?;
        let captures = regex.captures(text)?;
        let value = captures.get(1)?.as_str();
        serde_json::from_str::<String>(&format!("\"{value}\"")).ok()
    })
}

fn extract_partial_json_model(bytes: &[u8]) -> Option<String> {
    extract_partial_json_string_field(bytes, &["model"])
}

fn extract_partial_json_service_tier(bytes: &[u8]) -> Option<String> {
    extract_partial_json_string_field(bytes, &["service_tier", "serviceTier"])
        .and_then(|value| normalize_service_tier(&value))
}

fn normalize_service_tier(value: &str) -> Option<String> {
    let normalized = value.trim().to_ascii_lowercase();
    if normalized.is_empty() {
        None
    } else {
        Some(normalized)
    }
}

const AUTO_SERVICE_TIER: &str = "auto";
const DEFAULT_SERVICE_TIER: &str = "default";
const PRIORITY_SERVICE_TIER: &str = "priority";
const API_KEYS_BILLING_ACCOUNT_KIND: &str = "api_key_codex";
const REQUESTED_TIER_PRICE_VERSION_SUFFIX: &str = "@requested-tier";
const RESPONSE_TIER_PRICE_VERSION_SUFFIX: &str = "@response-tier";
const EXPLICIT_BILLING_PRICE_VERSION_SUFFIX: &str = "@explicit-billing";
const SERVICE_TIER_STREAM_BACKFILL_VERSION: &str = "stream-terminal-v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProxyPricingMode {
    ResponseTier,
    RequestedTier,
    ExplicitBilling,
}

impl ProxyPricingMode {
    fn price_version_suffix(self) -> &'static str {
        match self {
            Self::ResponseTier => RESPONSE_TIER_PRICE_VERSION_SUFFIX,
            Self::RequestedTier => REQUESTED_TIER_PRICE_VERSION_SUFFIX,
            Self::ExplicitBilling => EXPLICIT_BILLING_PRICE_VERSION_SUFFIX,
        }
    }
}

fn normalize_upstream_base_url_host(raw: &str) -> Option<String> {
    Url::parse(raw)
        .ok()
        .and_then(|url| url.host_str().map(|host| host.trim().to_ascii_lowercase()))
        .or_else(|| {
            let host = raw.trim().trim_end_matches('/').to_ascii_lowercase();
            if host.is_empty() || host.contains('/') {
                None
            } else {
                Some(host)
            }
        })
        .filter(|host| !host.is_empty())
}

fn api_keys_billing_matches_context(upstream_account_kind: Option<&str>) -> bool {
    upstream_account_kind
        .map(str::trim)
        .is_some_and(|kind| kind.eq_ignore_ascii_case(API_KEYS_BILLING_ACCOUNT_KIND))
}

fn resolve_proxy_billing_service_tier_and_pricing_mode(
    explicit_billing_service_tier: Option<&str>,
    requested_service_tier: Option<&str>,
    response_service_tier: Option<&str>,
    upstream_account_kind: Option<&str>,
) -> (Option<String>, ProxyPricingMode) {
    if let Some(explicit_billing_service_tier) =
        explicit_billing_service_tier.and_then(normalize_service_tier)
    {
        return (
            Some(explicit_billing_service_tier),
            ProxyPricingMode::ExplicitBilling,
        );
    }

    let normalized_requested_service_tier = requested_service_tier.and_then(normalize_service_tier);
    let normalized_response_service_tier = response_service_tier.and_then(normalize_service_tier);
    if api_keys_billing_matches_context(upstream_account_kind)
        && normalized_requested_service_tier.is_some()
    {
        return (
            normalized_requested_service_tier,
            ProxyPricingMode::RequestedTier,
        );
    }

    (
        normalized_response_service_tier,
        ProxyPricingMode::ResponseTier,
    )
}

fn resolve_proxy_billing_service_tier_and_pricing_mode_for_account(
    explicit_billing_service_tier: Option<&str>,
    requested_service_tier: Option<&str>,
    response_service_tier: Option<&str>,
    account: Option<&PoolResolvedAccount>,
) -> (Option<String>, ProxyPricingMode) {
    resolve_proxy_billing_service_tier_and_pricing_mode(
        explicit_billing_service_tier,
        requested_service_tier,
        response_service_tier,
        account.map(|entry| entry.kind.as_str()),
    )
}

fn payload_summary_upstream_account_kind(account: Option<&PoolResolvedAccount>) -> Option<&str> {
    account.map(|entry| entry.kind.as_str())
}

fn payload_summary_upstream_base_url_host(account: Option<&PoolResolvedAccount>) -> Option<&str> {
    account.and_then(|entry| entry.upstream_base_url.host_str())
}

fn resolve_backfill_upstream_account_kind(
    snapshot_kind: Option<&str>,
    live_kind: Option<&str>,
    allow_live_fallback: bool,
) -> Option<String> {
    if let Some(value) = snapshot_kind.map(str::trim).filter(|value| !value.is_empty()) {
        return Some(value.to_string());
    }

    if !allow_live_fallback {
        return None;
    }

    live_kind
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn allow_live_upstream_account_fallback(raw: Option<i64>) -> bool {
    raw == Some(1)
}

fn resolve_backfill_upstream_base_url_host(
    snapshot_host: Option<&str>,
    live_host: Option<&str>,
    allow_live_fallback: bool,
) -> Option<String> {
    snapshot_host.and_then(normalize_upstream_base_url_host).or_else(|| {
        allow_live_fallback
            .then_some(live_host)
            .flatten()
            .and_then(normalize_upstream_base_url_host)
    })
}

fn extract_service_tier_from_payload(value: &Value) -> Option<String> {
    [
        "/service_tier",
        "/serviceTier",
        "/response/service_tier",
        "/response/serviceTier",
    ]
    .iter()
    .find_map(|pointer| value.pointer(pointer).and_then(|v| v.as_str()))
    .and_then(normalize_service_tier)
}

fn extract_usage_from_payload(value: &Value) -> Option<ParsedUsage> {
    if let Some(usage) = value.get("usage") {
        let parsed = parse_usage_value(usage);
        if parsed.total_tokens.is_some()
            || parsed.input_tokens.is_some()
            || parsed.output_tokens.is_some()
        {
            return Some(parsed);
        }
    }
    if let Some(usage) = value.pointer("/response/usage") {
        let parsed = parse_usage_value(usage);
        if parsed.total_tokens.is_some()
            || parsed.input_tokens.is_some()
            || parsed.output_tokens.is_some()
        {
            return Some(parsed);
        }
    }
    None
}

fn parse_usage_value(value: &Value) -> ParsedUsage {
    let input_tokens = value
        .get("input_tokens")
        .and_then(json_value_to_i64)
        .or_else(|| value.get("prompt_tokens").and_then(json_value_to_i64));
    let output_tokens = value
        .get("output_tokens")
        .and_then(json_value_to_i64)
        .or_else(|| value.get("completion_tokens").and_then(json_value_to_i64));
    let cache_input_tokens = value
        .pointer("/input_tokens_details/cached_tokens")
        .and_then(json_value_to_i64)
        .or_else(|| {
            value
                .pointer("/prompt_tokens_details/cached_tokens")
                .and_then(json_value_to_i64)
        });
    let reasoning_tokens = value
        .pointer("/output_tokens_details/reasoning_tokens")
        .and_then(json_value_to_i64)
        .or_else(|| {
            value
                .pointer("/completion_tokens_details/reasoning_tokens")
                .and_then(json_value_to_i64)
        });

    let mut parsed = ParsedUsage {
        input_tokens,
        output_tokens,
        cache_input_tokens,
        reasoning_tokens,
        total_tokens: value.get("total_tokens").and_then(json_value_to_i64),
    };

    if parsed.total_tokens.is_none() {
        parsed.total_tokens = match (parsed.input_tokens, parsed.output_tokens) {
            (Some(input), Some(output)) => Some(input + output),
            _ => None,
        };
    }

    parsed
}

fn json_value_to_i64(value: &Value) -> Option<i64> {
    if let Some(v) = value.as_i64() {
        return Some(v);
    }
    if let Some(v) = value.as_u64() {
        return i64::try_from(v).ok();
    }
    value.as_str().and_then(|v| v.parse::<i64>().ok())
}

fn upstream_account_id_from_payload(payload: Option<&str>) -> Option<i64> {
    let payload = payload?;
    let value = serde_json::from_str::<Value>(payload).ok()?;
    value.get("upstreamAccountId").and_then(json_value_to_i64)
}

fn upstream_account_name_from_payload(payload: Option<&str>) -> Option<String> {
    let payload = payload?;
    let value = serde_json::from_str::<Value>(payload).ok()?;
    value
        .get("upstreamAccountName")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn prompt_cache_key_from_payload(payload: Option<&str>) -> Option<String> {
    let payload = payload?;
    let value = serde_json::from_str::<Value>(payload).ok()?;
    value
        .get("promptCacheKey")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn sticky_key_from_payload(payload: Option<&str>) -> Option<String> {
    let payload = payload?;
    let value = serde_json::from_str::<Value>(payload).ok()?;
    value
        .get("stickyKey")
        .or_else(|| value.get("promptCacheKey"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn shanghai_now_string() -> String {
    format_naive(Utc::now().with_timezone(&Shanghai).naive_local())
}

fn terminal_pool_upstream_request_attempt_phase(status: &str) -> &'static str {
    if status == POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS {
        POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_COMPLETED
    } else {
        POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_FAILED
    }
}

async fn insert_pool_upstream_request_attempt(
    pool: &Pool<Sqlite>,
    trace: &PoolUpstreamAttemptTraceContext,
    upstream_account_id: Option<i64>,
    upstream_route_key: Option<&str>,
    attempt_index: i64,
    distinct_account_index: i64,
    same_account_retry_index: i64,
    started_at: Option<&str>,
    finished_at: Option<&str>,
    status: &str,
    phase: Option<&str>,
    http_status: Option<StatusCode>,
    failure_kind: Option<&str>,
    error_message: Option<&str>,
    connect_latency_ms: Option<f64>,
    first_byte_latency_ms: Option<f64>,
    stream_latency_ms: Option<f64>,
    upstream_request_id: Option<&str>,
    compact_support_status: Option<&str>,
    compact_support_reason: Option<&str>,
) -> Result<i64> {
    let result = sqlx::query(
        r#"
        INSERT INTO pool_upstream_request_attempts (
            invoke_id,
            occurred_at,
            endpoint,
            route_mode,
            sticky_key,
            upstream_account_id,
            upstream_route_key,
            attempt_index,
            distinct_account_index,
            same_account_retry_index,
            requester_ip,
            started_at,
            finished_at,
            status,
            phase,
            http_status,
            failure_kind,
            error_message,
            connect_latency_ms,
            first_byte_latency_ms,
            stream_latency_ms,
            upstream_request_id,
            compact_support_status,
            compact_support_reason
        )
        VALUES (
            ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24
        )
        "#,
    )
    .bind(&trace.invoke_id)
    .bind(&trace.occurred_at)
    .bind(&trace.endpoint)
    .bind(INVOCATION_ROUTE_MODE_POOL)
    .bind(trace.sticky_key.as_deref())
    .bind(upstream_account_id)
    .bind(upstream_route_key)
    .bind(attempt_index)
    .bind(distinct_account_index)
    .bind(same_account_retry_index)
    .bind(trace.requester_ip.as_deref())
    .bind(started_at)
    .bind(finished_at)
    .bind(status)
    .bind(phase)
    .bind(http_status.map(|value| i64::from(value.as_u16())))
    .bind(failure_kind)
    .bind(error_message)
    .bind(connect_latency_ms)
    .bind(first_byte_latency_ms)
    .bind(stream_latency_ms)
    .bind(upstream_request_id)
    .bind(compact_support_status)
    .bind(compact_support_reason)
    .execute(pool)
    .await?;
    Ok(result.last_insert_rowid())
}

async fn begin_pool_upstream_request_attempt(
    pool: &Pool<Sqlite>,
    trace: &PoolUpstreamAttemptTraceContext,
    upstream_account_id: i64,
    upstream_route_key: &str,
    attempt_index: i64,
    distinct_account_index: i64,
    same_account_retry_index: i64,
    started_at: &str,
) -> PendingPoolAttemptRecord {
    let attempt_id = match insert_pool_upstream_request_attempt(
        pool,
        trace,
        Some(upstream_account_id),
        Some(upstream_route_key),
        attempt_index,
        distinct_account_index,
        same_account_retry_index,
        Some(started_at),
        None,
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_PENDING,
        Some(POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_CONNECTING),
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
    )
    .await
    {
        Ok(attempt_id) => Some(attempt_id),
        Err(err) => {
            warn!(
                invoke_id = trace.invoke_id,
                error = %err,
                "failed to persist pending pool attempt"
            );
            None
        }
    };

    PendingPoolAttemptRecord {
        attempt_id,
        invoke_id: trace.invoke_id.clone(),
        occurred_at: trace.occurred_at.clone(),
        endpoint: trace.endpoint.clone(),
        sticky_key: trace.sticky_key.clone(),
        requester_ip: trace.requester_ip.clone(),
        upstream_account_id,
        upstream_route_key: upstream_route_key.to_string(),
        attempt_index,
        distinct_account_index,
        same_account_retry_index,
        started_at: started_at.to_string(),
        connect_latency_ms: 0.0,
        first_byte_latency_ms: 0.0,
        compact_support_status: None,
        compact_support_reason: None,
    }
}

async fn update_pool_upstream_request_attempt_phase(
    pool: &Pool<Sqlite>,
    pending: &PendingPoolAttemptRecord,
    phase: &str,
) -> Result<bool> {
    let Some(attempt_id) = pending.attempt_id else {
        return Ok(false);
    };

    let result = sqlx::query(
        r#"
        UPDATE pool_upstream_request_attempts
        SET phase = ?2
        WHERE id = ?1
          AND COALESCE(phase, '') <> ?2
        "#,
    )
    .bind(attempt_id)
    .bind(phase)
    .execute(pool)
    .await?;

    Ok(result.rows_affected() > 0)
}

async fn advance_pool_upstream_request_attempt_phase(
    state: &AppState,
    pending: &PendingPoolAttemptRecord,
    phase: &str,
) -> Result<()> {
    if !update_pool_upstream_request_attempt_phase(&state.pool, pending, phase).await? {
        return Ok(());
    }

    broadcast_pool_upstream_attempts_snapshot(state, &pending.invoke_id).await
}

enum PoolAttemptRecoveryScope<'a> {
    AllPending,
    SpecificEarlyPhase { attempt_id: i64 },
    StaleEarlyPhase {
        responses_started_before: &'a str,
        compact_started_before: &'a str,
        default_started_before: &'a str,
    },
}

async fn recover_pool_upstream_request_attempts_with_scope(
    pool: &Pool<Sqlite>,
    scope: PoolAttemptRecoveryScope<'_>,
) -> Result<Vec<RecoveredPoolAttemptRow>> {
    let finished_at = shanghai_now_string();
    let recovered = match scope {
        PoolAttemptRecoveryScope::AllPending => {
            sqlx::query_as::<_, RecoveredPoolAttemptRow>(
                r#"
                UPDATE pool_upstream_request_attempts
                SET
                    finished_at = COALESCE(finished_at, ?1),
                    status = ?2,
                    phase = ?3,
                    failure_kind = COALESCE(failure_kind, ?4),
                    error_message = COALESCE(error_message, ?5)
                WHERE status = ?6
                  AND finished_at IS NULL
                  AND NOT EXISTS (
                        SELECT 1
                        FROM codex_invocations inv
                        WHERE inv.source = ?7
                          AND inv.invoke_id = pool_upstream_request_attempts.invoke_id
                          AND inv.occurred_at = pool_upstream_request_attempts.occurred_at
                          AND LOWER(TRIM(COALESCE(inv.status, ''))) NOT IN ('running', 'pending')
                  )
                RETURNING id, invoke_id, occurred_at
                "#,
            )
            .bind(finished_at)
            .bind(POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_TRANSPORT_FAILURE)
            .bind(POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_FAILED)
            .bind(PROXY_FAILURE_POOL_ATTEMPT_INTERRUPTED)
            .bind(POOL_ATTEMPT_INTERRUPTED_MESSAGE)
            .bind(POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_PENDING)
            .bind(SOURCE_PROXY)
            .fetch_all(pool)
            .await?
        }
        PoolAttemptRecoveryScope::SpecificEarlyPhase { attempt_id } => {
            sqlx::query_as::<_, RecoveredPoolAttemptRow>(
                r#"
                UPDATE pool_upstream_request_attempts
                SET
                    finished_at = COALESCE(finished_at, ?1),
                    status = ?2,
                    phase = ?3,
                    failure_kind = COALESCE(failure_kind, ?4),
                    error_message = COALESCE(error_message, ?5)
                WHERE id = ?6
                  AND status = ?7
                  AND finished_at IS NULL
                  AND LOWER(TRIM(COALESCE(phase, ''))) IN ('connecting', 'sending_request', 'waiting_first_byte')
                  AND NOT EXISTS (
                        SELECT 1
                        FROM codex_invocations inv
                        WHERE inv.source = ?8
                          AND inv.invoke_id = pool_upstream_request_attempts.invoke_id
                          AND inv.occurred_at = pool_upstream_request_attempts.occurred_at
                          AND LOWER(TRIM(COALESCE(inv.status, ''))) NOT IN ('running', 'pending')
                  )
                RETURNING id, invoke_id, occurred_at
                "#,
            )
            .bind(finished_at)
            .bind(POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_TRANSPORT_FAILURE)
            .bind(POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_FAILED)
            .bind(PROXY_FAILURE_POOL_ATTEMPT_INTERRUPTED)
            .bind(POOL_ATTEMPT_INTERRUPTED_MESSAGE)
            .bind(attempt_id)
            .bind(POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_PENDING)
            .bind(SOURCE_PROXY)
            .fetch_all(pool)
            .await?
        }
        PoolAttemptRecoveryScope::StaleEarlyPhase {
            responses_started_before,
            compact_started_before,
            default_started_before,
        } => {
            let candidates = sqlx::query_as::<_, RecoveredPoolAttemptRow>(
                r#"
                SELECT id, invoke_id, occurred_at
                FROM pool_upstream_request_attempts
                WHERE status = ?1
                  AND finished_at IS NULL
                  AND LOWER(TRIM(COALESCE(phase, ''))) IN ('connecting', 'sending_request', 'waiting_first_byte')
                  AND NOT EXISTS (
                        SELECT 1
                        FROM codex_invocations inv
                        WHERE inv.source = ?2
                          AND inv.invoke_id = pool_upstream_request_attempts.invoke_id
                          AND inv.occurred_at = pool_upstream_request_attempts.occurred_at
                          AND LOWER(TRIM(COALESCE(inv.status, ''))) NOT IN ('running', 'pending')
                  )
                  AND NOT EXISTS (
                        SELECT 1
                        FROM codex_invocations inv
                        WHERE inv.source = ?2
                          AND inv.invoke_id = pool_upstream_request_attempts.invoke_id
                          AND inv.occurred_at = pool_upstream_request_attempts.occurred_at
                          AND COALESCE(inv.t_upstream_ttfb_ms, 0) > 0
                  )
                  AND (
                        started_at IS NULL
                        OR (
                            endpoint = '/v1/responses'
                            AND started_at <= ?3
                        )
                        OR (
                            endpoint = '/v1/responses/compact'
                            AND started_at <= ?4
                        )
                        OR (
                            COALESCE(endpoint, '') NOT IN ('/v1/responses', '/v1/responses/compact')
                            AND started_at <= ?5
                        )
                  )
                "#,
            )
            .bind(POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_PENDING)
            .bind(SOURCE_PROXY)
            .bind(responses_started_before)
            .bind(compact_started_before)
            .bind(default_started_before)
            .fetch_all(pool)
            .await?;
            if candidates.is_empty() {
                Vec::new()
            } else {
                let candidate_ids = candidates.iter().map(|row| row.id).collect::<Vec<_>>();
                let mut query = QueryBuilder::<Sqlite>::new(
                    r#"
                    UPDATE pool_upstream_request_attempts
                    SET
                        finished_at = COALESCE(finished_at, "#,
                );
                query.push_bind(&finished_at);
                query.push(
                    r#"),
                        status = "#,
                );
                query.push_bind(POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_TRANSPORT_FAILURE);
                query.push(
                    r#",
                        phase = "#,
                );
                query.push_bind(POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_FAILED);
                query.push(
                    r#",
                        failure_kind = COALESCE(failure_kind, "#,
                );
                query.push_bind(PROXY_FAILURE_POOL_ATTEMPT_INTERRUPTED);
                query.push(
                    r#"),
                        error_message = COALESCE(error_message, "#,
                );
                query.push_bind(POOL_ATTEMPT_INTERRUPTED_MESSAGE);
                query.push(
                    r#")
                    WHERE id IN ("#,
                );
                let mut separated = query.separated(", ");
                for id in candidate_ids {
                    separated.push_bind(id);
                }
                separated.push_unseparated(")");
                query.push(
                    r#"
                    RETURNING id, invoke_id, occurred_at
                    "#,
                );
                query
                    .build_query_as::<RecoveredPoolAttemptRow>()
                    .fetch_all(pool)
                    .await?
            }
        }
    };

    Ok(recovered)
}

async fn recover_orphaned_pool_upstream_request_attempts(pool: &Pool<Sqlite>) -> Result<u64> {
    Ok(
        recover_pool_upstream_request_attempts_with_scope(pool, PoolAttemptRecoveryScope::AllPending)
            .await?
            .len() as u64,
    )
}

enum ProxyInvocationRecoveryScope<'a> {
    AllInFlight,
    Selectors(&'a [InvocationRecoverySelector]),
}

async fn recover_proxy_invocations_with_scope(
    pool: &Pool<Sqlite>,
    scope: ProxyInvocationRecoveryScope<'_>,
) -> Result<Vec<RecoveredInvocationRow>> {
    let mut tx = pool.begin().await?;
    let rows = match scope {
        ProxyInvocationRecoveryScope::AllInFlight => {
            sqlx::query_as::<_, RecoveredInvocationRow>(
                r#"
                UPDATE codex_invocations
                SET status = ?1,
                    error_message = ?2,
                    failure_kind = ?3,
                    failure_class = ?4,
                    is_actionable = 1
                WHERE source = ?5
                  AND LOWER(TRIM(COALESCE(status, ''))) IN ('running', 'pending')
                RETURNING id, invoke_id, occurred_at
                "#,
            )
            .bind(INVOCATION_STATUS_INTERRUPTED)
            .bind(INVOCATION_INTERRUPTED_MESSAGE)
            .bind(PROXY_FAILURE_INVOCATION_INTERRUPTED)
            .bind(FAILURE_CLASS_SERVICE)
            .bind(SOURCE_PROXY)
            .fetch_all(tx.as_mut())
            .await?
        }
        ProxyInvocationRecoveryScope::Selectors(selectors) => {
            let selectors: Vec<_> = selectors
                .iter()
                .cloned()
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect();
            if selectors.is_empty() {
                tx.commit().await?;
                return Ok(Vec::new());
            }

            let mut recovered = Vec::new();
            for chunk in selectors.chunks(PROXY_INVOCATION_RECOVERY_SELECTOR_BATCH_SIZE) {
                let mut query = QueryBuilder::<Sqlite>::new(
                    r#"
                    UPDATE codex_invocations
                    SET status = "#,
                );
                query.push_bind(INVOCATION_STATUS_INTERRUPTED);
                query.push(
                    r#",
                        error_message = "#,
                );
                query.push_bind(INVOCATION_INTERRUPTED_MESSAGE);
                query.push(
                    r#",
                        failure_kind = "#,
                );
                query.push_bind(PROXY_FAILURE_INVOCATION_INTERRUPTED);
                query.push(
                    r#",
                        failure_class = "#,
                );
                query.push_bind(FAILURE_CLASS_SERVICE);
                query.push(
                    r#",
                        is_actionable = 1
                    WHERE source = "#,
                );
                query.push_bind(SOURCE_PROXY);
                query.push(
                    r#"
                      AND LOWER(TRIM(COALESCE(status, ''))) IN ('running', 'pending')
                      AND (
                    "#,
                );
                let mut first = true;
                for selector in chunk {
                    if !first {
                        query.push(" OR ");
                    }
                    first = false;
                    query.push("(");
                    query.push("invoke_id = ");
                    query.push_bind(&selector.invoke_id);
                    query.push(" AND occurred_at = ");
                    query.push_bind(&selector.occurred_at);
                    query.push(")");
                }
                query.push(
                    r#"
                      )
                    RETURNING id, invoke_id, occurred_at
                    "#,
                );
                recovered.extend(
                    query
                        .build_query_as::<RecoveredInvocationRow>()
                        .fetch_all(tx.as_mut())
                        .await?,
                );
            }
            recovered
        }
    };

    if !rows.is_empty() {
        let updated_ids: Vec<i64> = rows.iter().map(|row| row.id).collect();
        recompute_invocation_hourly_rollups_for_ids_tx(tx.as_mut(), &updated_ids).await?;
        if let Some(max_id) = updated_ids.iter().copied().max() {
            save_hourly_rollup_live_progress_tx(
                tx.as_mut(),
                HOURLY_ROLLUP_DATASET_INVOCATIONS,
                max_id,
            )
            .await?;
        }
    }

    tx.commit().await?;
    Ok(rows)
}

async fn recover_orphaned_proxy_invocations(pool: &Pool<Sqlite>) -> Result<u64> {
    Ok(
        recover_proxy_invocations_with_scope(pool, ProxyInvocationRecoveryScope::AllInFlight)
            .await?
            .len() as u64,
    )
}

fn stale_started_before_string(timeout: Duration, grace: Duration) -> String {
    let cutoff = Utc::now().with_timezone(&Shanghai).naive_local()
        - ChronoDuration::from_std(timeout + grace)
            .expect("pool orphan recovery cutoff should fit chrono duration");
    format_naive(cutoff)
}

async fn load_persisted_api_invocation(
    pool: &Pool<Sqlite>,
    invoke_id: &str,
    occurred_at: &str,
) -> Result<ApiInvocation> {
    let mut tx = pool.begin().await?;
    let invocation = load_persisted_api_invocation_tx(tx.as_mut(), invoke_id, occurred_at).await?;
    tx.commit().await?;
    Ok(invocation)
}

async fn broadcast_recovered_proxy_invocations(
    state: &AppState,
    recovered: &[RecoveredInvocationRow],
) -> Result<()> {
    if recovered.is_empty() {
        return Ok(());
    }

    let selectors: Vec<_> = recovered
        .iter()
        .map(|row| InvocationRecoverySelector::new(row.invoke_id.clone(), row.occurred_at.clone()))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let mut records = Vec::new();
    for selector in selectors {
        match load_persisted_api_invocation(&state.pool, &selector.invoke_id, &selector.occurred_at)
            .await
        {
            Ok(record) => records.push(record),
            Err(err) => {
                warn!(
                    invoke_id = %selector.invoke_id,
                    occurred_at = %selector.occurred_at,
                    error = %err,
                    "failed to load recovered proxy invocation for runtime broadcast"
                );
            }
        }
    }

    if records.is_empty() {
        return Ok(());
    }

    if records.iter().any(|record| {
        record
            .prompt_cache_key
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty())
    }) {
        invalidate_prompt_cache_conversations_cache(&state.prompt_cache_conversation_cache).await;
    }

    if state.broadcaster.receiver_count() == 0 {
        return Ok(());
    }

    let summary_invoke_id = records[0].invoke_id.clone();
    state
        .broadcaster
        .send(BroadcastPayload::Records { records })
        .map_err(|err| anyhow!("failed to broadcast recovered proxy invocation records: {err}"))?;
    broadcast_proxy_capture_follow_up(
        &state.pool,
        &state.broadcaster,
        state.broadcast_state_cache.as_ref(),
        state.config.crs_stats.as_ref(),
        state.config.invocation_max_days,
        &summary_invoke_id,
    )
    .await;

    Ok(())
}

async fn recover_guard_dropped_pool_early_phase_orphan(
    state: &AppState,
    pending_attempt_record: PendingPoolAttemptRecord,
) -> Result<()> {
    let recovered_attempts = match pending_attempt_record.attempt_id {
        Some(attempt_id) => {
            recover_pool_upstream_request_attempts_with_scope(
                &state.pool,
                PoolAttemptRecoveryScope::SpecificEarlyPhase { attempt_id },
            )
            .await?
        }
        None => Vec::new(),
    };

    let skip_invocation_recovery = if pending_attempt_record.attempt_id.is_some()
        && recovered_attempts.is_empty()
    {
        let current_phase = sqlx::query_as::<_, (Option<String>,)>(
            r#"
            SELECT phase
            FROM pool_upstream_request_attempts
            WHERE id = ?1
            LIMIT 1
            "#,
        )
        .bind(
            pending_attempt_record
                .attempt_id
                .expect("attempt id checked above"),
        )
        .fetch_optional(&state.pool)
        .await?
        .and_then(|row| row.0);
        current_phase.as_deref().is_some_and(|phase| {
            phase
                .trim()
                .eq_ignore_ascii_case(POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_STREAMING_RESPONSE)
        })
    } else {
        false
    };

    let recovered_invocations = if skip_invocation_recovery {
        info!(
            invoke_id = %pending_attempt_record.invoke_id,
            attempt_id = pending_attempt_record.attempt_id,
            recovery_trigger = "drop_guard",
            "skipping guard-based invocation recovery because attempt already entered streaming_response"
        );
        Vec::new()
    } else {
        let selector = InvocationRecoverySelector::from(&pending_attempt_record);
        recover_proxy_invocations_with_scope(
            &state.pool,
            ProxyInvocationRecoveryScope::Selectors(std::slice::from_ref(&selector)),
        )
        .await?
    };

    if recovered_attempts.is_empty() && recovered_invocations.is_empty() {
        return Ok(());
    }

    if !recovered_attempts.is_empty()
        && let Err(err) = broadcast_pool_upstream_attempts_snapshot(state, &pending_attempt_record.invoke_id).await
    {
        warn!(
            invoke_id = %pending_attempt_record.invoke_id,
            error = %err,
            "failed to broadcast guard-recovered pool attempt snapshot"
        );
    }
    broadcast_recovered_proxy_invocations(state, &recovered_invocations).await?;

    info!(
        invoke_id = %pending_attempt_record.invoke_id,
        attempt_id = pending_attempt_record.attempt_id,
        recovered_attempts = recovered_attempts.len(),
        recovered_invocations = recovered_invocations.len(),
        recovery_trigger = "drop_guard",
        "recovered pool early-phase orphan after request future dropped"
    );

    Ok(())
}

pub(crate) async fn recover_stale_pool_early_phase_orphans_runtime(
    state: &AppState,
) -> Result<PoolOrphanRecoveryOutcome> {
    let timeouts = resolve_pool_routing_timeouts(&state.pool, &state.config).await?;
    let responses_started_before = stale_started_before_string(
        timeouts.responses_first_byte_timeout,
        POOL_EARLY_PHASE_ORPHAN_RECOVERY_GRACE,
    );
    let compact_started_before = stale_started_before_string(
        timeouts.compact_first_byte_timeout,
        POOL_EARLY_PHASE_ORPHAN_RECOVERY_GRACE,
    );
    let default_started_before = stale_started_before_string(
        timeouts.default_first_byte_timeout,
        POOL_EARLY_PHASE_ORPHAN_RECOVERY_GRACE,
    );
    let recovered_attempts = recover_pool_upstream_request_attempts_with_scope(
        &state.pool,
        PoolAttemptRecoveryScope::StaleEarlyPhase {
            responses_started_before: &responses_started_before,
            compact_started_before: &compact_started_before,
            default_started_before: &default_started_before,
        },
    )
    .await?;
    if recovered_attempts.is_empty() {
        return Ok(PoolOrphanRecoveryOutcome::default());
    }

    let selectors: Vec<_> = recovered_attempts
        .iter()
        .map(|row| InvocationRecoverySelector::new(row.invoke_id.clone(), row.occurred_at.clone()))
        .collect();
    let recovered_invocations = recover_proxy_invocations_with_scope(
        &state.pool,
        ProxyInvocationRecoveryScope::Selectors(&selectors),
    )
    .await?;

    for invoke_id in recovered_attempts
        .iter()
        .map(|row| row.invoke_id.as_str())
        .collect::<BTreeSet<_>>()
    {
        if let Err(err) = broadcast_pool_upstream_attempts_snapshot(state, invoke_id).await {
            warn!(
                invoke_id,
                error = %err,
                "failed to broadcast stale pool orphan recovery snapshot"
            );
        }
    }
    broadcast_recovered_proxy_invocations(state, &recovered_invocations).await?;

    let outcome = PoolOrphanRecoveryOutcome {
        recovered_attempts: recovered_attempts.len(),
        recovered_invocations: recovered_invocations.len(),
    };
    info!(
        recovered_attempts = outcome.recovered_attempts,
        recovered_invocations = outcome.recovered_invocations,
        recovery_trigger = "runtime_sweeper",
        "recovered stale pool early-phase orphans at runtime"
    );

    Ok(outcome)
}

async fn broadcast_pool_upstream_attempts_snapshot(
    state: &AppState,
    invoke_id: &str,
) -> Result<()> {
    if state.broadcaster.receiver_count() == 0 {
        return Ok(());
    }

    let attempts = query_pool_attempt_records_from_live(&state.pool, invoke_id)
        .await
        .map_err(|err| anyhow!("failed to load live pool attempts for SSE broadcast: {err:?}"))?;
    state
        .broadcaster
        .send(BroadcastPayload::PoolAttempts {
            invoke_id: invoke_id.to_string(),
            attempts,
        })
        .map_err(|err| anyhow!("failed to broadcast pool attempts snapshot: {err}"))?;
    Ok(())
}

async fn broadcast_pool_attempt_started_runtime_snapshot(
    state: &AppState,
    trace: &PoolUpstreamAttemptTraceContext,
    runtime_snapshot: &PoolAttemptRuntimeSnapshotContext,
    account: &PoolResolvedAccount,
    attempt_count: usize,
    distinct_account_count: usize,
) {
    let running_record = build_running_proxy_capture_record(
        &trace.invoke_id,
        &trace.occurred_at,
        runtime_snapshot.capture_target,
        &runtime_snapshot.request_info,
        trace.requester_ip.as_deref(),
        trace.sticky_key.as_deref(),
        runtime_snapshot.prompt_cache_key.as_deref(),
        true,
        Some(account.account_id),
        Some(account.display_name.as_str()),
        payload_summary_upstream_account_kind(Some(account)),
        payload_summary_upstream_base_url_host(Some(account)),
        None,
        Some(attempt_count),
        Some(distinct_account_count),
        None,
        None,
        runtime_snapshot.t_req_read_ms,
        runtime_snapshot.t_req_parse_ms,
        0.0,
        0.0,
    );
    if let Err(err) =
        persist_and_broadcast_proxy_capture_runtime_snapshot(state, running_record).await
    {
        warn!(
            ?err,
            invoke_id = %trace.invoke_id,
            "failed to broadcast pool attempt start runtime snapshot"
        );
    }
    if let Err(err) = broadcast_pool_upstream_attempts_snapshot(state, &trace.invoke_id).await {
        warn!(
            invoke_id = %trace.invoke_id,
            error = %err,
            "failed to broadcast pool attempt start snapshot"
        );
    }
}

async fn finalize_pool_upstream_request_attempt(
    pool: &Pool<Sqlite>,
    pending: &PendingPoolAttemptRecord,
    finished_at: &str,
    status: &str,
    http_status: Option<StatusCode>,
    failure_kind: Option<&str>,
    error_message: Option<&str>,
    connect_latency_ms: Option<f64>,
    first_byte_latency_ms: Option<f64>,
    stream_latency_ms: Option<f64>,
    upstream_request_id: Option<&str>,
    compact_support_status: Option<&str>,
    compact_support_reason: Option<&str>,
) -> Result<()> {
    let terminal_phase = terminal_pool_upstream_request_attempt_phase(status);
    let compact_support_status =
        compact_support_status.or(pending.compact_support_status.as_deref());
    let compact_support_reason =
        compact_support_reason.or(pending.compact_support_reason.as_deref());
    let trace = PoolUpstreamAttemptTraceContext {
        invoke_id: pending.invoke_id.clone(),
        occurred_at: pending.occurred_at.clone(),
        endpoint: pending.endpoint.clone(),
        sticky_key: pending.sticky_key.clone(),
        requester_ip: pending.requester_ip.clone(),
    };
    if let Some(attempt_id) = pending.attempt_id {
        let result = sqlx::query(
            r#"
            UPDATE pool_upstream_request_attempts
            SET
                finished_at = ?2,
                status = ?3,
                phase = ?4,
                http_status = ?5,
                failure_kind = ?6,
                error_message = ?7,
                connect_latency_ms = ?8,
                first_byte_latency_ms = ?9,
                stream_latency_ms = ?10,
                upstream_request_id = ?11,
                compact_support_status = ?12,
                compact_support_reason = ?13
            WHERE id = ?1
            "#,
        )
        .bind(attempt_id)
        .bind(finished_at)
        .bind(status)
        .bind(terminal_phase)
        .bind(http_status.map(|value| i64::from(value.as_u16())))
        .bind(failure_kind)
        .bind(error_message)
        .bind(connect_latency_ms)
        .bind(first_byte_latency_ms)
        .bind(stream_latency_ms)
        .bind(upstream_request_id)
        .bind(compact_support_status)
        .bind(compact_support_reason)
        .execute(pool)
        .await?;

        if result.rows_affected() > 0 {
            return Ok(());
        }
    }

    insert_pool_upstream_request_attempt(
        pool,
        &trace,
        Some(pending.upstream_account_id),
        Some(pending.upstream_route_key.as_str()),
        pending.attempt_index,
        pending.distinct_account_index,
        pending.same_account_retry_index,
        Some(pending.started_at.as_str()),
        Some(finished_at),
        status,
        Some(terminal_phase),
        http_status,
        failure_kind,
        error_message,
        connect_latency_ms,
        first_byte_latency_ms,
        stream_latency_ms,
        upstream_request_id,
        compact_support_status,
        compact_support_reason,
    )
    .await
    .map(|_| ())
}

async fn insert_pool_upstream_terminal_attempt(
    pool: &Pool<Sqlite>,
    trace: &PoolUpstreamAttemptTraceContext,
    final_error: &PoolUpstreamError,
    attempt_index: i64,
    distinct_account_index: i64,
    failure_kind: &'static str,
) -> Result<()> {
    let finished_at = shanghai_now_string();
    let upstream_route_key = final_error
        .account
        .as_ref()
        .map(|account| account.upstream_route_key());
    insert_pool_upstream_request_attempt(
        pool,
        trace,
        final_error
            .account
            .as_ref()
            .map(|account| account.account_id),
        upstream_route_key.as_deref(),
        attempt_index,
        distinct_account_index,
        0,
        Some(finished_at.as_str()),
        Some(finished_at.as_str()),
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_BUDGET_EXHAUSTED_FINAL,
        Some(POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_FAILED),
        Some(final_error.status),
        Some(failure_kind),
        Some(final_error.message.as_str()),
        None,
        None,
        None,
        final_error.upstream_request_id.as_deref(),
        None,
        None,
    )
    .await
    .map(|_| ())
}

async fn insert_and_broadcast_pool_upstream_terminal_attempt(
    state: &AppState,
    trace: &PoolUpstreamAttemptTraceContext,
    final_error: &PoolUpstreamError,
    attempt_index: i64,
    distinct_account_index: i64,
    failure_kind: &'static str,
) -> Result<()> {
    insert_pool_upstream_terminal_attempt(
        &state.pool,
        trace,
        final_error,
        attempt_index,
        distinct_account_index,
        failure_kind,
    )
    .await?;
    broadcast_pool_upstream_attempts_snapshot(state, &trace.invoke_id).await?;
    Ok(())
}

fn prompt_cache_upstream_account_rollup_key(
    upstream_account_id: Option<i64>,
    upstream_account_name: Option<&str>,
) -> String {
    let normalized_name = upstream_account_name
        .map(str::trim)
        .filter(|value| !value.is_empty());
    match (upstream_account_id, normalized_name) {
        (Some(account_id), Some(account_name)) => format!("id:{account_id}|name:{account_name}"),
        (Some(account_id), None) => format!("id:{account_id}"),
        (None, Some(account_name)) => format!("name:{account_name}"),
        (None, None) => "unknown".to_string(),
    }
}

async fn load_hourly_rollup_live_progress(pool: &Pool<Sqlite>, dataset: &str) -> Result<i64> {
    Ok(sqlx::query_scalar::<_, i64>(
        "SELECT cursor_id FROM hourly_rollup_live_progress WHERE dataset = ?1",
    )
    .bind(dataset)
    .fetch_optional(pool)
    .await?
    .unwrap_or(0))
}

async fn load_hourly_rollup_live_progress_tx(
    tx: &mut SqliteConnection,
    dataset: &str,
) -> Result<i64> {
    Ok(sqlx::query_scalar::<_, i64>(
        "SELECT cursor_id FROM hourly_rollup_live_progress WHERE dataset = ?1",
    )
    .bind(dataset)
    .fetch_optional(&mut *tx)
    .await?
    .unwrap_or(0))
}

async fn save_hourly_rollup_live_progress_tx(
    tx: &mut SqliteConnection,
    dataset: &str,
    cursor_id: i64,
) -> Result<()> {
    sqlx::query(
        r#"
        INSERT INTO hourly_rollup_live_progress (dataset, cursor_id, updated_at)
        VALUES (?1, ?2, datetime('now'))
        ON CONFLICT(dataset) DO UPDATE SET
            cursor_id = MAX(hourly_rollup_live_progress.cursor_id, excluded.cursor_id),
            updated_at = datetime('now')
        "#,
    )
    .bind(dataset)
    .bind(cursor_id)
    .execute(&mut *tx)
    .await?;
    Ok(())
}

async fn mark_hourly_rollup_archive_replayed_tx(
    tx: &mut SqliteConnection,
    target: &str,
    dataset: &str,
    file_path: &str,
) -> Result<()> {
    sqlx::query(
        r#"
        INSERT OR IGNORE INTO hourly_rollup_archive_replay (
            target,
            dataset,
            file_path,
            replayed_at
        )
        VALUES (?1, ?2, ?3, datetime('now'))
        "#,
    )
    .bind(target)
    .bind(dataset)
    .bind(file_path)
    .execute(&mut *tx)
    .await?;
    Ok(())
}

async fn hourly_rollup_archive_replayed_tx(
    tx: &mut SqliteConnection,
    target: &str,
    dataset: &str,
    file_path: &str,
) -> Result<bool> {
    Ok(
        sqlx::query_scalar::<_, i64>(
            "SELECT 1 FROM hourly_rollup_archive_replay WHERE target = ?1 AND dataset = ?2 AND file_path = ?3 LIMIT 1",
        )
        .bind(target)
        .bind(dataset)
        .bind(file_path)
        .fetch_optional(&mut *tx)
        .await?
        .is_some(),
    )
}

fn normalized_oauth_account_id(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|value| !value.is_empty())
}

fn looks_like_uuid_shape(value: &str) -> bool {
    let bytes = value.as_bytes();
    if bytes.len() != 36 {
        return false;
    }
    for (idx, byte) in bytes.iter().enumerate() {
        let is_hyphen = matches!(idx, 8 | 13 | 18 | 23);
        if is_hyphen {
            if *byte != b'-' {
                return false;
            }
        } else if !byte.is_ascii_hexdigit() {
            return false;
        }
    }
    true
}

fn oauth_account_id_shape(value: Option<&str>) -> &'static str {
    match normalized_oauth_account_id(value) {
        None => "empty",
        Some(value) if value.starts_with("org_") => "org",
        Some(value) if looks_like_uuid_shape(value) => "uuid",
        Some(_) => "other",
    }
}

fn oauth_account_header_attached_for_account(
    account: Option<&PoolResolvedAccount>,
) -> Option<bool> {
    let PoolResolvedAuth::Oauth {
        chatgpt_account_id, ..
    } = &account?.auth
    else {
        return None;
    };

    Some(normalized_oauth_account_id(chatgpt_account_id.as_deref()).is_some())
}

fn oauth_account_id_shape_for_account(
    account: Option<&PoolResolvedAccount>,
) -> Option<&'static str> {
    let PoolResolvedAuth::Oauth {
        chatgpt_account_id, ..
    } = &account?.auth
    else {
        return None;
    };

    Some(oauth_account_id_shape(chatgpt_account_id.as_deref()))
}

struct ProxyPayloadSummary<'a> {
    target: ProxyCaptureTarget,
    status: StatusCode,
    is_stream: bool,
    request_model: Option<&'a str>,
    requested_service_tier: Option<&'a str>,
    billing_service_tier: Option<&'a str>,
    reasoning_effort: Option<&'a str>,
    response_model: Option<&'a str>,
    usage_missing_reason: Option<&'a str>,
    request_parse_error: Option<&'a str>,
    failure_kind: Option<&'a str>,
    requester_ip: Option<&'a str>,
    upstream_scope: &'a str,
    route_mode: &'a str,
    sticky_key: Option<&'a str>,
    prompt_cache_key: Option<&'a str>,
    upstream_account_id: Option<i64>,
    upstream_account_name: Option<&'a str>,
    upstream_account_kind: Option<&'a str>,
    upstream_base_url_host: Option<&'a str>,
    oauth_account_header_attached: Option<bool>,
    oauth_account_id_shape: Option<&'a str>,
    oauth_forwarded_header_count: Option<usize>,
    oauth_forwarded_header_names: Option<&'a [String]>,
    oauth_fingerprint_version: Option<&'a str>,
    oauth_forwarded_header_fingerprints: Option<&'a BTreeMap<String, String>>,
    oauth_prompt_cache_header_forwarded: Option<bool>,
    oauth_request_body_prefix_fingerprint: Option<&'a str>,
    oauth_request_body_prefix_bytes: Option<usize>,
    oauth_responses_rewrite: Option<&'a oauth_bridge::OauthResponsesRewriteSummary>,
    service_tier: Option<&'a str>,
    stream_terminal_event: Option<&'a str>,
    upstream_error_code: Option<&'a str>,
    upstream_error_message: Option<&'a str>,
    upstream_request_id: Option<&'a str>,
    response_content_encoding: Option<&'a str>,
    proxy_display_name: Option<&'a str>,
    proxy_weight_delta: Option<f64>,
    pool_attempt_count: Option<usize>,
    pool_distinct_account_count: Option<usize>,
    pool_attempt_terminal_reason: Option<&'a str>,
}

fn build_proxy_payload_summary(summary: ProxyPayloadSummary<'_>) -> String {
    let ProxyPayloadSummary {
        target,
        status,
        is_stream,
        request_model,
        requested_service_tier,
        billing_service_tier,
        reasoning_effort,
        response_model,
        usage_missing_reason,
        request_parse_error,
        failure_kind,
        requester_ip,
        upstream_scope,
        route_mode,
        sticky_key,
        prompt_cache_key,
        upstream_account_id,
        upstream_account_name,
        upstream_account_kind,
        upstream_base_url_host,
        oauth_account_header_attached,
        oauth_account_id_shape,
        oauth_forwarded_header_count,
        oauth_forwarded_header_names,
        oauth_fingerprint_version,
        oauth_forwarded_header_fingerprints,
        oauth_prompt_cache_header_forwarded,
        oauth_request_body_prefix_fingerprint,
        oauth_request_body_prefix_bytes,
        oauth_responses_rewrite,
        service_tier,
        stream_terminal_event,
        upstream_error_code,
        upstream_error_message,
        upstream_request_id,
        response_content_encoding,
        proxy_display_name,
        proxy_weight_delta,
        pool_attempt_count,
        pool_distinct_account_count,
        pool_attempt_terminal_reason,
    } = summary;
    let payload = json!({
        "endpoint": target.endpoint(),
        "statusCode": status.as_u16(),
        "isStream": is_stream,
        "requestModel": request_model,
        "requestedServiceTier": requested_service_tier,
        "billingServiceTier": billing_service_tier,
        "reasoningEffort": reasoning_effort,
        "responseModel": response_model,
        "usageMissingReason": usage_missing_reason,
        "requestParseError": request_parse_error,
        "failureKind": failure_kind,
        "requesterIp": requester_ip,
        "upstreamScope": upstream_scope,
        "routeMode": route_mode,
        "stickyKey": sticky_key,
        "promptCacheKey": prompt_cache_key,
        "upstreamAccountId": upstream_account_id,
        "upstreamAccountName": upstream_account_name,
        "upstreamAccountKind": upstream_account_kind,
        "upstreamBaseUrlHost": upstream_base_url_host,
        "oauthAccountHeaderAttached": oauth_account_header_attached,
        "oauthAccountIdShape": oauth_account_id_shape,
        "oauthForwardedHeaderCount": oauth_forwarded_header_count,
        "oauthForwardedHeaderNames": oauth_forwarded_header_names,
        "oauthFingerprintVersion": oauth_fingerprint_version,
        "oauthForwardedHeaderFingerprints": oauth_forwarded_header_fingerprints,
        "oauthPromptCacheHeaderForwarded": oauth_prompt_cache_header_forwarded,
        "oauthRequestBodyPrefixFingerprint": oauth_request_body_prefix_fingerprint,
        "oauthRequestBodyPrefixBytes": oauth_request_body_prefix_bytes,
        "oauthResponsesRewrite": oauth_responses_rewrite,
        "serviceTier": service_tier,
        "streamTerminalEvent": stream_terminal_event,
        "upstreamErrorCode": upstream_error_code,
        "upstreamErrorMessage": upstream_error_message,
        "upstreamRequestId": upstream_request_id,
        "responseContentEncoding": response_content_encoding,
        "proxyDisplayName": proxy_display_name,
        "proxyWeightDelta": proxy_weight_delta,
        "poolAttemptCount": pool_attempt_count,
        "poolDistinctAccountCount": pool_distinct_account_count,
        "poolAttemptTerminalReason": pool_attempt_terminal_reason,
    });
    serde_json::to_string(&payload).unwrap_or_else(|_| "{}".to_string())
}

fn invocation_status_is_in_flight(status: Option<&str>) -> bool {
    matches!(
        status
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase()
            .as_str(),
        INVOCATION_STATUS_RUNNING | INVOCATION_STATUS_PENDING
    )
}

fn nullable_runtime_timing_value(value: f64) -> Option<f64> {
    (value.is_finite() && value > 0.0).then_some(value)
}

#[derive(Debug, FromRow)]
struct PersistedInvocationIdentityRow {
    id: i64,
    status: Option<String>,
}

async fn load_persisted_invocation_identity_tx(
    tx: &mut SqliteConnection,
    invoke_id: &str,
    occurred_at: &str,
) -> Result<Option<PersistedInvocationIdentityRow>> {
    sqlx::query_as::<_, PersistedInvocationIdentityRow>(
        r#"
        SELECT id, status
        FROM codex_invocations
        WHERE invoke_id = ?1 AND occurred_at = ?2
        ORDER BY id DESC
        LIMIT 1
        "#,
    )
    .bind(invoke_id)
    .bind(occurred_at)
    .fetch_optional(&mut *tx)
    .await
    .map_err(Into::into)
}

async fn load_persisted_api_invocation_tx(
    tx: &mut SqliteConnection,
    invoke_id: &str,
    occurred_at: &str,
) -> Result<ApiInvocation> {
    sqlx::query_as::<_, ApiInvocation>(
        r#"
        SELECT
            id,
            invoke_id,
            occurred_at,
            source,
            CASE WHEN json_valid(payload) THEN json_extract(payload, '$.proxyDisplayName') END AS proxy_display_name,
            model,
            input_tokens,
            output_tokens,
            cache_input_tokens,
            reasoning_tokens,
            CASE WHEN json_valid(payload) THEN json_extract(payload, '$.reasoningEffort') END AS reasoning_effort,
            total_tokens,
            cost,
            status,
            error_message,
            CASE WHEN json_valid(payload) THEN json_extract(payload, '$.endpoint') END AS endpoint,
            COALESCE(CASE WHEN json_valid(payload) THEN json_extract(payload, '$.failureKind') END, failure_kind) AS failure_kind,
            CASE WHEN json_valid(payload) THEN json_extract(payload, '$.streamTerminalEvent') END AS stream_terminal_event,
            CASE WHEN json_valid(payload) THEN json_extract(payload, '$.upstreamErrorCode') END AS upstream_error_code,
            CASE WHEN json_valid(payload) THEN json_extract(payload, '$.upstreamErrorMessage') END AS upstream_error_message,
            CASE WHEN json_valid(payload) THEN json_extract(payload, '$.upstreamRequestId') END AS upstream_request_id,
            failure_class,
            is_actionable,
            CASE WHEN json_valid(payload) THEN json_extract(payload, '$.requesterIp') END AS requester_ip,
            CASE WHEN json_valid(payload) THEN json_extract(payload, '$.promptCacheKey') END AS prompt_cache_key,
            CASE WHEN json_valid(payload) THEN json_extract(payload, '$.routeMode') END AS route_mode,
            CASE WHEN json_valid(payload) THEN json_extract(payload, '$.upstreamAccountId') END AS upstream_account_id,
            CASE WHEN json_valid(payload) THEN json_extract(payload, '$.upstreamAccountName') END AS upstream_account_name,
            CASE WHEN json_valid(payload) THEN json_extract(payload, '$.responseContentEncoding') END AS response_content_encoding,
            CASE
              WHEN json_valid(payload) AND json_type(payload, '$.poolAttemptCount') IN ('integer', 'real')
                THEN json_extract(payload, '$.poolAttemptCount')
            END AS pool_attempt_count,
            CASE
              WHEN json_valid(payload) AND json_type(payload, '$.poolDistinctAccountCount') IN ('integer', 'real')
                THEN json_extract(payload, '$.poolDistinctAccountCount')
            END AS pool_distinct_account_count,
            CASE
              WHEN json_valid(payload) AND json_type(payload, '$.poolAttemptTerminalReason') = 'text'
                THEN json_extract(payload, '$.poolAttemptTerminalReason')
            END AS pool_attempt_terminal_reason,
            CASE
              WHEN json_valid(payload) AND json_type(payload, '$.requestedServiceTier') = 'text'
                THEN json_extract(payload, '$.requestedServiceTier')
              WHEN json_valid(payload) AND json_type(payload, '$.requested_service_tier') = 'text'
                THEN json_extract(payload, '$.requested_service_tier') END AS requested_service_tier,
            CASE
              WHEN json_valid(payload) AND json_type(payload, '$.serviceTier') = 'text'
                THEN json_extract(payload, '$.serviceTier')
              WHEN json_valid(payload) AND json_type(payload, '$.service_tier') = 'text'
                THEN json_extract(payload, '$.service_tier') END AS service_tier,
            CASE
              WHEN json_valid(payload) AND json_type(payload, '$.billingServiceTier') = 'text'
                THEN json_extract(payload, '$.billingServiceTier')
              WHEN json_valid(payload) AND json_type(payload, '$.billing_service_tier') = 'text'
                THEN json_extract(payload, '$.billing_service_tier') END AS billing_service_tier,
            CASE WHEN json_valid(payload)
              AND json_type(payload, '$.proxyWeightDelta') IN ('integer', 'real')
              THEN json_extract(payload, '$.proxyWeightDelta') END AS proxy_weight_delta,
            cost_estimated,
            price_version,
            request_raw_path,
            request_raw_size,
            request_raw_truncated,
            request_raw_truncated_reason,
            response_raw_path,
            response_raw_size,
            response_raw_truncated,
            response_raw_truncated_reason,
            detail_level,
            detail_pruned_at,
            detail_prune_reason,
            t_total_ms,
            t_req_read_ms,
            t_req_parse_ms,
            t_upstream_connect_ms,
            t_upstream_ttfb_ms,
            t_upstream_stream_ms,
            t_resp_parse_ms,
            t_persist_ms,
            created_at
        FROM codex_invocations
        WHERE invoke_id = ?1 AND occurred_at = ?2
        ORDER BY id DESC
        LIMIT 1
        "#,
    )
    .bind(invoke_id)
    .bind(occurred_at)
    .fetch_one(&mut *tx)
    .await
    .map_err(Into::into)
}

async fn touch_invocation_upstream_account_last_activity_tx(
    tx: &mut SqliteConnection,
    occurred_at: &str,
    payload: Option<&str>,
) -> Result<()> {
    if let Some(upstream_account_id) = upstream_account_id_from_payload(payload) {
        sqlx::query(
            r#"
            UPDATE pool_upstream_accounts
            SET last_activity_at = CASE
                WHEN last_activity_at IS NULL OR last_activity_at < ?1 THEN ?1
                ELSE last_activity_at
            END
            WHERE id = ?2
            "#,
        )
        .bind(occurred_at)
        .bind(upstream_account_id)
        .execute(&mut *tx)
        .await?;
    }

    Ok(())
}

async fn persist_and_broadcast_proxy_capture_runtime_snapshot(
    state: &AppState,
    record: ProxyCaptureRecord,
) -> Result<()> {
    let persisted = persist_proxy_capture_runtime_record(&state.pool, record).await?;
    let Some(persisted_record) = persisted else {
        return Ok(());
    };

    if persisted_record
        .prompt_cache_key
        .as_deref()
        .is_some_and(|key| !key.trim().is_empty())
    {
        invalidate_prompt_cache_conversations_cache(&state.prompt_cache_conversation_cache).await;
    }

    if state.broadcaster.receiver_count() == 0 {
        return Ok(());
    }

    let invoke_id = persisted_record.invoke_id.clone();
    if let Err(err) = state.broadcaster.send(BroadcastPayload::Records {
        records: vec![persisted_record],
    }) {
        warn!(
            ?err,
            invoke_id = %invoke_id,
            "failed to broadcast runtime proxy capture snapshot"
        );
    }

    Ok(())
}

async fn persist_proxy_capture_runtime_record(
    pool: &Pool<Sqlite>,
    record: ProxyCaptureRecord,
) -> Result<Option<ApiInvocation>> {
    let failure = classify_invocation_failure(
        Some(record.status.as_str()),
        record.error_message.as_deref(),
    );
    let failure_kind = record
        .failure_kind
        .clone()
        .or_else(|| failure.failure_kind.clone());
    let t_req_read_ms = nullable_runtime_timing_value(record.timings.t_req_read_ms);
    let t_req_parse_ms = nullable_runtime_timing_value(record.timings.t_req_parse_ms);
    let t_upstream_connect_ms = nullable_runtime_timing_value(record.timings.t_upstream_connect_ms);
    let t_upstream_ttfb_ms = nullable_runtime_timing_value(record.timings.t_upstream_ttfb_ms);
    let mut tx = pool.begin().await?;
    let insert_result = sqlx::query(
        r#"
        INSERT OR IGNORE INTO codex_invocations (
            invoke_id,
            occurred_at,
            source,
            model,
            input_tokens,
            output_tokens,
            cache_input_tokens,
            reasoning_tokens,
            total_tokens,
            cost,
            cost_estimated,
            price_version,
            status,
            error_message,
            failure_kind,
            failure_class,
            is_actionable,
            payload,
            raw_response,
            request_raw_path,
            request_raw_size,
            request_raw_truncated,
            request_raw_truncated_reason,
            response_raw_path,
            response_raw_size,
            response_raw_truncated,
            response_raw_truncated_reason,
            t_total_ms,
            t_req_read_ms,
            t_req_parse_ms,
            t_upstream_connect_ms,
            t_upstream_ttfb_ms,
            t_upstream_stream_ms,
            t_resp_parse_ms,
            t_persist_ms
        )
        VALUES (
            ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19,
            ?20, ?21, ?22, ?23, ?24, ?25, ?26, ?27, ?28, ?29, ?30, ?31, ?32, ?33, ?34, ?35
        )
        "#,
    )
    .bind(&record.invoke_id)
    .bind(&record.occurred_at)
    .bind(SOURCE_PROXY)
    .bind(&record.model)
    .bind(record.usage.input_tokens)
    .bind(record.usage.output_tokens)
    .bind(record.usage.cache_input_tokens)
    .bind(record.usage.reasoning_tokens)
    .bind(record.usage.total_tokens)
    .bind(record.cost)
    .bind(record.cost_estimated as i64)
    .bind(record.price_version.as_deref())
    .bind(&record.status)
    .bind(record.error_message.as_deref())
    .bind(failure_kind.as_deref())
    .bind(failure.failure_class.as_str())
    .bind(failure.is_actionable as i64)
    .bind(record.payload.as_deref())
    .bind(&record.raw_response)
    .bind(record.req_raw.path.as_deref())
    .bind(record.req_raw.path.as_ref().map(|_| record.req_raw.size_bytes))
    .bind(record.req_raw.truncated as i64)
    .bind(record.req_raw.truncated_reason.as_deref())
    .bind(record.resp_raw.path.as_deref())
    .bind(record.resp_raw.path.as_ref().map(|_| record.resp_raw.size_bytes))
    .bind(record.resp_raw.truncated as i64)
    .bind(record.resp_raw.truncated_reason.as_deref())
    .bind(None::<f64>)
    .bind(t_req_read_ms)
    .bind(t_req_parse_ms)
    .bind(t_upstream_connect_ms)
    .bind(t_upstream_ttfb_ms)
    .bind(None::<f64>)
    .bind(None::<f64>)
    .bind(None::<f64>)
    .execute(tx.as_mut())
    .await?;

    let invocation_id = if insert_result.rows_affected() > 0 {
        let inserted_id = insert_result.last_insert_rowid();
        upsert_invocation_hourly_rollups_tx(
            tx.as_mut(),
            &[InvocationHourlySourceRecord {
                id: inserted_id,
                occurred_at: record.occurred_at.clone(),
                source: SOURCE_PROXY.to_string(),
                status: Some(record.status.clone()),
                detail_level: DETAIL_LEVEL_FULL.to_string(),
                total_tokens: record.usage.total_tokens,
                cost: record.cost,
                error_message: record.error_message.clone(),
                failure_kind: failure_kind.clone(),
                failure_class: Some(failure.failure_class.as_str().to_string()),
                is_actionable: Some(failure.is_actionable as i64),
                payload: record.payload.clone(),
                t_total_ms: None,
                t_req_read_ms,
                t_req_parse_ms,
                t_upstream_connect_ms,
                t_upstream_ttfb_ms,
                t_upstream_stream_ms: None,
                t_resp_parse_ms: None,
                t_persist_ms: None,
            }],
            &INVOCATION_HOURLY_ROLLUP_TARGETS,
        )
        .await?;
        save_hourly_rollup_live_progress_tx(
            tx.as_mut(),
            HOURLY_ROLLUP_DATASET_INVOCATIONS,
            inserted_id,
        )
        .await?;
        touch_invocation_upstream_account_last_activity_tx(
            tx.as_mut(),
            &record.occurred_at,
            record.payload.as_deref(),
        )
        .await?;
        inserted_id
    } else {
        let Some(existing) = load_persisted_invocation_identity_tx(
            tx.as_mut(),
            &record.invoke_id,
            &record.occurred_at,
        )
        .await?
        else {
            tx.commit().await?;
            return Ok(None);
        };
        if !invocation_status_is_in_flight(existing.status.as_deref()) {
            tx.commit().await?;
            return Ok(None);
        }

        let affected = sqlx::query(
            r#"
            UPDATE codex_invocations
            SET source = ?2,
                model = ?3,
                input_tokens = ?4,
                output_tokens = ?5,
                cache_input_tokens = ?6,
                reasoning_tokens = ?7,
                total_tokens = ?8,
                cost = ?9,
                cost_estimated = ?10,
                price_version = ?11,
                status = ?12,
                error_message = ?13,
                failure_kind = ?14,
                failure_class = ?15,
                is_actionable = ?16,
                payload = ?17,
                raw_response = ?18,
                request_raw_path = ?19,
                request_raw_size = ?20,
                request_raw_truncated = ?21,
                request_raw_truncated_reason = ?22,
                response_raw_path = ?23,
                response_raw_size = ?24,
                response_raw_truncated = ?25,
                response_raw_truncated_reason = ?26,
                t_total_ms = ?27,
                t_req_read_ms = ?28,
                t_req_parse_ms = ?29,
                t_upstream_connect_ms = ?30,
                t_upstream_ttfb_ms = ?31,
                t_upstream_stream_ms = ?32,
                t_resp_parse_ms = ?33,
                t_persist_ms = ?34
            WHERE id = ?1
              AND LOWER(TRIM(COALESCE(status, ''))) IN ('running', 'pending')
            "#,
        )
        .bind(existing.id)
        .bind(SOURCE_PROXY)
        .bind(&record.model)
        .bind(record.usage.input_tokens)
        .bind(record.usage.output_tokens)
        .bind(record.usage.cache_input_tokens)
        .bind(record.usage.reasoning_tokens)
        .bind(record.usage.total_tokens)
        .bind(record.cost)
        .bind(record.cost_estimated as i64)
        .bind(record.price_version.as_deref())
        .bind(&record.status)
        .bind(record.error_message.as_deref())
        .bind(failure_kind.as_deref())
        .bind(failure.failure_class.as_str())
        .bind(failure.is_actionable as i64)
        .bind(record.payload.as_deref())
        .bind(&record.raw_response)
        .bind(record.req_raw.path.as_deref())
        .bind(record.req_raw.path.as_ref().map(|_| record.req_raw.size_bytes))
        .bind(record.req_raw.truncated as i64)
        .bind(record.req_raw.truncated_reason.as_deref())
        .bind(record.resp_raw.path.as_deref())
        .bind(record.resp_raw.path.as_ref().map(|_| record.resp_raw.size_bytes))
        .bind(record.resp_raw.truncated as i64)
        .bind(record.resp_raw.truncated_reason.as_deref())
        .bind(None::<f64>)
        .bind(t_req_read_ms)
        .bind(t_req_parse_ms)
        .bind(t_upstream_connect_ms)
        .bind(t_upstream_ttfb_ms)
        .bind(None::<f64>)
        .bind(None::<f64>)
        .bind(None::<f64>)
        .execute(tx.as_mut())
        .await?
        .rows_affected();
        if affected == 0 {
            tx.commit().await?;
            return Ok(None);
        }
        recompute_invocation_hourly_rollups_for_ids_tx(tx.as_mut(), &[existing.id]).await?;
        save_hourly_rollup_live_progress_tx(
            tx.as_mut(),
            HOURLY_ROLLUP_DATASET_INVOCATIONS,
            existing.id,
        )
        .await?;
        touch_invocation_upstream_account_last_activity_tx(
            tx.as_mut(),
            &record.occurred_at,
            record.payload.as_deref(),
        )
        .await?;
        existing.id
    };

    let persisted = load_persisted_api_invocation_tx(tx.as_mut(), &record.invoke_id, &record.occurred_at)
        .await?;
    save_hourly_rollup_live_progress_tx(
        tx.as_mut(),
        HOURLY_ROLLUP_DATASET_INVOCATIONS,
        invocation_id,
    )
    .await?;
    tx.commit().await?;

    Ok(Some(persisted))
}

fn build_running_proxy_capture_record(
    invoke_id: &str,
    occurred_at: &str,
    target: ProxyCaptureTarget,
    request_info: &RequestCaptureInfo,
    requester_ip: Option<&str>,
    sticky_key: Option<&str>,
    prompt_cache_key: Option<&str>,
    pool_route_active: bool,
    upstream_account_id: Option<i64>,
    upstream_account_name: Option<&str>,
    upstream_account_kind: Option<&str>,
    upstream_base_url_host: Option<&str>,
    proxy_display_name: Option<&str>,
    pool_attempt_count: Option<usize>,
    pool_distinct_account_count: Option<usize>,
    pool_attempt_terminal_reason: Option<&str>,
    response_content_encoding: Option<&str>,
    t_req_read_ms: f64,
    t_req_parse_ms: f64,
    t_upstream_connect_ms: f64,
    t_upstream_ttfb_ms: f64,
) -> ProxyCaptureRecord {
    ProxyCaptureRecord {
        invoke_id: invoke_id.to_string(),
        occurred_at: occurred_at.to_string(),
        model: request_info.model.clone(),
        usage: ParsedUsage::default(),
        cost: None,
        cost_estimated: false,
        price_version: None,
        status: "running".to_string(),
        error_message: None,
        failure_kind: None,
        payload: Some(build_proxy_payload_summary(ProxyPayloadSummary {
            target,
            status: StatusCode::OK,
            is_stream: request_info.is_stream,
            request_model: request_info.model.as_deref(),
            requested_service_tier: request_info.requested_service_tier.as_deref(),
            billing_service_tier: None,
            reasoning_effort: request_info.reasoning_effort.as_deref(),
            response_model: None,
            usage_missing_reason: None,
            request_parse_error: request_info.parse_error.as_deref(),
            failure_kind: None,
            requester_ip,
            upstream_scope: if pool_route_active {
                INVOCATION_UPSTREAM_SCOPE_INTERNAL
            } else {
                INVOCATION_UPSTREAM_SCOPE_EXTERNAL
            },
            route_mode: if pool_route_active {
                INVOCATION_ROUTE_MODE_POOL
            } else {
                INVOCATION_ROUTE_MODE_FORWARD_PROXY
            },
            sticky_key,
            prompt_cache_key,
            upstream_account_id,
            upstream_account_name,
            upstream_account_kind,
            upstream_base_url_host,
            oauth_account_header_attached: None,
            oauth_account_id_shape: None,
            oauth_forwarded_header_count: None,
            oauth_forwarded_header_names: None,
            oauth_fingerprint_version: None,
            oauth_forwarded_header_fingerprints: None,
            oauth_prompt_cache_header_forwarded: None,
            oauth_request_body_prefix_fingerprint: None,
            oauth_request_body_prefix_bytes: None,
            oauth_responses_rewrite: None,
            service_tier: None,
            stream_terminal_event: None,
            upstream_error_code: None,
            upstream_error_message: None,
            upstream_request_id: None,
            response_content_encoding,
            proxy_display_name,
            proxy_weight_delta: None,
            pool_attempt_count,
            pool_distinct_account_count,
            pool_attempt_terminal_reason,
        })),
        raw_response: "{}".to_string(),
        req_raw: RawPayloadMeta::default(),
        resp_raw: RawPayloadMeta::default(),
        timings: StageTimings {
            t_total_ms: 0.0,
            t_req_read_ms,
            t_req_parse_ms,
            t_upstream_connect_ms,
            t_upstream_ttfb_ms,
            t_upstream_stream_ms: 0.0,
            t_resp_parse_ms: 0.0,
            t_persist_ms: 0.0,
        },
    }
}

fn resolve_invocation_proxy_display_name(
    selected_proxy: Option<&SelectedForwardProxy>,
) -> Option<String> {
    selected_proxy.map(|proxy| proxy.display_name.clone())
}

fn summarize_response_content_encoding(content_encoding: Option<&str>) -> String {
    let encodings = parse_content_encodings(content_encoding);
    if encodings.is_empty() {
        "identity".to_string()
    } else {
        encodings.join(", ")
    }
}

#[derive(Default)]
struct RawResponsePreviewBuffer {
    bytes: Vec<u8>,
}

impl RawResponsePreviewBuffer {
    fn append(&mut self, chunk: &[u8]) {
        let remaining = RAW_RESPONSE_PREVIEW_LIMIT.saturating_sub(self.bytes.len());
        if remaining == 0 || chunk.is_empty() {
            return;
        }
        self.bytes
            .extend_from_slice(&chunk[..chunk.len().min(remaining)]);
    }

    fn as_slice(&self) -> &[u8] {
        &self.bytes
    }

    fn into_preview(self) -> String {
        build_raw_response_preview(&self.bytes)
    }
}

struct BoundedResponseParseBuffer {
    bytes: Vec<u8>,
    limit: usize,
    exceeded_limit: bool,
}

impl BoundedResponseParseBuffer {
    fn new(limit: usize) -> Self {
        Self {
            bytes: Vec::new(),
            limit,
            exceeded_limit: false,
        }
    }

    fn append(&mut self, chunk: &[u8]) {
        if self.exceeded_limit || chunk.is_empty() {
            return;
        }

        let remaining = self.limit.saturating_sub(self.bytes.len());
        let take_len = remaining.min(chunk.len());
        if take_len > 0 {
            self.bytes.extend_from_slice(&chunk[..take_len]);
        }
        if take_len < chunk.len() {
            self.exceeded_limit = true;
        }
    }

    fn into_response_info(
        self,
        target: ProxyCaptureTarget,
        content_encoding: Option<&str>,
    ) -> ResponseCaptureInfo {
        let mut response_info =
            parse_target_response_payload(target, &self.bytes, false, content_encoding);
        if self.exceeded_limit {
            merge_response_capture_reason(
                &mut response_info,
                PROXY_USAGE_MISSING_NON_STREAM_PARSE_SKIPPED,
            );
        }
        response_info
    }
}

struct StreamingRawPayloadWriter {
    path: PathBuf,
    max_bytes: Option<usize>,
    written_bytes: usize,
    meta: RawPayloadMeta,
    file: Option<tokio::fs::File>,
}

impl StreamingRawPayloadWriter {
    fn new(config: &AppConfig, invoke_id: &str, kind: &str) -> Self {
        let path = config
            .resolved_proxy_raw_dir()
            .join(format!("{invoke_id}-{kind}.bin"));
        Self {
            path,
            max_bytes: config.proxy_raw_max_bytes,
            written_bytes: 0,
            meta: RawPayloadMeta::default(),
            file: None,
        }
    }

    async fn ensure_file(&mut self) -> io::Result<()> {
        if self.file.is_some() {
            return Ok(());
        }
        let Some(parent) = self.path.parent() else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("raw payload path has no parent: {}", self.path.display()),
            ));
        };
        tokio::fs::create_dir_all(parent).await?;
        let file = tokio::fs::File::create(&self.path).await?;
        self.meta.path = Some(self.path.to_string_lossy().to_string());
        self.file = Some(file);
        Ok(())
    }

    fn mark_max_bytes_exceeded(&mut self) {
        self.meta.truncated = true;
        self.meta
            .truncated_reason
            .get_or_insert_with(|| "max_bytes_exceeded".to_string());
    }

    async fn record_write_failure(&mut self, err: io::Error) {
        self.meta.truncated = true;
        self.meta.truncated_reason = Some(format!("write_failed:{err}"));
        self.file = None;
        if self.meta.path.is_some() {
            let _ = tokio::fs::remove_file(&self.path).await;
            self.meta.path = None;
        }
    }

    async fn append(&mut self, bytes: &[u8]) {
        if bytes.is_empty() {
            return;
        }

        self.meta.size_bytes = self.meta.size_bytes.saturating_add(bytes.len() as i64);

        if self
            .meta
            .truncated_reason
            .as_deref()
            .is_some_and(|reason| reason.starts_with("write_failed:"))
        {
            return;
        }

        if let Err(err) = self.ensure_file().await {
            self.record_write_failure(err).await;
            return;
        }

        let write_len = if let Some(limit) = self.max_bytes {
            let remaining = limit.saturating_sub(self.written_bytes);
            if remaining == 0 {
                self.mark_max_bytes_exceeded();
                return;
            }
            let write_len = remaining.min(bytes.len());
            if write_len < bytes.len() {
                self.mark_max_bytes_exceeded();
            }
            write_len
        } else {
            bytes.len()
        };

        if write_len == 0 {
            return;
        }

        if let Some(file) = self.file.as_mut() {
            if let Err(err) = file.write_all(&bytes[..write_len]).await {
                self.record_write_failure(err).await;
                return;
            }
            self.written_bytes = self.written_bytes.saturating_add(write_len);
        }
    }

    async fn finish(mut self) -> RawPayloadMeta {
        if let Some(file) = self.file.as_mut()
            && let Err(err) = file.flush().await
        {
            self.record_write_failure(err).await;
        }
        self.meta
    }
}

fn build_raw_response_preview(bytes: &[u8]) -> String {
    if bytes.is_empty() {
        return "{}".to_string();
    }
    let preview = if bytes.len() > RAW_RESPONSE_PREVIEW_LIMIT {
        &bytes[..RAW_RESPONSE_PREVIEW_LIMIT]
    } else {
        bytes
    };
    String::from_utf8_lossy(preview).to_string()
}

fn extract_error_message_from_response(bytes: &[u8]) -> Option<String> {
    let value = serde_json::from_slice::<Value>(bytes).ok()?;
    value
        .pointer("/error/message")
        .and_then(|v| v.as_str())
        .map(|v| v.to_string())
        .or_else(|| {
            value
                .get("message")
                .and_then(|v| v.as_str())
                .map(|v| v.to_string())
        })
}

fn summarize_plaintext_upstream_error(bytes: &[u8]) -> Option<String> {
    let text = std::str::from_utf8(bytes).ok()?.trim();
    if text.is_empty() {
        return None;
    }
    let lower = text.to_ascii_lowercase();
    if text.starts_with('<')
        || lower.starts_with("<!doctype")
        || lower.starts_with("<html")
        || lower.starts_with("<body")
    {
        return None;
    }
    Some(text.chars().take(240).collect())
}

fn extract_error_message_from_response_preview(bytes: &[u8]) -> Option<String> {
    extract_error_message_from_response(bytes).or_else(|| summarize_plaintext_upstream_error(bytes))
}

fn merge_response_capture_reason(
    response_info: &mut ResponseCaptureInfo,
    reason: impl Into<String>,
) {
    let reason = reason.into();
    let combined_reason = if let Some(existing) = response_info.usage_missing_reason.take() {
        format!("{reason};{existing}")
    } else {
        reason
    };
    response_info.usage_missing_reason = Some(combined_reason);
}

fn deflate_stream_uses_zlib_wrapper(header: &[u8]) -> bool {
    if header.len() < 2 {
        return true;
    }

    let cmf = header[0];
    let flg = header[1];
    let method = cmf & 0x0f;
    let window_bits = cmf >> 4;
    let header_word = (u16::from(cmf) << 8) | u16::from(flg);
    method == 8 && window_bits <= 7 && header_word % 31 == 0
}

#[allow(dead_code)]
fn wrap_decoded_response_reader(
    mut reader: Box<dyn Read + Send>,
    content_encoding: Option<&str>,
) -> std::result::Result<Box<dyn Read + Send>, String> {
    let encodings = parse_content_encodings(content_encoding);
    for encoding in encodings.iter().rev() {
        reader = match encoding.as_str() {
            "identity" => reader,
            "gzip" | "x-gzip" => Box::new(GzDecoder::new(reader)),
            "br" => Box::new(BrotliDecompressor::new(reader, 4096)),
            "deflate" => {
                let mut buffered = io::BufReader::new(reader);
                let header = buffered.fill_buf().map_err(|err| err.to_string())?;
                if deflate_stream_uses_zlib_wrapper(header) {
                    Box::new(ZlibDecoder::new(buffered))
                } else {
                    Box::new(DeflateDecoder::new(buffered))
                }
            }
            other => return Err(format!("unsupported_content_encoding:{other}")),
        };
    }
    Ok(reader)
}

#[allow(dead_code)]
fn open_decoded_response_reader(
    path: &Path,
    content_encoding: Option<&str>,
) -> std::result::Result<Box<dyn Read + Send>, String> {
    let file = fs::File::open(path).map_err(|err| err.to_string())?;
    wrap_decoded_response_reader(Box::new(file), content_encoding)
}

#[allow(dead_code)]
fn parse_nonstream_response_payload_from_raw_file(
    target: ProxyCaptureTarget,
    path: &Path,
    content_encoding: Option<&str>,
) -> std::result::Result<ResponseCaptureInfo, String> {
    let mut reader = open_decoded_response_reader(path, content_encoding)?;
    let mut decoded = Vec::new();
    reader
        .by_ref()
        .take((BOUNDED_NON_STREAM_RESPONSE_PARSE_LIMIT_BYTES + 1) as u64)
        .read_to_end(&mut decoded)
        .map_err(|err| err.to_string())?;
    if decoded.len() > BOUNDED_NON_STREAM_RESPONSE_PARSE_LIMIT_BYTES {
        decoded.truncate(BOUNDED_NON_STREAM_RESPONSE_PARSE_LIMIT_BYTES);
        let mut response_info = parse_target_response_payload(target, &decoded, false, None);
        merge_response_capture_reason(
            &mut response_info,
            PROXY_USAGE_MISSING_NON_STREAM_PARSE_SKIPPED,
        );
        return Ok(response_info);
    }
    Ok(parse_target_response_payload(target, &decoded, false, None))
}

#[allow(dead_code)]
fn parse_target_response_payload_from_raw_file(
    target: ProxyCaptureTarget,
    path: &Path,
    is_stream_hint: bool,
    content_encoding: Option<&str>,
) -> std::result::Result<ResponseCaptureInfo, String> {
    if is_stream_hint {
        let reader = open_decoded_response_reader(path, content_encoding)?;
        parse_stream_response_payload_from_reader(reader).map_err(|err| err.to_string())
    } else {
        parse_nonstream_response_payload_from_raw_file(target, path, content_encoding)
    }
}

#[allow(dead_code)]
fn parse_target_response_payload_from_capture(
    target: ProxyCaptureTarget,
    resp_raw: &RawPayloadMeta,
    preview_bytes: &[u8],
    is_stream_hint: bool,
    content_encoding: Option<&str>,
) -> ResponseCaptureInfo {
    #[cfg(test)]
    RESPONSE_CAPTURE_RAW_PARSE_FALLBACK_CALLS.fetch_add(1, Ordering::Relaxed);

    if let Some(path) = resp_raw.path.as_deref() {
        let path = PathBuf::from(path);
        match parse_target_response_payload_from_raw_file(
            target,
            &path,
            is_stream_hint,
            content_encoding,
        ) {
            Ok(response_info) => response_info,
            Err(reason) => {
                let mut response_info = parse_target_response_payload(
                    target,
                    preview_bytes,
                    is_stream_hint,
                    content_encoding,
                );
                merge_response_capture_reason(&mut response_info, reason);
                response_info
            }
        }
    } else {
        parse_target_response_payload(target, preview_bytes, is_stream_hint, content_encoding)
    }
}

fn summarize_pool_upstream_http_failure(
    status: StatusCode,
    upstream_request_id_header: Option<&str>,
    bytes: &[u8],
) -> (Option<String>, Option<String>, Option<String>, String) {
    let Ok(value) = serde_json::from_slice::<Value>(bytes) else {
        let detail = summarize_plaintext_upstream_error(bytes);
        let message = detail.as_deref().map_or_else(
            || format!("pool upstream responded with {}", status.as_u16()),
            |detail| {
                format!(
                    "pool upstream responded with {}: {}",
                    status.as_u16(),
                    detail
                )
            },
        );
        return (
            None,
            detail,
            upstream_request_id_header.map(|value| value.to_string()),
            message,
        );
    };
    let upstream_error_code = extract_upstream_error_code(&value);
    let upstream_error_message = extract_upstream_error_message(&value);
    let upstream_request_id = upstream_request_id_header
        .map(|value| value.to_string())
        .or_else(|| extract_upstream_request_id(&value));

    let detail = upstream_error_message
        .as_deref()
        .or_else(|| value.get("message").and_then(|entry| entry.as_str()))
        .map(str::trim)
        .filter(|detail| !detail.is_empty())
        .map(|detail| detail.chars().take(240).collect::<String>());

    let message = if let Some(detail) = detail {
        format!(
            "pool upstream responded with {}: {}",
            status.as_u16(),
            detail
        )
    } else {
        format!("pool upstream responded with {}", status.as_u16())
    };

    (
        upstream_error_code,
        upstream_error_message,
        upstream_request_id,
        message,
    )
}

async fn estimate_proxy_cost_from_shared_catalog(
    catalog: &Arc<RwLock<PricingCatalog>>,
    model: Option<&str>,
    usage: &ParsedUsage,
    billing_service_tier: Option<&str>,
    pricing_mode: ProxyPricingMode,
) -> (Option<f64>, bool, Option<String>) {
    let guard = catalog.read().await;
    estimate_proxy_cost(&guard, model, usage, billing_service_tier, pricing_mode)
}

fn has_billable_usage(usage: &ParsedUsage) -> bool {
    usage.input_tokens.unwrap_or(0).max(0) > 0
        || usage.output_tokens.unwrap_or(0).max(0) > 0
        || usage.cache_input_tokens.unwrap_or(0).max(0) > 0
        || usage.reasoning_tokens.unwrap_or(0).max(0) > 0
}

fn resolve_pricing_for_model<'a>(
    catalog: &'a PricingCatalog,
    model: &str,
) -> Option<&'a ModelPricing> {
    if let Some(pricing) = catalog.models.get(model) {
        return Some(pricing);
    }
    dated_model_alias_base(model).and_then(|base| catalog.models.get(base))
}

fn dated_model_alias_base(model: &str) -> Option<&str> {
    const DATED_SUFFIX_LEN: usize = 11; // -YYYY-MM-DD
    if model.len() <= DATED_SUFFIX_LEN {
        return None;
    }
    let suffix = &model.as_bytes()[model.len() - DATED_SUFFIX_LEN..];
    let is_dated_suffix = suffix[0] == b'-'
        && suffix[1].is_ascii_digit()
        && suffix[2].is_ascii_digit()
        && suffix[3].is_ascii_digit()
        && suffix[4].is_ascii_digit()
        && suffix[5] == b'-'
        && suffix[6].is_ascii_digit()
        && suffix[7].is_ascii_digit()
        && suffix[8] == b'-'
        && suffix[9].is_ascii_digit()
        && suffix[10].is_ascii_digit();
    if !is_dated_suffix {
        return None;
    }
    let base = &model[..model.len() - DATED_SUFFIX_LEN];
    if base.is_empty() { None } else { Some(base) }
}

fn is_gpt_5_4_long_context_surcharge_model(model: &str) -> bool {
    let base = dated_model_alias_base(model).unwrap_or(model);
    matches!(base, "gpt-5.4" | "gpt-5.4-pro")
}

fn proxy_price_version(catalog_version: &str, pricing_mode: ProxyPricingMode) -> String {
    format!("{catalog_version}{}", pricing_mode.price_version_suffix())
}

fn pricing_backfill_attempt_version(catalog: &PricingCatalog) -> String {
    fn mix_fvn1a(hash: &mut u64, bytes: &[u8]) {
        for byte in bytes {
            *hash ^= u64::from(*byte);
            *hash = hash.wrapping_mul(0x100000001b3);
        }
    }

    let mut hash = 0xcbf29ce484222325_u64;
    mix_fvn1a(&mut hash, COST_BACKFILL_ALGO_VERSION.as_bytes());
    mix_fvn1a(&mut hash, &[0xfc]);
    mix_fvn1a(&mut hash, catalog.version.as_bytes());
    mix_fvn1a(&mut hash, &[0xff]);
    mix_fvn1a(&mut hash, API_KEYS_BILLING_ACCOUNT_KIND.as_bytes());
    mix_fvn1a(&mut hash, &[0xfb]);
    mix_fvn1a(&mut hash, REQUESTED_TIER_PRICE_VERSION_SUFFIX.as_bytes());
    mix_fvn1a(&mut hash, &[0xfa]);
    mix_fvn1a(&mut hash, RESPONSE_TIER_PRICE_VERSION_SUFFIX.as_bytes());
    mix_fvn1a(&mut hash, &[0xf9]);
    mix_fvn1a(&mut hash, EXPLICIT_BILLING_PRICE_VERSION_SUFFIX.as_bytes());
    mix_fvn1a(&mut hash, &[0xf8]);

    let mut models = catalog.models.iter().collect::<Vec<_>>();
    models.sort_by(|(a, _), (b, _)| a.cmp(b));
    for (model, pricing) in models {
        mix_fvn1a(&mut hash, model.as_bytes());
        mix_fvn1a(&mut hash, &[0xfe]);
        mix_fvn1a(&mut hash, &pricing.input_per_1m.to_bits().to_le_bytes());
        mix_fvn1a(&mut hash, &pricing.output_per_1m.to_bits().to_le_bytes());

        match pricing.cache_input_per_1m {
            Some(value) => {
                mix_fvn1a(&mut hash, &[1]);
                mix_fvn1a(&mut hash, &value.to_bits().to_le_bytes());
            }
            None => mix_fvn1a(&mut hash, &[0]),
        }
        match pricing.reasoning_per_1m {
            Some(value) => {
                mix_fvn1a(&mut hash, &[1]);
                mix_fvn1a(&mut hash, &value.to_bits().to_le_bytes());
            }
            None => mix_fvn1a(&mut hash, &[0]),
        }
        mix_fvn1a(&mut hash, &[0xfd]);
    }

    format!("{}@{:016x}", catalog.version, hash)
}

fn estimate_proxy_cost(
    catalog: &PricingCatalog,
    model: Option<&str>,
    usage: &ParsedUsage,
    billing_service_tier: Option<&str>,
    pricing_mode: ProxyPricingMode,
) -> (Option<f64>, bool, Option<String>) {
    let price_version = Some(proxy_price_version(&catalog.version, pricing_mode));
    let Some(model) = model else {
        return (None, false, price_version);
    };
    let Some(pricing) = resolve_pricing_for_model(catalog, model) else {
        return (None, false, price_version);
    };
    let input_tokens = usage.input_tokens.unwrap_or(0).max(0);
    let output_tokens = usage.output_tokens.unwrap_or(0).max(0) as f64;
    let cache_input_tokens = usage.cache_input_tokens.unwrap_or(0).max(0);
    let reasoning_tokens = usage.reasoning_tokens.unwrap_or(0).max(0) as f64;
    if !has_billable_usage(usage) {
        return (None, false, price_version);
    }

    let apply_long_context_surcharge = is_gpt_5_4_long_context_surcharge_model(model)
        && input_tokens > GPT_5_4_LONG_CONTEXT_THRESHOLD_TOKENS;
    let apply_priority_billing_multiplier = billing_service_tier
        .and_then(normalize_service_tier)
        .as_deref()
        .is_some_and(|tier| tier == PRIORITY_SERVICE_TIER);

    let billable_cache_tokens = if pricing.cache_input_per_1m.is_some() {
        cache_input_tokens
    } else {
        0
    };
    let non_cached_input_tokens = input_tokens.saturating_sub(billable_cache_tokens);

    let non_cached_input_cost =
        (non_cached_input_tokens as f64 / 1_000_000.0) * pricing.input_per_1m;
    let cache_input_cost = pricing
        .cache_input_per_1m
        .map(|cache_price| (billable_cache_tokens as f64 / 1_000_000.0) * cache_price)
        .unwrap_or(0.0);
    let mut input_cost = non_cached_input_cost + cache_input_cost;

    let mut output_cost = (output_tokens / 1_000_000.0) * pricing.output_per_1m;

    let mut reasoning_cost = pricing
        .reasoning_per_1m
        .map(|reasoning_price| (reasoning_tokens / 1_000_000.0) * reasoning_price)
        .unwrap_or(0.0);

    if apply_long_context_surcharge {
        input_cost *= 2.0;
        output_cost *= 1.5;
        reasoning_cost *= 1.5;
    }

    if apply_priority_billing_multiplier {
        input_cost *= 2.0;
        output_cost *= 2.0;
        reasoning_cost *= 2.0;
    }

    let cost = input_cost + output_cost + reasoning_cost;

    (Some(cost), true, price_version)
}

fn store_raw_payload_file(
    config: &AppConfig,
    invoke_id: &str,
    kind: &str,
    bytes: &[u8],
) -> RawPayloadMeta {
    let mut meta = RawPayloadMeta {
        path: None,
        size_bytes: bytes.len() as i64,
        truncated: false,
        truncated_reason: None,
    };

    if bytes.is_empty() {
        return meta;
    }

    let mut write_len = bytes.len();
    if let Some(limit) = config.proxy_raw_max_bytes
        && write_len > limit
    {
        write_len = limit;
        meta.truncated = true;
        meta.truncated_reason = Some("max_bytes_exceeded".to_string());
    }
    let content = &bytes[..write_len];

    let raw_dir = config.resolved_proxy_raw_dir();

    if let Err(err) = fs::create_dir_all(&raw_dir) {
        meta.truncated = true;
        meta.truncated_reason = Some(format!("write_failed:{err}"));
        return meta;
    }

    let filename = format!("{invoke_id}-{kind}.bin");
    let path = raw_dir.join(filename);
    match fs::File::create(&path).and_then(|mut f| f.write_all(content)) {
        Ok(_) => {
            meta.path = Some(path.to_string_lossy().to_string());
        }
        Err(err) => {
            meta.truncated = true;
            meta.truncated_reason = Some(format!("write_failed:{err}"));
        }
    }
    meta
}

async fn broadcast_proxy_capture_follow_up(
    pool: &Pool<Sqlite>,
    broadcaster: &broadcast::Sender<BroadcastPayload>,
    broadcast_state_cache: &Mutex<BroadcastStateCache>,
    relay_config: Option<&CrsStatsConfig>,
    invocation_max_days: u64,
    invoke_id: &str,
) {
    if broadcaster.receiver_count() == 0 {
        return;
    }

    match collect_summary_snapshots(pool, relay_config, invocation_max_days).await {
        Ok(summaries) => {
            for summary in summaries {
                if let Err(err) = broadcast_summary_if_changed(
                    broadcaster,
                    broadcast_state_cache,
                    &summary.window,
                    summary.summary,
                )
                .await
                {
                    warn!(
                        ?err,
                        invoke_id = %invoke_id,
                        window = %summary.window,
                        "failed to broadcast proxy summary payload"
                    );
                }
            }
        }
        Err(err) => {
            warn!(
                ?err,
                invoke_id = %invoke_id,
                "failed to collect summary snapshots after proxy capture persistence"
            );
        }
    }

    if broadcaster.receiver_count() == 0 {
        return;
    }

    match QuotaSnapshotResponse::fetch_latest(pool).await {
        Ok(Some(snapshot)) => {
            if let Err(err) =
                broadcast_quota_if_changed(broadcaster, broadcast_state_cache, snapshot).await
            {
                warn!(
                    ?err,
                    invoke_id = %invoke_id,
                    "failed to broadcast proxy quota snapshot"
                );
            }
        }
        Ok(None) => {}
        Err(err) => {
            warn!(
                ?err,
                invoke_id = %invoke_id,
                "failed to fetch latest quota snapshot after proxy capture persistence"
            );
        }
    }
}

struct SummaryQuotaBroadcastIdleContext<'a> {
    latest_broadcast_seq: &'a AtomicU64,
    broadcast_running: &'a AtomicBool,
    shutdown: &'a CancellationToken,
    pool: &'a Pool<Sqlite>,
    broadcaster: &'a broadcast::Sender<BroadcastPayload>,
    broadcast_state_cache: &'a Mutex<BroadcastStateCache>,
    relay_config: Option<&'a CrsStatsConfig>,
    invocation_max_days: u64,
    invoke_id: &'a str,
}

async fn finish_summary_quota_broadcast_idle(
    ctx: SummaryQuotaBroadcastIdleContext<'_>,
    synced_seq: u64,
) -> bool {
    ctx.broadcast_running.store(false, Ordering::Release);

    let pending_seq = ctx.latest_broadcast_seq.load(Ordering::Acquire);
    if pending_seq == synced_seq {
        return false;
    }

    if ctx.shutdown.is_cancelled() {
        info!(
            invoke_id = %ctx.invoke_id,
            pending_seq,
            synced_seq,
            "flushing final summary/quota snapshots inline because shutdown arrived during broadcast worker idle handoff"
        );
        broadcast_proxy_capture_follow_up(
            ctx.pool,
            ctx.broadcaster,
            ctx.broadcast_state_cache,
            ctx.relay_config,
            ctx.invocation_max_days,
            ctx.invoke_id,
        )
        .await;
        return false;
    }

    ctx.broadcast_running
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_ok()
}

async fn persist_and_broadcast_proxy_capture(
    state: &AppState,
    capture_started: Instant,
    record: ProxyCaptureRecord,
) -> Result<()> {
    let inserted = persist_proxy_capture_record(&state.pool, capture_started, record).await?;
    let Some(inserted_record) = inserted else {
        return Ok(());
    };
    if inserted_record
        .prompt_cache_key
        .as_deref()
        .is_some_and(|key| !key.trim().is_empty())
    {
        invalidate_prompt_cache_conversations_cache(&state.prompt_cache_conversation_cache).await;
    }
    if state.broadcaster.receiver_count() == 0 {
        return Ok(());
    }

    let invoke_id = inserted_record.invoke_id.clone();
    if let Err(err) = state.broadcaster.send(BroadcastPayload::Records {
        records: vec![inserted_record],
    }) {
        warn!(
            ?err,
            invoke_id = %invoke_id,
            "failed to broadcast new proxy capture record"
        );
    }

    if state.shutdown.is_cancelled() {
        info!(
            invoke_id = %invoke_id,
            "broadcasting final summary/quota snapshots inline because shutdown is in progress"
        );
        broadcast_proxy_capture_follow_up(
            &state.pool,
            &state.broadcaster,
            state.broadcast_state_cache.as_ref(),
            state.config.crs_stats.as_ref(),
            state.config.invocation_max_days,
            &invoke_id,
        )
        .await;
        return Ok(());
    }

    state
        .proxy_summary_quota_broadcast_seq
        .fetch_add(1, Ordering::Relaxed);
    if state.shutdown.is_cancelled() {
        info!(
            invoke_id = %invoke_id,
            "broadcasting final summary/quota snapshots inline because shutdown started after record broadcast"
        );
        broadcast_proxy_capture_follow_up(
            &state.pool,
            &state.broadcaster,
            state.broadcast_state_cache.as_ref(),
            state.config.crs_stats.as_ref(),
            state.config.invocation_max_days,
            &invoke_id,
        )
        .await;
        return Ok(());
    }
    if state
        .proxy_summary_quota_broadcast_running
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return Ok(());
    }

    let latest_broadcast_seq = state.proxy_summary_quota_broadcast_seq.clone();
    let broadcast_running = state.proxy_summary_quota_broadcast_running.clone();
    let pool = state.pool.clone();
    let broadcaster = state.broadcaster.clone();
    let broadcast_state_cache = state.broadcast_state_cache.clone();
    let relay_config = state.config.crs_stats.clone();
    let invocation_max_days = state.config.invocation_max_days;
    let shutdown = state.shutdown.clone();
    let broadcast_handle_slot = state.proxy_summary_quota_broadcast_handle.clone();
    let handle = tokio::spawn(async move {
        let mut synced_seq = 0_u64;
        loop {
            let target_seq = latest_broadcast_seq.load(Ordering::Acquire);
            if shutdown.is_cancelled() {
                if target_seq != synced_seq {
                    info!(
                        invoke_id = %invoke_id,
                        "flushing final summary/quota snapshots inline before shutdown"
                    );
                    broadcast_proxy_capture_follow_up(
                        &pool,
                        &broadcaster,
                        broadcast_state_cache.as_ref(),
                        relay_config.as_ref(),
                        invocation_max_days,
                        &invoke_id,
                    )
                    .await;
                }
                broadcast_running.store(false, Ordering::Release);
                info!(
                    invoke_id = %invoke_id,
                    "stopping summary/quota broadcast worker because shutdown is in progress"
                );
                break;
            }

            if target_seq == synced_seq {
                if finish_summary_quota_broadcast_idle(
                    SummaryQuotaBroadcastIdleContext {
                        latest_broadcast_seq: latest_broadcast_seq.as_ref(),
                        broadcast_running: broadcast_running.as_ref(),
                        shutdown: &shutdown,
                        pool: &pool,
                        broadcaster: &broadcaster,
                        broadcast_state_cache: broadcast_state_cache.as_ref(),
                        relay_config: relay_config.as_ref(),
                        invocation_max_days,
                        invoke_id: &invoke_id,
                    },
                    synced_seq,
                )
                .await
                {
                    continue;
                }
                break;
            }
            synced_seq = target_seq;

            if broadcaster.receiver_count() == 0 {
                continue;
            }

            let summaries = tokio::select! {
                _ = shutdown.cancelled() => {
                    broadcast_proxy_capture_follow_up(
                        &pool,
                        &broadcaster,
                        broadcast_state_cache.as_ref(),
                        relay_config.as_ref(),
                        invocation_max_days,
                        &invoke_id,
                    )
                    .await;
                    broadcast_running.store(false, Ordering::Release);
                    info!(
                        invoke_id = %invoke_id,
                        "summary/quota broadcast worker flushed follow-up before collecting summaries during shutdown"
                    );
                    break;
                }
                result = collect_summary_snapshots(&pool, relay_config.as_ref(), invocation_max_days) => result,
            };
            match summaries {
                Ok(summaries) => {
                    for summary in summaries {
                        if let Err(err) = broadcast_summary_if_changed(
                            &broadcaster,
                            broadcast_state_cache.as_ref(),
                            &summary.window,
                            summary.summary,
                        )
                        .await
                        {
                            warn!(
                                ?err,
                                invoke_id = %invoke_id,
                                window = %summary.window,
                                "failed to broadcast proxy summary payload"
                            );
                        }
                    }
                }
                Err(err) => {
                    warn!(
                        ?err,
                        invoke_id = %invoke_id,
                        "failed to collect summary snapshots after proxy capture persistence"
                    );
                }
            }

            if broadcaster.receiver_count() == 0 {
                continue;
            }

            let quota = tokio::select! {
                _ = shutdown.cancelled() => {
                    broadcast_proxy_capture_follow_up(
                        &pool,
                        &broadcaster,
                        broadcast_state_cache.as_ref(),
                        relay_config.as_ref(),
                        invocation_max_days,
                        &invoke_id,
                    )
                    .await;
                    broadcast_running.store(false, Ordering::Release);
                    info!(
                        invoke_id = %invoke_id,
                        "summary/quota broadcast worker flushed follow-up before fetching quota during shutdown"
                    );
                    break;
                }
                result = QuotaSnapshotResponse::fetch_latest(&pool) => result,
            };
            match quota {
                Ok(Some(snapshot)) => {
                    if let Err(err) = broadcast_quota_if_changed(
                        &broadcaster,
                        broadcast_state_cache.as_ref(),
                        snapshot,
                    )
                    .await
                    {
                        warn!(
                            ?err,
                            invoke_id = %invoke_id,
                            "failed to broadcast proxy quota snapshot"
                        );
                    }
                }
                Ok(None) => {}
                Err(err) => {
                    warn!(
                        ?err,
                        invoke_id = %invoke_id,
                        "failed to fetch latest quota snapshot after proxy capture persistence"
                    );
                }
            }
        }
    });

    let finished_handles = {
        let mut guard = broadcast_handle_slot.lock().await;
        let mut active_handles = std::mem::take(&mut *guard);
        let mut finished_handles = Vec::new();
        let mut idx = 0;
        while idx < active_handles.len() {
            if active_handles[idx].is_finished() {
                finished_handles.push(active_handles.remove(idx));
            } else {
                idx += 1;
            }
        }
        active_handles.push(handle);
        *guard = active_handles;
        finished_handles
    };
    for finished_handle in finished_handles {
        if let Err(err) = finished_handle.await {
            error!(
                ?err,
                "summary/quota broadcast worker terminated unexpectedly"
            );
        }
    }

    Ok(())
}

async fn persist_proxy_capture_record(
    pool: &Pool<Sqlite>,
    capture_started: Instant,
    mut record: ProxyCaptureRecord,
) -> Result<Option<ApiInvocation>> {
    let failure = classify_invocation_failure(
        Some(record.status.as_str()),
        record.error_message.as_deref(),
    );
    let failure_kind = record
        .failure_kind
        .clone()
        .or_else(|| failure.failure_kind.clone());
    let persist_started = Instant::now();

    let mut tx = pool.begin().await?;
    let insert_result = sqlx::query(
        r#"
        INSERT OR IGNORE INTO codex_invocations (
            invoke_id,
            occurred_at,
            source,
            model,
            input_tokens,
            output_tokens,
            cache_input_tokens,
            reasoning_tokens,
            total_tokens,
            cost,
            cost_estimated,
            price_version,
            status,
            error_message,
            failure_kind,
            failure_class,
            is_actionable,
            payload,
            raw_response,
            request_raw_path,
            request_raw_size,
            request_raw_truncated,
            request_raw_truncated_reason,
            response_raw_path,
            response_raw_size,
            response_raw_truncated,
            response_raw_truncated_reason,
            t_total_ms,
            t_req_read_ms,
            t_req_parse_ms,
            t_upstream_connect_ms,
            t_upstream_ttfb_ms,
            t_upstream_stream_ms,
            t_resp_parse_ms,
            t_persist_ms
        )
        VALUES (
            ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19,
            ?20, ?21, ?22, ?23, ?24, ?25, ?26, ?27, ?28, ?29, ?30, ?31, ?32, ?33, ?34, ?35
        )
        "#,
    )
    .bind(&record.invoke_id)
    .bind(&record.occurred_at)
    .bind(SOURCE_PROXY)
    .bind(&record.model)
    .bind(record.usage.input_tokens)
    .bind(record.usage.output_tokens)
    .bind(record.usage.cache_input_tokens)
    .bind(record.usage.reasoning_tokens)
    .bind(record.usage.total_tokens)
    .bind(record.cost)
    .bind(record.cost_estimated as i64)
    .bind(record.price_version.as_deref())
    .bind(&record.status)
    .bind(record.error_message.as_deref())
    .bind(failure_kind.as_deref())
    .bind(failure.failure_class.as_str())
    .bind(failure.is_actionable as i64)
    .bind(record.payload.as_deref())
    .bind(&record.raw_response)
    .bind(record.req_raw.path.as_deref())
    .bind(record.req_raw.path.as_ref().map(|_| record.req_raw.size_bytes))
    .bind(record.req_raw.truncated as i64)
    .bind(record.req_raw.truncated_reason.as_deref())
    .bind(record.resp_raw.path.as_deref())
    .bind(record.resp_raw.path.as_ref().map(|_| record.resp_raw.size_bytes))
    .bind(record.resp_raw.truncated as i64)
    .bind(record.resp_raw.truncated_reason.as_deref())
    .bind(None::<f64>)
    .bind(record.timings.t_req_read_ms)
    .bind(record.timings.t_req_parse_ms)
    .bind(record.timings.t_upstream_connect_ms)
    .bind(record.timings.t_upstream_ttfb_ms)
    .bind(record.timings.t_upstream_stream_ms)
    .bind(record.timings.t_resp_parse_ms)
    .bind(None::<f64>)
    .execute(tx.as_mut())
    .await?;

    let (invocation_id, inserted_new_row) = if insert_result.rows_affected() > 0 {
        (insert_result.last_insert_rowid(), true)
    } else {
        let Some(existing) = load_persisted_invocation_identity_tx(
            tx.as_mut(),
            &record.invoke_id,
            &record.occurred_at,
        )
        .await?
        else {
            tx.commit().await?;
            return Ok(None);
        };
        if !invocation_status_is_in_flight(existing.status.as_deref()) {
            tx.commit().await?;
            return Ok(None);
        }

        let affected = sqlx::query(
            r#"
            UPDATE codex_invocations
            SET source = ?2,
                model = ?3,
                input_tokens = ?4,
                output_tokens = ?5,
                cache_input_tokens = ?6,
                reasoning_tokens = ?7,
                total_tokens = ?8,
                cost = ?9,
                cost_estimated = ?10,
                price_version = ?11,
                status = ?12,
                error_message = ?13,
                failure_kind = ?14,
                failure_class = ?15,
                is_actionable = ?16,
                payload = ?17,
                raw_response = ?18,
                request_raw_path = ?19,
                request_raw_size = ?20,
                request_raw_truncated = ?21,
                request_raw_truncated_reason = ?22,
                response_raw_path = ?23,
                response_raw_size = ?24,
                response_raw_truncated = ?25,
                response_raw_truncated_reason = ?26,
                t_total_ms = ?27,
                t_req_read_ms = ?28,
                t_req_parse_ms = ?29,
                t_upstream_connect_ms = ?30,
                t_upstream_ttfb_ms = ?31,
                t_upstream_stream_ms = ?32,
                t_resp_parse_ms = ?33,
                t_persist_ms = ?34
            WHERE id = ?1
              AND LOWER(TRIM(COALESCE(status, ''))) IN ('running', 'pending')
            "#,
        )
        .bind(existing.id)
        .bind(SOURCE_PROXY)
        .bind(&record.model)
        .bind(record.usage.input_tokens)
        .bind(record.usage.output_tokens)
        .bind(record.usage.cache_input_tokens)
        .bind(record.usage.reasoning_tokens)
        .bind(record.usage.total_tokens)
        .bind(record.cost)
        .bind(record.cost_estimated as i64)
        .bind(record.price_version.as_deref())
        .bind(&record.status)
        .bind(record.error_message.as_deref())
        .bind(failure_kind.as_deref())
        .bind(failure.failure_class.as_str())
        .bind(failure.is_actionable as i64)
        .bind(record.payload.as_deref())
        .bind(&record.raw_response)
        .bind(record.req_raw.path.as_deref())
        .bind(record.req_raw.path.as_ref().map(|_| record.req_raw.size_bytes))
        .bind(record.req_raw.truncated as i64)
        .bind(record.req_raw.truncated_reason.as_deref())
        .bind(record.resp_raw.path.as_deref())
        .bind(record.resp_raw.path.as_ref().map(|_| record.resp_raw.size_bytes))
        .bind(record.resp_raw.truncated as i64)
        .bind(record.resp_raw.truncated_reason.as_deref())
        .bind(None::<f64>)
        .bind(record.timings.t_req_read_ms)
        .bind(record.timings.t_req_parse_ms)
        .bind(record.timings.t_upstream_connect_ms)
        .bind(record.timings.t_upstream_ttfb_ms)
        .bind(record.timings.t_upstream_stream_ms)
        .bind(record.timings.t_resp_parse_ms)
        .bind(None::<f64>)
        .execute(tx.as_mut())
        .await?
        .rows_affected();
        if affected == 0 {
            tx.commit().await?;
            return Ok(None);
        }

        (existing.id, false)
    };

    touch_invocation_upstream_account_last_activity_tx(
        tx.as_mut(),
        &record.occurred_at,
        record.payload.as_deref(),
    )
    .await?;

    if inserted_new_row {
        upsert_invocation_hourly_rollups_tx(
            tx.as_mut(),
            &[InvocationHourlySourceRecord {
                id: invocation_id,
                occurred_at: record.occurred_at.clone(),
                source: SOURCE_PROXY.to_string(),
                status: Some(record.status.clone()),
                detail_level: DETAIL_LEVEL_FULL.to_string(),
                total_tokens: record.usage.total_tokens,
                cost: record.cost,
                error_message: record.error_message.clone(),
                failure_kind: failure_kind.clone(),
                failure_class: Some(failure.failure_class.as_str().to_string()),
                is_actionable: Some(failure.is_actionable as i64),
                payload: record.payload.clone(),
                t_total_ms: None,
                t_req_read_ms: Some(record.timings.t_req_read_ms),
                t_req_parse_ms: Some(record.timings.t_req_parse_ms),
                t_upstream_connect_ms: Some(record.timings.t_upstream_connect_ms),
                t_upstream_ttfb_ms: Some(record.timings.t_upstream_ttfb_ms),
                t_upstream_stream_ms: Some(record.timings.t_upstream_stream_ms),
                t_resp_parse_ms: Some(record.timings.t_resp_parse_ms),
                t_persist_ms: None,
            }],
            &INVOCATION_HOURLY_ROLLUP_TARGETS,
        )
        .await?;
    } else {
        recompute_invocation_hourly_rollups_for_ids_tx(tx.as_mut(), &[invocation_id]).await?;
    }

    save_hourly_rollup_live_progress_tx(
        tx.as_mut(),
        HOURLY_ROLLUP_DATASET_INVOCATIONS,
        invocation_id,
    )
    .await?;

    record.timings.t_persist_ms = elapsed_ms(persist_started);
    record.timings.t_total_ms = elapsed_ms(capture_started);

    sqlx::query(
        r#"
        UPDATE codex_invocations
        SET t_total_ms = ?2,
            t_persist_ms = ?3
        WHERE id = ?1
        "#,
    )
    .bind(invocation_id)
    .bind(record.timings.t_total_ms)
    .bind(record.timings.t_persist_ms)
    .execute(tx.as_mut())
    .await?;

    recompute_invocation_hourly_rollups_for_ids_tx(tx.as_mut(), &[invocation_id]).await?;

    let persisted =
        load_persisted_api_invocation_tx(tx.as_mut(), &record.invoke_id, &record.occurred_at)
            .await?;
    tx.commit().await?;

    Ok(Some(persisted))
}

fn read_proxy_raw_bytes(path: &str, fallback_root: Option<&Path>) -> io::Result<Vec<u8>> {
    let mut last_error = None;
    for candidate in resolved_raw_path_read_candidates(path, fallback_root) {
        match fs::read(&candidate) {
            Ok(content) => return decode_proxy_raw_file_bytes(&candidate, content),
            Err(err) if err.kind() == io::ErrorKind::NotFound => {
                last_error = Some(err);
            }
            Err(err) => return Err(err),
        }
    }
    Err(last_error.unwrap_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            format!("raw payload file not found for path {path}"),
        )
    }))
}

fn decode_proxy_raw_file_bytes(path: &Path, bytes: Vec<u8>) -> io::Result<Vec<u8>> {
    if path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("gz"))
    {
        let mut decoder = GzDecoder::new(bytes.as_slice());
        let mut decoded = Vec::new();
        decoder.read_to_end(&mut decoded).map_err(|err| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("failed to decompress raw payload {}: {err}", path.display()),
            )
        })?;
        Ok(decoded)
    } else {
        Ok(bytes)
    }
}

async fn current_proxy_usage_backfill_snapshot_max_id(pool: &Pool<Sqlite>) -> Result<i64> {
    Ok(sqlx::query_scalar(
        r#"
        SELECT COALESCE(MAX(id), 0)
        FROM codex_invocations
        WHERE source = ?1
          AND status = 'success'
          AND total_tokens IS NULL
          AND response_raw_path IS NOT NULL
        "#,
    )
    .bind(SOURCE_PROXY)
    .fetch_one(pool)
    .await?)
}

async fn backfill_proxy_usage_tokens_from_cursor(
    pool: &Pool<Sqlite>,
    start_after_id: i64,
    snapshot_max_id: i64,
    raw_path_fallback_root: Option<&Path>,
    scan_limit: Option<u64>,
    max_elapsed: Option<Duration>,
) -> Result<BackfillBatchOutcome<ProxyUsageBackfillSummary>> {
    let started_at = Instant::now();
    let mut summary = ProxyUsageBackfillSummary::default();
    let mut last_seen_id = start_after_id;
    let mut hit_budget = false;
    let mut samples = Vec::new();

    loop {
        if startup_backfill_budget_reached(started_at, summary.scanned, scan_limit, max_elapsed) {
            hit_budget = true;
            break;
        }

        let candidates = sqlx::query_as::<_, ProxyUsageBackfillCandidate>(
            r#"
            SELECT id, response_raw_path, payload
            FROM codex_invocations
            WHERE source = ?1
              AND status = 'success'
              AND total_tokens IS NULL
              AND response_raw_path IS NOT NULL
              AND id > ?2
              AND id <= ?3
            ORDER BY id ASC
            LIMIT ?4
            "#,
        )
        .bind(SOURCE_PROXY)
        .bind(last_seen_id)
        .bind(snapshot_max_id)
        .bind(startup_backfill_query_limit(summary.scanned, scan_limit))
        .fetch_all(pool)
        .await?;

        if candidates.is_empty() {
            break;
        }

        let mut updates = Vec::new();
        for candidate in candidates {
            last_seen_id = candidate.id;
            summary.scanned += 1;

            let raw_response =
                match read_proxy_raw_bytes(&candidate.response_raw_path, raw_path_fallback_root) {
                    Ok(content) => content,
                    Err(_) => {
                        summary.skipped_missing_file += 1;
                        push_backfill_sample(
                            &mut samples,
                            format!(
                                "id={} response_raw_path={} reason=missing_file",
                                candidate.id, candidate.response_raw_path
                            ),
                        );
                        continue;
                    }
                };

            let (target, is_stream) = parse_proxy_capture_summary(candidate.payload.as_deref());
            let (payload_for_parse, decode_error) =
                decode_response_payload_for_usage(&raw_response, None);
            let response_info =
                parse_target_response_payload(target, payload_for_parse.as_ref(), is_stream, None);
            let usage = response_info.usage;
            let has_usage = usage.total_tokens.is_some()
                || usage.input_tokens.is_some()
                || usage.output_tokens.is_some()
                || usage.cache_input_tokens.is_some()
                || usage.reasoning_tokens.is_some();
            if !has_usage {
                if decode_error.is_some() {
                    summary.skipped_decode_error += 1;
                } else {
                    summary.skipped_without_usage += 1;
                }
                continue;
            }

            updates.push(ProxyUsageBackfillUpdate {
                id: candidate.id,
                usage,
            });
        }

        if !updates.is_empty() {
            let mut tx = pool.begin().await?;
            let mut updated_this_batch = 0_u64;
            let mut updated_ids = Vec::new();
            for update in updates {
                let affected = sqlx::query(
                    r#"
                    UPDATE codex_invocations
                    SET input_tokens = ?1,
                        output_tokens = ?2,
                        cache_input_tokens = ?3,
                        reasoning_tokens = ?4,
                        total_tokens = ?5
                    WHERE id = ?6
                      AND source = ?7
                      AND total_tokens IS NULL
                    "#,
                )
                .bind(update.usage.input_tokens)
                .bind(update.usage.output_tokens)
                .bind(update.usage.cache_input_tokens)
                .bind(update.usage.reasoning_tokens)
                .bind(update.usage.total_tokens)
                .bind(update.id)
                .bind(SOURCE_PROXY)
                .execute(&mut *tx)
                .await?
                .rows_affected();
                updated_this_batch += affected;
                if affected > 0 {
                    updated_ids.push(update.id);
                }
            }
            if !updated_ids.is_empty() {
                recompute_invocation_hourly_rollups_for_ids_tx(tx.as_mut(), &updated_ids).await?;
            }
            tx.commit().await?;
            summary.updated += updated_this_batch;
        }
    }

    Ok(BackfillBatchOutcome {
        summary,
        next_cursor_id: last_seen_id,
        hit_budget,
        samples,
    })
}

#[cfg(test)]
async fn backfill_proxy_usage_tokens(
    pool: &Pool<Sqlite>,
    raw_path_fallback_root: Option<&Path>,
) -> Result<ProxyUsageBackfillSummary> {
    let snapshot_max_id = current_proxy_usage_backfill_snapshot_max_id(pool).await?;
    Ok(backfill_proxy_usage_tokens_from_cursor(
        pool,
        0,
        snapshot_max_id,
        raw_path_fallback_root,
        None,
        None,
    )
    .await?
    .summary)
}

#[cfg(test)]
async fn backfill_proxy_usage_tokens_up_to_id(
    pool: &Pool<Sqlite>,
    snapshot_max_id: i64,
    raw_path_fallback_root: Option<&Path>,
) -> Result<ProxyUsageBackfillSummary> {
    Ok(backfill_proxy_usage_tokens_from_cursor(
        pool,
        0,
        snapshot_max_id,
        raw_path_fallback_root,
        None,
        None,
    )
    .await?
    .summary)
}

#[cfg(test)]
async fn run_backfill_with_retry(
    pool: &Pool<Sqlite>,
    raw_path_fallback_root: Option<&Path>,
) -> Result<ProxyUsageBackfillSummary> {
    let mut attempt = 1_u32;
    loop {
        match backfill_proxy_usage_tokens(pool, raw_path_fallback_root).await {
            Ok(summary) => return Ok(summary),
            Err(err)
                if attempt < BACKFILL_LOCK_RETRY_MAX_ATTEMPTS && is_sqlite_lock_error(&err) =>
            {
                warn!(
                    attempt,
                    max_attempts = BACKFILL_LOCK_RETRY_MAX_ATTEMPTS,
                    retry_delay_secs = BACKFILL_LOCK_RETRY_DELAY_SECS,
                    error = %err,
                    "proxy usage startup backfill hit sqlite lock; retrying"
                );
                attempt += 1;
                sleep(Duration::from_secs(BACKFILL_LOCK_RETRY_DELAY_SECS)).await;
            }
            Err(err) => {
                return Err(err).with_context(|| {
                    format!(
                        "proxy usage startup backfill failed after {attempt}/{} attempt(s)",
                        BACKFILL_LOCK_RETRY_MAX_ATTEMPTS
                    )
                });
            }
        }
    }
}

async fn current_proxy_cost_backfill_snapshot_max_id(
    pool: &Pool<Sqlite>,
    attempt_version: &str,
    requested_tier_price_version: &str,
    response_tier_price_version: &str,
) -> Result<i64> {
    Ok(sqlx::query_scalar(
        r#"
        WITH base AS (
            SELECT
                inv.id,
                inv.cost,
                inv.price_version,
                CASE
                  WHEN json_valid(inv.payload) AND json_type(inv.payload, '$.requestedServiceTier') = 'text'
                    THEN json_extract(inv.payload, '$.requestedServiceTier')
                  WHEN json_valid(inv.payload) AND json_type(inv.payload, '$.requested_service_tier') = 'text'
                    THEN json_extract(inv.payload, '$.requested_service_tier')
                END AS requested_service_tier,
                CASE
                  WHEN json_valid(inv.payload) AND json_type(inv.payload, '$.billingServiceTier') = 'text'
                    THEN json_extract(inv.payload, '$.billingServiceTier')
                  WHEN json_valid(inv.payload) AND json_type(inv.payload, '$.billing_service_tier') = 'text'
                    THEN json_extract(inv.payload, '$.billing_service_tier')
                END AS billing_service_tier,
                CASE
                  WHEN json_valid(inv.payload) AND json_type(inv.payload, '$.serviceTier') = 'text'
                    THEN json_extract(inv.payload, '$.serviceTier')
                  WHEN json_valid(inv.payload) AND json_type(inv.payload, '$.service_tier') = 'text'
                    THEN json_extract(inv.payload, '$.service_tier')
                END AS service_tier,
                CASE
                  WHEN json_valid(inv.payload) AND json_type(inv.payload, '$.upstreamAccountKind') = 'text'
                    THEN json_extract(inv.payload, '$.upstreamAccountKind')
                  WHEN json_valid(inv.payload) AND json_type(inv.payload, '$.upstream_account_kind') = 'text'
                    THEN json_extract(inv.payload, '$.upstream_account_kind')
                END AS snapshot_upstream_account_kind,
                acc.kind AS live_upstream_account_kind,
                CASE
                  WHEN acc.created_at IS NOT NULL
                    AND TRIM(CAST(acc.created_at AS TEXT)) != ''
                    AND inv.occurred_at IS NOT NULL
                    AND TRIM(CAST(inv.occurred_at AS TEXT)) != ''
                    AND julianday(acc.created_at) <= julianday(inv.occurred_at)
                    AND (
                        acc.updated_at IS NULL
                        OR TRIM(CAST(acc.updated_at AS TEXT)) = ''
                        OR julianday(acc.updated_at) <= julianday(inv.occurred_at)
                    )
                  THEN 1
                  ELSE 0
                END AS live_upstream_account_snapshot_safe
            FROM codex_invocations inv
            LEFT JOIN pool_upstream_accounts acc
              ON acc.id = CASE
                  WHEN json_valid(inv.payload)
                    THEN CAST(json_extract(inv.payload, '$.upstreamAccountId') AS INTEGER)
                END
            WHERE inv.source = ?1
              AND LOWER(TRIM(COALESCE(inv.status, ''))) IN ('success', 'failed')
              AND inv.model IS NOT NULL
              AND (
                  COALESCE(inv.input_tokens, 0) > 0
                  OR COALESCE(inv.output_tokens, 0) > 0
                  OR COALESCE(inv.cache_input_tokens, 0) > 0
                  OR COALESCE(inv.reasoning_tokens, 0) > 0
              )
        ),
        cost_candidates AS (
            SELECT
                *,
                CASE
                  WHEN LOWER(TRIM(COALESCE(
                        snapshot_upstream_account_kind,
                        CASE WHEN live_upstream_account_snapshot_safe = 1 THEN live_upstream_account_kind END,
                        ''
                    ))) = ?4
                    AND TRIM(COALESCE(requested_service_tier, '')) != ''
                  THEN 1
                  ELSE 0
                END AS uses_requested_tier_strategy
            FROM base
        )
        SELECT COALESCE(MAX(id), 0)
        FROM cost_candidates
        WHERE (
            uses_requested_tier_strategy = 1
            AND (
                LOWER(TRIM(COALESCE(billing_service_tier, ''))) != LOWER(TRIM(COALESCE(requested_service_tier, '')))
                OR (cost IS NULL AND (price_version IS NULL OR price_version != ?2))
                OR (cost IS NOT NULL AND (price_version IS NULL OR price_version != ?3))
            )
        )
        OR (
            uses_requested_tier_strategy = 0
            AND (
                LOWER(TRIM(COALESCE(billing_service_tier, ''))) != LOWER(TRIM(COALESCE(service_tier, '')))
                OR (cost IS NULL AND (price_version IS NULL OR price_version != ?2))
                OR (cost IS NOT NULL AND (price_version IS NULL OR price_version != ?5))
            )
        )
        "#,
    )
    .bind(SOURCE_PROXY)
    .bind(attempt_version)
    .bind(requested_tier_price_version)
    .bind(API_KEYS_BILLING_ACCOUNT_KIND)
    .bind(response_tier_price_version)
    .fetch_one(pool)
    .await?)
}

async fn backfill_proxy_missing_costs_from_cursor(
    pool: &Pool<Sqlite>,
    start_after_id: i64,
    snapshot_max_id: i64,
    catalog: &PricingCatalog,
    attempt_version: &str,
    requested_tier_price_version: &str,
    response_tier_price_version: &str,
    scan_limit: Option<u64>,
    max_elapsed: Option<Duration>,
) -> Result<BackfillBatchOutcome<ProxyCostBackfillSummary>> {
    let started_at = Instant::now();
    let mut summary = ProxyCostBackfillSummary::default();
    let mut last_seen_id = start_after_id;
    let mut hit_budget = false;
    let mut samples = Vec::new();

    loop {
        if startup_backfill_budget_reached(started_at, summary.scanned, scan_limit, max_elapsed) {
            hit_budget = true;
            break;
        }

        let candidates = sqlx::query_as::<_, ProxyCostBackfillCandidate>(
            r#"
            WITH base AS (
                SELECT
                    inv.id,
                    inv.model,
                    inv.input_tokens,
                    inv.output_tokens,
                    inv.cache_input_tokens,
                    inv.reasoning_tokens,
                    inv.total_tokens,
                    inv.cost,
                    inv.price_version,
                    CASE
                      WHEN json_valid(inv.payload) AND json_type(inv.payload, '$.requestedServiceTier') = 'text'
                        THEN json_extract(inv.payload, '$.requestedServiceTier')
                      WHEN json_valid(inv.payload) AND json_type(inv.payload, '$.requested_service_tier') = 'text'
                        THEN json_extract(inv.payload, '$.requested_service_tier')
                    END AS requested_service_tier,
                    CASE
                      WHEN json_valid(inv.payload) AND json_type(inv.payload, '$.serviceTier') = 'text'
                        THEN json_extract(inv.payload, '$.serviceTier')
                      WHEN json_valid(inv.payload) AND json_type(inv.payload, '$.service_tier') = 'text'
                        THEN json_extract(inv.payload, '$.service_tier')
                    END AS service_tier,
                    CASE
                      WHEN json_valid(inv.payload) AND json_type(inv.payload, '$.billingServiceTier') = 'text'
                        THEN json_extract(inv.payload, '$.billingServiceTier')
                      WHEN json_valid(inv.payload) AND json_type(inv.payload, '$.billing_service_tier') = 'text'
                        THEN json_extract(inv.payload, '$.billing_service_tier')
                    END AS billing_service_tier,
                    CASE
                      WHEN json_valid(inv.payload) AND json_type(inv.payload, '$.upstreamAccountKind') = 'text'
                        THEN json_extract(inv.payload, '$.upstreamAccountKind')
                      WHEN json_valid(inv.payload) AND json_type(inv.payload, '$.upstream_account_kind') = 'text'
                        THEN json_extract(inv.payload, '$.upstream_account_kind')
                    END AS snapshot_upstream_account_kind,
                    CASE
                      WHEN json_valid(inv.payload) AND json_type(inv.payload, '$.upstreamBaseUrlHost') = 'text'
                        THEN json_extract(inv.payload, '$.upstreamBaseUrlHost')
                      WHEN json_valid(inv.payload) AND json_type(inv.payload, '$.upstream_base_url_host') = 'text'
                        THEN json_extract(inv.payload, '$.upstream_base_url_host')
                    END AS snapshot_upstream_base_url_host,
                    acc.kind AS live_upstream_account_kind,
                    CASE
                      WHEN acc.upstream_base_url IS NULL OR TRIM(CAST(acc.upstream_base_url AS TEXT)) = '' THEN NULL
                      ELSE
                        CASE
                          WHEN INSTR(
                            CASE
                              WHEN INSTR(REPLACE(REPLACE(LOWER(TRIM(CAST(acc.upstream_base_url AS TEXT))), 'https://', ''), 'http://', ''), '/') > 0
                                THEN SUBSTR(
                                  REPLACE(REPLACE(LOWER(TRIM(CAST(acc.upstream_base_url AS TEXT))), 'https://', ''), 'http://', ''),
                                  1,
                                  INSTR(REPLACE(REPLACE(LOWER(TRIM(CAST(acc.upstream_base_url AS TEXT))), 'https://', ''), 'http://', ''), '/') - 1
                                )
                              ELSE RTRIM(REPLACE(REPLACE(LOWER(TRIM(CAST(acc.upstream_base_url AS TEXT))), 'https://', ''), 'http://', ''), '/')
                            END,
                            ':'
                          ) > 0
                            THEN SUBSTR(
                              CASE
                                WHEN INSTR(REPLACE(REPLACE(LOWER(TRIM(CAST(acc.upstream_base_url AS TEXT))), 'https://', ''), 'http://', ''), '/') > 0
                                  THEN SUBSTR(
                                    REPLACE(REPLACE(LOWER(TRIM(CAST(acc.upstream_base_url AS TEXT))), 'https://', ''), 'http://', ''),
                                    1,
                                    INSTR(REPLACE(REPLACE(LOWER(TRIM(CAST(acc.upstream_base_url AS TEXT))), 'https://', ''), 'http://', ''), '/') - 1
                                  )
                                ELSE RTRIM(REPLACE(REPLACE(LOWER(TRIM(CAST(acc.upstream_base_url AS TEXT))), 'https://', ''), 'http://', ''), '/')
                              END,
                              1,
                              INSTR(
                                CASE
                                  WHEN INSTR(REPLACE(REPLACE(LOWER(TRIM(CAST(acc.upstream_base_url AS TEXT))), 'https://', ''), 'http://', ''), '/') > 0
                                    THEN SUBSTR(
                                      REPLACE(REPLACE(LOWER(TRIM(CAST(acc.upstream_base_url AS TEXT))), 'https://', ''), 'http://', ''),
                                      1,
                                      INSTR(REPLACE(REPLACE(LOWER(TRIM(CAST(acc.upstream_base_url AS TEXT))), 'https://', ''), 'http://', ''), '/') - 1
                                    )
                                  ELSE RTRIM(REPLACE(REPLACE(LOWER(TRIM(CAST(acc.upstream_base_url AS TEXT))), 'https://', ''), 'http://', ''), '/')
                                END,
                                ':'
                              ) - 1
                            )
                          ELSE
                            CASE
                              WHEN INSTR(REPLACE(REPLACE(LOWER(TRIM(CAST(acc.upstream_base_url AS TEXT))), 'https://', ''), 'http://', ''), '/') > 0
                                THEN SUBSTR(
                                  REPLACE(REPLACE(LOWER(TRIM(CAST(acc.upstream_base_url AS TEXT))), 'https://', ''), 'http://', ''),
                                  1,
                                  INSTR(REPLACE(REPLACE(LOWER(TRIM(CAST(acc.upstream_base_url AS TEXT))), 'https://', ''), 'http://', ''), '/') - 1
                                )
                              ELSE RTRIM(REPLACE(REPLACE(LOWER(TRIM(CAST(acc.upstream_base_url AS TEXT))), 'https://', ''), 'http://', ''), '/')
                            END
                        END
                    END AS live_upstream_base_url_host,
                    CASE
                      WHEN acc.created_at IS NOT NULL
                        AND TRIM(CAST(acc.created_at AS TEXT)) != ''
                        AND inv.occurred_at IS NOT NULL
                        AND TRIM(CAST(inv.occurred_at AS TEXT)) != ''
                        AND julianday(acc.created_at) <= julianday(inv.occurred_at)
                        AND (
                            acc.updated_at IS NULL
                            OR TRIM(CAST(acc.updated_at AS TEXT)) = ''
                            OR julianday(acc.updated_at) <= julianday(inv.occurred_at)
                        )
                      THEN 1
                      ELSE 0
                    END AS live_upstream_account_snapshot_safe
                FROM codex_invocations inv
                LEFT JOIN pool_upstream_accounts acc
                  ON acc.id = CASE
                      WHEN json_valid(inv.payload)
                        THEN CAST(json_extract(inv.payload, '$.upstreamAccountId') AS INTEGER)
                    END
                WHERE inv.source = ?1
                  AND LOWER(TRIM(COALESCE(inv.status, ''))) IN ('success', 'failed')
                  AND inv.model IS NOT NULL
                  AND (
                      COALESCE(inv.input_tokens, 0) > 0
                      OR COALESCE(inv.output_tokens, 0) > 0
                      OR COALESCE(inv.cache_input_tokens, 0) > 0
                      OR COALESCE(inv.reasoning_tokens, 0) > 0
                  )
                  AND inv.id > ?2
                  AND inv.id <= ?3
            ),
            cost_candidates AS (
                SELECT
                    *,
                    CASE
                      WHEN LOWER(TRIM(COALESCE(
                            snapshot_upstream_account_kind,
                            CASE WHEN live_upstream_account_snapshot_safe = 1 THEN live_upstream_account_kind END,
                            ''
                        ))) = ?6
                        AND TRIM(COALESCE(requested_service_tier, '')) != ''
                      THEN 1
                      ELSE 0
                    END AS uses_requested_tier_strategy
                FROM base
            )
            SELECT
                id,
                model,
                input_tokens,
                output_tokens,
                cache_input_tokens,
                reasoning_tokens,
                total_tokens,
                requested_service_tier,
                service_tier,
                snapshot_upstream_account_kind,
                snapshot_upstream_base_url_host,
                live_upstream_base_url_host,
                live_upstream_account_kind,
                live_upstream_account_snapshot_safe
            FROM cost_candidates
            WHERE (
                uses_requested_tier_strategy = 1
                AND (
                    LOWER(TRIM(COALESCE(billing_service_tier, ''))) != LOWER(TRIM(COALESCE(requested_service_tier, '')))
                    OR (cost IS NULL AND (price_version IS NULL OR price_version != ?4))
                    OR (cost IS NOT NULL AND (price_version IS NULL OR price_version != ?5))
                )
            )
            OR (
                uses_requested_tier_strategy = 0
                AND (
                    LOWER(TRIM(COALESCE(billing_service_tier, ''))) != LOWER(TRIM(COALESCE(service_tier, '')))
                    OR (cost IS NULL AND (price_version IS NULL OR price_version != ?4))
                    OR (cost IS NOT NULL AND (price_version IS NULL OR price_version != ?7))
                )
            )
            ORDER BY id ASC
            LIMIT ?8
            "#,
        )
        .bind(SOURCE_PROXY)
        .bind(last_seen_id)
        .bind(snapshot_max_id)
        .bind(attempt_version)
        .bind(requested_tier_price_version)
        .bind(API_KEYS_BILLING_ACCOUNT_KIND)
        .bind(response_tier_price_version)
        .bind(startup_backfill_query_limit(summary.scanned, scan_limit))
        .fetch_all(pool)
        .await?;

        if candidates.is_empty() {
            break;
        }

        let mut updates = Vec::new();
        for candidate in candidates {
            last_seen_id = candidate.id;
            summary.scanned += 1;
            let Some(model) = candidate.model.as_deref() else {
                summary.skipped_unpriced_model += 1;
                continue;
            };
            let usage = ParsedUsage {
                input_tokens: candidate.input_tokens,
                output_tokens: candidate.output_tokens,
                cache_input_tokens: candidate.cache_input_tokens,
                reasoning_tokens: candidate.reasoning_tokens,
                total_tokens: candidate.total_tokens,
            };
            if !has_billable_usage(&usage) {
                summary.skipped_unpriced_model += 1;
                continue;
            }

            let allow_live_fallback =
                allow_live_upstream_account_fallback(Some(
                    candidate.live_upstream_account_snapshot_safe,
                ));
            let upstream_account_kind = resolve_backfill_upstream_account_kind(
                candidate.snapshot_upstream_account_kind.as_deref(),
                candidate.live_upstream_account_kind.as_deref(),
                allow_live_fallback,
            );
            let upstream_base_url_host = resolve_backfill_upstream_base_url_host(
                candidate.snapshot_upstream_base_url_host.as_deref(),
                candidate.live_upstream_base_url_host.as_deref(),
                allow_live_fallback,
            );
            let (billing_service_tier, pricing_mode) =
                resolve_proxy_billing_service_tier_and_pricing_mode(
                    None,
                    candidate.requested_service_tier.as_deref(),
                    candidate.service_tier.as_deref(),
                    upstream_account_kind.as_deref(),
                );
            let (cost, cost_estimated, price_version) = estimate_proxy_cost(
                catalog,
                Some(model),
                &usage,
                billing_service_tier.as_deref(),
                pricing_mode,
            );
            if cost.is_none() || !cost_estimated {
                summary.skipped_unpriced_model += 1;
                push_backfill_sample(
                    &mut samples,
                    format!("id={} model={} reason=unpriced_model", candidate.id, model),
                );
            }
            let persisted_price_version = if cost_estimated && cost.is_some() {
                price_version
            } else {
                Some(attempt_version.to_string())
            };
            updates.push(ProxyCostBackfillUpdate {
                id: candidate.id,
                cost,
                cost_estimated,
                price_version: persisted_price_version,
                billing_service_tier,
                upstream_account_kind,
                upstream_base_url_host,
            });
        }

        if !updates.is_empty() {
            let mut tx = pool.begin().await?;
            let mut updated_this_batch = 0_u64;
            let mut updated_ids = Vec::new();
            for update in updates {
                let affected = sqlx::query(
                    r#"
                    UPDATE codex_invocations
                    SET payload = json_set(
                            json_set(
                                json_set(
                                    CASE WHEN json_valid(payload) THEN payload ELSE '{}' END,
                                    '$.billingServiceTier',
                                    ?1
                                ),
                                '$.upstreamAccountKind',
                                ?2
                            ),
                            '$.upstreamBaseUrlHost',
                            ?3
                        ),
                        cost = ?4,
                        cost_estimated = ?5,
                        price_version = ?6
                    WHERE id = ?7
                      AND source = ?8
                    "#,
                )
                .bind(update.billing_service_tier.as_deref())
                .bind(update.upstream_account_kind.as_deref())
                .bind(update.upstream_base_url_host.as_deref())
                .bind(update.cost)
                .bind(update.cost_estimated as i64)
                .bind(update.price_version.as_deref())
                .bind(update.id)
                .bind(SOURCE_PROXY)
                .execute(&mut *tx)
                .await?
                .rows_affected();
                updated_this_batch += affected;
                if affected > 0 {
                    updated_ids.push(update.id);
                }
            }
            if !updated_ids.is_empty() {
                recompute_invocation_hourly_rollups_for_ids_tx(tx.as_mut(), &updated_ids).await?;
            }
            tx.commit().await?;
            summary.updated += updated_this_batch;
        }
    }

    Ok(BackfillBatchOutcome {
        summary,
        next_cursor_id: last_seen_id,
        hit_budget,
        samples,
    })
}

#[cfg(test)]
async fn backfill_proxy_missing_costs(
    pool: &Pool<Sqlite>,
    catalog: &PricingCatalog,
) -> Result<ProxyCostBackfillSummary> {
    let attempt_version = pricing_backfill_attempt_version(catalog);
    let requested_tier_price_version =
        proxy_price_version(&catalog.version, ProxyPricingMode::RequestedTier);
    let response_tier_price_version =
        proxy_price_version(&catalog.version, ProxyPricingMode::ResponseTier);
    let snapshot_max_id = current_proxy_cost_backfill_snapshot_max_id(
        pool,
        &attempt_version,
        &requested_tier_price_version,
        &response_tier_price_version,
    )
    .await?;
    Ok(backfill_proxy_missing_costs_from_cursor(
        pool,
        0,
        snapshot_max_id,
        catalog,
        &attempt_version,
        &requested_tier_price_version,
        &response_tier_price_version,
        None,
        None,
    )
    .await?
    .summary)
}

#[cfg(test)]
#[allow(dead_code)]
async fn backfill_proxy_missing_costs_up_to_id(
    pool: &Pool<Sqlite>,
    snapshot_max_id: i64,
    catalog: &PricingCatalog,
    attempt_version: &str,
) -> Result<ProxyCostBackfillSummary> {
    let requested_tier_price_version =
        proxy_price_version(&catalog.version, ProxyPricingMode::RequestedTier);
    let response_tier_price_version =
        proxy_price_version(&catalog.version, ProxyPricingMode::ResponseTier);
    Ok(backfill_proxy_missing_costs_from_cursor(
        pool,
        0,
        snapshot_max_id,
        catalog,
        attempt_version,
        &requested_tier_price_version,
        &response_tier_price_version,
        None,
        None,
    )
    .await?
    .summary)
}

#[cfg(test)]
async fn run_cost_backfill_with_retry(
    pool: &Pool<Sqlite>,
    catalog: &PricingCatalog,
) -> Result<ProxyCostBackfillSummary> {
    let mut attempt = 1_u32;
    loop {
        match backfill_proxy_missing_costs(pool, catalog).await {
            Ok(summary) => return Ok(summary),
            Err(err)
                if attempt < BACKFILL_LOCK_RETRY_MAX_ATTEMPTS && is_sqlite_lock_error(&err) =>
            {
                warn!(
                    attempt,
                    max_attempts = BACKFILL_LOCK_RETRY_MAX_ATTEMPTS,
                    retry_delay_secs = BACKFILL_LOCK_RETRY_DELAY_SECS,
                    error = %err,
                    "proxy cost startup backfill hit sqlite lock; retrying"
                );
                attempt += 1;
                sleep(Duration::from_secs(BACKFILL_LOCK_RETRY_DELAY_SECS)).await;
            }
            Err(err) => {
                return Err(err).with_context(|| {
                    format!(
                        "proxy cost startup backfill failed after {attempt}/{} attempt(s)",
                        BACKFILL_LOCK_RETRY_MAX_ATTEMPTS
                    )
                });
            }
        }
    }
}

async fn backfill_proxy_prompt_cache_keys_from_cursor(
    pool: &Pool<Sqlite>,
    start_after_id: i64,
    raw_path_fallback_root: Option<&Path>,
    scan_limit: Option<u64>,
    max_elapsed: Option<Duration>,
) -> Result<BackfillBatchOutcome<ProxyPromptCacheKeyBackfillSummary>> {
    let started_at = Instant::now();
    let mut summary = ProxyPromptCacheKeyBackfillSummary::default();
    let mut last_seen_id = start_after_id;
    let mut hit_budget = false;
    let mut samples = Vec::new();

    loop {
        if startup_backfill_budget_reached(started_at, summary.scanned, scan_limit, max_elapsed) {
            hit_budget = true;
            break;
        }

        let candidates = sqlx::query_as::<_, ProxyPromptCacheKeyBackfillCandidate>(
            r#"
            SELECT id, request_raw_path
            FROM codex_invocations
            WHERE source = ?1
              AND request_raw_path IS NOT NULL
              AND id > ?2
              AND (
                payload IS NULL
                OR NOT json_valid(payload)
                OR json_extract(payload, '$.promptCacheKey') IS NULL
                OR TRIM(CAST(json_extract(payload, '$.promptCacheKey') AS TEXT)) = ''
              )
            ORDER BY id ASC
            LIMIT ?3
            "#,
        )
        .bind(SOURCE_PROXY)
        .bind(last_seen_id)
        .bind(startup_backfill_query_limit(summary.scanned, scan_limit))
        .fetch_all(pool)
        .await?;

        if candidates.is_empty() {
            break;
        }

        let mut updates = Vec::new();
        for candidate in candidates {
            last_seen_id = candidate.id;
            summary.scanned += 1;

            let raw_request =
                match read_proxy_raw_bytes(&candidate.request_raw_path, raw_path_fallback_root) {
                    Ok(content) => content,
                    Err(_) => {
                        summary.skipped_missing_file += 1;
                        push_backfill_sample(
                            &mut samples,
                            format!(
                                "id={} request_raw_path={} reason=missing_file",
                                candidate.id, candidate.request_raw_path
                            ),
                        );
                        continue;
                    }
                };

            let request_payload = match serde_json::from_slice::<Value>(&raw_request) {
                Ok(payload) => payload,
                Err(_) => {
                    summary.skipped_invalid_json += 1;
                    push_backfill_sample(
                        &mut samples,
                        format!(
                            "id={} request_raw_path={} reason=invalid_json",
                            candidate.id, candidate.request_raw_path
                        ),
                    );
                    continue;
                }
            };

            let Some(prompt_cache_key) =
                extract_prompt_cache_key_from_request_body(&request_payload)
            else {
                summary.skipped_missing_key += 1;
                continue;
            };
            updates.push((candidate.id, prompt_cache_key));
        }

        if !updates.is_empty() {
            let mut tx = pool.begin().await?;
            let mut updated_ids = Vec::new();
            for (id, prompt_cache_key) in updates {
                let affected = sqlx::query(
                    r#"
                    UPDATE codex_invocations
                    SET payload = json_remove(
                        json_set(
                            CASE WHEN json_valid(payload) THEN payload ELSE '{}' END,
                            '$.promptCacheKey',
                            ?1
                        ),
                        '$.codexSessionId'
                    )
                    WHERE id = ?2
                      AND source = ?3
                      AND request_raw_path IS NOT NULL
                      AND (
                        payload IS NULL
                        OR NOT json_valid(payload)
                        OR json_extract(payload, '$.promptCacheKey') IS NULL
                        OR TRIM(CAST(json_extract(payload, '$.promptCacheKey') AS TEXT)) = ''
                      )
                    "#,
                )
                .bind(prompt_cache_key)
                .bind(id)
                .bind(SOURCE_PROXY)
                .execute(&mut *tx)
                .await?
                .rows_affected();
                summary.updated += affected;
                if affected > 0 {
                    updated_ids.push(id);
                }
            }
            if !updated_ids.is_empty() {
                recompute_invocation_hourly_rollups_for_ids_tx(tx.as_mut(), &updated_ids).await?;
            }
            tx.commit().await?;
        }
    }

    Ok(BackfillBatchOutcome {
        summary,
        next_cursor_id: last_seen_id,
        hit_budget,
        samples,
    })
}

#[cfg(test)]
async fn backfill_proxy_prompt_cache_keys(
    pool: &Pool<Sqlite>,
    raw_path_fallback_root: Option<&Path>,
) -> Result<ProxyPromptCacheKeyBackfillSummary> {
    Ok(
        backfill_proxy_prompt_cache_keys_from_cursor(pool, 0, raw_path_fallback_root, None, None)
            .await?
            .summary,
    )
}

async fn backfill_proxy_requested_service_tiers_from_cursor(
    pool: &Pool<Sqlite>,
    start_after_id: i64,
    raw_path_fallback_root: Option<&Path>,
    scan_limit: Option<u64>,
    max_elapsed: Option<Duration>,
) -> Result<BackfillBatchOutcome<ProxyRequestedServiceTierBackfillSummary>> {
    let started_at = Instant::now();
    let mut summary = ProxyRequestedServiceTierBackfillSummary::default();
    let mut last_seen_id = start_after_id;
    let mut hit_budget = false;
    let mut samples = Vec::new();

    loop {
        if startup_backfill_budget_reached(started_at, summary.scanned, scan_limit, max_elapsed) {
            hit_budget = true;
            break;
        }

        let candidates = sqlx::query_as::<_, ProxyRequestedServiceTierBackfillCandidate>(
            r#"
            SELECT id, request_raw_path
            FROM codex_invocations
            WHERE source = ?1
              AND request_raw_path IS NOT NULL
              AND id > ?2
              AND (
                payload IS NULL
                OR NOT json_valid(payload)
                OR json_extract(payload, '$.requestedServiceTier') IS NULL
                OR TRIM(CAST(json_extract(payload, '$.requestedServiceTier') AS TEXT)) = ''
              )
            ORDER BY id ASC
            LIMIT ?3
            "#,
        )
        .bind(SOURCE_PROXY)
        .bind(last_seen_id)
        .bind(startup_backfill_query_limit(summary.scanned, scan_limit))
        .fetch_all(pool)
        .await?;

        if candidates.is_empty() {
            break;
        }

        for candidate in candidates {
            last_seen_id = candidate.id;
            summary.scanned += 1;

            let raw_request =
                match read_proxy_raw_bytes(&candidate.request_raw_path, raw_path_fallback_root) {
                    Ok(content) => content,
                    Err(_) => {
                        summary.skipped_missing_file += 1;
                        push_backfill_sample(
                            &mut samples,
                            format!(
                                "id={} request_raw_path={} reason=missing_file",
                                candidate.id, candidate.request_raw_path
                            ),
                        );
                        continue;
                    }
                };

            let request_payload = match serde_json::from_slice::<Value>(&raw_request) {
                Ok(payload) => payload,
                Err(_) => {
                    summary.skipped_invalid_json += 1;
                    push_backfill_sample(
                        &mut samples,
                        format!(
                            "id={} request_raw_path={} reason=invalid_json",
                            candidate.id, candidate.request_raw_path
                        ),
                    );
                    continue;
                }
            };

            let Some(requested_service_tier) =
                extract_requested_service_tier_from_request_body(&request_payload)
            else {
                summary.skipped_missing_tier += 1;
                continue;
            };

            let affected = sqlx::query(
                r#"
                UPDATE codex_invocations
                SET payload = json_set(
                    CASE WHEN json_valid(payload) THEN payload ELSE '{}' END,
                    '$.requestedServiceTier',
                    ?1
                )
                WHERE id = ?2
                  AND source = ?3
                  AND request_raw_path IS NOT NULL
                  AND (
                    payload IS NULL
                    OR NOT json_valid(payload)
                    OR json_extract(payload, '$.requestedServiceTier') IS NULL
                    OR TRIM(CAST(json_extract(payload, '$.requestedServiceTier') AS TEXT)) = ''
                  )
                "#,
            )
            .bind(requested_service_tier)
            .bind(candidate.id)
            .bind(SOURCE_PROXY)
            .execute(pool)
            .await?
            .rows_affected();
            summary.updated += affected;
        }
    }

    Ok(BackfillBatchOutcome {
        summary,
        next_cursor_id: last_seen_id,
        hit_budget,
        samples,
    })
}

#[cfg(test)]
async fn backfill_proxy_requested_service_tiers(
    pool: &Pool<Sqlite>,
    raw_path_fallback_root: Option<&Path>,
) -> Result<ProxyRequestedServiceTierBackfillSummary> {
    Ok(backfill_proxy_requested_service_tiers_from_cursor(
        pool,
        0,
        raw_path_fallback_root,
        None,
        None,
    )
    .await?
    .summary)
}

async fn backfill_proxy_reasoning_efforts_from_cursor(
    pool: &Pool<Sqlite>,
    start_after_id: i64,
    raw_path_fallback_root: Option<&Path>,
    scan_limit: Option<u64>,
    max_elapsed: Option<Duration>,
) -> Result<BackfillBatchOutcome<ProxyReasoningEffortBackfillSummary>> {
    let started_at = Instant::now();
    let mut summary = ProxyReasoningEffortBackfillSummary::default();
    let mut last_seen_id = start_after_id;
    let mut hit_budget = false;
    let mut samples = Vec::new();

    loop {
        if startup_backfill_budget_reached(started_at, summary.scanned, scan_limit, max_elapsed) {
            hit_budget = true;
            break;
        }

        let candidates = sqlx::query_as::<_, ProxyReasoningEffortBackfillCandidate>(
            r#"
            SELECT id, request_raw_path
            FROM codex_invocations
            WHERE source = ?1
              AND request_raw_path IS NOT NULL
              AND id > ?2
              AND (
                payload IS NULL
                OR NOT json_valid(payload)
                OR json_extract(payload, '$.reasoningEffort') IS NULL
                OR TRIM(CAST(json_extract(payload, '$.reasoningEffort') AS TEXT)) = ''
              )
            ORDER BY id ASC
            LIMIT ?3
            "#,
        )
        .bind(SOURCE_PROXY)
        .bind(last_seen_id)
        .bind(startup_backfill_query_limit(summary.scanned, scan_limit))
        .fetch_all(pool)
        .await?;

        if candidates.is_empty() {
            break;
        }

        for candidate in candidates {
            last_seen_id = candidate.id;
            summary.scanned += 1;

            let raw_request =
                match read_proxy_raw_bytes(&candidate.request_raw_path, raw_path_fallback_root) {
                    Ok(content) => content,
                    Err(_) => {
                        summary.skipped_missing_file += 1;
                        push_backfill_sample(
                            &mut samples,
                            format!(
                                "id={} request_raw_path={} reason=missing_file",
                                candidate.id, candidate.request_raw_path
                            ),
                        );
                        continue;
                    }
                };

            let request_payload = match serde_json::from_slice::<Value>(&raw_request) {
                Ok(payload) => payload,
                Err(_) => {
                    summary.skipped_invalid_json += 1;
                    push_backfill_sample(
                        &mut samples,
                        format!(
                            "id={} request_raw_path={} reason=invalid_json",
                            candidate.id, candidate.request_raw_path
                        ),
                    );
                    continue;
                }
            };

            let Some(reasoning_effort) = extract_reasoning_effort_from_request_body(
                infer_proxy_capture_target_from_payload(&request_payload),
                &request_payload,
            ) else {
                summary.skipped_missing_effort += 1;
                continue;
            };

            let affected = sqlx::query(
                r#"
                UPDATE codex_invocations
                SET payload = json_set(
                    CASE WHEN json_valid(payload) THEN payload ELSE '{}' END,
                    '$.reasoningEffort',
                    ?1
                )
                WHERE id = ?2
                  AND source = ?3
                  AND request_raw_path IS NOT NULL
                  AND (
                    payload IS NULL
                    OR NOT json_valid(payload)
                    OR json_extract(payload, '$.reasoningEffort') IS NULL
                    OR TRIM(CAST(json_extract(payload, '$.reasoningEffort') AS TEXT)) = ''
                  )
                "#,
            )
            .bind(reasoning_effort)
            .bind(candidate.id)
            .bind(SOURCE_PROXY)
            .execute(pool)
            .await?
            .rows_affected();
            summary.updated += affected;
        }
    }

    Ok(BackfillBatchOutcome {
        summary,
        next_cursor_id: last_seen_id,
        hit_budget,
        samples,
    })
}

#[cfg(test)]
async fn backfill_proxy_reasoning_efforts(
    pool: &Pool<Sqlite>,
    raw_path_fallback_root: Option<&Path>,
) -> Result<ProxyReasoningEffortBackfillSummary> {
    Ok(
        backfill_proxy_reasoning_efforts_from_cursor(pool, 0, raw_path_fallback_root, None, None)
            .await?
            .summary,
    )
}

fn infer_proxy_capture_target_from_payload(value: &Value) -> ProxyCaptureTarget {
    if value.get("messages").is_some() || value.get("reasoning_effort").is_some() {
        ProxyCaptureTarget::ChatCompletions
    } else if value.get("previous_response_id").is_some() {
        ProxyCaptureTarget::ResponsesCompact
    } else {
        ProxyCaptureTarget::Responses
    }
}

#[derive(Debug, FromRow)]
struct InvocationServiceTierBackfillCandidate {
    id: i64,
    source: String,
    raw_response: String,
    response_raw_path: Option<String>,
    current_service_tier: Option<String>,
}

async fn backfill_invocation_service_tiers_from_cursor(
    pool: &Pool<Sqlite>,
    start_after_id: i64,
    raw_path_fallback_root: Option<&Path>,
    scan_limit: Option<u64>,
    max_elapsed: Option<Duration>,
) -> Result<BackfillBatchOutcome<InvocationServiceTierBackfillSummary>> {
    let started_at = Instant::now();
    let mut summary = InvocationServiceTierBackfillSummary::default();
    let mut last_seen_id = start_after_id;
    let mut hit_budget = false;
    let mut samples = Vec::new();

    loop {
        if startup_backfill_budget_reached(started_at, summary.scanned, scan_limit, max_elapsed) {
            hit_budget = true;
            break;
        }

        let candidates = sqlx::query_as::<_, InvocationServiceTierBackfillCandidate>(
            r#"
            SELECT
                id,
                source,
                raw_response,
                response_raw_path,
                CASE
                  WHEN json_valid(payload) AND json_type(payload, '$.serviceTier') = 'text'
                    THEN json_extract(payload, '$.serviceTier')
                  WHEN json_valid(payload) AND json_type(payload, '$.service_tier') = 'text'
                    THEN json_extract(payload, '$.service_tier')
                END AS current_service_tier
            FROM codex_invocations
            WHERE id > ?1
              AND (
                payload IS NULL
                OR NOT json_valid(payload)
                OR COALESCE(json_extract(payload, '$.serviceTier'), json_extract(payload, '$.service_tier')) IS NULL
                OR TRIM(CAST(COALESCE(json_extract(payload, '$.serviceTier'), json_extract(payload, '$.service_tier')) AS TEXT)) = ''
                OR (
                    source = ?2
                    AND COALESCE(
                        CASE
                          WHEN json_valid(payload) AND json_type(payload, '$.serviceTierBackfillVersion') = 'text'
                            THEN json_extract(payload, '$.serviceTierBackfillVersion')
                          WHEN json_valid(payload) AND json_type(payload, '$.service_tier_backfill_version') = 'text'
                            THEN json_extract(payload, '$.service_tier_backfill_version')
                        END,
                        ''
                    ) != ?3
                    AND (
                        response_raw_path IS NOT NULL
                        OR INSTR(LOWER(COALESCE(raw_response, '')), 'service_tier') > 0
                        OR INSTR(LOWER(COALESCE(raw_response, '')), 'servicetier') > 0
                        OR INSTR(LOWER(COALESCE(raw_response, '')), 'response.completed') > 0
                        OR INSTR(LOWER(COALESCE(raw_response, '')), 'response.failed') > 0
                        OR INSTR(LOWER(COALESCE(raw_response, '')), 'response.created') > 0
                        OR INSTR(LOWER(COALESCE(raw_response, '')), 'response.in_progress') > 0
                    )
                )
              )
            ORDER BY id ASC
            LIMIT ?4
            "#,
        )
        .bind(last_seen_id)
        .bind(SOURCE_PROXY)
        .bind(SERVICE_TIER_STREAM_BACKFILL_VERSION)
        .bind(startup_backfill_query_limit(summary.scanned, scan_limit))
        .fetch_all(pool)
        .await?;

        if candidates.is_empty() {
            break;
        }

        for candidate in candidates {
            last_seen_id = candidate.id;
            summary.scanned += 1;

            let mut service_tier = parse_target_response_payload(
                ProxyCaptureTarget::Responses,
                candidate.raw_response.as_bytes(),
                false,
                None,
            )
            .service_tier;

            if service_tier.is_none()
                && candidate.source == SOURCE_PROXY
                && let Some(path) = candidate.response_raw_path.as_deref()
            {
                match read_proxy_raw_bytes(path, raw_path_fallback_root) {
                    Ok(bytes) => {
                        let (payload_for_parse, _) =
                            decode_response_payload_for_usage(&bytes, None);
                        service_tier = parse_target_response_payload(
                            ProxyCaptureTarget::Responses,
                            payload_for_parse.as_ref(),
                            false,
                            None,
                        )
                        .service_tier;
                    }
                    Err(_) => {
                        summary.skipped_missing_file += 1;
                        push_backfill_sample(
                            &mut samples,
                            format!(
                                "id={} response_raw_path={} reason=missing_file",
                                candidate.id, path
                            ),
                        );
                        continue;
                    }
                }
            }

            let Some(service_tier) = service_tier else {
                summary.skipped_missing_tier += 1;
                continue;
            };

            let should_mark_stream_backfill = candidate.source == SOURCE_PROXY;
            if candidate
                .current_service_tier
                .as_deref()
                .and_then(normalize_service_tier)
                .is_some_and(|current| current == service_tier)
                && !should_mark_stream_backfill
            {
                continue;
            }

            let affected = sqlx::query(
                r#"
                UPDATE codex_invocations
                SET payload = CASE
                    WHEN ?3 IS NULL THEN json_set(
                        CASE WHEN json_valid(payload) THEN payload ELSE '{}' END,
                        '$.serviceTier',
                        ?1
                    )
                    ELSE json_set(
                        json_set(
                            CASE WHEN json_valid(payload) THEN payload ELSE '{}' END,
                            '$.serviceTier',
                            ?1
                        ),
                        '$.serviceTierBackfillVersion',
                        ?3
                    )
                END
                WHERE id = ?2
                "#,
            )
            .bind(&service_tier)
            .bind(candidate.id)
            .bind(should_mark_stream_backfill.then_some(SERVICE_TIER_STREAM_BACKFILL_VERSION))
            .execute(pool)
            .await?
            .rows_affected();
            summary.updated += affected;
        }
    }

    Ok(BackfillBatchOutcome {
        summary,
        next_cursor_id: last_seen_id,
        hit_budget,
        samples,
    })
}

#[cfg(test)]
async fn backfill_invocation_service_tiers(
    pool: &Pool<Sqlite>,
    raw_path_fallback_root: Option<&Path>,
) -> Result<InvocationServiceTierBackfillSummary> {
    Ok(
        backfill_invocation_service_tiers_from_cursor(pool, 0, raw_path_fallback_root, None, None)
            .await?
            .summary,
    )
}

#[derive(Debug, FromRow)]
struct FailureClassificationBackfillRow {
    id: i64,
    source: String,
    status: Option<String>,
    error_message: Option<String>,
    failure_kind: Option<String>,
    failure_class: Option<String>,
    is_actionable: Option<i64>,
    payload: Option<String>,
    raw_response: String,
    response_raw_path: Option<String>,
}

fn parse_proxy_response_capture_from_stored_bytes(
    target: ProxyCaptureTarget,
    bytes: &[u8],
    is_stream: bool,
) -> ResponseCaptureInfo {
    let (payload_for_parse, _) = decode_response_payload_for_usage(bytes, None);
    parse_target_response_payload(target, payload_for_parse.as_ref(), is_stream, None)
}

fn format_upstream_response_failed_message(response_info: &ResponseCaptureInfo) -> String {
    let upstream_message = response_info
        .upstream_error_message
        .as_deref()
        .unwrap_or("upstream response failed");
    if let Some(code) = response_info.upstream_error_code.as_deref() {
        format!(
            "[{}] {}: {}",
            PROXY_FAILURE_UPSTREAM_RESPONSE_FAILED, code, upstream_message
        )
    } else {
        format!(
            "[{}] {}",
            PROXY_FAILURE_UPSTREAM_RESPONSE_FAILED, upstream_message
        )
    }
}

fn update_proxy_payload_failure_details(
    payload: Option<&str>,
    failure_kind: Option<&str>,
    response_info: &ResponseCaptureInfo,
) -> String {
    let mut value = payload
        .and_then(|raw| serde_json::from_str::<Value>(raw).ok())
        .filter(|value| value.is_object())
        .unwrap_or_else(|| json!({}));
    let object = value
        .as_object_mut()
        .expect("payload summary must be an object");

    object.insert(
        "failureKind".to_string(),
        failure_kind
            .map(|value| Value::String(value.to_string()))
            .unwrap_or(Value::Null),
    );
    object.insert(
        "streamTerminalEvent".to_string(),
        response_info
            .stream_terminal_event
            .as_ref()
            .map(|value| Value::String(value.clone()))
            .unwrap_or(Value::Null),
    );
    object.insert(
        "upstreamErrorCode".to_string(),
        response_info
            .upstream_error_code
            .as_ref()
            .map(|value| Value::String(value.clone()))
            .unwrap_or(Value::Null),
    );
    object.insert(
        "upstreamErrorMessage".to_string(),
        response_info
            .upstream_error_message
            .as_ref()
            .map(|value| Value::String(value.clone()))
            .unwrap_or(Value::Null),
    );
    object.insert(
        "upstreamRequestId".to_string(),
        response_info
            .upstream_request_id
            .as_ref()
            .map(|value| Value::String(value.clone()))
            .unwrap_or(Value::Null),
    );
    object.insert(
        "usageMissingReason".to_string(),
        response_info
            .usage_missing_reason
            .as_ref()
            .map(|value| Value::String(value.clone()))
            .unwrap_or(Value::Null),
    );

    serde_json::to_string(&value).unwrap_or_else(|_| "{}".to_string())
}

fn should_upgrade_to_upstream_response_failed(
    row: &FailureClassificationBackfillRow,
    existing_kind: Option<&str>,
) -> bool {
    if matches!(
        existing_kind,
        Some(PROXY_STREAM_TERMINAL_DOWNSTREAM_CLOSED)
            | Some(PROXY_FAILURE_UPSTREAM_STREAM_ERROR)
            | Some(PROXY_FAILURE_FAILED_CONTACT_UPSTREAM)
            | Some(PROXY_FAILURE_UPSTREAM_HANDSHAKE_TIMEOUT)
            | Some(PROXY_FAILURE_REQUEST_BODY_READ_TIMEOUT)
            | Some(PROXY_FAILURE_REQUEST_BODY_STREAM_ERROR_CLIENT_CLOSED)
    ) {
        return false;
    }

    invocation_status_is_success_like(row.status.as_deref(), row.error_message.as_deref())
        || existing_kind.is_none()
        || existing_kind == Some(PROXY_FAILURE_UPSTREAM_RESPONSE_FAILED)
}

fn parse_proxy_response_failure_from_persisted_record(
    row: &FailureClassificationBackfillRow,
    raw_path_fallback_root: Option<&Path>,
) -> Result<Option<ResponseCaptureInfo>> {
    if row.source != SOURCE_PROXY {
        return Ok(None);
    }

    let (target, is_stream) = parse_proxy_capture_summary(row.payload.as_deref());
    let preview_info = parse_proxy_response_capture_from_stored_bytes(
        target,
        row.raw_response.as_bytes(),
        is_stream,
    );
    let preview_has_failure = preview_info.stream_terminal_event.is_some();
    let preview_is_complete = preview_has_failure
        && preview_info.upstream_error_message.is_some()
        && preview_info.upstream_request_id.is_some();

    if preview_is_complete || row.response_raw_path.is_none() {
        return Ok(preview_has_failure.then_some(preview_info));
    }

    let Some(path) = row.response_raw_path.as_deref() else {
        return Ok(preview_has_failure.then_some(preview_info));
    };

    match read_proxy_raw_bytes(path, raw_path_fallback_root) {
        Ok(bytes) => {
            let full_info =
                parse_proxy_response_capture_from_stored_bytes(target, &bytes, is_stream);
            if full_info.stream_terminal_event.is_some() {
                Ok(Some(full_info))
            } else {
                Ok(preview_has_failure.then_some(preview_info))
            }
        }
        Err(_err) if preview_has_failure => Ok(Some(preview_info)),
        Err(err) => Err(err.into()),
    }
}

async fn backfill_failure_classification_from_cursor(
    pool: &Pool<Sqlite>,
    start_after_id: i64,
    raw_path_fallback_root: Option<&Path>,
    scan_limit: Option<u64>,
    max_elapsed: Option<Duration>,
) -> Result<BackfillBatchOutcome<FailureClassificationBackfillSummary>> {
    let started_at = Instant::now();
    let mut summary = FailureClassificationBackfillSummary::default();
    let mut last_seen_id = start_after_id;
    let mut hit_budget = false;
    let mut samples = Vec::new();

    loop {
        if startup_backfill_budget_reached(started_at, summary.scanned, scan_limit, max_elapsed) {
            hit_budget = true;
            break;
        }

        let rows = sqlx::query_as::<_, FailureClassificationBackfillRow>(
            r#"
            SELECT
                id,
                source,
                status,
                error_message,
                failure_kind,
                failure_class,
                is_actionable,
                payload,
                raw_response,
                response_raw_path
            FROM codex_invocations
            WHERE id > ?1
              AND (
                failure_class IS NULL
                OR TRIM(COALESCE(failure_class, '')) = ''
                OR is_actionable IS NULL
                OR (
                    LOWER(TRIM(COALESCE(status, ''))) != 'success'
                    AND TRIM(COALESCE(status, '')) != ''
                    AND TRIM(COALESCE(failure_kind, '')) = ''
                )
                OR (
                    LOWER(TRIM(COALESCE(status, ''))) != 'success'
                    AND TRIM(COALESCE(failure_class, '')) = 'none'
                )
                OR (
                    source = ?2
                    AND LOWER(TRIM(COALESCE(status, ''))) = 'success'
                    AND (
                        raw_response LIKE '%response.failed%'
                        OR raw_response LIKE '%"type":"error"%'
                        OR (
                            json_valid(payload)
                            AND (
                                TRIM(COALESCE(CAST(json_extract(payload, '$.usageMissingReason') AS TEXT), '')) IN ('usage_missing_in_stream', 'upstream_response_failed')
                                OR TRIM(COALESCE(CAST(json_extract(payload, '$.streamTerminalEvent') AS TEXT), '')) != ''
                            )
                        )
                        OR (
                            response_raw_path IS NOT NULL
                            AND COALESCE(response_raw_size, LENGTH(raw_response)) >= 16384
                            AND json_valid(payload)
                            AND COALESCE(CAST(json_extract(payload, '$.endpoint') AS TEXT), '') = '/v1/responses'
                            AND COALESCE(json_extract(payload, '$.isStream'), 0) = 1
                            AND TRIM(COALESCE(failure_kind, '')) = ''
                        )
                    )
                )
              )
            ORDER BY id ASC
            LIMIT ?3
            "#,
        )
        .bind(last_seen_id)
        .bind(SOURCE_PROXY)
        .bind(startup_backfill_query_limit(summary.scanned, scan_limit))
        .fetch_all(pool)
        .await?;

        if rows.is_empty() {
            break;
        }

        if let Some(last) = rows.last() {
            last_seen_id = last.id;
        }
        summary.scanned += rows.len() as u64;

        let mut tx = pool.begin().await?;
        let mut updated_ids = Vec::new();
        for row in rows {
            let existing_kind = row
                .failure_kind
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned);
            let existing_class = row
                .failure_class
                .as_deref()
                .and_then(FailureClass::from_db_str);
            let existing_actionable = row.is_actionable.map(|value| value != 0);

            let response_failure = match parse_proxy_response_failure_from_persisted_record(
                &row,
                raw_path_fallback_root,
            ) {
                Ok(result) => result,
                Err(err) => {
                    push_backfill_sample(
                        &mut samples,
                        format!(
                            "id={} reason=response_failure_parse_error err={err}",
                            row.id
                        ),
                    );
                    None
                }
            };

            if let Some(response_info) = response_failure.as_ref().filter(|_| {
                should_upgrade_to_upstream_response_failed(&row, existing_kind.as_deref())
            }) {
                let error_message = format_upstream_response_failed_message(response_info);
                let resolved = classify_invocation_failure(Some("http_200"), Some(&error_message));
                let next_payload = update_proxy_payload_failure_details(
                    row.payload.as_deref(),
                    Some(PROXY_FAILURE_UPSTREAM_RESPONSE_FAILED),
                    response_info,
                );
                let affected = sqlx::query(
                    r#"
                    UPDATE codex_invocations
                    SET status = ?1,
                        error_message = ?2,
                        failure_kind = ?3,
                        failure_class = ?4,
                        is_actionable = ?5,
                        payload = ?6
                    WHERE id = ?7
                    "#,
                )
                .bind("http_200")
                .bind(&error_message)
                .bind(PROXY_FAILURE_UPSTREAM_RESPONSE_FAILED)
                .bind(resolved.failure_class.as_str())
                .bind(resolved.is_actionable as i64)
                .bind(next_payload)
                .bind(row.id)
                .execute(&mut *tx)
                .await?
                .rows_affected();
                summary.updated += affected;
                if affected > 0 {
                    updated_ids.push(row.id);
                }
                continue;
            }

            let resolved = resolve_failure_classification(
                row.status.as_deref(),
                row.error_message.as_deref(),
                row.failure_kind.as_deref(),
                row.failure_class.as_deref(),
                row.is_actionable,
            );

            let next_kind = existing_kind.clone().or(resolved.failure_kind.clone());
            let should_update = existing_class != Some(resolved.failure_class)
                || existing_actionable != Some(resolved.is_actionable)
                || existing_kind != next_kind;

            if !should_update {
                continue;
            }

            let affected = sqlx::query(
                r#"
                UPDATE codex_invocations
                SET failure_kind = ?1,
                    failure_class = ?2,
                    is_actionable = ?3
                WHERE id = ?4
                "#,
            )
            .bind(next_kind.as_deref())
            .bind(resolved.failure_class.as_str())
            .bind(resolved.is_actionable as i64)
            .bind(row.id)
            .execute(&mut *tx)
            .await?
            .rows_affected();
            summary.updated += affected;
            if affected > 0 {
                updated_ids.push(row.id);
            }
        }
        if !updated_ids.is_empty() {
            recompute_invocation_hourly_rollups_for_ids_tx(tx.as_mut(), &updated_ids).await?;
        }
        tx.commit().await?;
    }

    Ok(BackfillBatchOutcome {
        summary,
        next_cursor_id: last_seen_id,
        hit_budget,
        samples,
    })
}

#[cfg(test)]
#[allow(dead_code)]
async fn backfill_failure_classification(
    pool: &Pool<Sqlite>,
    raw_path_fallback_root: Option<&Path>,
) -> Result<FailureClassificationBackfillSummary> {
    Ok(
        backfill_failure_classification_from_cursor(pool, 0, raw_path_fallback_root, None, None)
            .await?
            .summary,
    )
}

fn is_sqlite_lock_error(err: &anyhow::Error) -> bool {
    if err.chain().any(|cause| {
        let Some(sqlx_err) = cause.downcast_ref::<sqlx::Error>() else {
            return false;
        };
        let sqlx::Error::Database(db_err) = sqlx_err else {
            return false;
        };
        matches!(
            db_err.code().as_deref(),
            Some("5") | Some("6") | Some("SQLITE_BUSY") | Some("SQLITE_LOCKED")
        )
    }) {
        return true;
    }

    err.chain().any(|cause| {
        let message = cause.to_string().to_ascii_lowercase();
        message.contains("database is locked")
            || message.contains("database table is locked")
            || message.contains("sqlite_busy")
            || message.contains("sqlite_locked")
            || message.contains("(code: 5)")
            || message.contains("(code: 6)")
    })
}

fn parse_proxy_capture_summary(payload: Option<&str>) -> (ProxyCaptureTarget, bool) {
    let mut target = ProxyCaptureTarget::Responses;
    let mut is_stream = false;

    let Some(raw) = payload else {
        return (target, is_stream);
    };
    let Ok(value) = serde_json::from_str::<Value>(raw) else {
        return (target, is_stream);
    };

    if let Some(endpoint) = value.get("endpoint").and_then(|v| v.as_str()) {
        target = ProxyCaptureTarget::from_endpoint(endpoint);
    }
    if let Some(stream) = value.get("isStream").and_then(|v| v.as_bool()) {
        is_stream = stream;
    }

    (target, is_stream)
}

fn elapsed_ms(started: Instant) -> f64 {
    started.elapsed().as_secs_f64() * 1000.0
}

fn percentile_sorted_f64(sorted_values: &[f64], p: f64) -> f64 {
    if sorted_values.is_empty() {
        return 0.0;
    }
    if sorted_values.len() == 1 {
        return sorted_values[0];
    }
    let clamped = p.clamp(0.0, 1.0);
    let rank = clamped * (sorted_values.len() - 1) as f64;
    let lower = rank.floor() as usize;
    let upper = rank.ceil() as usize;
    if lower == upper {
        return sorted_values[lower];
    }
    let weight = rank - lower as f64;
    sorted_values[lower] + (sorted_values[upper] - sorted_values[lower]) * weight
}

fn next_proxy_request_id() -> u64 {
    NEXT_PROXY_REQUEST_ID.fetch_add(1, Ordering::Relaxed)
}

#[derive(Debug, Clone)]
struct PoolRoutingReservation {
    account_id: i64,
    proxy_key: Option<String>,
    #[allow(dead_code)]
    created_at: Instant,
}

#[derive(Debug, Default, Clone)]
struct PoolRoutingReservationSnapshot {
    counts_by_account: HashMap<i64, i64>,
    proxy_keys_by_account: HashMap<i64, HashSet<String>>,
    reserved_proxy_keys: HashSet<String>,
}

impl PoolRoutingReservationSnapshot {
    fn count_for_account(&self, account_id: i64) -> i64 {
        self.counts_by_account
            .get(&account_id)
            .copied()
            .unwrap_or_default()
    }

    fn pinned_proxy_keys_for_account(
        &self,
        account_id: i64,
        valid_proxy_keys: &[String],
        occupied_proxy_keys: &HashSet<String>,
    ) -> Vec<String> {
        let Some(proxy_keys) = self.proxy_keys_by_account.get(&account_id) else {
            return Vec::new();
        };
        valid_proxy_keys
            .iter()
            .filter(|proxy_key| {
                proxy_keys.contains(proxy_key.as_str())
                    && !occupied_proxy_keys.contains(proxy_key.as_str())
            })
            .cloned()
            .collect()
    }

    fn reserved_proxy_keys_for_group(&self, valid_proxy_keys: &[String]) -> HashSet<String> {
        let valid_proxy_keys = valid_proxy_keys
            .iter()
            .map(String::as_str)
            .collect::<HashSet<_>>();
        self.reserved_proxy_keys
            .iter()
            .filter(|proxy_key| valid_proxy_keys.contains(proxy_key.as_str()))
            .cloned()
            .collect()
    }
}

#[derive(Debug)]
struct PoolRoutingReservationDropGuard {
    state: Arc<AppState>,
    reservation_key: String,
    active: bool,
}

impl PoolRoutingReservationDropGuard {
    fn new(state: Arc<AppState>, reservation_key: String) -> Self {
        Self {
            state,
            reservation_key,
            active: true,
        }
    }

    fn disarm(&mut self) {
        self.active = false;
    }
}

impl Drop for PoolRoutingReservationDropGuard {
    fn drop(&mut self) {
        if self.active {
            release_pool_routing_reservation(self.state.as_ref(), &self.reservation_key);
        }
    }
}

fn build_pool_routing_reservation_key(proxy_request_id: u64) -> String {
    format!("pool-route-{proxy_request_id}")
}

fn pool_routing_reservation_count(state: &AppState, account_id: i64) -> i64 {
    let reservations = state
        .pool_routing_reservations
        .lock()
        .expect("pool routing reservations mutex poisoned");
    reservations
        .values()
        .filter(|reservation| reservation.account_id == account_id)
        .count() as i64
}

fn pool_routing_reservation_snapshot(state: &AppState) -> PoolRoutingReservationSnapshot {
    let reservations = state
        .pool_routing_reservations
        .lock()
        .expect("pool routing reservations mutex poisoned");
    let mut snapshot = PoolRoutingReservationSnapshot::default();
    for reservation in reservations.values() {
        *snapshot
            .counts_by_account
            .entry(reservation.account_id)
            .or_default() += 1;
        if let Some(proxy_key) = reservation.proxy_key.as_deref() {
            snapshot.reserved_proxy_keys.insert(proxy_key.to_string());
            snapshot
                .proxy_keys_by_account
                .entry(reservation.account_id)
                .or_default()
                .insert(proxy_key.to_string());
        }
    }
    snapshot
}

fn reserve_pool_routing_account(
    state: &AppState,
    reservation_key: &str,
    account: &PoolResolvedAccount,
) {
    let proxy_key = match &account.forward_proxy_scope {
        ForwardProxyRouteScope::PinnedProxyKey(proxy_key) => Some(proxy_key.clone()),
        _ => None,
    };
    if account.routing_source == PoolRoutingSelectionSource::StickyReuse && proxy_key.is_none() {
        return;
    }
    let mut reservations = state
        .pool_routing_reservations
        .lock()
        .expect("pool routing reservations mutex poisoned");
    reservations.insert(
        reservation_key.to_string(),
        PoolRoutingReservation {
            account_id: account.account_id,
            proxy_key,
            created_at: Instant::now(),
        },
    );
}

fn release_pool_routing_reservation(state: &AppState, reservation_key: &str) {
    let mut reservations = state
        .pool_routing_reservations
        .lock()
        .expect("pool routing reservations mutex poisoned");
    reservations.remove(reservation_key);
}

fn consume_pool_routing_reservation(state: &AppState, reservation_key: &str) {
    release_pool_routing_reservation(state, reservation_key);
}

fn is_body_too_large_error(err: &reqwest::Error) -> bool {
    error_chain_contains(err, "length limit exceeded")
        || error_chain_contains(err, PROXY_REQUEST_BODY_LIMIT_EXCEEDED)
}

fn error_chain_contains(err: &(dyn std::error::Error + 'static), needle: &str) -> bool {
    if err.to_string().contains(needle) {
        return true;
    }
    let mut source = err.source();
    while let Some(inner) = source {
        if inner.to_string().contains(needle) {
            return true;
        }
        source = inner.source();
    }
    false
}

fn build_proxy_upstream_url(base: &Url, original_uri: &Uri) -> Result<Url> {
    if path_has_forbidden_dot_segment(original_uri.path()) {
        bail!(PROXY_DOT_SEGMENT_PATH_NOT_ALLOWED);
    }
    if has_invalid_percent_encoding(original_uri.path())
        || original_uri
            .query()
            .is_some_and(has_invalid_percent_encoding)
    {
        bail!(PROXY_INVALID_REQUEST_TARGET);
    }

    let host = base
        .host_str()
        .ok_or_else(|| anyhow!("OPENAI_UPSTREAM_BASE_URL is missing host"))?;
    let mut target = String::new();
    target.push_str(base.scheme());
    target.push_str("://");
    if !base.username().is_empty() {
        target.push_str(base.username());
        if let Some(password) = base.password() {
            target.push(':');
            target.push_str(password);
        }
        target.push('@');
    }
    if host.contains(':') && !(host.starts_with('[') && host.ends_with(']')) {
        target.push('[');
        target.push_str(host);
        target.push(']');
    } else {
        target.push_str(host);
    }
    if let Some(port) = base.port() {
        target.push(':');
        target.push_str(&port.to_string());
    }

    let base_path = if base.path() == "/" {
        ""
    } else {
        base.path().trim_end_matches('/')
    };
    target.push_str(base_path);
    let request_path = original_uri.path();
    if !request_path.starts_with('/') {
        target.push('/');
    }
    target.push_str(request_path);
    if let Some(query) = original_uri.query() {
        target.push('?');
        target.push_str(query);
    }

    Url::parse(&target).context("failed to parse proxy upstream url")
}

fn path_has_forbidden_dot_segment(path: &str) -> bool {
    let mut candidate = path.to_string();
    for _ in 0..3 {
        if decoded_path_has_forbidden_dot_segment(&candidate) {
            return true;
        }
        let decoded = percent_decode_once_lossy(&candidate);
        if decoded == candidate {
            break;
        }
        candidate = decoded;
    }
    decoded_path_has_forbidden_dot_segment(&candidate)
}

fn has_invalid_percent_encoding(input: &str) -> bool {
    let bytes = input.as_bytes();
    let mut idx = 0usize;
    while idx < bytes.len() {
        if bytes[idx] == b'%' {
            if idx + 2 >= bytes.len()
                || decode_hex_nibble(bytes[idx + 1]).is_none()
                || decode_hex_nibble(bytes[idx + 2]).is_none()
            {
                return true;
            }
            idx += 3;
            continue;
        }
        idx += 1;
    }
    false
}

fn decoded_path_has_forbidden_dot_segment(path: &str) -> bool {
    path.split(['/', '\\']).any(is_forbidden_dot_segment)
}

fn is_forbidden_dot_segment(segment: &str) -> bool {
    segment == "." || segment == ".."
}

fn percent_decode_once_lossy(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut idx = 0usize;
    while idx < bytes.len() {
        if bytes[idx] == b'%'
            && idx + 2 < bytes.len()
            && let (Some(hi), Some(lo)) = (
                decode_hex_nibble(bytes[idx + 1]),
                decode_hex_nibble(bytes[idx + 2]),
            )
        {
            decoded.push((hi << 4) | lo);
            idx += 3;
            continue;
        }
        decoded.push(bytes[idx]);
        idx += 1;
    }
    String::from_utf8_lossy(&decoded).into_owned()
}

fn decode_hex_nibble(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}

pub(crate) fn connection_scoped_header_names(headers: &HeaderMap) -> HashSet<HeaderName> {
    let mut names = HashSet::new();
    for value in headers.get_all(header::CONNECTION).iter() {
        let Ok(raw) = value.to_str() else {
            continue;
        };
        for token in raw.split(',') {
            let token = token.trim();
            if token.is_empty() {
                continue;
            }
            if let Ok(header_name) = HeaderName::from_bytes(token.as_bytes()) {
                names.insert(header_name);
            }
        }
    }
    names
}

pub(crate) fn should_forward_proxy_header(
    name: &HeaderName,
    connection_scoped: &HashSet<HeaderName>,
) -> bool {
    should_transport_proxy_header(name) && !connection_scoped.contains(name)
}

fn request_may_have_body(method: &Method, headers: &HeaderMap) -> bool {
    if headers.contains_key(header::TRANSFER_ENCODING) {
        return true;
    }
    if let Some(content_length) = headers
        .get(header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok())
    {
        return content_length > 0;
    }
    !matches!(*method, Method::GET | Method::HEAD | Method::OPTIONS)
}

fn location_rewrite_upstream_base<'a>(
    pool_account: Option<&'a PoolResolvedAccount>,
    global_upstream_base_url: &'a Url,
) -> &'a Url {
    pool_account
        .map(|account| &account.upstream_base_url)
        .unwrap_or(global_upstream_base_url)
}

fn normalize_proxy_location_header(
    status: StatusCode,
    headers: &HeaderMap,
    upstream_base: &Url,
) -> Result<Option<String>> {
    let Some(raw_location) = headers.get(header::LOCATION) else {
        return Ok(None);
    };

    let raw_location = raw_location
        .to_str()
        .context("upstream Location header is not valid UTF-8")?;
    if raw_location.is_empty() {
        return Ok(None);
    }

    if !status.is_redirection() {
        return Ok(Some(raw_location.to_string()));
    }

    if raw_location.starts_with("//") {
        bail!("cross-origin redirect is not allowed");
    }

    if let Ok(parsed) = Url::parse(raw_location) {
        if !is_same_origin(&parsed, upstream_base) {
            bail!("cross-origin redirect is not allowed");
        }
        let mut normalized = rewrite_proxy_location_path(parsed.path(), upstream_base).to_string();
        if let Some(query) = parsed.query() {
            normalized.push('?');
            normalized.push_str(query);
        }
        if let Some(fragment) = parsed.fragment() {
            normalized.push('#');
            normalized.push_str(fragment);
        }
        return Ok(Some(normalized));
    }

    if raw_location.starts_with('/') {
        return Ok(Some(rewrite_proxy_relative_location(
            raw_location,
            upstream_base,
        )));
    }

    Ok(Some(raw_location.to_string()))
}

fn rewrite_proxy_relative_location(location: &str, upstream_base: &Url) -> String {
    let (path_and_query, fragment) = match location.split_once('#') {
        Some((pq, frag)) => (pq, Some(frag)),
        None => (location, None),
    };
    let (path, query) = match path_and_query.split_once('?') {
        Some((p, q)) => (p, Some(q)),
        None => (path_and_query, None),
    };

    let mut rewritten = rewrite_proxy_location_path(path, upstream_base);
    if let Some(query) = query {
        rewritten.push('?');
        rewritten.push_str(query);
    }
    if let Some(fragment) = fragment {
        rewritten.push('#');
        rewritten.push_str(fragment);
    }
    rewritten
}

fn rewrite_proxy_location_path(upstream_path: &str, upstream_base: &Url) -> String {
    let base_path = upstream_base.path().trim_end_matches('/');
    if base_path.is_empty() || base_path == "/" {
        return upstream_path.to_string();
    }
    if upstream_path == base_path {
        return "/".to_string();
    }
    if let Some(stripped) = upstream_path.strip_prefix(base_path)
        && stripped.starts_with('/')
    {
        return stripped.to_string();
    }
    upstream_path.to_string()
}

fn is_same_origin(lhs: &Url, rhs: &Url) -> bool {
    lhs.scheme() == rhs.scheme()
        && lhs.host_str() == rhs.host_str()
        && effective_port(lhs) == effective_port(rhs)
}

fn effective_port(url: &Url) -> Option<u16> {
    url.port_or_known_default()
}

fn should_transport_proxy_header(name: &HeaderName) -> bool {
    !matches!(
        name.as_str(),
        "host"
            | "connection"
            | "proxy-connection"
            | "keep-alive"
            | "proxy-authenticate"
            | "proxy-authorization"
            | "te"
            | "trailer"
            | "transfer-encoding"
            | "upgrade"
            | "forwarded"
            | "via"
            | "x-real-ip"
            | "x-forwarded-for"
            | "x-forwarded-host"
            | "x-forwarded-proto"
            | "x-forwarded-port"
            | "x-forwarded-client-cert"
    )
}

fn build_cors_layer(config: &AppConfig) -> CorsLayer {
    let allowed = config
        .cors_allowed_origins
        .iter()
        .cloned()
        .collect::<HashSet<_>>();
    let allow_origin = AllowOrigin::predicate(move |origin, _request| {
        let Ok(origin_raw) = origin.to_str() else {
            return false;
        };
        origin_allowed(origin_raw, &allowed)
    });
    CorsLayer::new()
        .allow_origin(allow_origin)
        .allow_methods(Any)
        .allow_headers(Any)
}

fn origin_allowed(origin_raw: &str, configured: &HashSet<String>) -> bool {
    let Some(origin) = normalize_cors_origin(origin_raw) else {
        return false;
    };
    if configured.contains(&origin) {
        return true;
    }
    is_loopback_origin(origin_raw)
}

fn is_loopback_origin(origin_raw: &str) -> bool {
    let Ok(origin) = Url::parse(origin_raw) else {
        return false;
    };
    if !matches!(origin.scheme(), "http" | "https") {
        return false;
    }
    origin
        .host_str()
        .map(is_loopback_authority_host)
        .unwrap_or(false)
}

fn parse_cors_allowed_origins_env(name: &str) -> Result<Vec<String>> {
    match env::var(name) {
        Ok(raw) => parse_cors_allowed_origins(&raw),
        Err(env::VarError::NotPresent) => Ok(Vec::new()),
        Err(err) => Err(anyhow!("failed to read {name}: {err}")),
    }
}

fn parse_cors_allowed_origins(raw: &str) -> Result<Vec<String>> {
    let mut entries = Vec::new();
    let mut seen = HashSet::new();
    for candidate in raw.split(',').map(str::trim).filter(|v| !v.is_empty()) {
        let normalized = normalize_cors_origin(candidate)
            .ok_or_else(|| anyhow!("invalid {ENV_CORS_ALLOWED_ORIGINS} entry: {candidate}"))?;
        if seen.insert(normalized.clone()) {
            entries.push(normalized);
        }
    }
    Ok(entries)
}

fn normalize_cors_origin(origin_raw: &str) -> Option<String> {
    let origin = Url::parse(origin_raw).ok()?;
    if !matches!(origin.scheme(), "http" | "https") {
        return None;
    }
    if origin.cannot_be_a_base()
        || !origin.username().is_empty()
        || origin.password().is_some()
        || origin.query().is_some()
        || origin.fragment().is_some()
    {
        return None;
    }
    if origin.path() != "/" {
        return None;
    }

    let host = origin.host_str()?;
    let host = if host.contains(':') {
        format!("[{host}]")
    } else {
        host.to_ascii_lowercase()
    };
    let scheme = origin.scheme().to_ascii_lowercase();
    let port = origin.port();
    let default_port = default_port_for_scheme(&scheme);

    if port.is_none() || port == default_port {
        Some(format!("{scheme}://{host}"))
    } else {
        Some(format!("{scheme}://{host}:{}", port?))
    }
}

fn is_models_list_path(path: &str) -> bool {
    path == "/v1/models"
}

// Browser-side CSRF mitigation for settings writes.
//
// This is intentionally not a full authentication mechanism: non-browser clients
// (CLI/automation) may omit Origin and are allowed by policy. The security boundary
// is deployment-level network isolation (trusted gateway only), documented in
// docs/deployment.md.
fn is_same_origin_settings_write(headers: &HeaderMap) -> bool {
    if matches!(
        header_value_as_str(headers, "sec-fetch-site"),
        Some(site)
            if site.eq_ignore_ascii_case("cross-site")
    ) {
        return false;
    }

    let Some(origin_raw) = headers.get(header::ORIGIN) else {
        // Non-browser clients may omit Origin (for example curl or internal tooling).
        // We only treat explicit browser cross-site signals as forbidden above.
        return true;
    };
    let Ok(origin) = origin_raw.to_str() else {
        return false;
    };
    let Ok(origin_url) = Url::parse(origin) else {
        return false;
    };
    if !matches!(origin_url.scheme(), "http" | "https") {
        return false;
    }

    let Some(origin_host) = origin_url.host_str() else {
        return false;
    };
    let Some((request_host, request_port)) =
        forwarded_or_host_authority(headers, origin_url.scheme())
    else {
        return false;
    };

    let origin_port = origin_url.port_or_known_default();
    if origin_host.eq_ignore_ascii_case(&request_host) && origin_port == request_port {
        return true;
    }

    // Dev loopback proxies (for example Vite on 60080 -> backend on 8080) may rewrite Host and/or port,
    // but both ends remain loopback. Allow that local-only mismatch.
    //
    // For non-loopback deployments behind reverse proxies, we accept trusted forwarded
    // host/proto/port headers for origin matching, but these headers are never relayed
    // to upstream/downstream proxy traffic (see should_proxy_header).
    is_loopback_authority_host(origin_host) && is_loopback_authority_host(&request_host)
}

fn forwarded_or_host_authority(
    headers: &HeaderMap,
    origin_scheme: &str,
) -> Option<(String, Option<u16>)> {
    if let Some(forwarded_host_raw) = header_value_as_str(headers, "x-forwarded-host") {
        // This service expects a single trusted edge gateway. If forwarded headers
        // arrive as a chain, treat it as unsupported/misconfigured and reject writes.
        let forwarded_host = single_forwarded_header_value(forwarded_host_raw)?;
        let authority = Authority::from_str(forwarded_host).ok()?;
        let forwarded_proto = match header_value_as_str(headers, "x-forwarded-proto") {
            Some(raw) => {
                let proto = single_forwarded_header_value(raw)?.to_ascii_lowercase();
                if proto == "http" || proto == "https" {
                    Some(proto)
                } else {
                    return None;
                }
            }
            None => None,
        };
        let scheme = forwarded_proto.as_deref().unwrap_or(origin_scheme);
        let forwarded_port = match header_value_as_str(headers, "x-forwarded-port") {
            Some(raw) => {
                let value = single_forwarded_header_value(raw)?;
                Some(value.parse::<u16>().ok()?)
            }
            None => None,
        };
        let port = authority
            .port_u16()
            .or(forwarded_port)
            .or_else(|| default_port_for_scheme(scheme));
        return Some((authority.host().to_string(), port));
    }

    let host_raw = headers.get(header::HOST)?;
    let host_value = host_raw.to_str().ok()?;
    let authority = Authority::from_str(host_value).ok()?;
    Some((
        authority.host().to_string(),
        authority
            .port_u16()
            .or_else(|| default_port_for_scheme(origin_scheme)),
    ))
}

fn single_forwarded_header_value(raw: &str) -> Option<&str> {
    let mut parts = raw
        .split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let first = parts.next()?;
    if parts.next().is_some() {
        return None;
    }
    Some(first)
}

fn default_port_for_scheme(scheme: &str) -> Option<u16> {
    match scheme {
        "http" => Some(80),
        "https" => Some(443),
        _ => None,
    }
}

fn header_value_as_str<'a>(headers: &'a HeaderMap, name: &'static str) -> Option<&'a str> {
    headers
        .get(HeaderName::from_static(name))
        .and_then(|value| value.to_str().ok())
}

fn extract_requester_ip(headers: &HeaderMap, peer_ip: Option<IpAddr>) -> Option<String> {
    if let Some(x_forwarded_for) = header_value_as_str(headers, "x-forwarded-for")
        && let Some(ip) = extract_first_ip_from_x_forwarded_for(x_forwarded_for)
    {
        return Some(ip);
    }

    if let Some(x_real_ip) = header_value_as_str(headers, "x-real-ip")
        && let Some(ip) = extract_ip_from_header_value(x_real_ip)
    {
        return Some(ip);
    }

    if let Some(forwarded) = header_value_as_str(headers, "forwarded")
        && let Some(ip) = extract_ip_from_forwarded_header(forwarded)
    {
        return Some(ip);
    }

    peer_ip.map(|ip| ip.to_string())
}

fn extract_sticky_key_from_headers(headers: &HeaderMap) -> Option<String> {
    for header_name in [
        "x-sticky-key",
        "sticky-key",
        "x-prompt-cache-key",
        "prompt-cache-key",
        "x-openai-prompt-cache-key",
    ] {
        if let Some(raw_value) = header_value_as_str(headers, header_name) {
            let candidate = raw_value
                .split(',')
                .next()
                .map(str::trim)
                .unwrap_or(raw_value.trim())
                .trim_matches('"');
            if !candidate.is_empty() {
                return Some(candidate.to_string());
            }
        }
    }
    None
}

fn extract_prompt_cache_key_from_headers(headers: &HeaderMap) -> Option<String> {
    for header_name in [
        "x-prompt-cache-key",
        "prompt-cache-key",
        "x-openai-prompt-cache-key",
    ] {
        if let Some(raw_value) = header_value_as_str(headers, header_name) {
            let candidate = raw_value
                .split(',')
                .next()
                .map(str::trim)
                .unwrap_or(raw_value.trim())
                .trim_matches('"');
            if !candidate.is_empty() {
                return Some(candidate.to_string());
            }
        }
    }
    None
}

fn extract_first_ip_from_x_forwarded_for(raw: &str) -> Option<String> {
    let first = raw.split(',').next()?.trim();
    extract_ip_from_header_value(first)
}

fn extract_ip_from_forwarded_header(raw: &str) -> Option<String> {
    for entry in raw.split(',') {
        for segment in entry.split(';') {
            let pair = segment.trim();
            if pair.len() >= 4 && pair[..4].eq_ignore_ascii_case("for=") {
                let value = &pair[4..];
                if let Some(ip) = extract_ip_from_header_value(value) {
                    return Some(ip);
                }
            }
        }
    }
    None
}

fn extract_ip_from_header_value(raw: &str) -> Option<String> {
    let normalized = raw.trim().trim_matches('"');
    if normalized.is_empty()
        || normalized.eq_ignore_ascii_case("unknown")
        || normalized.starts_with('_')
    {
        return None;
    }

    if let Some(value) = normalized.strip_prefix("for=") {
        return extract_ip_from_header_value(value);
    }

    if normalized.starts_with('[')
        && let Some(end) = normalized.find(']')
        && let Ok(ip) = normalized[1..end].parse::<IpAddr>()
    {
        return Some(ip.to_string());
    }

    if let Ok(ip) = normalized.parse::<IpAddr>() {
        return Some(ip.to_string());
    }

    if let Some((host, port)) = normalized.rsplit_once(':')
        && !host.contains(':')
        && port.parse::<u16>().is_ok()
        && let Ok(ip) = host.parse::<IpAddr>()
    {
        return Some(ip.to_string());
    }

    None
}

fn is_loopback_authority_host(host: &str) -> bool {
    if host.eq_ignore_ascii_case("localhost") {
        return true;
    }
    host.parse::<IpAddr>().is_ok_and(|ip| ip.is_loopback())
}

fn build_preset_models_payload(enabled_model_ids: &[String]) -> Value {
    let data = enabled_model_ids
        .iter()
        .map(|id| {
            json!({
                "id": id,
                "object": "model",
                "owned_by": "proxy",
                "created": 0
            })
        })
        .collect::<Vec<_>>();
    json!({
        "object": "list",
        "data": data
    })
}

fn merge_models_payload_with_upstream(
    upstream_payload: &Value,
    enabled_model_ids: &[String],
) -> Result<Value> {
    let upstream_items = upstream_payload
        .get("data")
        .and_then(|v| v.as_array())
        .ok_or_else(|| anyhow!("upstream models payload missing data array"))?;
    let mut merged = build_preset_models_payload(enabled_model_ids)
        .get("data")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let mut seen_ids: HashSet<String> = enabled_model_ids.iter().cloned().collect();

    for item in upstream_items {
        if let Some(id) = item.get("id").and_then(|v| v.as_str())
            && seen_ids.insert(id.to_string())
        {
            merged.push(item.clone());
        }
    }

    Ok(json!({
        "object": "list",
        "data": merged
    }))
}
