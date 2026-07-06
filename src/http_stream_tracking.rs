use std::{
    future::{Future, poll_fn},
    io,
    net::SocketAddr,
    pin::Pin,
    sync::{Arc, Mutex},
    task::{Context, Poll},
    time::{Duration, Instant},
};

use axum::{Extension, Router, body::Body, extract::Request};
use futures_util::{FutureExt, pin_mut};
use hyper::body::Incoming;
use hyper_util::{
    rt::{TokioExecutor, TokioIo},
    server::conn::auto::Builder,
    service::TowerToHyperService,
};
use tokio::{
    io::{AsyncRead, AsyncWrite, ReadBuf},
    net::{TcpListener, TcpStream},
    sync::{Notify, watch},
};
use tower::{Layer, Service, ServiceExt};
use tracing::{error, trace};

#[derive(Clone, Debug)]
pub(crate) struct DownstreamTransportObserver {
    inner: Arc<DownstreamTransportObserverInner>,
}

#[derive(Debug)]
struct DownstreamTransportObserverInner {
    state: Mutex<DownstreamTransportObserverState>,
    notify: Notify,
    reset_monitor_notify: Notify,
}

#[derive(Debug, Default)]
struct DownstreamTransportObserverState {
    next_request_seq: u64,
    current_request_seq: Option<u64>,
    last_write_error: Option<DownstreamWriteErrorSnapshot>,
    reset_monitor_stream: Option<TcpStream>,
    active_reset_monitor_request_seq: Option<u64>,
    connection_closed: bool,
}

#[derive(Clone, Debug)]
pub(crate) struct DownstreamRequestObserver {
    observer: DownstreamTransportObserver,
    request_seq: u64,
}

#[derive(Clone, Debug)]
pub(crate) struct DownstreamWriteErrorSnapshot {
    pub(crate) request_seq: u64,
    pub(crate) kind: &'static str,
    pub(crate) message: String,
}

impl DownstreamTransportObserver {
    pub(crate) fn new() -> Self {
        Self {
            inner: Arc::new(DownstreamTransportObserverInner {
                state: Mutex::new(DownstreamTransportObserverState {
                    next_request_seq: 1,
                    ..DownstreamTransportObserverState::default()
                }),
                notify: Notify::new(),
                reset_monitor_notify: Notify::new(),
            }),
        }
    }

    pub(crate) fn begin_request(&self) -> DownstreamRequestObserver {
        let mut notify_reset_monitor = false;
        let mut state = self
            .inner
            .state
            .lock()
            .expect("downstream transport observer mutex poisoned");
        let request_seq = state.next_request_seq;
        state.next_request_seq = state.next_request_seq.saturating_add(1);
        state.current_request_seq = Some(request_seq);
        if state
            .active_reset_monitor_request_seq
            .is_some_and(|active_request_seq| active_request_seq != request_seq)
        {
            state.active_reset_monitor_request_seq = None;
            notify_reset_monitor = true;
        }
        if state
            .last_write_error
            .as_ref()
            .is_some_and(|snapshot| snapshot.request_seq < request_seq)
        {
            state.last_write_error = None;
        }
        drop(state);
        if notify_reset_monitor {
            self.inner.reset_monitor_notify.notify_waiters();
        }
        #[cfg(test)]
        eprintln!("[DEBUG-stream-rootcause-20260706] begin_request request_seq={request_seq}");
        DownstreamRequestObserver {
            observer: self.clone(),
            request_seq,
        }
    }

    fn set_reset_monitor_stream(&self, stream: TcpStream) {
        let mut state = self
            .inner
            .state
            .lock()
            .expect("downstream transport observer mutex poisoned");
        state.reset_monitor_stream = Some(stream);
        state.connection_closed = false;
    }

