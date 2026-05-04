trait AsyncReadWrite: AsyncRead + AsyncWrite + Unpin + Send {}

impl<T> AsyncReadWrite for T where T: AsyncRead + AsyncWrite + Unpin + Send {}

type BoxedWsIo = Box<dyn AsyncReadWrite>;
type UpstreamWsStream = WebSocketStream<MaybeTlsStream<BoxedWsIo>>;

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

pub(crate) async fn proxy_openai_v1_ws_common(
    state: Arc<AppState>,
    peer_ip: Option<IpAddr>,
    ws: WebSocketUpgrade,
    original_uri: Uri,
    method: Method,
    headers: HeaderMap,
) -> Response {
    let proxy_request_id = next_proxy_request_id();
    let invoke_id = format!("proxy-ws-{proxy_request_id}-{}", Utc::now().timestamp_millis());
    let started_at = Instant::now();

    info!(
        proxy_request_id,
        method = %method,
        uri = %original_uri,
        peer_ip = ?peer_ip,
        "openai websocket proxy request started"
    );

    if method != Method::GET {
        return build_proxy_error_response(
            ProxyErrorResponse {
                status: StatusCode::METHOD_NOT_ALLOWED,
                message: "websocket proxy requires GET".to_string(),
                cvm_id: None,
                retry_after_secs: None,
            },
            &invoke_id,
        );
    }

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

    let runtime_timeouts = match resolve_proxy_route_context_for_request(
        state.as_ref(),
        proxy_request_id,
        &method,
        &original_uri,
        &headers,
    )
    .await
    {
        Ok(timeouts) => timeouts,
        Err(err) => return build_proxy_error_response(err, &invoke_id),
    };

    let proxy_request_permit = acquire_proxy_request_concurrency_permit(
        state.as_ref(),
        proxy_request_id,
        &method,
        &original_uri,
    )
    .await;

    let sticky_key = extract_sticky_key_from_headers(&headers);
    let requester_ip = extract_requester_ip(&headers, peer_ip);
    let trace = PoolUpstreamAttemptTraceContext {
        invoke_id: format!("pool-ws-{proxy_request_id}"),
        occurred_at: shanghai_now_string(),
        endpoint: original_uri.path().to_string(),
        sticky_key: sticky_key.clone(),
        requester_ip,
    };

    let prepared = match prepare_upstream_websocket(
        state.clone(),
        proxy_request_id,
        &original_uri,
        &headers,
        &runtime_timeouts,
        sticky_key.as_deref(),
        &trace,
    )
    .await
    {
        Ok(prepared) => prepared,
        Err(err) => {
            drop(proxy_request_permit);
            let retry_after_secs = retry_after_secs_for_proxy_error(err.status, &err.message);
            warn!(
                proxy_request_id,
                method = %method,
                uri = %original_uri,
                status = %err.status,
                error = %err.message,
                elapsed_ms = started_at.elapsed().as_millis(),
                "openai websocket proxy request failed before upgrade"
            );
            return build_proxy_error_response(
                ProxyErrorResponse {
                    status: err.status,
                    message: err.message,
                    cvm_id: Some(invoke_id.clone()),
                    retry_after_secs,
                },
                &invoke_id,
            );
        }
    };

    let ws = match prepared.selected_subprotocol.clone() {
        Some(protocol) => ws.protocols([protocol]),
        None => ws,
    };
    ws.on_upgrade(move |downstream| async move {
        proxy_websocket_tunnel(state, downstream, prepared, proxy_request_permit).await;
    })
}

struct PreparedUpstreamWebSocket {
    upstream: UpstreamWsStream,
    pending_attempt_record: Option<PendingPoolAttemptRecord>,
    deferred_cleanup_guard: Option<PoolEarlyPhaseOrphanCleanupGuard>,
    reservation_guard: PoolRoutingReservationGuard,
    account: PoolResolvedAccount,
    trace: PoolUpstreamAttemptTraceContext,
    selected_subprotocol: Option<String>,
    connect_latency_ms: f64,
}

struct PoolRoutingReservationGuard {
    state: Arc<AppState>,
    reservation_key: String,
    armed: bool,
}

impl PoolRoutingReservationGuard {
    fn new(state: Arc<AppState>, reservation_key: String) -> Self {
        Self {
            state,
            reservation_key,
            armed: true,
        }
    }

    fn release(&mut self) {
        if !self.armed {
            return;
        }
        release_pool_routing_reservation(self.state.as_ref(), &self.reservation_key);
        self.armed = false;
    }
}

impl Drop for PoolRoutingReservationGuard {
    fn drop(&mut self) {
        self.release();
    }
}

#[derive(Debug)]
struct WsPrepareError {
    status: StatusCode,
    message: String,
}

struct WsAttemptFailure {
    status: StatusCode,
    message: String,
    retryable: bool,
    account_id: Option<i64>,
    upstream_route_key: Option<String>,
}

