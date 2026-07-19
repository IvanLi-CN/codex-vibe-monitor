use std::{
    io,
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    task::{Context, Poll},
};

use super::*;
use hyper::body::Incoming;
use hyper::client::conn::http1;
use hyper_util::rt::TokioIo;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct UpstreamSocketByteTotals {
    pub(crate) upload_bytes: usize,
    pub(crate) download_bytes: usize,
}

impl UpstreamSocketByteTotals {
    pub(crate) fn add_assign(&mut self, other: Self) {
        self.upload_bytes = self.upload_bytes.saturating_add(other.upload_bytes);
        self.download_bytes = self.download_bytes.saturating_add(other.download_bytes);
    }

    pub(crate) fn delta_since(self, last: Self) -> Self {
        Self {
            upload_bytes: self.upload_bytes.saturating_sub(last.upload_bytes),
            download_bytes: self.download_bytes.saturating_sub(last.download_bytes),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub(crate) struct UpstreamSocketByteMeter {
    upload_bytes: Arc<AtomicU64>,
    download_bytes: Arc<AtomicU64>,
}

impl UpstreamSocketByteMeter {
    pub(crate) fn snapshot(&self) -> UpstreamSocketByteTotals {
        UpstreamSocketByteTotals {
            upload_bytes: usize::try_from(self.upload_bytes.load(Ordering::Relaxed))
                .unwrap_or(usize::MAX),
            download_bytes: usize::try_from(self.download_bytes.load(Ordering::Relaxed))
                .unwrap_or(usize::MAX),
        }
    }

    fn add_upload(&self, bytes: usize) {
        let Ok(bytes) = u64::try_from(bytes) else {
            self.upload_bytes.store(u64::MAX, Ordering::Relaxed);
            return;
        };
        let _ = self
            .upload_bytes
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
                Some(current.saturating_add(bytes))
            });
    }

    fn add_download(&self, bytes: usize) {
        let Ok(bytes) = u64::try_from(bytes) else {
            self.download_bytes.store(u64::MAX, Ordering::Relaxed);
            return;
        };
        let _ = self
            .download_bytes
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
                Some(current.saturating_add(bytes))
            });
    }
}

struct PrefixedIo {
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
        cx: &mut Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let remaining = self.prefix.get_ref().len() as u64 - self.prefix.position();
        if remaining > 0 {
            let available = self.prefix.get_ref().len() - self.prefix.position() as usize;
            let to_copy = available.min(buf.remaining());
            let start = self.prefix.position() as usize;
            let end = start + to_copy;
            buf.put_slice(&self.prefix.get_ref()[start..end]);
            self.prefix.set_position(end as u64);
            return Poll::Ready(Ok(()));
        }
        Pin::new(&mut self.inner).poll_read(cx, buf)
    }
}

impl AsyncWrite for PrefixedIo {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.inner).poll_write(cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}

#[derive(Debug, Clone)]
pub(crate) struct UpstreamTrafficReporter {
    state: Arc<AppState>,
    invoke_id: String,
    occurred_at: String,
    upstream_account_id: Option<i64>,
    upstream_base_url_host: Option<String>,
}

impl UpstreamTrafficReporter {
    pub(crate) fn new(
        state: Arc<AppState>,
        invoke_id: impl Into<String>,
        occurred_at: impl Into<String>,
        upstream_account_id: Option<i64>,
        upstream_base_url_host: Option<&str>,
    ) -> Self {
        Self {
            state,
            invoke_id: invoke_id.into(),
            occurred_at: occurred_at.into(),
            upstream_account_id,
            upstream_base_url_host: upstream_base_url_host.map(str::to_string),
        }
    }