    fn try_arm_reset_monitor(&self, request_seq: u64) -> Option<TcpStream> {
        let mut state = self
            .inner
            .state
            .lock()
            .expect("downstream transport observer mutex poisoned");
        if state.connection_closed || state.current_request_seq != Some(request_seq) {
            return None;
        }
        if state.active_reset_monitor_request_seq == Some(request_seq) {
            return None;
        }
        let monitor_stream =
            duplicate_tcp_stream_for_monitor(state.reset_monitor_stream.as_ref()?).ok()?;
        state.active_reset_monitor_request_seq = Some(request_seq);
        Some(monitor_stream)
    }

    fn activate_reset_monitor(&self, request_seq: u64) {
        let Some(monitor_stream) = self.try_arm_reset_monitor(request_seq) else {
            return;
        };
        spawn_downstream_reset_monitor(monitor_stream, self.clone(), request_seq);
    }

    fn finish_request_monitoring(&self, request_seq: u64) {
        let should_notify = {
            let mut state = self
                .inner
                .state
                .lock()
                .expect("downstream transport observer mutex poisoned");
            if state.active_reset_monitor_request_seq != Some(request_seq) {
                false
            } else {
                state.active_reset_monitor_request_seq = None;
                true
            }
        };
        if should_notify {
            self.inner.reset_monitor_notify.notify_waiters();
        }
    }

    fn mark_connection_closed(&self) {
        let should_notify = {
            let mut state = self
                .inner
                .state
                .lock()
                .expect("downstream transport observer mutex poisoned");
            if state.connection_closed {
                false
            } else {
                state.connection_closed = true;
                state.active_reset_monitor_request_seq = None;
                state.reset_monitor_stream = None;
                true
            }
        };
        if should_notify {
            self.inner.reset_monitor_notify.notify_waiters();
        }
    }

    fn reset_monitor_should_continue(&self, request_seq: u64) -> bool {
        let state = self
            .inner
            .state
            .lock()
            .expect("downstream transport observer mutex poisoned");
        !state.connection_closed
            && state.current_request_seq == Some(request_seq)
            && state.active_reset_monitor_request_seq == Some(request_seq)
    }

    fn record_write_error(&self, kind: &'static str, message: String) {
        let request_seq = {
            let state = self
                .inner
                .state
                .lock()
                .expect("downstream transport observer mutex poisoned");
            state.current_request_seq
        };
        let Some(request_seq) = request_seq else {
            return;
        };
        self.record_write_error_for_request(request_seq, kind, message);
    }

    fn record_write_error_for_request(
        &self,
        request_seq: u64,
        kind: &'static str,
        message: String,
    ) {
        let mut state = self
            .inner
            .state
            .lock()
            .expect("downstream transport observer mutex poisoned");
        if state.current_request_seq != Some(request_seq) {
            return;
        }
        if state
            .last_write_error
            .as_ref()
            .is_some_and(|snapshot| snapshot.request_seq == request_seq)
        {
            return;
        }
        state.last_write_error = Some(DownstreamWriteErrorSnapshot {
            request_seq,
            kind,
            message,
        });
        #[cfg(test)]
        eprintln!(
            "[DEBUG-stream-rootcause-20260706] record_write_error request_seq={request_seq} kind={kind}"
        );
        drop(state);
        self.inner.notify.notify_waiters();
    }

    fn current_write_error_for(&self, request_seq: u64) -> Option<DownstreamWriteErrorSnapshot> {
        let state = self
            .inner
            .state
            .lock()
            .expect("downstream transport observer mutex poisoned");
        state
            .last_write_error
            .as_ref()
            .filter(|snapshot| snapshot.request_seq == request_seq)
            .cloned()
    }

    fn request_advanced_past(&self, request_seq: u64) -> bool {
        let state = self
            .inner
            .state
            .lock()
            .expect("downstream transport observer mutex poisoned");
        state
            .current_request_seq
            .is_some_and(|current_request_seq| current_request_seq > request_seq)
    }
}

impl DownstreamRequestObserver {
    pub(crate) fn activate_reset_monitor(&self) {
        self.observer.activate_reset_monitor(self.request_seq);
    }

    pub(crate) fn finish_response_monitoring(&self) {
        self.observer.finish_request_monitoring(self.request_seq);
    }