async fn prepare_upstream_websocket(
    state: Arc<AppState>,
    proxy_request_id: u64,
    original_uri: &Uri,
    headers: &HeaderMap,
    runtime_timeouts: &PoolRoutingTimeoutSettingsResolved,
    sticky_key: Option<&str>,
    trace: &PoolUpstreamAttemptTraceContext,
) -> Result<PreparedUpstreamWebSocket, WsPrepareError> {
    let mut excluded_account_ids = Vec::new();
    let mut excluded_upstream_route_keys = HashSet::new();
    let mut ws_retry_account_ids = HashSet::new();
    let mut last_failure: Option<WsAttemptFailure> = None;

    let upstream_websocket_default_enabled = state
        .proxy_model_settings
        .read()
        .await
        .upstream_websocket_default_enabled;
    if !upstream_websocket_default_enabled {
        return Err(WsPrepareError {
            status: StatusCode::SERVICE_UNAVAILABLE,
            message: format!(
                "upstream websocket transport is disabled in Settings; enable it or set {ENV_OPENAI_PROXY_UPSTREAM_WEBSOCKET_DEFAULT_ENABLED}=true before first startup"
            ),
        });
    }

    loop {
        if ws_retry_account_ids.len() >= POOL_UPSTREAM_MAX_DISTINCT_ACCOUNTS {
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

        let account = match resolve_pool_account_for_request(
            state.as_ref(),
            sticky_key,
            &excluded_account_ids,
            &excluded_upstream_route_keys,
        )
        .await
        {
            Ok(PoolAccountResolution::Resolved(account)) => account,
            Ok(PoolAccountResolution::Unavailable | PoolAccountResolution::NoCandidate) => {
                return Err(WsPrepareError {
                    status: last_failure
                        .as_ref()
                        .map(|failure| failure.status)
                        .unwrap_or(StatusCode::SERVICE_UNAVAILABLE),
                    message: last_failure
                        .map(|failure| failure.message)
                        .unwrap_or_else(|| POOL_NO_AVAILABLE_ACCOUNT_MESSAGE.to_string()),
                });
            }
            Ok(PoolAccountResolution::RateLimited) => {
                return Err(WsPrepareError {
                    status: StatusCode::TOO_MANY_REQUESTS,
                    message: POOL_ALL_ACCOUNTS_RATE_LIMITED_MESSAGE.to_string(),
                });
            }
            Ok(PoolAccountResolution::DegradedOnly) => {
                return Err(WsPrepareError {
                    status: StatusCode::SERVICE_UNAVAILABLE,
                    message: POOL_ALL_ACCOUNTS_DEGRADED_MESSAGE.to_string(),
                });
            }
            Ok(PoolAccountResolution::AssignedBlocked(blocked)) => {
                let terminal = ViaPoolResolutionTerminalError::assigned_blocked(blocked);
                terminal.persist_if_needed(state.as_ref(), Some(trace)).await;
                return Err(WsPrepareError {
                    status: terminal.status,
                    message: terminal.message,
                });
            }
            Ok(PoolAccountResolution::BlockedByPolicy(message)) => {
                return Err(WsPrepareError {
                    status: StatusCode::SERVICE_UNAVAILABLE,
                    message,
                });
            }
            Err(err) => {
                return Err(WsPrepareError {
                    status: StatusCode::BAD_GATEWAY,
                    message: format!("failed to resolve pool account: {err}"),
                });
            }
        };
        match account_supports_upstream_websocket(state.as_ref(), &account).await {
            Ok(true) => {}
            Ok(false) => {
                excluded_account_ids.push(account.account_id);
                last_failure = Some(WsAttemptFailure {
                    status: StatusCode::SERVICE_UNAVAILABLE,
                    message: "selected upstream account is tagged as not supporting websocket"
                        .to_string(),
                    retryable: true,
                    account_id: Some(account.account_id),
                    upstream_route_key: Some(account.upstream_route_key()),
                });
                continue;
            }
            Err(err) => {
                return Err(WsPrepareError {
                    status: StatusCode::BAD_GATEWAY,
                    message: format!("failed to inspect websocket support tag: {err}"),
                });
            }
        }

        match prepare_single_upstream_websocket_attempt(
            state.clone(),
            proxy_request_id,
            original_uri,
            headers,
            runtime_timeouts,
            trace,
            account,
            ws_retry_account_ids.len() + 1,
        )
        .await
        {
            Ok(prepared) => return Ok(prepared),
            Err(failure) if failure.retryable => {
                if let Some(account_id) = failure.account_id {
                    ws_retry_account_ids.insert(account_id);
                }
                if let Err(err) = exclude_retryable_ws_attempt_failure(
                    &failure,
                    &mut excluded_account_ids,
                    &mut excluded_upstream_route_keys,
                ) {
                    warn!(
                        proxy_request_id,
                        error = %failure.message,
                        "websocket upstream attempt failed without account exclusion context"
                    );
                    return Err(err);
                }
                last_failure = Some(failure);
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

fn exclude_retryable_ws_attempt_failure(
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

async fn account_supports_upstream_websocket(
    state: &AppState,
    account: &PoolResolvedAccount,
) -> Result<bool> {
    Ok(!account_has_websocket_unsupported_tag(&state.pool, account.account_id).await?)
}

async fn prepare_single_upstream_websocket_attempt(
    state: Arc<AppState>,
    proxy_request_id: u64,
    original_uri: &Uri,
    headers: &HeaderMap,
    runtime_timeouts: &PoolRoutingTimeoutSettingsResolved,
    trace: &PoolUpstreamAttemptTraceContext,
    account: PoolResolvedAccount,
    attempt_index: usize,
) -> Result<PreparedUpstreamWebSocket, WsAttemptFailure> {
    let reservation_key = build_pool_routing_reservation_key(proxy_request_id);
    reserve_pool_routing_account(state.as_ref(), &reservation_key, &account);
    let mut reservation_guard = PoolRoutingReservationGuard::new(state.clone(), reservation_key);

    let (forward_proxy_scope, selected_proxy, _client) =
        match select_pool_account_forward_proxy_client(state.as_ref(), &account).await {
            Ok(selection) => selection,
            Err(message) => {
                reservation_guard.release();
                return Err(WsAttemptFailure {
                    status: StatusCode::BAD_GATEWAY,
                    message,
                    retryable: false,
                    account_id: Some(account.account_id),
                    upstream_route_key: Some(account.upstream_route_key()),
                });
            }
        };

    let upstream_url = match build_websocket_upstream_url(&account.upstream_base_url, original_uri) {
        Ok(url) => url,
        Err(err) => {
            reservation_guard.release();
            return Err(WsAttemptFailure {
                status: StatusCode::BAD_GATEWAY,
                message: format!("failed to build pool websocket upstream url: {err}"),
                retryable: false,
                account_id: Some(account.account_id),
                upstream_route_key: Some(account.upstream_route_key()),
            });
        }
    };

    let proxy_binding_key_snapshot =
        live_first_proxy_binding_key_snapshot(state.as_ref(), Some(&selected_proxy)).await;
    let group_name_snapshot = normalize_pool_attempt_group_name(account.group_name.clone());
    let pending_attempt_record = Some(
        begin_pool_upstream_request_attempt_with_scope(
            &state.pool,
            trace,
            group_name_snapshot.as_deref(),
            proxy_binding_key_snapshot.as_deref(),
            account.account_id,
            account.upstream_route_key().as_str(),
            attempt_index as i64,
            attempt_index as i64,
            0,
            shanghai_now_string().as_str(),
        )
        .await,
    );
    if let Some(pending) = pending_attempt_record.as_ref()
        && let Err(err) = advance_pool_upstream_request_attempt_phase(
            state.as_ref(),
            pending,
            POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_SENDING_REQUEST,
        )
        .await
    {
        warn!(
            invoke_id = %pending.invoke_id,
            error = %err,
            "failed to advance websocket pool attempt into sending-request phase"
        );
    }
    let mut deferred_cleanup_guard = pending_attempt_record
        .as_ref()
        .map(|pending| PoolEarlyPhaseOrphanCleanupGuard::new(state.clone(), pending.clone()));

    let request = match build_upstream_ws_request(&upstream_url, headers, &account) {
        Ok(request) => request,
        Err(err) => {
            let message = format!("failed to build upstream websocket request: {err}");
            finalize_ws_attempt(
                state.as_ref(),
                pending_attempt_record.as_ref(),
                POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_TRANSPORT_FAILURE,
                Some(PROXY_FAILURE_FAILED_CONTACT_UPSTREAM),
                Some(message.as_str()),
                None,
                None,
                None,
            )
            .await;
            complete_deferred_pool_early_phase_cleanup_guard(&mut deferred_cleanup_guard);
            reservation_guard.release();
            return Err(WsAttemptFailure {
                status: StatusCode::BAD_GATEWAY,
                message,
                retryable: false,
                account_id: Some(account.account_id),
                upstream_route_key: Some(account.upstream_route_key()),
            });
        }
    };

    let connect_started = Instant::now();
    let connect_timeout = runtime_timeouts.default_send_timeout;
    let connect_result = timeout(
        connect_timeout,
        connect_upstream_websocket(request, &upstream_url, selected_proxy.endpoint_url.as_ref()),
    )
    .await;
    let (upstream, selected_subprotocol) = match connect_result {
        Ok(Ok((stream, response))) => {
            let selected_subprotocol = response
                .headers()
                .get(HeaderName::from_static("sec-websocket-protocol"))
                .and_then(|value| value.to_str().ok())
                .map(str::to_string);
            (stream, selected_subprotocol)
        }
        Ok(Err(err)) => {
            let message = format!("failed to contact websocket upstream: {err}");
            let should_mark_ws_unsupported = websocket_upstream_error_marks_account_ws_unsupported(&err);
            finalize_ws_attempt(
                state.as_ref(),
                pending_attempt_record.as_ref(),
                POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_TRANSPORT_FAILURE,
                Some(PROXY_FAILURE_FAILED_CONTACT_UPSTREAM),
                Some(message.as_str()),
                Some(elapsed_ms(connect_started)),
                None,
                None,
            )
            .await;
            complete_deferred_pool_early_phase_cleanup_guard(&mut deferred_cleanup_guard);
            record_pool_account_forward_proxy_result(
                state.as_ref(),
                &forward_proxy_scope,
                &selected_proxy,
                ForwardProxyRouteResultKind::NetworkFailure,
            )
            .await;
            if let Err(err) = record_pool_route_transport_failure(
                &state.pool,
                account.account_id,
                trace.sticky_key.as_deref(),
                &message,
                Some(trace.invoke_id.as_str()),
            )
            .await
            {
                warn!(
                    invoke_id = %trace.invoke_id,
                    account_id = account.account_id,
                    error = %err,
                    "failed to record websocket pool route transport failure"
                );
            }
            if should_mark_ws_unsupported {
                if let Err(err) =
                    ensure_account_has_websocket_unsupported_tag(&state.pool, account.account_id)
                        .await
                {
                    warn!(
                        invoke_id = %trace.invoke_id,
                        account_id = account.account_id,
                        error = %err,
                        "failed to mark upstream account as websocket unsupported"
                    );
                }
            }
            reservation_guard.release();
            return Err(WsAttemptFailure {
                status: StatusCode::BAD_GATEWAY,
                message,
                retryable: true,
                account_id: Some(account.account_id),
                upstream_route_key: Some(account.upstream_route_key()),
            });
        }
        Err(_) => {
            let message = proxy_request_send_timeout_message(None, connect_timeout);
            finalize_ws_attempt(
                state.as_ref(),
                pending_attempt_record.as_ref(),
                POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_TRANSPORT_FAILURE,
                Some(PROXY_FAILURE_UPSTREAM_HANDSHAKE_TIMEOUT),
                Some(message.as_str()),
                Some(elapsed_ms(connect_started)),
                None,
                None,
            )
            .await;
            complete_deferred_pool_early_phase_cleanup_guard(&mut deferred_cleanup_guard);
            record_pool_account_forward_proxy_result(
                state.as_ref(),
                &forward_proxy_scope,
                &selected_proxy,
                ForwardProxyRouteResultKind::NetworkFailure,
            )
            .await;
            if let Err(err) = record_pool_route_transport_failure(
                &state.pool,
                account.account_id,
                trace.sticky_key.as_deref(),
                &message,
                Some(trace.invoke_id.as_str()),
            )
            .await
            {
                warn!(
                    invoke_id = %trace.invoke_id,
                    account_id = account.account_id,
                    error = %err,
                    "failed to record websocket pool route timeout failure"
                );
            }
            reservation_guard.release();
            return Err(WsAttemptFailure {
                status: StatusCode::BAD_GATEWAY,
                message,
                retryable: true,
                account_id: Some(account.account_id),
                upstream_route_key: Some(account.upstream_route_key()),
            });
        }
    };

    if let Some(pending) = pending_attempt_record.as_ref()
        && let Err(err) = advance_pool_upstream_request_attempt_phase(
            state.as_ref(),
            pending,
            POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_STREAMING_RESPONSE,
        )
        .await
    {
        warn!(
            invoke_id = %pending.invoke_id,
            error = %err,
            "failed to advance websocket pool attempt into streaming-response phase"
        );
    }

    record_pool_account_forward_proxy_result(
        state.as_ref(),
        &forward_proxy_scope,
        &selected_proxy,
        ForwardProxyRouteResultKind::CompletedRequest,
    )
    .await;
    if let Err(err) = record_pool_route_success(
        &state.pool,
        account.account_id,
        Utc::now(),
        trace.sticky_key.as_deref(),
        Some(trace.invoke_id.as_str()),
    )
    .await
    {
        warn!(
            invoke_id = %trace.invoke_id,
            account_id = account.account_id,
            error = %err,
            "failed to record websocket pool route success"
        );
    }

    Ok(PreparedUpstreamWebSocket {
        upstream,
        pending_attempt_record,
        deferred_cleanup_guard,
        reservation_guard,
        account,
        trace: trace.clone(),
        selected_subprotocol,
        connect_latency_ms: elapsed_ms(connect_started),
    })
}

async fn proxy_websocket_tunnel(
    state: Arc<AppState>,
    downstream: WebSocket,
    prepared: PreparedUpstreamWebSocket,
    _proxy_request_permit: ProxyRequestConcurrencyPermit,
) {
    let PreparedUpstreamWebSocket {
        upstream,
        pending_attempt_record,
        mut deferred_cleanup_guard,
        mut reservation_guard,
        account,
        trace,
        connect_latency_ms,
        selected_subprotocol: _,
    } = prepared;
    let stream_started = Instant::now();
    let (mut downstream_tx, mut downstream_rx) = downstream.split();
    let (mut upstream_tx, mut upstream_rx) = upstream.split();
    let mut failure: Option<String> = None;
    let mut upstream_route_failure: Option<String> = None;
    let mut usage_tracker = WsUsageTracker::new(account, trace);
    let mut saw_downstream_request = false;
    let mut saw_terminal_upstream_event = false;

    loop {
        tokio::select! {
            downstream_msg = downstream_rx.next() => {
                match downstream_msg {
                    Some(Ok(message)) => {
                        let close_seen = matches!(message, AxumWsMessage::Close(_));
                        if matches!(message, AxumWsMessage::Text(_) | AxumWsMessage::Binary(_)) {
                            saw_downstream_request = true;
                        }
                        match axum_to_tungstenite_message(message) {
                            Some(message) => {
                                if let Err(err) = upstream_tx.send(message).await {
                                    let message = format!("failed to forward downstream websocket frame upstream: {err}");
                                    upstream_route_failure = Some(message.clone());
                                    failure = Some(message);
                                    break;
                                }
                            }
                            None => {}
                        }
                        if close_seen {
                            break;
                        }
                    }
                    Some(Err(err)) => {
                        failure = Some(format!("downstream websocket error: {err}"));
                        break;
                    }
                    None => break,
                }
            }
            upstream_msg = upstream_rx.next() => {
                match upstream_msg {
                    Some(Ok(message)) => {
                        let close_seen = matches!(message, TungsteniteMessage::Close(_));
                        let upstream_text = match &message {
                            TungsteniteMessage::Text(text) => Some(text.as_str().to_owned()),
                            _ => None,
                        };
                        if upstream_text
                            .as_deref()
                            .is_some_and(ws_text_event_is_terminal)
                        {
                            saw_terminal_upstream_event = true;
                        }
                        if let Some(message) = tungstenite_to_axum_message(message)
                            && let Err(err) = downstream_tx.send(message).await
                        {
                            failure = Some(format!("failed to forward upstream websocket frame downstream: {err}"));
                            break;
                        }
                        if let Some(text) = upstream_text.as_deref() {
                            usage_tracker.observe_upstream_text(state.as_ref(), text).await;
                        }
                        if close_seen {
                            if saw_downstream_request && !saw_terminal_upstream_event {
                                failure = Some(
                                    "upstream websocket closed before response.completed".to_string(),
                                );
                            }
                            break;
                        }
                    }
                    Some(Err(err)) => {
                        let message = format!("upstream websocket error: {err}");
                        upstream_route_failure = Some(message.clone());
                        failure = Some(message);
                        let _ = downstream_tx
                            .send(AxumWsMessage::Close(Some(axum::extract::ws::CloseFrame {
                                code: axum::extract::ws::close_code::AGAIN,
                                reason: "upstream_unavailable; retry".into(),
                            })))
                            .await;
                        break;
                    }
                    None => {
                        let message = if saw_downstream_request && !saw_terminal_upstream_event {
                            "upstream websocket closed before response.completed".to_string()
                        } else {
                            "upstream websocket closed without a close frame".to_string()
                        };
                        upstream_route_failure = Some(message.clone());
                        failure = Some(message);
                        let _ = downstream_tx
                            .send(AxumWsMessage::Close(Some(axum::extract::ws::CloseFrame {
                                code: axum::extract::ws::close_code::AGAIN,
                                reason: "upstream_unavailable; retry".into(),
                            })))
                            .await;
                        break;
                    },
                }
            }
        }
    }

    let status = if failure.is_some() {
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_TRANSPORT_FAILURE
    } else {
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS
    };
    finalize_ws_attempt(
        state.as_ref(),
        pending_attempt_record.as_ref(),
        status,
        failure.as_ref().map(|_| PROXY_FAILURE_UPSTREAM_STREAM_ERROR),
        failure.as_deref(),
        Some(connect_latency_ms),
        None,
        Some(elapsed_ms(stream_started)),
    )
    .await;
    if let Some(message) = upstream_route_failure.as_deref()
        && let Err(err) = record_pool_route_transport_failure(
            &state.pool,
            usage_tracker.account.account_id,
            usage_tracker.trace.sticky_key.as_deref(),
            message,
            Some(usage_tracker.trace.invoke_id.as_str()),
        )
        .await
    {
        warn!(
            invoke_id = %usage_tracker.trace.invoke_id,
            account_id = usage_tracker.account.account_id,
            error = %err,
            "failed to record post-upgrade websocket pool route transport failure"
        );
    }
    complete_deferred_pool_early_phase_cleanup_guard(&mut deferred_cleanup_guard);
    reservation_guard.release();
}

struct WsUsageTracker {
    account: PoolResolvedAccount,
    trace: PoolUpstreamAttemptTraceContext,
    ordinal: u64,
}

impl WsUsageTracker {
    fn new(account: PoolResolvedAccount, trace: PoolUpstreamAttemptTraceContext) -> Self {
        Self {
            account,
            trace,
            ordinal: 0,
        }
    }

    async fn observe_upstream_text(&mut self, state: &AppState, text: &str) {
        let Some(event) = parse_ws_usage_event(text) else {
            return;
        };
        self.ordinal = self.ordinal.saturating_add(1);
        if let Err(err) = persist_ws_usage_event(
            state,
            &self.account,
            &self.trace,
            self.ordinal,
            event,
            text,
        )
        .await
        {
            warn!(
                invoke_id = %self.trace.invoke_id,
                account_id = self.account.account_id,
                error = %err,
                "failed to persist websocket usage event"
            );
        }
    }
}

struct WsUsageEvent {
    event_type: String,
    response_id: Option<String>,
    model: Option<String>,
    service_tier: Option<String>,
    usage: ParsedUsage,
}

fn parse_ws_usage_event(text: &str) -> Option<WsUsageEvent> {
    let value = serde_json::from_str::<Value>(text).ok()?;
    let event_type = value.get("type")?.as_str()?.trim().to_string();
    if !ws_event_type_has_billable_usage(event_type.as_str()) {
        return None;
    }
    let usage_value = value.pointer("/response/usage")?;
    let usage = parse_usage_value(usage_value);
    if usage.input_tokens.is_none() || usage.output_tokens.is_none() {
        return None;
    }
    Some(WsUsageEvent {
        event_type,
        response_id: value
            .pointer("/response/id")
            .or_else(|| value.get("response_id"))
            .or_else(|| value.get("id"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string),
        model: value
            .pointer("/response/model")
            .or_else(|| value.get("model"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string),
        service_tier: extract_service_tier_from_payload(&value),
        usage,
    })
}

fn ws_event_type_has_billable_usage(event_type: &str) -> bool {
    matches!(
        event_type,
        "response.completed" | "response.done" | "response.failed"
    )
}

fn websocket_upstream_error_marks_account_ws_unsupported(err: &tungstenite::Error) -> bool {
    match err {
        tungstenite::Error::Http(response) => matches!(
            response.status(),
            tungstenite::http::StatusCode::FORBIDDEN
                | tungstenite::http::StatusCode::NOT_FOUND
                | tungstenite::http::StatusCode::METHOD_NOT_ALLOWED
                | tungstenite::http::StatusCode::UPGRADE_REQUIRED
                | tungstenite::http::StatusCode::NOT_IMPLEMENTED
        ),
        _ => false,
    }
}

fn ws_text_event_is_terminal(event_type: &str) -> bool {
    let Ok(value) = serde_json::from_str::<Value>(event_type) else {
        return false;
    };
    value
        .get("type")
        .and_then(Value::as_str)
        .is_some_and(ws_event_type_has_billable_usage)
}

async fn persist_ws_usage_event(
    state: &AppState,
    account: &PoolResolvedAccount,
    trace: &PoolUpstreamAttemptTraceContext,
    ordinal: u64,
    event: WsUsageEvent,
    raw_event: &str,
) -> Result<()> {
    let model = event.model.as_deref();
    let (billing_service_tier, pricing_mode) =
        resolve_proxy_billing_service_tier_and_pricing_mode_for_account(
            None,
            None,
            event.service_tier.as_deref(),
            Some(account),
        );
    let (cost, cost_estimated, price_version) = estimate_proxy_cost_from_shared_catalog(
        &state.pricing_catalog,
        model,
        &event.usage,
        billing_service_tier.as_deref(),
        pricing_mode,
    )
    .await;
    let occurred_at = shanghai_now_string();
    let response_id = event.response_id.as_deref();
    let invoke_id = response_id
        .map(|id| format!("{}-{id}", trace.invoke_id))
        .unwrap_or_else(|| format!("{}-turn-{ordinal}", trace.invoke_id));
    let payload = build_proxy_payload_summary(ProxyPayloadSummary {
        target: ProxyCaptureTarget::Responses,
        status: StatusCode::OK,
        is_stream: true,
        request_model: None,
        requested_service_tier: None,
        billing_service_tier: billing_service_tier.as_deref(),
        reasoning_effort: None,
        response_model: model,
        usage_missing_reason: None,
        request_parse_error: None,
        failure_kind: None,
        requester_ip: trace.requester_ip.as_deref(),
        upstream_scope: INVOCATION_UPSTREAM_SCOPE_INTERNAL,
        route_mode: INVOCATION_ROUTE_MODE_POOL,
        sticky_key: trace.sticky_key.as_deref(),
        prompt_cache_key: trace.sticky_key.as_deref(),
        prompt_cache_key_attribution_source: trace.sticky_key.as_ref().map(|_| "websocket_trace"),
        client_fingerprint: None,
        client_header_fingerprints: None,
        upstream_account_id: Some(account.account_id),
        upstream_account_name: Some(account.display_name.as_str()),
        upstream_account_kind: Some(account.kind.as_str()),
        upstream_base_url_host: account.upstream_base_url.host_str(),
        oauth_account_header_attached: oauth_account_header_attached_for_account(Some(account)),
        oauth_account_id_shape: oauth_account_id_shape_for_account(Some(account)),
        oauth_forwarded_header_count: None,
        oauth_forwarded_header_names: None,
        oauth_fingerprint_version: None,
        oauth_forwarded_header_fingerprints: None,
        oauth_prompt_cache_header_forwarded: None,
        oauth_request_body_prefix_fingerprint: None,
        oauth_request_body_prefix_bytes: None,
        oauth_request_body_snapshot_kind: None,
        oauth_responses_body_mode: None,
        oauth_responses_rewrite: None,
        service_tier: event.service_tier.as_deref(),
        stream_terminal_event: Some(event.event_type.as_str()),
        upstream_error_code: None,
        upstream_error_message: None,
        downstream_status_code: None,
        downstream_error_message: None,
        upstream_request_id: response_id,
        response_content_encoding: None,
        proxy_display_name: None,
        proxy_weight_delta: None,
        pool_attempt_count: None,
        pool_distinct_account_count: None,
        pool_attempt_terminal_reason: None,
    });
    let is_failed_terminal_event = event.event_type == "response.failed";
    let failure_kind = ws_terminal_event_failure_kind(&event.event_type);
    persist_and_broadcast_proxy_capture_runtime_snapshot(
        state,
        ProxyCaptureRecord {
            invoke_id,
            occurred_at,
            model: event.model,
            usage: event.usage,
            cost,
            cost_estimated,
            price_version,
            status: if is_failed_terminal_event {
                "failed".to_string()
            } else {
                "success".to_string()
            },
            error_message: failure_kind.map(|value| format!("[{value}] response.failed")),
            failure_kind: failure_kind.map(str::to_string),
            payload: Some(payload),
            raw_response: raw_event.to_string(),
            req_raw: RawPayloadMeta::default(),
            resp_raw: RawPayloadMeta::default(),
            timings: StageTimings {
                t_total_ms: 0.0,
                t_req_read_ms: 0.0,
                t_req_parse_ms: 0.0,
                t_upstream_connect_ms: 0.0,
                t_upstream_ttfb_ms: 0.0,
                t_upstream_stream_ms: 0.0,
                t_resp_parse_ms: 0.0,
                t_persist_ms: 0.0,
            },
        },
    )
    .await
}

fn ws_terminal_event_failure_kind(event_type: &str) -> Option<&'static str> {
    if event_type == "response.failed" {
        Some(PROXY_FAILURE_UPSTREAM_RESPONSE_FAILED)
    } else {
        None
    }
}

async fn finalize_ws_attempt(
    state: &AppState,
    pending: Option<&PendingPoolAttemptRecord>,
    status: &str,
    failure_kind: Option<&str>,
    error_message: Option<&str>,
    connect_latency_ms: Option<f64>,
    first_byte_latency_ms: Option<f64>,
    stream_latency_ms: Option<f64>,
) {
    let Some(pending) = pending else {
        return;
    };
    let finished_at = shanghai_now_string();
    if let Err(err) = finalize_pool_upstream_request_attempt(
        &state.pool,
        pending,
        finished_at.as_str(),
        status,
        None,
        None,
        failure_kind,
        error_message,
        None,
        connect_latency_ms,
        first_byte_latency_ms,
        stream_latency_ms,
        None,
        None,
        None,
    )
    .await
    {
        warn!(
            invoke_id = %pending.invoke_id,
            error = %err,
            "failed to finalize websocket pool attempt"
        );
    }
    if let Err(err) = broadcast_pool_upstream_attempts_snapshot(state, &pending.invoke_id).await {
        warn!(
            invoke_id = %pending.invoke_id,
            error = %err,
            "failed to broadcast websocket pool attempt snapshot"
        );
    }
}

pub(crate) fn build_websocket_upstream_url(base: &Url, original_uri: &Uri) -> Result<Url> {
    let mut ws_base = base.clone();
    let ws_scheme = match base.scheme() {
        "https" | "wss" => "wss",
        "http" | "ws" => "ws",
        scheme => bail!("unsupported websocket upstream base scheme: {scheme}"),
    };
    ws_base
        .set_scheme(ws_scheme)
        .map_err(|_| anyhow!("failed to set websocket upstream scheme"))?;
    build_proxy_upstream_url(&ws_base, original_uri)
}

fn build_upstream_ws_request(
    upstream_url: &Url,
    headers: &HeaderMap,
    account: &PoolResolvedAccount,
) -> Result<TungsteniteRequest<()>> {
    let mut request = upstream_url
        .as_str()
        .into_client_request()
        .context("failed to create websocket client request")?;
    let connection_scoped = connection_scoped_header_names(headers);
    for (name, value) in headers {
        if *name == header::AUTHORIZATION || *name == header::CONTENT_LENGTH {
            continue;
        }
        if should_forward_websocket_header(name, &connection_scoped) {
            request.headers_mut().insert(name.clone(), value.clone());
        }
    }
    let authorization = match &account.auth {
        PoolResolvedAuth::ApiKey { authorization } => authorization.clone(),
        PoolResolvedAuth::Oauth {
            access_token,
            chatgpt_account_id,
        } => {
            request.headers_mut().insert(
                HeaderName::from_static("openai-beta"),
                HeaderValue::from_static("responses=experimental"),
            );
            if let Some(account_id) = chatgpt_account_id
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                request.headers_mut().insert(
                    HeaderName::from_static("chatgpt-account-id"),
                    HeaderValue::from_str(account_id)
                        .context("invalid ChatGPT account header value")?,
                );
            }
            format!("Bearer {access_token}")
        }
    };
    request.headers_mut().insert(
        header::AUTHORIZATION,
        HeaderValue::from_str(&authorization).context("invalid upstream authorization header")?,
    );
    Ok(request)
}

async fn connect_upstream_websocket(
    request: TungsteniteRequest<()>,
    upstream_url: &Url,
    forward_proxy_url: Option<&Url>,
) -> std::result::Result<(UpstreamWsStream, tungstenite::handshake::client::Response), tungstenite::Error>
{
    let Some(forward_proxy_url) = forward_proxy_url else {
        let stream = connect_tcp_target(upstream_url).await?;
        return client_async_tls_with_config(request, Box::new(stream) as BoxedWsIo, None, None).await;
    };

    let proxy_host = forward_proxy_url
        .host_str()
        .ok_or_else(|| tungstenite::Error::Io(io::Error::new(
            io::ErrorKind::InvalidInput,
            "forward proxy endpoint is missing host",
        )))?;
    let proxy_port = forward_proxy_url.port_or_known_default().ok_or_else(|| {
        tungstenite::Error::Io(io::Error::new(
            io::ErrorKind::InvalidInput,
            "forward proxy endpoint is missing port",
        ))
    })?;
    let upstream_host = upstream_url
        .host_str()
        .ok_or_else(|| tungstenite::Error::Url(tungstenite::error::UrlError::NoHostName))?;
    let upstream_port = upstream_url
        .port_or_known_default()
        .ok_or(tungstenite::Error::Url(
            tungstenite::error::UrlError::UnsupportedUrlScheme,
        ))?;
    let target_authority = if upstream_host.contains(':') {
        format!("[{upstream_host}]:{upstream_port}")
    } else {
        format!("{upstream_host}:{upstream_port}")
    };
    let proxy_scheme = forward_proxy_url.scheme();
    if matches!(proxy_scheme, "socks5" | "socks5h") {
        let socks_target_host = if proxy_scheme == "socks5" {
            resolve_socks5_local_target_host(upstream_host, upstream_port).await?
        } else {
            upstream_host.to_string()
        };
        let stream = connect_socks5_forward_proxy(
            forward_proxy_url,
            proxy_host,
            proxy_port,
            &socks_target_host,
            upstream_port,
        )
        .await?;
        return client_async_tls_with_config(request, Box::new(stream) as BoxedWsIo, None, None)
            .await;
    }
    if !matches!(proxy_scheme, "http" | "https") {
        return Err(tungstenite::Error::Io(io::Error::new(
            io::ErrorKind::Unsupported,
            format!("websocket proxy only supports HTTP CONNECT, HTTPS CONNECT, or SOCKS5 forward proxy endpoints, got {proxy_scheme}"),
        )));
    }

    let mut stream = connect_http_forward_proxy(forward_proxy_url, proxy_host, proxy_port).await?;
    let mut connect_request = format!(
        "CONNECT {target_authority} HTTP/1.1\r\nHost: {target_authority}\r\n"
    );
    if let Some(credential) = forward_proxy_basic_auth_credential(forward_proxy_url) {
        let encoded = base64::engine::general_purpose::STANDARD.encode(credential);
        connect_request.push_str("Proxy-Authorization: Basic ");
        connect_request.push_str(&encoded);
        connect_request.push_str("\r\n");
    }
    connect_request.push_str("\r\n");
    stream
        .write_all(connect_request.as_bytes())
        .await
        .map_err(tungstenite::Error::Io)?;

    let mut response = Vec::with_capacity(256);
    let mut buffer = [0_u8; 1024];
    loop {
        let read = stream.read(&mut buffer).await.map_err(tungstenite::Error::Io)?;
        if read == 0 {
            return Err(tungstenite::Error::Io(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "forward proxy closed before CONNECT response completed",
            )));
        }
        response.extend_from_slice(&buffer[..read]);
        if response.windows(4).any(|window| window == b"\r\n\r\n") {
            break;
        }
        if response.len() > 16 * 1024 {
            return Err(tungstenite::Error::Io(io::Error::new(
                io::ErrorKind::InvalidData,
                "forward proxy CONNECT response exceeded 16KiB",
            )));
        }
    }
    let header_end = response
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .ok_or_else(|| {
            tungstenite::Error::Io(io::Error::new(
                io::ErrorKind::InvalidData,
                "forward proxy CONNECT response missing header terminator",
            ))
        })?;
    let status_line_end = response
        .windows(2)
        .position(|window| window == b"\r\n")
        .ok_or_else(|| tungstenite::Error::Io(io::Error::new(
            io::ErrorKind::InvalidData,
            "forward proxy CONNECT response missing status line",
        )))?;
    let status_line = std::str::from_utf8(&response[..status_line_end]).map_err(|err| {
        tungstenite::Error::Io(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("forward proxy CONNECT status line is not UTF-8: {err}"),
        ))
    })?;
    if !status_line.starts_with("HTTP/1.1 200") && !status_line.starts_with("HTTP/1.0 200") {
        return Err(tungstenite::Error::Io(io::Error::other(format!(
            "forward proxy CONNECT failed: {status_line}"
        ))));
    }
    let extra_read = response[(header_end + 4)..].to_vec();
    if !extra_read.is_empty() {
        stream = Box::new(PrefixedIo::new(extra_read, stream));
    }

    client_async_tls_with_config(request, stream, None, None).await
}

async fn connect_tcp_target(upstream_url: &Url) -> std::result::Result<TcpStream, tungstenite::Error> {
    let host = upstream_url
        .host_str()
        .ok_or_else(|| tungstenite::Error::Url(tungstenite::error::UrlError::NoHostName))?;
    let port = upstream_url
        .port_or_known_default()
        .ok_or(tungstenite::Error::Url(
            tungstenite::error::UrlError::UnsupportedUrlScheme,
        ))?;
    TcpStream::connect((host, port))
        .await
        .map_err(tungstenite::Error::Io)
}

async fn connect_http_forward_proxy(
    forward_proxy_url: &Url,
    proxy_host: &str,
    proxy_port: u16,
) -> std::result::Result<BoxedWsIo, tungstenite::Error> {
    let stream = TcpStream::connect((proxy_host, proxy_port))
        .await
        .map_err(tungstenite::Error::Io)?;
    if forward_proxy_url.scheme() != "https" {
        return Ok(Box::new(stream));
    }
    let root_store = rustls::RootCertStore {
        roots: webpki_roots::TLS_SERVER_ROOTS.to_vec(),
    };
    let config = rustls::ClientConfig::builder()
        .with_root_certificates(root_store)
        .with_no_client_auth();
    let connector = TlsConnector::from(Arc::new(config));
    let server_name = rustls_pki_types::ServerName::try_from(proxy_host.to_string()).map_err(|err| {
        tungstenite::Error::Io(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("invalid HTTPS forward proxy host for TLS SNI: {err}"),
        ))
    })?;
    let stream = connector
        .connect(server_name, stream)
        .await
        .map_err(tungstenite::Error::Io)?;
    Ok(Box::new(stream))
}

async fn connect_socks5_forward_proxy(
    forward_proxy_url: &Url,
    proxy_host: &str,
    proxy_port: u16,
    upstream_host: &str,
    upstream_port: u16,
) -> std::result::Result<TcpStream, tungstenite::Error> {
    let mut stream = TcpStream::connect((proxy_host, proxy_port))
        .await
        .map_err(tungstenite::Error::Io)?;
    let username = forward_proxy_username(forward_proxy_url);
    let password = forward_proxy_password(forward_proxy_url).unwrap_or_default();
    let use_password_auth = !username.is_empty();
    if use_password_auth {
        stream
            .write_all(&[0x05, 0x02, 0x00, 0x02])
            .await
            .map_err(tungstenite::Error::Io)?;
    } else {
        stream
            .write_all(&[0x05, 0x01, 0x00])
            .await
            .map_err(tungstenite::Error::Io)?;
    }
    let mut method_response = [0_u8; 2];
    stream
        .read_exact(&mut method_response)
        .await
        .map_err(tungstenite::Error::Io)?;
    if method_response[0] != 0x05 || method_response[1] == 0xff {
        return Err(tungstenite::Error::Io(io::Error::other(
            "SOCKS5 forward proxy did not accept an authentication method",
        )));
    }
    if method_response[1] == 0x02 {
        let username = username.as_bytes();
        let password = password.as_bytes();
        if username.len() > u8::MAX as usize || password.len() > u8::MAX as usize {
            return Err(tungstenite::Error::Io(io::Error::new(
                io::ErrorKind::InvalidInput,
                "SOCKS5 credentials exceed 255 bytes",
            )));
        }
        let mut auth_request = Vec::with_capacity(username.len() + password.len() + 3);
        auth_request.push(0x01);
        auth_request.push(username.len() as u8);
        auth_request.extend_from_slice(username);
        auth_request.push(password.len() as u8);
        auth_request.extend_from_slice(password);
        stream
            .write_all(&auth_request)
            .await
            .map_err(tungstenite::Error::Io)?;
        let mut auth_response = [0_u8; 2];
        stream
            .read_exact(&mut auth_response)
            .await
            .map_err(tungstenite::Error::Io)?;
        if auth_response != [0x01, 0x00] {
            return Err(tungstenite::Error::Io(io::Error::other(
                "SOCKS5 forward proxy rejected username/password authentication",
            )));
        }
    } else if method_response[1] != 0x00 {
        return Err(tungstenite::Error::Io(io::Error::other(format!(
            "SOCKS5 forward proxy selected unsupported authentication method {}",
            method_response[1]
        ))));
    }

    let mut connect_request = Vec::with_capacity(8 + upstream_host.len());
    connect_request.extend_from_slice(&[0x05, 0x01, 0x00]);
    if let Ok(ip) = upstream_host.parse::<IpAddr>() {
        match ip {
            IpAddr::V4(addr) => {
                connect_request.push(0x01);
                connect_request.extend_from_slice(&addr.octets());
            }
            IpAddr::V6(addr) => {
                connect_request.push(0x04);
                connect_request.extend_from_slice(&addr.octets());
            }
        }
    } else {
        let host = upstream_host.as_bytes();
        if host.len() > u8::MAX as usize {
            return Err(tungstenite::Error::Io(io::Error::new(
                io::ErrorKind::InvalidInput,
                "SOCKS5 upstream host exceeds 255 bytes",
            )));
        }
        connect_request.push(0x03);
        connect_request.push(host.len() as u8);
        connect_request.extend_from_slice(host);
    }
    connect_request.extend_from_slice(&upstream_port.to_be_bytes());
    stream
        .write_all(&connect_request)
        .await
        .map_err(tungstenite::Error::Io)?;

    let mut reply_head = [0_u8; 4];
    stream
        .read_exact(&mut reply_head)
        .await
        .map_err(tungstenite::Error::Io)?;
    if reply_head[0] != 0x05 || reply_head[1] != 0x00 {
        return Err(tungstenite::Error::Io(io::Error::other(format!(
            "SOCKS5 forward proxy connect failed with status {}",
            reply_head[1]
        ))));
    }
    let address_len = match reply_head[3] {
        0x01 => 4,
        0x03 => {
            let mut len = [0_u8; 1];
            stream
                .read_exact(&mut len)
                .await
                .map_err(tungstenite::Error::Io)?;
            len[0] as usize
        }
        0x04 => 16,
        atyp => {
            return Err(tungstenite::Error::Io(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("SOCKS5 forward proxy returned unsupported address type {atyp}"),
            )));
        }
    };
    let mut discard = vec![0_u8; address_len + 2];
    stream
        .read_exact(&mut discard)
        .await
        .map_err(tungstenite::Error::Io)?;
    Ok(stream)
}

async fn resolve_socks5_local_target_host(
    upstream_host: &str,
    upstream_port: u16,
) -> std::result::Result<String, tungstenite::Error> {
    if upstream_host.parse::<IpAddr>().is_ok() {
        return Ok(upstream_host.to_string());
    }
    let mut addresses = tokio::net::lookup_host((upstream_host, upstream_port))
        .await
        .map_err(tungstenite::Error::Io)?;
    let Some(address) = addresses.next() else {
        return Err(tungstenite::Error::Io(io::Error::new(
            io::ErrorKind::NotFound,
            format!("no local DNS address resolved for SOCKS5 target {upstream_host}"),
        )));
    };
    Ok(address.ip().to_string())
}

fn forward_proxy_basic_auth_credential(forward_proxy_url: &Url) -> Option<String> {
    let username = forward_proxy_username(forward_proxy_url);
    if username.is_empty() {
        return None;
    }
    Some(match forward_proxy_password(forward_proxy_url) {
        Some(password) => format!("{username}:{password}"),
        None => username,
    })
}

fn forward_proxy_username(forward_proxy_url: &Url) -> String {
    percent_decode_once_lossy(forward_proxy_url.username())
}

fn forward_proxy_password(forward_proxy_url: &Url) -> Option<String> {
    forward_proxy_url.password().map(percent_decode_once_lossy)
}

fn should_forward_websocket_header(
    name: &HeaderName,
    connection_scoped: &HashSet<HeaderName>,
) -> bool {
    should_forward_proxy_header(name, connection_scoped)
        && !matches!(
            name.as_str(),
            "sec-websocket-accept"
                | "sec-websocket-extensions"
                | "sec-websocket-key"
                | "sec-websocket-version"
        )
}

fn axum_to_tungstenite_message(message: AxumWsMessage) -> Option<TungsteniteMessage> {
    match message {
        AxumWsMessage::Text(value) => Some(TungsteniteMessage::Text(value.into())),
        AxumWsMessage::Binary(value) => Some(TungsteniteMessage::Binary(value.into())),
        AxumWsMessage::Ping(value) => Some(TungsteniteMessage::Ping(value.into())),
        AxumWsMessage::Pong(value) => Some(TungsteniteMessage::Pong(value.into())),
        AxumWsMessage::Close(frame) => Some(TungsteniteMessage::Close(frame.map(|frame| {
            tungstenite::protocol::CloseFrame {
                code: tungstenite::protocol::frame::coding::CloseCode::from(u16::from(frame.code)),
                reason: frame.reason.to_string().into(),
            }
        }))),
    }
}

fn tungstenite_to_axum_message(message: TungsteniteMessage) -> Option<AxumWsMessage> {
    match message {
        TungsteniteMessage::Text(value) => Some(AxumWsMessage::Text(value.to_string())),
        TungsteniteMessage::Binary(value) => Some(AxumWsMessage::Binary(value.to_vec())),
        TungsteniteMessage::Ping(value) => Some(AxumWsMessage::Ping(value.to_vec())),
        TungsteniteMessage::Pong(value) => Some(AxumWsMessage::Pong(value.to_vec())),
        TungsteniteMessage::Close(frame) => Some(AxumWsMessage::Close(frame.map(|frame| {
            axum::extract::ws::CloseFrame {
                code: u16::from(frame.code).into(),
                reason: frame.reason.to_string().into(),
            }
        }))),
        TungsteniteMessage::Frame(_) => None,
    }
}

#[cfg(test)]
mod websocket_tests {
    use super::*;

    fn api_key_account(upstream_base_url: Url) -> PoolResolvedAccount {
        PoolResolvedAccount {
            account_id: 42,
            display_name: "ws-test".to_string(),
            kind: "api_key".to_string(),
            auth: PoolResolvedAuth::ApiKey {
                authorization: "Bearer upstream-secret".to_string(),
            },
            group_name: None,
            bound_proxy_keys: Vec::new(),
            forward_proxy_scope: ForwardProxyRouteScope::Automatic,
            group_upstream_429_retry_enabled: false,
            group_upstream_429_max_retries: 0,
            fast_mode_rewrite_mode: TagFastModeRewriteMode::default(),
            upstream_base_url,
            routing_source: PoolRoutingSelectionSource::FreshAssignment,
        }
    }

    fn oauth_account(upstream_base_url: Url) -> PoolResolvedAccount {
        PoolResolvedAccount {
            account_id: 43,
            display_name: "ws-oauth-test".to_string(),
            kind: "oauth".to_string(),
            auth: PoolResolvedAuth::Oauth {
                access_token: "oauth-upstream-token".to_string(),
                chatgpt_account_id: Some("acct-test".to_string()),
            },
            group_name: None,
            bound_proxy_keys: Vec::new(),
            forward_proxy_scope: ForwardProxyRouteScope::Automatic,
            group_upstream_429_retry_enabled: false,
            group_upstream_429_max_retries: 0,
            fast_mode_rewrite_mode: TagFastModeRewriteMode::default(),
            upstream_base_url,
            routing_source: PoolRoutingSelectionSource::FreshAssignment,
        }
    }

    #[test]
    fn websocket_upgrade_detection_is_case_insensitive() {
        let mut headers = HeaderMap::new();
        headers.insert(header::UPGRADE, HeaderValue::from_static("WebSocket"));

        assert!(is_websocket_upgrade_request(&headers));
    }

    #[test]
    fn websocket_upstream_url_maps_https_to_wss_and_preserves_base_path_query() {
        let base = Url::parse("https://api.example.test/gateway/").expect("valid base");
        let uri = "/v1/responses?model=gpt-5.5".parse::<Uri>().expect("valid uri");

        let target = build_websocket_upstream_url(&base, &uri).expect("ws url");

        assert_eq!(
            target.as_str(),
            "wss://api.example.test/gateway/v1/responses?model=gpt-5.5"
        );
    }

    #[test]
    fn websocket_upstream_url_maps_http_to_ws() {
        let base = Url::parse("http://127.0.0.1:9000").expect("valid base");
        let uri = "/v1/realtime".parse::<Uri>().expect("valid uri");

        let target = build_websocket_upstream_url(&base, &uri).expect("ws url");

        assert_eq!(target.as_str(), "ws://127.0.0.1:9000/v1/realtime");
    }

    #[test]
    fn upstream_ws_request_replaces_auth_and_drops_upgrade_hop_headers() {
        let account = api_key_account(Url::parse("https://api.example.test").expect("valid base"));
        let mut headers = HeaderMap::new();
        headers.insert(
            header::AUTHORIZATION,
            HeaderValue::from_static("Bearer downstream-pool-key"),
        );
        headers.insert(header::CONNECTION, HeaderValue::from_static("upgrade, x-drop-me"));
        headers.insert(header::UPGRADE, HeaderValue::from_static("websocket"));
        headers.insert(
            HeaderName::from_static("openai-beta"),
            HeaderValue::from_static("realtime=v1"),
        );
        headers.insert(
            HeaderName::from_static("x-drop-me"),
            HeaderValue::from_static("drop"),
        );
        headers.insert(
            HeaderName::from_static("sec-websocket-key"),
            HeaderValue::from_static("downstream-key"),
        );

        let request = build_upstream_ws_request(
            &Url::parse("wss://api.example.test/v1/responses").expect("valid target"),
            &headers,
            &account,
        )
        .expect("request");

        assert_eq!(
            request
                .headers()
                .get(header::AUTHORIZATION)
                .and_then(|value| value.to_str().ok()),
            Some("Bearer upstream-secret")
        );
        assert_eq!(
            request
                .headers()
                .get("openai-beta")
                .and_then(|value| value.to_str().ok()),
            Some("realtime=v1")
        );
        assert!(!request.headers().contains_key("x-drop-me"));
        assert_ne!(
            request
                .headers()
                .get("sec-websocket-key")
                .and_then(|value| value.to_str().ok()),
            Some("downstream-key")
        );
    }

    #[test]
    fn upstream_ws_request_uses_oauth_auth_and_account_headers() {
        let account = oauth_account(Url::parse("https://api.example.test").expect("valid base"));
        let mut headers = HeaderMap::new();
        headers.insert(
            HeaderName::from_static("openai-beta"),
            HeaderValue::from_static("downstream-beta"),
        );
        let request = build_upstream_ws_request(
            &Url::parse("wss://api.example.test/v1/responses").expect("valid target"),
            &headers,
            &account,
        )
        .expect("request");

        assert_eq!(
            request
                .headers()
                .get(header::AUTHORIZATION)
                .and_then(|value| value.to_str().ok()),
            Some("Bearer oauth-upstream-token")
        );
        assert_eq!(
            request
                .headers()
                .get("openai-beta")
                .and_then(|value| value.to_str().ok()),
            Some("responses=experimental")
        );
        assert_eq!(
            request
                .headers()
                .get("chatgpt-account-id")
                .and_then(|value| value.to_str().ok()),
            Some("acct-test")
        );
    }

    #[test]
    fn forward_proxy_credentials_are_percent_decoded() {
        let proxy_url =
            Url::parse("http://user%2Bname:p%40ss%3Aword@proxy.example.test:8080")
                .expect("valid proxy url");

        assert_eq!(
            forward_proxy_basic_auth_credential(&proxy_url).as_deref(),
            Some("user+name:p@ss:word")
        );
    }

    #[tokio::test]
    async fn prefixed_io_replays_buffered_connect_bytes() {
        let (mut client, server) = tokio::io::duplex(64);
        client
            .write_all(b"inner")
            .await
            .expect("write inner bytes");
        drop(client);
        let mut stream = PrefixedIo::new(b"prefix-".to_vec(), Box::new(server));
        let mut bytes = Vec::new();

        stream
            .read_to_end(&mut bytes)
            .await
            .expect("read combined bytes");

        assert_eq!(bytes, b"prefix-inner");
    }

    #[tokio::test]
    async fn socks5_local_target_host_resolves_hostname_locally() {
        let target = resolve_socks5_local_target_host("localhost", 443)
            .await
            .expect("resolve localhost");

        assert!(
            target.parse::<IpAddr>().is_ok(),
            "socks5:// should pass a locally resolved IP address to the proxy"
        );
    }

    #[test]
    fn retryable_ws_failure_excludes_account_and_route_key_for_next_pool_selection() {
        let failure = WsAttemptFailure {
            status: StatusCode::BAD_GATEWAY,
            message: "failed to contact websocket upstream".to_string(),
            retryable: true,
            account_id: Some(42),
            upstream_route_key: Some("api_key:42".to_string()),
        };
        let mut excluded_account_ids = Vec::new();
        let mut excluded_upstream_route_keys: HashSet<String> = HashSet::new();

        exclude_retryable_ws_attempt_failure(
            &failure,
            &mut excluded_account_ids,
            &mut excluded_upstream_route_keys,
        )
        .expect("exclusion context");

        assert_eq!(excluded_account_ids, vec![42]);
        assert!(excluded_upstream_route_keys.contains("api_key:42"));
    }

    #[test]
    fn retryable_ws_failure_without_account_context_becomes_terminal() {
        let failure = WsAttemptFailure {
            status: StatusCode::BAD_GATEWAY,
            message: "failed without account".to_string(),
            retryable: true,
            account_id: None,
            upstream_route_key: None,
        };
        let mut excluded_account_ids = Vec::new();
        let mut excluded_upstream_route_keys: HashSet<String> = HashSet::new();

        let err = exclude_retryable_ws_attempt_failure(
            &failure,
            &mut excluded_account_ids,
            &mut excluded_upstream_route_keys,
        )
        .expect_err("missing account context is terminal");

        assert_eq!(err.status, StatusCode::BAD_GATEWAY);
        assert_eq!(err.message, "failed without account");
        assert!(excluded_account_ids.is_empty());
        assert!(excluded_upstream_route_keys.is_empty());
    }

    #[test]
    fn websocket_usage_event_parses_terminal_response_usage() {
        let event = parse_ws_usage_event(
            r#"{
                "type": "response.completed",
                "response": {
                    "id": "resp_1",
                    "model": "gpt-5.5",
                    "service_tier": "default",
                    "usage": {
                        "input_tokens": 7,
                        "output_tokens": 3,
                        "input_tokens_details": {
                            "cached_tokens": 2
                        }
                    }
                }
            }"#,
        )
        .expect("usage event");

        assert_eq!(event.event_type, "response.completed");
        assert_eq!(event.response_id.as_deref(), Some("resp_1"));
        assert_eq!(event.model.as_deref(), Some("gpt-5.5"));
        assert_eq!(event.service_tier.as_deref(), Some("default"));
        assert_eq!(event.usage.input_tokens, Some(7));
        assert_eq!(event.usage.output_tokens, Some(3));
        assert_eq!(event.usage.cache_input_tokens, Some(2));
        assert_eq!(event.usage.total_tokens, Some(10));
    }

    #[test]
    fn websocket_usage_event_rejects_non_terminal_or_partial_usage() {
        assert!(parse_ws_usage_event(
            r#"{"type":"response.in_progress","response":{"usage":{"input_tokens":7,"output_tokens":3}}}"#
        )
        .is_none());
        assert!(parse_ws_usage_event(
            r#"{"type":"response.completed","response":{"usage":{"input_tokens":7}}}"#
        )
        .is_none());
        assert!(parse_ws_usage_event(
            r#"{"type":"response.completed","response":{"usage":{"input_tokens":"bad","output_tokens":3}}}"#
        )
        .is_none());
    }

    #[test]
    fn websocket_terminal_failure_kind_marks_response_failed() {
        assert_eq!(
            ws_terminal_event_failure_kind("response.failed"),
            Some(PROXY_FAILURE_UPSTREAM_RESPONSE_FAILED)
        );
        assert_eq!(ws_terminal_event_failure_kind("response.completed"), None);
    }

    #[test]
    fn websocket_upstream_http_unsupported_errors_mark_account_no_ws() {
        for status in [
            tungstenite::http::StatusCode::FORBIDDEN,
            tungstenite::http::StatusCode::NOT_FOUND,
            tungstenite::http::StatusCode::METHOD_NOT_ALLOWED,
            tungstenite::http::StatusCode::UPGRADE_REQUIRED,
            tungstenite::http::StatusCode::NOT_IMPLEMENTED,
        ] {
            let response = tungstenite::http::Response::builder()
                .status(status)
                .body(None)
                .expect("response");
            assert!(
                websocket_upstream_error_marks_account_ws_unsupported(&tungstenite::Error::Http(
                    Box::new(response),
                )),
                "status {status} should mark no-ws"
            );
        }

        let response = tungstenite::http::Response::builder()
            .status(tungstenite::http::StatusCode::BAD_GATEWAY)
            .body(None)
            .expect("response");
        assert!(!websocket_upstream_error_marks_account_ws_unsupported(
            &tungstenite::Error::Http(Box::new(response))
        ));
        assert!(!websocket_upstream_error_marks_account_ws_unsupported(
            &tungstenite::Error::ConnectionClosed
        ));
    }

    #[tokio::test]
    async fn websocket_capability_respects_no_ws_system_tag() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("sqlite pool");
        ensure_schema(&pool).await.expect("schema");
        sqlx::query(
            r#"
            INSERT INTO pool_upstream_accounts (
                id, provider, kind, display_name, enabled, status, encrypted_credentials, created_at, updated_at
            )
            VALUES (42, 'codex', 'api_key', 'ws-test', 1, 'active', 'secret', 'now', 'now')
            "#,
        )
        .execute(&pool)
        .await
        .expect("insert account");

        assert!(
            !account_has_websocket_unsupported_tag(&pool, 42)
                .await
                .expect("tag lookup")
        );
        let tag_id = sqlx::query_scalar::<_, i64>(
            "SELECT id FROM pool_tags WHERE system_key = 'unsupported_transport:websocket'",
        )
        .fetch_one(&pool)
        .await
        .expect("tag id");
        sqlx::query(
            "INSERT INTO pool_upstream_account_tags (account_id, tag_id, created_at, updated_at) VALUES (42, ?1, 'now', 'now')",
        )
        .bind(tag_id)
        .execute(&pool)
        .await
        .expect("attach tag");

        assert!(
            account_has_websocket_unsupported_tag(&pool, 42)
                .await
                .expect("tag lookup")
        );
    }

    #[test]
    fn websocket_capability_tag_does_not_consume_retry_budget() {
        let mut excluded_account_ids = Vec::new();
        let excluded_upstream_route_keys: HashSet<String> = HashSet::new();
        let mut ws_retry_account_ids = HashSet::new();

        for account_id in [11, 12, 13] {
            excluded_account_ids.push(account_id);
        }

        assert_eq!(excluded_account_ids.len(), 3);
        assert!(ws_retry_account_ids.is_empty());
        assert!(excluded_upstream_route_keys.is_empty());
        ws_retry_account_ids.insert(99);
        assert_eq!(ws_retry_account_ids.len(), 1);
    }

    #[test]
    fn websocket_message_conversion_preserves_payload_frames() {
        assert_eq!(
            axum_to_tungstenite_message(AxumWsMessage::Text("hello".to_string()))
                .expect("text")
                .into_text()
                .expect("text payload")
                .as_str(),
            "hello"
        );
        assert_eq!(
            tungstenite_to_axum_message(TungsteniteMessage::Binary(vec![1, 2, 3].into())),
            Some(AxumWsMessage::Binary(vec![1, 2, 3]))
        );
    }
}