    pub(crate) fn record_delta(&self, delta: UpstreamSocketByteTotals, observed_at: DateTime<Utc>) {
        let mut recorded = false;
        if delta.upload_bytes > 0 {
            self.state
                .dashboard_network_speed_cache
                .record_request_bytes(
                    &self.invoke_id,
                    &self.occurred_at,
                    self.upstream_account_id,
                    self.upstream_base_url_host.as_deref(),
                    delta.upload_bytes,
                    observed_at,
                );
            recorded = true;
        }
        if delta.download_bytes > 0 {
            self.state
                .dashboard_network_speed_cache
                .record_response_chunk_bytes(
                    &self.invoke_id,
                    &self.occurred_at,
                    self.upstream_account_id,
                    self.upstream_base_url_host.as_deref(),
                    delta.download_bytes,
                    observed_at,
                );
            recorded = true;
        }
        if recorded {
            schedule_dashboard_activity_live_snapshot(self.state.as_ref());
        }
    }
}

#[derive(Debug)]
pub(crate) struct CountedIo<T> {
    inner: T,
    meter: UpstreamSocketByteMeter,
}

impl<T> CountedIo<T> {
    pub(crate) fn new(inner: T, meter: UpstreamSocketByteMeter) -> Self {
        Self { inner, meter }
    }
}

impl<T> AsyncRead for CountedIo<T>
where
    T: AsyncRead + Unpin,
{
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let filled_before = buf.filled().len();
        let result = Pin::new(&mut self.inner).poll_read(cx, buf);
        if let Poll::Ready(Ok(())) = &result {
            let filled_after = buf.filled().len();
            if filled_after > filled_before {
                self.meter.add_download(filled_after - filled_before);
            }
        }
        result
    }
}

impl<T> AsyncWrite for CountedIo<T>
where
    T: AsyncWrite + Unpin,
{
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        let result = Pin::new(&mut self.inner).poll_write(cx, buf);
        if let Poll::Ready(Ok(written)) = &result
            && *written > 0
        {
            self.meter.add_upload(*written);
        }
        result
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }

    fn poll_write_vectored(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &[io::IoSlice<'_>],
    ) -> Poll<io::Result<usize>> {
        let result = Pin::new(&mut self.inner).poll_write_vectored(cx, bufs);
        if let Poll::Ready(Ok(written)) = &result
            && *written > 0
        {
            self.meter.add_upload(*written);
        }
        result
    }

    fn is_write_vectored(&self) -> bool {
        self.inner.is_write_vectored()
    }
}

#[derive(Debug)]
struct TrackedIncomingBody {
    inner: Incoming,
    meter: UpstreamSocketByteMeter,
    reporter: Option<UpstreamTrafficReporter>,
    last_reported: UpstreamSocketByteTotals,
    finished: bool,
}

impl TrackedIncomingBody {
    fn new(
        inner: Incoming,
        meter: UpstreamSocketByteMeter,
        reporter: Option<UpstreamTrafficReporter>,
        last_reported: UpstreamSocketByteTotals,
    ) -> Self {
        Self {
            inner,
            meter,
            reporter,
            last_reported,
            finished: false,
        }
    }

    fn flush_transport_delta(&mut self) {
        let Some(reporter) = self.reporter.as_ref() else {
            return;
        };
        let snapshot = self.meter.snapshot();
        let delta = snapshot.delta_since(self.last_reported);
        self.last_reported = snapshot;
        if delta.upload_bytes > 0 || delta.download_bytes > 0 {
            reporter.record_delta(delta, Utc::now());
        }
    }
}

impl futures_util::Stream for TrackedIncomingBody {
    type Item = Result<Bytes, io::Error>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        loop {
            match Pin::new(&mut self.inner).poll_frame(cx) {
                Poll::Ready(Some(Ok(frame))) => {
                    self.flush_transport_delta();
                    match frame.into_data() {
                        Ok(bytes) => return Poll::Ready(Some(Ok(bytes))),
                        Err(_) => continue,
                    }
                }
                Poll::Ready(Some(Err(err))) => {
                    self.flush_transport_delta();
                    self.finished = true;
                    return Poll::Ready(Some(Err(io::Error::other(err.to_string()))));
                }
                Poll::Ready(None) => {
                    self.flush_transport_delta();
                    self.finished = true;
                    return Poll::Ready(None);
                }
                Poll::Pending => return Poll::Pending,
            }
        }
    }
}