    pub(crate) async fn wait_for_write_error_window(
        &self,
        grace_period: Duration,
    ) -> Option<DownstreamWriteErrorSnapshot> {
        if let Some(snapshot) = self.observer.current_write_error_for(self.request_seq) {
            #[cfg(test)]
            eprintln!(
                "[DEBUG-stream-rootcause-20260706] wait_window immediate_hit request_seq={} kind={}",
                self.request_seq, snapshot.kind
            );
            return Some(snapshot);
        }
        if self.observer.request_advanced_past(self.request_seq) || grace_period.is_zero() {
            #[cfg(test)]
            eprintln!(
                "[DEBUG-stream-rootcause-20260706] wait_window short_circuit request_seq={}",
                self.request_seq
            );
            return None;
        }
        let deadline = Instant::now() + grace_period;
        loop {
            let notified = self.observer.inner.notify.notified();
            if let Some(snapshot) = self.observer.current_write_error_for(self.request_seq) {
                #[cfg(test)]
                eprintln!(
                    "[DEBUG-stream-rootcause-20260706] wait_window notified_hit request_seq={} kind={}",
                    self.request_seq, snapshot.kind
                );
                return Some(snapshot);
            }
            if self.observer.request_advanced_past(self.request_seq) {
                #[cfg(test)]
                eprintln!(
                    "[DEBUG-stream-rootcause-20260706] wait_window advanced request_seq={}",
                    self.request_seq
                );
                return None;
            }
            let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
                let snapshot = self.observer.current_write_error_for(self.request_seq);
                #[cfg(test)]
                eprintln!(
                    "[DEBUG-stream-rootcause-20260706] wait_window deadline request_seq={} hit={}",
                    self.request_seq,
                    snapshot.is_some()
                );
                return snapshot;
            };
            if tokio::time::timeout(remaining, notified).await.is_err() {
                let snapshot = self.observer.current_write_error_for(self.request_seq);
                #[cfg(test)]
                eprintln!(
                    "[DEBUG-stream-rootcause-20260706] wait_window timeout request_seq={} hit={}",
                    self.request_seq,
                    snapshot.is_some()
                );
                return snapshot;
            }
        }
    }
}

#[derive(Debug)]
struct ObservedTcpStream {
    inner: TcpStream,
    observer: DownstreamTransportObserver,
}

impl ObservedTcpStream {
    fn new(inner: TcpStream, observer: DownstreamTransportObserver) -> Self {
        Self { inner, observer }
    }

    fn record_write_error(&self, err: &io::Error) {
        self.observer
            .record_write_error(downstream_transport_write_error_kind(err), err.to_string());
    }

    fn record_pending_socket_error(&self) {
        if let Ok(Some(err)) = self.inner.take_error() {
            self.record_write_error(&err);
        }
    }
}

impl AsyncRead for ObservedTcpStream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_read(cx, buf)
    }
}

impl AsyncWrite for ObservedTcpStream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        let result = Pin::new(&mut self.inner).poll_write(cx, buf);
        match &result {
            Poll::Ready(Err(err)) => self.record_write_error(err),
            Poll::Ready(Ok(_)) => self.record_pending_socket_error(),
            Poll::Pending => {}
        }
        result
    }

    fn poll_write_vectored(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &[io::IoSlice<'_>],
    ) -> Poll<io::Result<usize>> {
        let result = Pin::new(&mut self.inner).poll_write_vectored(cx, bufs);
        match &result {
            Poll::Ready(Err(err)) => self.record_write_error(err),
            Poll::Ready(Ok(_)) => self.record_pending_socket_error(),
            Poll::Pending => {}
        }
        result
    }

    fn is_write_vectored(&self) -> bool {
        self.inner.is_write_vectored()
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let result = Pin::new(&mut self.inner).poll_flush(cx);
        match &result {
            Poll::Ready(Err(err)) => self.record_write_error(err),
            Poll::Ready(Ok(())) => self.record_pending_socket_error(),
            Poll::Pending => {}
        }
        result
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let result = Pin::new(&mut self.inner).poll_shutdown(cx);
        match &result {
            Poll::Ready(Err(err)) => self.record_write_error(err),
            Poll::Ready(Ok(())) => self.record_pending_socket_error(),
            Poll::Pending => {}
        }
        result
    }
}

