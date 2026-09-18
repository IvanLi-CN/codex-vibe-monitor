use super::*;
use std::sync::{Arc as StdArc, Mutex as StdMutex};

pub(crate) trait AsyncReadWrite: AsyncRead + AsyncWrite + Unpin + Send {}

impl<T> AsyncReadWrite for T where T: AsyncRead + AsyncWrite + Unpin + Send {}

pub(crate) type BoxedWsIo = Box<dyn AsyncReadWrite>;
pub(crate) type UpstreamWsStream = WebSocketStream<MaybeTlsStream<BoxedWsIo>>;
pub(crate) const WS_UPSTREAM_DRAIN_AFTER_DOWNSTREAM_CLOSE_TIMEOUT: Duration =
    Duration::from_millis(1500);

pub(crate) struct PrefixedIo {
    prefix: std::io::Cursor<Vec<u8>>,
    inner: BoxedWsIo,
}

impl PrefixedIo {
    fn new(prefix: Vec<u8>, inner: BoxedWsIo) -> Self {
        Self {
            prefix: std::io::Cursor::new(prefix),
            inner,
        }
    }
}

impl AsyncRead for PrefixedIo {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<io::Result<()>> {
        let remaining = self.prefix.get_ref().len() as u64 - self.prefix.position();
        if remaining > 0 {
            let available = self.prefix.get_ref().len() - self.prefix.position() as usize;
            let to_copy = available.min(buf.remaining());
            let start = self.prefix.position() as usize;
            let end = start + to_copy;
            buf.put_slice(&self.prefix.get_ref()[start..end]);
            self.prefix.set_position(end as u64);
            return std::task::Poll::Ready(Ok(()));
        }
        Pin::new(&mut self.inner).poll_read(cx, buf)
    }
}

impl AsyncWrite for PrefixedIo {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<io::Result<usize>> {
        Pin::new(&mut self.inner).poll_write(cx, buf)
    }

    fn poll_flush(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }

    fn poll_shutdown(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}

pub(crate) fn is_websocket_upgrade_request(headers: &HeaderMap) -> bool {
    headers
        .get(header::UPGRADE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.eq_ignore_ascii_case("websocket"))
}

pub(crate) fn websocket_routing_keys_from_headers(
    headers: &HeaderMap,
) -> (Option<String>, Option<String>) {
    (
        extract_sticky_key_from_headers(headers),
        extract_prompt_cache_key_from_headers(headers),
    )
}

pub(crate) fn requested_websocket_subprotocol(headers: &HeaderMap) -> Option<String> {
    headers
        .get(HeaderName::from_static("sec-websocket-protocol"))
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(',').next())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

pub(crate) fn websocket_effective_prompt_cache_key(prompt_cache_key: Option<&str>) -> Option<&str> {
    prompt_cache_key
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

pub(crate) fn extract_requested_model_from_websocket_uri(original_uri: &Uri) -> Option<String> {
    let raw_query = original_uri.query()?;
    url::form_urlencoded::parse(raw_query.as_bytes())
        .find_map(|(key, value)| (key == "model").then(|| value.trim()).map(str::to_string))
}

pub(crate) async fn proxy_openai_v1_ws_common(
    state: Arc<AppState>,
    peer_ip: Option<IpAddr>,
    ws: WebSocketUpgrade,
    original_uri: Uri,
    method: Method,
    headers: HeaderMap,
) -> Response {
    let proxy_request_id = next_proxy_request_id();
    let invoke_id = format!(
        "proxy-ws-{proxy_request_id}-{}",
        Utc::now().timestamp_millis()
    );
    info!(
        proxy_request_id,
        method = %method,
        uri = %original_uri,
        peer_ip = ?peer_ip,
        "openai websocket proxy request started"
    );

    if let Some(error) = websocket_upgrade_request_error(&method, &headers) {
        return build_proxy_error_response(error, &invoke_id);
    }
    let context = match build_websocket_tunnel_preparation_context(
        state,
        proxy_request_id,
        original_uri,
        &method,
        headers,
        peer_ip,
    )
    .await
    {
        Ok(context) => context,
        Err(error) => return build_proxy_error_response(error, &invoke_id),
    };
    let requires_response_create_first_frame = websocket_should_wait_for_initial_model(
        context.original_uri.path(),
        context.requested_model.as_deref(),
    );
    let ws = match context.required_subprotocol.clone() {
        Some(protocol) => ws.protocols([protocol]),
        None => ws,
    };
    ws.on_upgrade(move |downstream| async move {
        if requires_response_create_first_frame {
            proxy_websocket_tunnel_deferred_prepare(context, downstream).await;
        } else {
            proxy_websocket_tunnel_immediate_prepare(context, downstream, None).await;
        }
    })
}

fn websocket_upgrade_request_error(
    method: &Method,
    headers: &HeaderMap,
) -> Option<ProxyErrorResponse> {
    if method != Method::GET {
        return Some(ProxyErrorResponse {
            status: StatusCode::METHOD_NOT_ALLOWED,
            message: "websocket proxy requires GET".to_string(),
            cvm_id: None,
            retry_after_secs: None,
            code: None,
            blocked_binding: None,
        });
    }
    (extract_bearer_token(headers).is_none()).then(|| ProxyErrorResponse {
        status: StatusCode::UNAUTHORIZED,
        message: PROXY_POOL_ROUTE_KEY_MISSING_OR_INVALID_MESSAGE.to_string(),
        cvm_id: None,
        retry_after_secs: None,
        code: None,
        blocked_binding: None,
    })
}

async fn build_websocket_tunnel_preparation_context(
    state: Arc<AppState>,
    proxy_request_id: u64,
    original_uri: Uri,
    method: &Method,
    headers: HeaderMap,
    peer_ip: Option<IpAddr>,
) -> Result<WebSocketTunnelPreparationContext, ProxyErrorResponse> {
    let runtime_timeouts = resolve_proxy_route_context_for_request(
        state.as_ref(),
        proxy_request_id,
        method,
        &original_uri,
        &headers,
    )
    .await?;
    let proxy_request_permit = acquire_proxy_request_concurrency_permit(
        state.as_ref(),
        proxy_request_id,
        method,
        &original_uri,
    )
    .await;
    let (sticky_key, header_prompt_cache_key) = websocket_routing_keys_from_headers(&headers);
    let requested_model = extract_requested_model_from_websocket_uri(&original_uri);
    let required_subprotocol = requested_websocket_subprotocol(&headers);
    let trace = PoolUpstreamAttemptTraceContext {
        invoke_id: format!("pool-ws-{proxy_request_id}"),
        occurred_at: shanghai_now_string(),
        endpoint: original_uri.path().to_string(),
        sticky_key: sticky_key.clone(),
        requester_ip: extract_requester_ip(&headers, peer_ip),
        upstream_base_url_host: None,
        request_model: requested_model.clone(),
    };
    Ok(WebSocketTunnelPreparationContext {
        state,
        proxy_request_id,
        original_uri,
        headers,
        runtime_timeouts,
        sticky_key,
        requested_model,
        header_prompt_cache_key,
        required_subprotocol,
        trace,
        proxy_request_permit,
    })
}

pub(crate) fn websocket_requires_response_create_first_frame(path: &str) -> bool {
    path == "/v1/responses" || path.starts_with("/v1/responses/")
}

pub(crate) fn websocket_should_wait_for_initial_model(
    path: &str,
    requested_model: Option<&str>,
) -> bool {
    websocket_requires_response_create_first_frame(path)
        || (path == "/v1/realtime" && requested_model.is_none())
}

pub(crate) struct PreparedUpstreamWebSocket {
    upstream: UpstreamWsStream,
    transport_flush_task: Option<UpstreamSocketFlushTask>,
    pending_attempt_record: Option<PendingPoolAttemptRecord>,
    model_mapping: Option<ResolvedModelMapping>,
    deferred_cleanup_guard: Option<PoolEarlyPhaseOrphanCleanupGuard>,
    reservation_guard: PoolRoutingReservationGuard,
    account: PoolResolvedAccount,
    trace: PoolUpstreamAttemptTraceContext,
    prompt_cache_key: Option<String>,
    connect_latency_ms: f64,
    requires_response_create_first_frame: bool,
}

struct WebSocketTunnelPreparationContext {
    state: Arc<AppState>,
    proxy_request_id: u64,
    original_uri: Uri,
    headers: HeaderMap,
    runtime_timeouts: PoolRoutingTimeoutSettingsResolved,
    sticky_key: Option<String>,
    requested_model: Option<String>,
    header_prompt_cache_key: Option<String>,
    required_subprotocol: Option<String>,
    trace: PoolUpstreamAttemptTraceContext,
    proxy_request_permit: ProxyRequestConcurrencyPermit,
}

struct UpstreamSocketFlushTask {
    task: JoinHandle<()>,
    meter: UpstreamSocketByteMeter,
    reporter: UpstreamTrafficReporter,
    last_reported: StdArc<StdMutex<UpstreamSocketByteTotals>>,
}

impl UpstreamSocketFlushTask {
    fn spawn(meter: UpstreamSocketByteMeter, reporter: UpstreamTrafficReporter) -> Self {
        let last_reported = StdArc::new(StdMutex::new(meter.snapshot()));
        let task_last_reported = last_reported.clone();
        let task_meter = meter.clone();
        let task_reporter = reporter.clone();
        let task = tokio::spawn(async move {
            let mut ticker = interval(Duration::from_secs(1));
            ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
            loop {
                ticker.tick().await;
                let mut last_reported = task_last_reported
                    .lock()
                    .expect("lock websocket upstream byte flush state");
                let snapshot = task_meter.snapshot();
                let delta = snapshot.delta_since(*last_reported);
                *last_reported = snapshot;
                if delta.upload_bytes > 0 || delta.download_bytes > 0 {
                    task_reporter.record_delta(delta, Utc::now());
                }
            }
        });
        Self {
            task,
            meter,
            reporter,
            last_reported,
        }
    }
}

impl Drop for UpstreamSocketFlushTask {
    fn drop(&mut self) {
        self.task.abort();
        let mut last_reported = self
            .last_reported
            .lock()
            .expect("lock websocket upstream byte flush state");
        let snapshot = self.meter.snapshot();
        let delta = snapshot.delta_since(*last_reported);
        *last_reported = snapshot;
        if delta.upload_bytes > 0 || delta.download_bytes > 0 {
            self.reporter.record_delta(delta, Utc::now());
        }
    }
}

pub(crate) struct PoolRoutingReservationGuard {
    state: Arc<AppState>,
    reservation_key: String,
    armed: bool,
    publish_availability: bool,
}

impl PoolRoutingReservationGuard {
    fn new(state: Arc<AppState>, reservation_key: String) -> Self {
        Self {
            state,
            reservation_key,
            armed: true,
            publish_availability: true,
        }
    }

    fn suppress_availability_publish(&mut self) {
        self.publish_availability = false;
    }

    fn set_availability_publish(&mut self, publish_availability: bool) {
        self.publish_availability = publish_availability;
    }

    fn release_after_persisted_failure(&mut self) {
        if !self.armed {
            return;
        }
        release_pool_routing_reservation(self.state.as_ref(), &self.reservation_key);
        self.armed = false;
    }

    fn release(&mut self) {
        if !self.armed {
            return;
        }
        if self.publish_availability {
            release_pool_routing_reservation(self.state.as_ref(), &self.reservation_key);
        } else {
            release_pool_routing_reservation_without_availability(
                self.state.as_ref(),
                &self.reservation_key,
            );
        }
        self.armed = false;
    }
}

impl Drop for PoolRoutingReservationGuard {
    fn drop(&mut self) {
        self.release();
    }
}

#[derive(Debug)]
pub(crate) struct WsPrepareError {
    pub(crate) status: StatusCode,
    pub(crate) message: String,
}

pub(crate) struct WsAttemptFailure {
    status: StatusCode,
    message: String,
    failure_kind: &'static str,
    retryable: bool,
    account_id: Option<i64>,
    upstream_route_key: Option<String>,
}