impl Drop for TrackedIncomingBody {
    fn drop(&mut self) {
        if !self.finished {
            self.flush_transport_delta();
        }
    }
}

#[derive(Debug)]
pub(crate) struct CountedHttpRequestError {
    pub(crate) message: String,
    pub(crate) socket_totals: UpstreamSocketByteTotals,
}

impl CountedHttpRequestError {
    pub(crate) fn is_timeout(&self) -> bool {
        let message = self.message.to_ascii_lowercase();
        message.contains("timed out") || message.contains("timeout")
    }
}

impl std::fmt::Display for CountedHttpRequestError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for CountedHttpRequestError {}

#[derive(Debug)]
pub(crate) struct CountedHttpResponse {
    pub(crate) response: Response,
    pub(crate) socket_meter: UpstreamSocketByteMeter,
}

struct UpstreamTransportReportGuard {
    meter: UpstreamSocketByteMeter,
    reporter: Option<UpstreamTrafficReporter>,
    last_reported: UpstreamSocketByteTotals,
    armed: bool,
}

impl UpstreamTransportReportGuard {
    fn new(meter: UpstreamSocketByteMeter, reporter: Option<UpstreamTrafficReporter>) -> Self {
        Self {
            meter,
            reporter,
            last_reported: UpstreamSocketByteTotals::default(),
            armed: true,
        }
    }

    fn record_now(&mut self) {
        let Some(reporter) = self.reporter.as_ref() else {
            return;
        };
        let snapshot = self.meter.snapshot();
        let delta = snapshot.delta_since(self.last_reported);
        self.last_reported = snapshot;
        if delta.upload_bytes > 0 || delta.download_bytes > 0 {
            reporter.record_delta(delta, Utc::now());
        }
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for UpstreamTransportReportGuard {
    fn drop(&mut self) {
        if self.armed {
            self.record_now();
        }
    }
}

fn upstream_request_origin_form(target_url: &Url) -> Result<String, String> {
    let mut uri = target_url.path().to_string();
    if uri.is_empty() {
        uri.push('/');
    }
    if let Some(query) = target_url.query() {
        uri.push('?');
        uri.push_str(query);
    }
    uri.parse::<Uri>()
        .map(|_| uri)
        .map_err(|err| format!("failed to build upstream request URI: {err}"))
}

fn upstream_request_host_header(target_url: &Url) -> Result<HeaderValue, String> {
    let host = target_url
        .host_str()
        .ok_or_else(|| "upstream URL is missing host".to_string())?;
    let port = target_url
        .port_or_known_default()
        .ok_or_else(|| "upstream URL is missing port".to_string())?;
    let authority = if target_url.port().is_some() {
        if host.contains(':') {
            format!("[{host}]:{port}")
        } else {
            format!("{host}:{port}")
        }
    } else {
        host.to_string()
    };
    HeaderValue::from_str(&authority)
        .map_err(|err| format!("invalid upstream host header value: {err}"))
}

fn apply_request_headers(
    request: &mut Request<Body>,
    headers: &HeaderMap,
    target_url: &Url,
) -> Result<(), String> {
    request.headers_mut().remove(header::HOST);
    request
        .headers_mut()
        .insert(header::HOST, upstream_request_host_header(target_url)?);
    for (name, value) in headers {
        if *name == header::HOST {
            continue;
        }
        request.headers_mut().append(name.clone(), value.clone());
    }
    Ok(())
}

fn build_http1_request(
    method: Method,
    target_url: &Url,
    headers: &HeaderMap,
    body: Body,
) -> Result<Request<Body>, String> {
    let uri = upstream_request_origin_form(target_url)?;
    let mut request = Request::builder()
        .method(method)
        .uri(uri)
        .body(body)
        .map_err(|err| format!("failed to build upstream request: {err}"))?;
    apply_request_headers(&mut request, headers, target_url)?;
    Ok(request)
}

async fn maybe_tls_wrap_target_stream<T>(
    stream: T,
    target_url: &Url,
) -> Result<BoxedWsIo, io::Error>
where
    T: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    if target_url.scheme() != "https" {
        return Ok(Box::new(stream));
    }
    let host = target_url
        .host_str()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "upstream URL missing host"))?;
    let root_store = rustls::RootCertStore {
        roots: webpki_roots::TLS_SERVER_ROOTS.to_vec(),
    };
    let mut config = rustls::ClientConfig::builder()
        .with_root_certificates(root_store)
        .with_no_client_auth();
    config.alpn_protocols = vec![b"http/1.1".to_vec()];
    let connector = TlsConnector::from(Arc::new(config));
    let server_name = rustls_pki_types::ServerName::try_from(host.to_string()).map_err(|err| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("invalid upstream TLS host for SNI: {err}"),
        )
    })?;
    let stream = connector.connect(server_name, stream).await?;
    Ok(Box::new(stream))
}