fn downstream_transport_write_error_kind(err: &io::Error) -> &'static str {
    match err.kind() {
        io::ErrorKind::BrokenPipe => "broken_pipe",
        io::ErrorKind::ConnectionReset => "connection_reset",
        io::ErrorKind::ConnectionAborted => "connection_aborted",
        io::ErrorKind::TimedOut => "timeout",
        io::ErrorKind::UnexpectedEof => "unexpected_eof",
        _ => "other",
    }
}

fn is_connection_error(err: &io::Error) -> bool {
    matches!(
        err.kind(),
        io::ErrorKind::ConnectionRefused
            | io::ErrorKind::ConnectionAborted
            | io::ErrorKind::ConnectionReset
    )
}

#[cfg(unix)]
fn duplicate_tcp_stream_for_monitor(stream: &TcpStream) -> io::Result<TcpStream> {
    use std::os::fd::{AsRawFd, FromRawFd};

    let duplicated_fd = unsafe { libc::dup(stream.as_raw_fd()) };
    if duplicated_fd < 0 {
        return Err(io::Error::last_os_error());
    }
    let std_stream = unsafe { std::net::TcpStream::from_raw_fd(duplicated_fd) };
    std_stream.set_nonblocking(true)?;
    TcpStream::from_std(std_stream)
}

#[cfg(not(unix))]
fn duplicate_tcp_stream_for_monitor(_stream: &TcpStream) -> io::Result<TcpStream> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "tcp reset monitor requires unix fd duplication",
    ))
}

fn spawn_downstream_reset_monitor(
    monitor_stream: TcpStream,
    observer: DownstreamTransportObserver,
    request_seq: u64,
) {
    tokio::spawn(monitor_downstream_reset_stream(
        monitor_stream,
        observer,
        request_seq,
    ));
}

async fn monitor_downstream_reset_stream(
    monitor_stream: TcpStream,
    observer: DownstreamTransportObserver,
    request_seq: u64,
) {
    let mut peek_buf = [0u8; 1];
    loop {
        if !observer.reset_monitor_should_continue(request_seq) {
            break;
        }
        tokio::select! {
            readable = monitor_stream.readable() => {
                if readable.is_err() {
                    break;
                }
            }
            _ = observer.inner.reset_monitor_notify.notified() => {
                continue;
            }
        }
        if !observer.reset_monitor_should_continue(request_seq) {
            break;
        }
        if let Ok(Some(err)) = monitor_stream.take_error() {
            observer.record_write_error_for_request(
                request_seq,
                downstream_transport_write_error_kind(&err),
                format!("socket_take_error:{err}"),
            );
            break;
        }
        match monitor_stream.peek(&mut peek_buf).await {
            Ok(0) => break,
            // Once ordinary readable bytes show up, this side channel can no longer
            // distinguish a reset for the current response from later keep-alive traffic.
            Ok(_) => break,
            Err(err) if err.kind() == io::ErrorKind::WouldBlock => continue,
            Err(err) => {
                observer.record_write_error_for_request(
                    request_seq,
                    downstream_transport_write_error_kind(&err),
                    format!("socket_peek_error:{err}"),
                );
                break;
            }
        }
    }
    observer.finish_request_monitoring(request_seq);
}

async fn tcp_accept(listener: &TcpListener) -> Option<(TcpStream, SocketAddr)> {
    match listener.accept().await {
        Ok(connection) => Some(connection),
        Err(err) => {
            if is_connection_error(&err) {
                return None;
            }
            error!("accept error: {err}");
            tokio::time::sleep(Duration::from_secs(1)).await;
            None
        }
    }
}

pub(crate) async fn serve_router_with_graceful_shutdown<F>(
    tcp_listener: TcpListener,
    router: Router,
    signal: F,
) -> io::Result<()>
where
    F: Future<Output = ()> + Send + 'static,
{
    let mut make_service = router.into_make_service_with_connect_info::<SocketAddr>();
    let (signal_tx, signal_rx) = watch::channel(());
    let signal_tx = Arc::new(signal_tx);
    tokio::spawn(async move {
        signal.await;
        trace!("received graceful shutdown signal. Telling tasks to shutdown");
        drop(signal_rx);
    });

    let (close_tx, close_rx) = watch::channel(());

    loop {
        let (tcp_stream, remote_addr) = tokio::select! {
            connection = tcp_accept(&tcp_listener) => {
                match connection {
                    Some(connection) => connection,
                    None => continue,
                }
            }
            _ = signal_tx.closed() => {
                trace!("signal received, not accepting new connections");
                break;
            }
        };

        let observer = DownstreamTransportObserver::new();
        if let Ok(monitor_stream) = duplicate_tcp_stream_for_monitor(&tcp_stream) {
            observer.set_reset_monitor_stream(monitor_stream);
        }
        let tcp_stream = TokioIo::new(ObservedTcpStream::new(tcp_stream, observer.clone()));

        poll_fn(|cx| Service::<SocketAddr>::poll_ready(&mut make_service, cx))
            .await
            .unwrap_or_else(|err| match err {});

        let tower_service = Service::<SocketAddr>::call(&mut make_service, remote_addr)
            .await
            .unwrap_or_else(|err| match err {});
        let tower_service = Extension(observer.clone())
            .layer(tower_service)
            .map_request(|request: Request<Incoming>| request.map(Body::new));
        let hyper_service = TowerToHyperService::new(tower_service);
        let signal_tx = Arc::clone(&signal_tx);
        let close_rx = close_rx.clone();
        let observer_for_task = observer.clone();

        tokio::spawn(async move {
            let builder = Builder::new(TokioExecutor::new());
            let connection = builder.serve_connection_with_upgrades(tcp_stream, hyper_service);
            pin_mut!(connection);

            let signal_closed = signal_tx.closed().fuse();
            pin_mut!(signal_closed);

            loop {
                tokio::select! {
                    result = connection.as_mut() => {
                        if let Err(err) = result {
                            observer_for_task.record_write_error(
                                "connection_driver",
                                format!("serve_connection:{err}"),
                            );
                            trace!("failed to serve connection: {err:#}");
                        }
                        break;
                    }
                    _ = &mut signal_closed => {
                        trace!("signal received in task, starting graceful shutdown");
                        connection.as_mut().graceful_shutdown();
                    }
                }
            }

            observer_for_task.mark_connection_closed();
            drop(close_rx);
        });
    }

    drop(close_rx);
    drop(tcp_listener);
    close_tx.closed().await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncWriteExt;

    #[tokio::test]
    async fn downstream_request_observer_ignores_later_request_write_errors() {
        let observer = DownstreamTransportObserver::new();
        let first_request = observer.begin_request();
        let second_request = observer.begin_request();

        observer.record_write_error("connection_reset", "late reset".to_string());

        assert!(
            first_request
                .wait_for_write_error_window(Duration::from_millis(1))
                .await
                .is_none()
        );
        let second_snapshot = second_request
            .wait_for_write_error_window(Duration::from_millis(1))
            .await
            .expect("second request should observe its own write error");
        assert_eq!(second_snapshot.kind, "connection_reset");
        assert_eq!(second_snapshot.request_seq, 2);
    }

    #[tokio::test]
    async fn downstream_reset_monitor_stops_on_readable_keepalive_data() {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind keepalive test listener");
        let addr = listener.local_addr().expect("keepalive test listener addr");
        let accept = tokio::spawn(async move {
            listener
                .accept()
                .await
                .expect("accept keepalive test connection")
                .0
        });
        let mut client = TcpStream::connect(addr)
            .await
            .expect("connect keepalive test client");
        let server = accept.await.expect("join keepalive accept task");

        let observer = DownstreamTransportObserver::new();
        let request = observer.begin_request();
        let source_stream =
            duplicate_tcp_stream_for_monitor(&server).expect("duplicate keepalive test stream");
        observer.set_reset_monitor_stream(source_stream);
        let monitor_stream = observer
            .try_arm_reset_monitor(request.request_seq)
            .expect("arm keepalive reset monitor");
        let monitor = tokio::spawn(monitor_downstream_reset_stream(
            monitor_stream,
            observer.clone(),
            request.request_seq,
        ));

        client
            .write_all(b"GET /next HTTP/1.1\r\nHost: keepalive\r\n\r\n")
            .await
            .expect("write keepalive bytes");
        tokio::time::timeout(Duration::from_millis(200), monitor)
            .await
            .expect("monitor should stop once keepalive bytes become readable")
            .expect("join keepalive monitor");
        assert!(
            request
                .wait_for_write_error_window(Duration::from_millis(20))
                .await
                .is_none()
        );
    }

    #[tokio::test]
    async fn downstream_reset_monitor_stops_when_request_observation_finishes() {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind idle test listener");
        let addr = listener.local_addr().expect("idle test listener addr");
        let accept = tokio::spawn(async move {
            listener
                .accept()
                .await
                .expect("accept idle test connection")
                .0
        });
        let _client = TcpStream::connect(addr)
            .await
            .expect("connect idle test client");
        let server = accept.await.expect("join idle accept task");

        let observer = DownstreamTransportObserver::new();
        let source_stream =
            duplicate_tcp_stream_for_monitor(&server).expect("duplicate idle test stream");
        observer.set_reset_monitor_stream(source_stream);
        let request = observer.begin_request();
        let monitor_stream = observer
            .try_arm_reset_monitor(request.request_seq)
            .expect("arm idle reset monitor");
        let monitor = tokio::spawn(monitor_downstream_reset_stream(
            monitor_stream,
            observer.clone(),
            request.request_seq,
        ));

        request.finish_response_monitoring();

        tokio::time::timeout(Duration::from_millis(200), monitor)
            .await
            .expect("monitor should stop after observation finishes")
            .expect("join idle monitor");
        assert!(
            request
                .wait_for_write_error_window(Duration::from_millis(20))
                .await
                .is_none()
        );
    }

    #[tokio::test]
    async fn downstream_reset_monitor_can_rearm_for_reused_connection() {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind reused connection test listener");
        let addr = listener
            .local_addr()
            .expect("reused connection test listener addr");
        let accept =
            tokio::spawn(
                async move { listener.accept().await.expect("accept reused connection").0 },
            );
        let mut client = TcpStream::connect(addr)
            .await
            .expect("connect reused connection client");
        let server = accept.await.expect("join reused accept task");

        let observer = DownstreamTransportObserver::new();
        let source_stream =
            duplicate_tcp_stream_for_monitor(&server).expect("duplicate reused test stream");
        observer.set_reset_monitor_stream(source_stream);

        let first_request = observer.begin_request();
        let first_monitor_stream = observer
            .try_arm_reset_monitor(first_request.request_seq)
            .expect("arm first reused monitor");
        let first_monitor = tokio::spawn(monitor_downstream_reset_stream(
            first_monitor_stream,
            observer.clone(),
            first_request.request_seq,
        ));

        client
            .write_all(b"GET /next HTTP/1.1\r\nHost: keepalive\r\n\r\n")
            .await
            .expect("write reused keepalive bytes");
        tokio::time::timeout(Duration::from_millis(200), first_monitor)
            .await
            .expect("first reused monitor should stop on keepalive bytes")
            .expect("join first reused monitor");

        let second_request = observer.begin_request();
        let second_monitor_stream = observer
            .try_arm_reset_monitor(second_request.request_seq)
            .expect("rearm second reused monitor");
        let second_monitor = tokio::spawn(monitor_downstream_reset_stream(
            second_monitor_stream,
            observer.clone(),
            second_request.request_seq,
        ));
        second_request.finish_response_monitoring();
        tokio::time::timeout(Duration::from_millis(200), second_monitor)
            .await
            .expect("second reused monitor should stop when observation finishes")
            .expect("join second reused monitor");
    }
}