async fn connect_via_counted_transport(
    target_url: &Url,
    forward_proxy_url: Option<&Url>,
    meter: UpstreamSocketByteMeter,
) -> Result<BoxedWsIo, io::Error> {
    let Some(forward_proxy_url) = forward_proxy_url else {
        let host = target_url.host_str().ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "upstream URL missing host")
        })?;
        let port = target_url.port_or_known_default().ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "upstream URL missing port")
        })?;
        let stream = TcpStream::connect((host, port)).await?;
        let counted = CountedIo::new(stream, meter);
        return maybe_tls_wrap_target_stream(counted, target_url).await;
    };

    let proxy_host = forward_proxy_url.host_str().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "forward proxy endpoint is missing host",
        )
    })?;
    let proxy_port = forward_proxy_url.port_or_known_default().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "forward proxy endpoint is missing port",
        )
    })?;
    let upstream_host = target_url
        .host_str()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "upstream URL missing host"))?;
    let upstream_port = target_url
        .port_or_known_default()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "upstream URL missing port"))?;
    let target_authority = if upstream_host.contains(':') {
        format!("[{upstream_host}]:{upstream_port}")
    } else {
        format!("{upstream_host}:{upstream_port}")
    };

    let proxy_scheme = forward_proxy_url.scheme();
    if matches!(proxy_scheme, "socks5" | "socks5h") {
        let stream = TcpStream::connect((proxy_host, proxy_port)).await?;
        let mut stream = CountedIo::new(stream, meter);
        let socks_target_host = if proxy_scheme == "socks5" {
            super::websocket::resolve_socks5_local_target_host(upstream_host, upstream_port)
                .await
                .map_err(|err| io::Error::other(err.to_string()))?
        } else {
            upstream_host.to_string()
        };
        let username = super::websocket::forward_proxy_username(forward_proxy_url);
        let password =
            super::websocket::forward_proxy_password(forward_proxy_url).unwrap_or_default();
        let use_password_auth = !username.is_empty();
        if use_password_auth {
            stream.write_all(&[0x05, 0x02, 0x00, 0x02]).await?;
        } else {
            stream.write_all(&[0x05, 0x01, 0x00]).await?;
        }
        let mut method_response = [0_u8; 2];
        stream.read_exact(&mut method_response).await?;
        if method_response[0] != 0x05 || method_response[1] == 0xff {
            return Err(io::Error::other(
                "SOCKS5 forward proxy did not accept an authentication method",
            ));
        }
        if method_response[1] == 0x02 {
            let mut auth_request = Vec::with_capacity(3 + username.len() + password.len());
            auth_request.push(0x01);
            auth_request.push(u8::try_from(username.len()).map_err(|_| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "SOCKS5 username exceeds 255 bytes",
                )
            })?);
            auth_request.extend_from_slice(username.as_bytes());
            auth_request.push(u8::try_from(password.len()).map_err(|_| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "SOCKS5 password exceeds 255 bytes",
                )
            })?);
            auth_request.extend_from_slice(password.as_bytes());
            stream.write_all(&auth_request).await?;
            let mut auth_response = [0_u8; 2];
            stream.read_exact(&mut auth_response).await?;
            if auth_response != [0x01, 0x00] {
                return Err(io::Error::other(
                    "SOCKS5 username/password authentication failed",
                ));
            }
        }
        let mut connect_request = Vec::with_capacity(6 + socks_target_host.len());
        connect_request.push(0x05);
        connect_request.push(0x01);
        connect_request.push(0x00);
        if let Ok(ipv4) = socks_target_host.parse::<std::net::Ipv4Addr>() {
            connect_request.push(0x01);
            connect_request.extend_from_slice(&ipv4.octets());
        } else if let Ok(ipv6) = socks_target_host.parse::<std::net::Ipv6Addr>() {
            connect_request.push(0x04);
            connect_request.extend_from_slice(&ipv6.octets());
        } else {
            connect_request.push(0x03);
            let host_bytes = socks_target_host.as_bytes();
            connect_request.push(u8::try_from(host_bytes.len()).map_err(|_| {
                io::Error::new(io::ErrorKind::InvalidInput, "SOCKS5 host exceeds 255 bytes")
            })?);
            connect_request.extend_from_slice(host_bytes);
        }
        connect_request.extend_from_slice(&upstream_port.to_be_bytes());
        stream.write_all(&connect_request).await?;
        let mut response_head = [0_u8; 4];
        stream.read_exact(&mut response_head).await?;
        if response_head[0] != 0x05 || response_head[1] != 0x00 {
            return Err(io::Error::other("SOCKS5 forward proxy CONNECT failed"));
        }
        match response_head[3] {
            0x01 => {
                let mut skip = [0_u8; 4 + 2];
                stream.read_exact(&mut skip).await?;
            }
            0x03 => {
                let mut len = [0_u8; 1];
                stream.read_exact(&mut len).await?;
                let mut skip = vec![0_u8; usize::from(len[0]) + 2];
                stream.read_exact(&mut skip).await?;
            }
            0x04 => {
                let mut skip = [0_u8; 16 + 2];
                stream.read_exact(&mut skip).await?;
            }
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "SOCKS5 forward proxy returned invalid address type",
                ));
            }
        }
        return maybe_tls_wrap_target_stream(stream, target_url).await;
    }
    if !matches!(proxy_scheme, "http" | "https") {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            format!(
                "HTTP transport only supports HTTP CONNECT, HTTPS CONNECT, or SOCKS5 forward proxy endpoints, got {proxy_scheme}"
            ),
        ));
    }

    let stream = TcpStream::connect((proxy_host, proxy_port)).await?;
    let mut stream: BoxedWsIo = if forward_proxy_url.scheme() == "https" {
        maybe_tls_wrap_target_stream(CountedIo::new(stream, meter.clone()), forward_proxy_url)
            .await?
    } else {
        Box::new(CountedIo::new(stream, meter.clone()))
    };
    let mut connect_request =
        format!("CONNECT {target_authority} HTTP/1.1\r\nHost: {target_authority}\r\n");
    if let Some(credential) =
        super::websocket::forward_proxy_basic_auth_credential(forward_proxy_url)
    {
        let encoded = base64::engine::general_purpose::STANDARD.encode(credential);
        connect_request.push_str("Proxy-Authorization: Basic ");
        connect_request.push_str(&encoded);
        connect_request.push_str("\r\n");
    }
    connect_request.push_str("\r\n");
    stream.write_all(connect_request.as_bytes()).await?;

    let mut response = Vec::with_capacity(256);
    let mut buffer = [0_u8; 1024];
    loop {
        let read = stream.read(&mut buffer).await?;
        if read == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "forward proxy closed before CONNECT response completed",
            ));
        }
        response.extend_from_slice(&buffer[..read]);
        if response.windows(4).any(|window| window == b"\r\n\r\n") {
            break;
        }
        if response.len() > 16 * 1024 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "forward proxy CONNECT response exceeded 16KiB",
            ));
        }
    }

    let header_end = response
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "forward proxy CONNECT response missing header terminator",
            )
        })?;
    let status_line_end = response
        .windows(2)
        .position(|window| window == b"\r\n")
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "forward proxy CONNECT response missing status line",
            )
        })?;
    let status_line = std::str::from_utf8(&response[..status_line_end]).map_err(|err| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("forward proxy CONNECT status line is not UTF-8: {err}"),
        )
    })?;
    if !status_line.starts_with("HTTP/1.1 200") && !status_line.starts_with("HTTP/1.0 200") {
        return Err(io::Error::other(format!(
            "forward proxy CONNECT failed: {status_line}"
        )));
    }

    let extra_read = response[(header_end + 4)..].to_vec();
    let stream: BoxedWsIo = if extra_read.is_empty() {
        stream
    } else {
        Box::new(PrefixedIo::new(extra_read, stream))
    };
    maybe_tls_wrap_target_stream(stream, target_url).await
}

pub(crate) async fn send_counted_upstream_http_request(
    method: Method,
    target_url: &Url,
    headers: &HeaderMap,
    body: Body,
    forward_proxy_url: Option<&Url>,
    reporter: Option<UpstreamTrafficReporter>,
) -> Result<CountedHttpResponse, CountedHttpRequestError> {
    let meter = UpstreamSocketByteMeter::default();
    let mut report_guard = UpstreamTransportReportGuard::new(meter.clone(), reporter.clone());
    let stream = connect_via_counted_transport(target_url, forward_proxy_url, meter.clone())
        .await
        .map_err(|err| {
            report_guard.record_now();
            report_guard.disarm();
            CountedHttpRequestError {
                message: format!("failed to connect upstream transport: {err}"),
                socket_totals: meter.snapshot(),
            }
        })?;
    let (mut sender, connection) = http1::Builder::new()
        .handshake(TokioIo::new(stream))
        .await
        .map_err(|err| {
            report_guard.record_now();
            report_guard.disarm();
            CountedHttpRequestError {
                message: format!("failed to establish upstream HTTP/1 connection: {err}"),
                socket_totals: meter.snapshot(),
            }
        })?;
    tokio::spawn(async move {
        if let Err(err) = connection.await {
            debug!(error = %err, "counted upstream HTTP connection closed with error");
        }
    });

    let request = build_http1_request(method, target_url, headers, body).map_err(|message| {
        report_guard.record_now();
        report_guard.disarm();
        CountedHttpRequestError {
            message,
            socket_totals: meter.snapshot(),
        }
    })?;
    let response = sender.send_request(request).await.map_err(|err| {
        report_guard.record_now();
        report_guard.disarm();
        CountedHttpRequestError {
            message: format!("failed to send upstream HTTP request: {err}"),
            socket_totals: meter.snapshot(),
        }
    })?;

    let initial_snapshot = meter.snapshot();
    report_guard.record_now();
    report_guard.disarm();
    let (parts, body) = response.into_parts();
    let tracked_body = Body::from_stream(TrackedIncomingBody::new(
        body,
        meter.clone(),
        reporter,
        initial_snapshot,
    ));
    Ok(CountedHttpResponse {
        response: Response::from_parts(parts, tracked_body),
        socket_meter: meter,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn counted_http_request_reports_socket_bytes() {
        let app = Router::new().route(
            "/",
            any(|| async {
                (
                    StatusCode::OK,
                    [(header::CONTENT_TYPE, HeaderValue::from_static("text/plain"))],
                    "upstream-response",
                )
            }),
        );
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind counted upstream test server");
        let address = listener
            .local_addr()
            .expect("read counted upstream address");
        let server = tokio::spawn(async move {
            axum::serve(listener, app)
                .await
                .expect("counted upstream test server should run");
        });

        let target_url = Url::parse(&format!("http://{address}/")).expect("valid counted target");
        let response = send_counted_upstream_http_request(
            Method::POST,
            &target_url,
            &HeaderMap::new(),
            Body::from("client-request"),
            None,
            None,
        )
        .await
        .expect("counted upstream request should succeed");
        let response_body = axum::body::to_bytes(response.response.into_body(), usize::MAX)
            .await
            .expect("read counted upstream response");
        assert_eq!(response_body.as_ref(), b"upstream-response");

        let totals = response.socket_meter.snapshot();
        assert!(totals.upload_bytes > 0);
        assert!(totals.download_bytes > 0);

        server.abort();
    }
}
