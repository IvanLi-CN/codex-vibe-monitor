trait AsyncReadWrite: AsyncRead + AsyncWrite + Unpin + Send {}

impl<T> AsyncReadWrite for T where T: AsyncRead + AsyncWrite + Unpin + Send {}

type BoxedWsIo = Box<dyn AsyncReadWrite>;
type UpstreamWsStream = WebSocketStream<MaybeTlsStream<BoxedWsIo>>;
const WS_UPSTREAM_DRAIN_AFTER_DOWNSTREAM_CLOSE_TIMEOUT: Duration = Duration::from_millis(1500);

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

pub(crate) fn websocket_routing_keys_from_headers(
    headers: &HeaderMap,
) -> (Option<String>, Option<String>) {
    (
        extract_sticky_key_from_headers(headers),
        extract_prompt_cache_key_from_headers(headers),
    )
}

fn requested_websocket_subprotocol(headers: &HeaderMap) -> Option<String> {
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

fn extract_requested_model_from_websocket_uri(original_uri: &Uri) -> Option<String> {
    let raw_query = original_uri.query()?;
    url::form_urlencoded::parse(raw_query.as_bytes()).find_map(|(key, value)| {
        (key == "model")
            .then(|| value.trim())
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    })
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

    let (sticky_key, header_prompt_cache_key) = websocket_routing_keys_from_headers(&headers);
    let requested_model = extract_requested_model_from_websocket_uri(&original_uri);
    let requester_ip = extract_requester_ip(&headers, peer_ip);
    let trace = PoolUpstreamAttemptTraceContext {
        invoke_id: format!("pool-ws-{proxy_request_id}"),
        occurred_at: shanghai_now_string(),
        endpoint: original_uri.path().to_string(),
        sticky_key: sticky_key.clone(),
        requester_ip,
    };

    let downstream_subprotocol = requested_websocket_subprotocol(&headers);
    let requires_response_create_first_frame =
        websocket_requires_response_create_first_frame(original_uri.path());
    let ws = match downstream_subprotocol.clone() {
        Some(protocol) => ws.protocols([protocol]),
        None => ws,
    };
    ws.on_upgrade(move |downstream| async move {
        if requires_response_create_first_frame {
            proxy_websocket_tunnel_deferred_prepare(
                state,
                downstream,
                proxy_request_id,
                original_uri,
                headers,
                runtime_timeouts,
                sticky_key,
                requested_model,
                header_prompt_cache_key,
                downstream_subprotocol,
                trace,
                proxy_request_permit,
            )
            .await;
        } else {
            proxy_websocket_tunnel_immediate_prepare(
                state,
                downstream,
                proxy_request_id,
                original_uri,
                headers,
                runtime_timeouts,
                sticky_key,
                requested_model,
                header_prompt_cache_key,
                downstream_subprotocol,
                trace,
                proxy_request_permit,
            )
            .await;
        }
    })
}

fn websocket_requires_response_create_first_frame(path: &str) -> bool {
    path == "/v1/responses" || path.starts_with("/v1/responses/")
}

pub(crate) struct PreparedUpstreamWebSocket {
    upstream: UpstreamWsStream,
    pending_attempt_record: Option<PendingPoolAttemptRecord>,
    deferred_cleanup_guard: Option<PoolEarlyPhaseOrphanCleanupGuard>,
    reservation_guard: PoolRoutingReservationGuard,
    account: PoolResolvedAccount,
    trace: PoolUpstreamAttemptTraceContext,
    prompt_cache_key: Option<String>,
    connect_latency_ms: f64,
    requires_response_create_first_frame: bool,
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
pub(crate) struct WsPrepareError {
    pub(crate) status: StatusCode,
    pub(crate) message: String,
}

struct WsAttemptFailure {
    status: StatusCode,
    message: String,
    retryable: bool,
    account_id: Option<i64>,
    upstream_route_key: Option<String>,
}

pub(crate) async fn prepare_upstream_websocket(
    state: Arc<AppState>,
    proxy_request_id: u64,
    original_uri: &Uri,
    headers: &HeaderMap,
    _runtime_timeouts: &PoolRoutingTimeoutSettingsResolved,
    sticky_key: Option<&str>,
    requested_model: Option<&str>,
    prompt_cache_key: Option<&str>,
    binding_constraint: Option<PromptCacheConversationBindingConstraint>,
    conversation_override: Option<ConversationRoutingOverride>,
    owner_auto_guard_active: bool,
    trace: &PoolUpstreamAttemptTraceContext,
    required_subprotocol: Option<&str>,
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

        let mut no_available_wait_deadline = None;
        let account = match resolve_pool_account_for_request_with_wait_and_binding_constraint_with_image_intent_and_override(
            state.as_ref(),
            sticky_key,
            requested_model,
            &excluded_account_ids,
            &excluded_upstream_route_keys,
            None,
            binding_constraint.as_ref(),
            conversation_override.as_ref(),
            false,
            &mut no_available_wait_deadline,
            None,
            crate::ImageIntent::Unknown,
        )
        .await
        {
            Ok(PoolAccountResolutionWithWait::Resolution(PoolAccountResolution::Resolved(
                account,
            ))) => account,
            Ok(PoolAccountResolutionWithWait::Resolution(PoolAccountResolution::Unavailable))
            | Ok(PoolAccountResolutionWithWait::Resolution(PoolAccountResolution::NoCandidate)) => {
                if owner_auto_guard_active {
                    let err = build_encrypted_session_owner_unavailable_error(
                        None,
                        ws_retry_account_ids.len(),
                        distinct_account_count,
                    );
                    let _ = insert_and_broadcast_pool_upstream_terminal_attempt(
                        state.as_ref(),
                        trace,
                        &err,
                        (ws_retry_account_ids.len() + 1) as i64,
                        distinct_account_count as i64,
                        PROXY_FAILURE_ENCRYPTED_SESSION_OWNER_UNAVAILABLE,
                    )
                    .await;
                    return Err(WsPrepareError {
                        status: err.status,
                        message: err.message,
                    });
                }
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
            Ok(PoolAccountResolutionWithWait::Resolution(PoolAccountResolution::RateLimited)) => {
                if owner_auto_guard_active {
                    let err = build_encrypted_session_owner_unavailable_error(
                        None,
                        ws_retry_account_ids.len(),
                        distinct_account_count,
                    );
                    let _ = insert_and_broadcast_pool_upstream_terminal_attempt(
                        state.as_ref(),
                        trace,
                        &err,
                        (ws_retry_account_ids.len() + 1) as i64,
                        distinct_account_count as i64,
                        PROXY_FAILURE_ENCRYPTED_SESSION_OWNER_UNAVAILABLE,
                    )
                    .await;
                    return Err(WsPrepareError {
                        status: err.status,
                        message: err.message,
                    });
                }
                return Err(WsPrepareError {
                    status: StatusCode::TOO_MANY_REQUESTS,
                    message: POOL_ALL_ACCOUNTS_RATE_LIMITED_MESSAGE.to_string(),
                });
            }
            Ok(PoolAccountResolutionWithWait::Resolution(PoolAccountResolution::DegradedOnly)) => {
                if owner_auto_guard_active {
                    let err = build_encrypted_session_owner_unavailable_error(
                        None,
                        ws_retry_account_ids.len(),
                        distinct_account_count,
                    );
                    let _ = insert_and_broadcast_pool_upstream_terminal_attempt(
                        state.as_ref(),
                        trace,
                        &err,
                        (ws_retry_account_ids.len() + 1) as i64,
                        distinct_account_count as i64,
                        PROXY_FAILURE_ENCRYPTED_SESSION_OWNER_UNAVAILABLE,
                    )
                    .await;
                    return Err(WsPrepareError {
                        status: err.status,
                        message: err.message,
                    });
                }
                return Err(WsPrepareError {
                    status: StatusCode::SERVICE_UNAVAILABLE,
                    message: POOL_ALL_ACCOUNTS_DEGRADED_MESSAGE.to_string(),
                });
            }
            Ok(PoolAccountResolutionWithWait::Resolution(
                PoolAccountResolution::AssignedBlocked(blocked),
            )) => {
                if owner_auto_guard_active {
                    let err = build_encrypted_session_owner_unavailable_error(
                        None,
                        ws_retry_account_ids.len(),
                        distinct_account_count,
                    );
                    let _ = insert_and_broadcast_pool_upstream_terminal_attempt(
                        state.as_ref(),
                        trace,
                        &err,
                        (ws_retry_account_ids.len() + 1) as i64,
                        distinct_account_count as i64,
                        PROXY_FAILURE_ENCRYPTED_SESSION_OWNER_UNAVAILABLE,
                    )
                    .await;
                    return Err(WsPrepareError {
                        status: err.status,
                        message: err.message,
                    });
                }
                let terminal = ViaPoolResolutionTerminalError::assigned_blocked(blocked);
                terminal
                    .persist_if_needed(state.as_ref(), Some(trace))
                    .await;
                return Err(WsPrepareError {
                    status: terminal.status,
                    message: terminal.message,
                });
            }
            Ok(PoolAccountResolutionWithWait::Resolution(
                PoolAccountResolution::BlockedByPolicy(message),
            )) => {
                if owner_auto_guard_active {
                    let err = build_encrypted_session_owner_unavailable_error(
                        None,
                        ws_retry_account_ids.len(),
                        distinct_account_count,
                    );
                    let _ = insert_and_broadcast_pool_upstream_terminal_attempt(
                        state.as_ref(),
                        trace,
                        &err,
                        (ws_retry_account_ids.len() + 1) as i64,
                        distinct_account_count as i64,
                        PROXY_FAILURE_ENCRYPTED_SESSION_OWNER_UNAVAILABLE,
                    )
                    .await;
                    return Err(WsPrepareError {
                        status: err.status,
                        message: err.message,
                    });
                }
                return Err(WsPrepareError {
                    status: StatusCode::SERVICE_UNAVAILABLE,
                    message,
                });
            }
            Ok(PoolAccountResolutionWithWait::TotalTimeoutExpired) => {
                return Err(WsPrepareError {
                    status: StatusCode::GATEWAY_TIMEOUT,
                    message: "pool websocket account selection timed out".to_string(),
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
            &load_effective_request_path_timeouts_for_account(
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
            .2,
            trace,
            prompt_cache_key,
            account,
            ws_retry_account_ids.len() + 1,
            required_subprotocol,
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
    prompt_cache_key: Option<&str>,
    account: PoolResolvedAccount,
    attempt_index: usize,
    required_subprotocol: Option<&str>,
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

    let upstream_url = match build_websocket_upstream_url(&account.upstream_base_url, original_uri)
    {
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

    let request = match build_upstream_ws_request(
        &upstream_url,
        headers,
        &account,
        websocket_requires_response_create_first_frame(original_uri.path()),
    ) {
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
            let should_mark_ws_unsupported =
                websocket_upstream_error_marks_account_ws_unsupported(&err);
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
    if let Some(required_subprotocol) = required_subprotocol
        && selected_subprotocol.as_deref() != Some(required_subprotocol)
    {
        let message = match selected_subprotocol.as_deref() {
            Some(selected) => format!(
                "websocket upstream selected subprotocol {selected}, expected {required_subprotocol}"
            ),
            None => format!(
                "websocket upstream did not select required subprotocol {required_subprotocol}"
            ),
        };
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
                "failed to record websocket subprotocol mismatch route failure"
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
        prompt_cache_key: prompt_cache_key.map(str::to_string),
        connect_latency_ms: elapsed_ms(connect_started),
        requires_response_create_first_frame: websocket_requires_response_create_first_frame(
            original_uri.path(),
        ),
    })
}

async fn proxy_websocket_tunnel(
    state: Arc<AppState>,
    downstream: WebSocket,
    prepared: PreparedUpstreamWebSocket,
    _proxy_request_permit: ProxyRequestConcurrencyPermit,
    initial_downstream_message: Option<AxumWsMessage>,
) {
    let PreparedUpstreamWebSocket {
        upstream,
        pending_attempt_record,
        mut deferred_cleanup_guard,
        mut reservation_guard,
        account,
        trace,
        prompt_cache_key,
        connect_latency_ms,
        requires_response_create_first_frame,
    } = prepared;
    let stream_started = Instant::now();
    let (mut downstream_tx, mut downstream_rx) = downstream.split();
    let (mut upstream_tx, mut upstream_rx) = upstream.split();
    let mut failure: Option<String> = None;
    let mut failure_kind_override: Option<&'static str> = None;
    let mut upstream_route_failure: Option<String> = None;
    let mut mark_account_ws_unsupported_after_close = false;
    let mut usage_tracker = WsUsageTracker::new(account, trace, prompt_cache_key);
    let mut active_turn_waiting_terminal = false;
    let mut saw_terminal_upstream_event = false;
    let mut drain_upstream_after_downstream_close = false;
    let mut pending_first_downstream_message = initial_downstream_message;

    loop {
        if let Some(message) = pending_first_downstream_message.take() {
            let close_seen = matches!(message, AxumWsMessage::Close(_));
            if ws_message_starts_response_create_turn(&message) {
                active_turn_waiting_terminal = true;
                saw_terminal_upstream_event = false;
            }
            if let Some(payload_bytes) = ws_message_payload_bytes(&message) {
                match apply_ws_downstream_payload_guard(
                    state.as_ref(),
                    &mut usage_tracker,
                    payload_bytes,
                )
                .await
                {
                    Ok(true) => {
                        failure_kind_override =
                            Some(PROXY_FAILURE_ENCRYPTED_SESSION_OWNER_UNAVAILABLE);
                        failure = Some(ENCRYPTED_SESSION_OWNER_UNAVAILABLE_MESSAGE.to_string());
                        let _ = downstream_tx
                            .send(AxumWsMessage::Close(Some(axum::extract::ws::CloseFrame {
                                code: axum::extract::ws::close_code::AGAIN,
                                reason: "encrypted_session_owner_unavailable; retry".into(),
                            })))
                            .await;
                        break;
                    }
                    Ok(false) => {}
                    Err(err) => {
                        failure = Some(format!(
                            "failed to inspect websocket payload routing constraint: {err}"
                        ));
                        break;
                    }
                }
            }
            if close_seen && active_turn_waiting_terminal {
                drain_upstream_after_downstream_close = true;
                break;
            }
            match axum_to_tungstenite_message(message) {
                Some(message) => {
                    if let Err(err) = upstream_tx.send(message).await {
                        let message =
                            format!("failed to forward downstream websocket frame upstream: {err}");
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
            continue;
        }
        tokio::select! {
            downstream_msg = downstream_rx.next() => {
                match downstream_msg {
                    Some(Ok(message)) => {
                        let close_seen = matches!(message, AxumWsMessage::Close(_));
                        if ws_message_starts_response_create_turn(&message) {
                            active_turn_waiting_terminal = true;
                            saw_terminal_upstream_event = false;
                        }
                        if let Some(payload_bytes) = ws_message_payload_bytes(&message) {
                            match apply_ws_downstream_payload_guard(
                                state.as_ref(),
                                &mut usage_tracker,
                                payload_bytes,
                            )
                            .await
                            {
                                Ok(true) => {
                                    failure_kind_override =
                                        Some(PROXY_FAILURE_ENCRYPTED_SESSION_OWNER_UNAVAILABLE);
                                    failure = Some(
                                        ENCRYPTED_SESSION_OWNER_UNAVAILABLE_MESSAGE.to_string(),
                                    );
                                    let _ = downstream_tx
                                        .send(AxumWsMessage::Close(Some(
                                            axum::extract::ws::CloseFrame {
                                                code: axum::extract::ws::close_code::AGAIN,
                                                reason: "encrypted_session_owner_unavailable; retry"
                                                    .into(),
                                            },
                                        )))
                                        .await;
                                    break;
                                }
                                Ok(false) => {}
                                Err(err) => {
                                    failure = Some(format!(
                                        "failed to inspect websocket payload routing constraint: {err}"
                                    ));
                                    break;
                                }
                            }
                        }
                        if close_seen && active_turn_waiting_terminal {
                            drain_upstream_after_downstream_close = true;
                            break;
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
                        if active_turn_waiting_terminal {
                            drain_upstream_after_downstream_close = true;
                            break;
                        }
                        failure = Some(format!("downstream websocket error: {err}"));
                        break;
                    }
                    None => {
                        if active_turn_waiting_terminal {
                            drain_upstream_after_downstream_close = true;
                        }
                        break;
                    }
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
                        let terminal_for_message = upstream_text
                            .as_deref()
                            .is_some_and(ws_text_event_is_terminal);
                        if let Some(text) = upstream_text.as_deref() {
                            usage_tracker.observe_upstream_text(state.as_ref(), text).await;
                        }
                        if terminal_for_message {
                            saw_terminal_upstream_event = true;
                            active_turn_waiting_terminal = false;
                        }
                        if close_seen
                            && ws_upstream_close_requires_retry(
                                active_turn_waiting_terminal,
                                saw_terminal_upstream_event,
                            )
                        {
                            let message =
                                "upstream websocket closed before response.completed".to_string();
                            upstream_route_failure = Some(message.clone());
                            failure = Some(message);
                            mark_account_ws_unsupported_after_close =
                                websocket_post_upgrade_close_marks_account_ws_unsupported(
                                    requires_response_create_first_frame,
                                    &usage_tracker.account,
                                );
                            let _ = downstream_tx
                                .send(AxumWsMessage::Close(Some(axum::extract::ws::CloseFrame {
                                    code: axum::extract::ws::close_code::AGAIN,
                                    reason: "upstream_unavailable; retry".into(),
                                })))
                                .await;
                            break;
                        }
                        if let Some(message) = tungstenite_to_axum_message(message)
                            && let Err(err) = downstream_tx.send(message).await
                        {
                            if terminal_for_message
                                || (!active_turn_waiting_terminal
                                    && saw_terminal_upstream_event
                                    && close_seen)
                            {
                                break;
                            }
                            if active_turn_waiting_terminal {
                                drain_upstream_after_downstream_close = true;
                                break;
                            }
                            failure = Some(format!("failed to forward upstream websocket frame downstream: {err}"));
                            break;
                        }
                        if close_seen {
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
                        if active_turn_waiting_terminal {
                            let message =
                                "upstream websocket closed before response.completed".to_string();
                            upstream_route_failure = Some(message.clone());
                            failure = Some(message);
                            mark_account_ws_unsupported_after_close =
                                websocket_post_upgrade_close_marks_account_ws_unsupported(
                                    requires_response_create_first_frame,
                                    &usage_tracker.account,
                                );
                            let _ = downstream_tx
                                .send(AxumWsMessage::Close(Some(axum::extract::ws::CloseFrame {
                                    code: axum::extract::ws::close_code::AGAIN,
                                    reason: "upstream_unavailable; retry".into(),
                                })))
                                .await;
                        }
                        break;
                    },
                }
            }
        }
    }

    if drain_upstream_after_downstream_close {
        let drain_started = Instant::now();
        loop {
            if !active_turn_waiting_terminal {
                break;
            }
            let remaining = WS_UPSTREAM_DRAIN_AFTER_DOWNSTREAM_CLOSE_TIMEOUT
                .saturating_sub(drain_started.elapsed());
            if remaining.is_zero() {
                failure_kind_override = Some(PROXY_STREAM_TERMINAL_DOWNSTREAM_CLOSED);
                failure = Some("downstream websocket closed before upstream terminal event".to_string());
                break;
            }
            match timeout(remaining, upstream_rx.next()).await {
                Ok(Some(Ok(message))) => {
                    let close_seen = matches!(message, TungsteniteMessage::Close(_));
                    if let TungsteniteMessage::Text(text) = &message {
                        let text = text.as_str();
                        let terminal_for_message = ws_text_event_is_terminal(text);
                        usage_tracker.observe_upstream_text(state.as_ref(), text).await;
                        if terminal_for_message {
                            break;
                        }
                    }
                    if close_seen && active_turn_waiting_terminal {
                        let message =
                            "upstream websocket closed before response.completed".to_string();
                        upstream_route_failure = Some(message.clone());
                        failure = Some(message);
                        break;
                    }
                }
                Ok(Some(Err(err))) => {
                    let message = format!("upstream websocket error during downstream drain: {err}");
                    upstream_route_failure = Some(message.clone());
                    failure = Some(message);
                    break;
                }
                Ok(None) => {
                    let message = "upstream websocket closed before response.completed".to_string();
                    upstream_route_failure = Some(message.clone());
                    failure = Some(message);
                    break;
                }
                Err(_) => {
                    failure_kind_override = Some(PROXY_STREAM_TERMINAL_DOWNSTREAM_CLOSED);
                    failure = Some(
                        "downstream websocket closed before upstream terminal event".to_string(),
                    );
                    break;
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
        failure
            .as_ref()
            .map(|_| failure_kind_override.unwrap_or(PROXY_FAILURE_UPSTREAM_STREAM_ERROR)),
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
    if mark_account_ws_unsupported_after_close
        && let Err(err) = ensure_account_has_websocket_unsupported_tag(
            &state.pool,
            usage_tracker.account.account_id,
        )
        .await
    {
        warn!(
            invoke_id = %usage_tracker.trace.invoke_id,
            account_id = usage_tracker.account.account_id,
            error = %err,
            "failed to mark post-upgrade websocket account as unsupported"
        );
    }
    complete_deferred_pool_early_phase_cleanup_guard(&mut deferred_cleanup_guard);
    reservation_guard.release();
}

async fn proxy_websocket_tunnel_deferred_prepare(
    state: Arc<AppState>,
    mut downstream: WebSocket,
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
) {
    let first_downstream_message =
        match timeout(runtime_timeouts.request_read_timeout, downstream.next()).await {
            Ok(Some(Ok(message))) => message,
            Ok(Some(Err(err))) => {
                let message = format!(
                    "failed to read websocket first response.create frame: {err}"
                );
                warn!(
                    proxy_request_id,
                    error = %message,
                    "downstream websocket closed before deferred upstream prepare"
                );
                record_ws_pre_upstream_failure(
                    state.as_ref(),
                    &trace,
                    PROXY_FAILURE_REQUEST_BODY_READ_TIMEOUT,
                    message.as_str(),
                )
                .await;
                return;
            }
            Ok(None) => {
                debug!(
                    proxy_request_id,
                    "downstream websocket closed before first response.create frame"
                );
                return;
            }
            Err(_) => {
                let message = "websocket first response.create timed out";
                warn!(
                    proxy_request_id,
                    timeout_secs = runtime_timeouts.request_read_timeout.as_secs_f64(),
                    "websocket first response.create timed out"
                );
                record_ws_pre_upstream_failure(
                    state.as_ref(),
                    &trace,
                    PROXY_FAILURE_REQUEST_BODY_READ_TIMEOUT,
                    message,
                )
                .await;
                let _ = downstream
                    .send(AxumWsMessage::Close(Some(axum::extract::ws::CloseFrame {
                        code: axum::extract::ws::close_code::ERROR,
                        reason: message.into(),
                    })))
                    .await;
                return;
            }
        };
    if matches!(first_downstream_message, AxumWsMessage::Close(_)) {
        return;
    }

    let payload_inspection = match inspect_ws_initial_response_create_message(
        &first_downstream_message,
    ) {
        Ok(inspection) => inspection,
        Err(reason) => {
            warn!(
                proxy_request_id,
                reason,
                "websocket first downstream frame rejected"
            );
            record_ws_pre_upstream_failure(
                state.as_ref(),
                &trace,
                PROXY_FAILURE_REQUEST_BODY_READ_TIMEOUT,
                reason,
            )
            .await;
            let _ = downstream
                .send(AxumWsMessage::Close(Some(axum::extract::ws::CloseFrame {
                    code: axum::extract::ws::close_code::ERROR,
                    reason: reason.into(),
                })))
                .await;
            return;
        }
    };

    let requested_model = payload_inspection
        .requested_model
        .clone()
        .or(requested_model);
    let prompt_cache_key = payload_inspection
        .prompt_cache_key
        .clone()
        .or(header_prompt_cache_key);
    let request_contains_encrypted_content = payload_inspection.contains_encrypted_content;
    debug!(
        proxy_request_id,
        requested_model = ?requested_model,
        previous_response_id = ?payload_inspection.previous_response_id,
        prompt_cache_key_present = prompt_cache_key.is_some(),
        request_contains_encrypted_content,
        "websocket first response.create inspected"
    );
    let (binding_constraint, owner_auto_guard_active, conversation_override) =
        match load_via_pool_effective_routing(
            state.as_ref(),
            prompt_cache_key.as_deref(),
            request_contains_encrypted_content,
        )
        .await
        {
            Ok(result) => result,
            Err((_status, message)) => {
                let _ = downstream
                    .send(AxumWsMessage::Close(Some(axum::extract::ws::CloseFrame {
                        code: axum::extract::ws::close_code::ERROR,
                        reason: message.into(),
                    })))
                    .await;
                return;
            }
        };
    let prepared = match prepare_upstream_websocket(
        state.clone(),
        proxy_request_id,
        &original_uri,
        &headers,
        &runtime_timeouts,
        sticky_key.as_deref(),
        requested_model.as_deref(),
        prompt_cache_key.as_deref(),
        binding_constraint,
        conversation_override,
        owner_auto_guard_active,
        &trace,
        required_subprotocol.as_deref(),
    )
    .await
    {
        Ok(prepared) => prepared,
        Err(err) => {
            let close_frame = if err.message == ENCRYPTED_SESSION_OWNER_UNAVAILABLE_MESSAGE {
                axum::extract::ws::CloseFrame {
                    code: axum::extract::ws::close_code::AGAIN,
                    reason: "encrypted_session_owner_unavailable; retry".into(),
                }
            } else {
                axum::extract::ws::CloseFrame {
                    code: axum::extract::ws::close_code::AGAIN,
                    reason: "upstream_unavailable; retry".into(),
                }
            };
            let _ = downstream.send(AxumWsMessage::Close(Some(close_frame))).await;
            return;
        }
    };
    proxy_websocket_tunnel(
        state,
        downstream,
        prepared,
        proxy_request_permit,
        Some(first_downstream_message),
    )
    .await;
}

async fn record_ws_pre_upstream_failure(
    state: &AppState,
    trace: &PoolUpstreamAttemptTraceContext,
    failure_kind: &'static str,
    message: &str,
) {
    let now = shanghai_now_string();
    if let Err(err) = insert_pool_upstream_request_attempt_with_scope(
        &state.pool,
        trace,
        None,
        None,
        None,
        None,
        1,
        0,
        0,
        Some(now.as_str()),
        Some(now.as_str()),
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_TRANSPORT_FAILURE,
        Some(POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_FAILED),
        None,
        Some(StatusCode::BAD_REQUEST),
        Some(failure_kind),
        Some(message),
        Some(message),
        None,
        None,
        None,
        None,
        None,
        None,
    )
    .await
    {
        warn!(
            invoke_id = %trace.invoke_id,
            error = %err,
            "failed to record websocket pre-upstream failure"
        );
        return;
    }
    if let Err(err) = broadcast_pool_upstream_attempts_snapshot(state, &trace.invoke_id).await {
        warn!(
            invoke_id = %trace.invoke_id,
            error = %err,
            "failed to broadcast websocket pre-upstream failure"
        );
    }
}

async fn proxy_websocket_tunnel_immediate_prepare(
    state: Arc<AppState>,
    mut downstream: WebSocket,
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
) {
    debug!(
        proxy_request_id,
        requested_model = ?requested_model,
        prompt_cache_key_present = header_prompt_cache_key.is_some(),
        path = %original_uri.path(),
        "websocket passthrough prepare without response.create first frame"
    );
    let (binding_constraint, owner_auto_guard_active, conversation_override) =
        match load_via_pool_effective_routing(
            state.as_ref(),
            header_prompt_cache_key.as_deref(),
            false,
        )
        .await
        {
            Ok(result) => result,
            Err((_status, message)) => {
                let _ = downstream
                    .send(AxumWsMessage::Close(Some(axum::extract::ws::CloseFrame {
                        code: axum::extract::ws::close_code::ERROR,
                        reason: message.into(),
                    })))
                    .await;
                return;
            }
        };
    let prepared = match prepare_upstream_websocket(
        state.clone(),
        proxy_request_id,
        &original_uri,
        &headers,
        &runtime_timeouts,
        sticky_key.as_deref(),
        requested_model.as_deref(),
        header_prompt_cache_key.as_deref(),
        binding_constraint,
        conversation_override,
        owner_auto_guard_active,
        &trace,
        required_subprotocol.as_deref(),
    )
    .await
    {
        Ok(prepared) => prepared,
        Err(err) => {
            let close_frame = if err.message == ENCRYPTED_SESSION_OWNER_UNAVAILABLE_MESSAGE {
                axum::extract::ws::CloseFrame {
                    code: axum::extract::ws::close_code::AGAIN,
                    reason: "encrypted_session_owner_unavailable; retry".into(),
                }
            } else {
                axum::extract::ws::CloseFrame {
                    code: axum::extract::ws::close_code::AGAIN,
                    reason: "upstream_unavailable; retry".into(),
                }
            };
            let _ = downstream.send(AxumWsMessage::Close(Some(close_frame))).await;
            return;
        }
    };
    proxy_websocket_tunnel(
        state,
        downstream,
        prepared,
        proxy_request_permit,
        None,
    )
    .await;
}

struct WsUsageTracker {
    account: PoolResolvedAccount,
    trace: PoolUpstreamAttemptTraceContext,
    prompt_cache_key: Option<String>,
    ordinal: u64,
    request_contains_encrypted_content: bool,
}

impl WsUsageTracker {
    fn new(
        account: PoolResolvedAccount,
        trace: PoolUpstreamAttemptTraceContext,
        prompt_cache_key: Option<String>,
    ) -> Self {
        Self {
            account,
            trace,
            prompt_cache_key,
            ordinal: 0,
            request_contains_encrypted_content: false,
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
            self.prompt_cache_key.as_deref(),
            self.ordinal,
            event,
            self.request_contains_encrypted_content,
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
    response_status: Option<String>,
    model: Option<String>,
    service_tier: Option<String>,
    usage: ParsedUsage,
    contains_encrypted_content: bool,
}

#[derive(Debug)]
struct WsRequestPayloadInspection {
    event_type: Option<String>,
    requested_model: Option<String>,
    previous_response_id: Option<String>,
    prompt_cache_key: Option<String>,
    contains_encrypted_content: bool,
}

pub(crate) struct WsRequestPayloadGuardOutcome {
    pub(crate) prompt_cache_key: Option<String>,
    pub(crate) contains_encrypted_content: bool,
    pub(crate) owner_guard_blocked: bool,
}

fn inspect_ws_initial_response_create_message(
    message: &AxumWsMessage,
) -> std::result::Result<WsRequestPayloadInspection, &'static str> {
    let AxumWsMessage::Text(text) = message else {
        return Err("websocket first frame must be text response.create");
    };
    let Some(inspection) = inspect_ws_request_payload(text.as_bytes()) else {
        return Err("websocket first frame must be JSON response.create");
    };
    if inspection.event_type.as_deref() != Some("response.create") {
        return Err("websocket first frame must be response.create");
    }
    Ok(inspection)
}

fn ws_message_payload_bytes(message: &AxumWsMessage) -> Option<&[u8]> {
    match message {
        AxumWsMessage::Text(text) => Some(text.as_bytes()),
        AxumWsMessage::Binary(bytes) => Some(bytes.as_ref()),
        _ => None,
    }
}

fn ws_message_starts_response_create_turn(message: &AxumWsMessage) -> bool {
    match message {
        AxumWsMessage::Text(text) => std::str::from_utf8(text.as_bytes())
            .ok()
            .is_some_and(ws_text_event_is_response_create),
        _ => false,
    }
}

fn ws_text_event_is_response_create(text: &str) -> bool {
    let Ok(value) = serde_json::from_str::<Value>(text) else {
        return false;
    };
    value
        .get("type")
        .and_then(Value::as_str)
        .is_some_and(|event_type| event_type == "response.create")
}

fn extract_nonempty_json_string(value: &Value, pointers: &[&str]) -> Option<String> {
    pointers.iter().find_map(|pointer| {
        value
            .pointer(pointer)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    })
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
        response_status: value
            .pointer("/response/status")
            .or_else(|| value.get("status"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string),
        service_tier: extract_service_tier_from_payload(&value),
        usage,
        contains_encrypted_content: value_contains_encrypted_content(&value),
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

fn websocket_post_upgrade_close_marks_account_ws_unsupported(
    requires_response_create_first_frame: bool,
    account: &PoolResolvedAccount,
) -> bool {
    requires_response_create_first_frame
        && account
            .kind
            .eq_ignore_ascii_case(API_KEYS_BILLING_ACCOUNT_KIND)
        && !account.auth.is_oauth()
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

fn ws_upstream_close_requires_retry(
    saw_downstream_request: bool,
    saw_terminal_upstream_event: bool,
) -> bool {
    saw_downstream_request && !saw_terminal_upstream_event
}

fn inspect_ws_request_payload(bytes: &[u8]) -> Option<WsRequestPayloadInspection> {
    let value = serde_json::from_slice::<Value>(bytes).ok()?;
    Some(WsRequestPayloadInspection {
        event_type: extract_nonempty_json_string(&value, &["/type"]),
        requested_model: extract_nonempty_json_string(&value, &["/model", "/response/model"]),
        previous_response_id: extract_nonempty_json_string(
            &value,
            &[
                "/previous_response_id",
                "/previousResponseId",
                "/response/previous_response_id",
                "/response/previousResponseId",
            ],
        ),
        prompt_cache_key: extract_prompt_cache_key_from_request_body(&value),
        contains_encrypted_content: value_contains_encrypted_content(&value),
    })
}

fn ws_request_payload_contains_encrypted_content(bytes: &[u8]) -> bool {
    inspect_ws_request_payload(bytes).is_some_and(|value| value.contains_encrypted_content)
}

pub(crate) async fn inspect_ws_request_payload_guard(
    state: &AppState,
    account: &PoolResolvedAccount,
    existing_prompt_cache_key: Option<&str>,
    bytes: &[u8],
) -> Result<WsRequestPayloadGuardOutcome> {
    let inspection = inspect_ws_request_payload(bytes);
    let prompt_cache_key = inspection
        .as_ref()
        .and_then(|value| value.prompt_cache_key.clone())
        .or_else(|| existing_prompt_cache_key.map(str::to_string));
    let contains_encrypted_content = inspection
        .as_ref()
        .is_some_and(|value| value.contains_encrypted_content);
    let mut owner_guard_blocked = false;

    if let Some(prompt_cache_key) = prompt_cache_key.as_deref() {
        let (binding_constraint, owner_auto_guard_active) =
            load_via_pool_effective_routing_constraint(
                state,
                Some(prompt_cache_key),
                contains_encrypted_content,
            )
            .await
            .map_err(|(_status, message)| anyhow!(message))?;
        owner_guard_blocked = owner_auto_guard_active
            && binding_constraint.as_ref().is_some_and(|constraint| {
                !binding_constraint_accepts_upstream_account_id(
                    constraint,
                    account.account_id,
                    account.group_name.as_deref(),
                )
            });
    }

    Ok(WsRequestPayloadGuardOutcome {
        prompt_cache_key,
        contains_encrypted_content,
        owner_guard_blocked,
    })
}

async fn apply_ws_downstream_payload_guard(
    state: &AppState,
    usage_tracker: &mut WsUsageTracker,
    payload_bytes: &[u8],
) -> Result<bool> {
    let outcome = inspect_ws_request_payload_guard(
        state,
        &usage_tracker.account,
        websocket_effective_prompt_cache_key(usage_tracker.prompt_cache_key.as_deref()),
        payload_bytes,
    )
    .await?;
    if let Some(prompt_cache_key) = outcome.prompt_cache_key {
        usage_tracker.prompt_cache_key = Some(prompt_cache_key);
    }
    if outcome.contains_encrypted_content {
        usage_tracker.request_contains_encrypted_content = true;
    }
    Ok(outcome.owner_guard_blocked)
}

fn ws_usage_event_is_completed_success(event: &WsUsageEvent) -> bool {
    match event.event_type.as_str() {
        "response.completed" => true,
        "response.done" => event
            .response_status
            .as_deref()
            .is_some_and(|status| status.eq_ignore_ascii_case("completed")),
        _ => false,
    }
}

async fn persist_ws_usage_event(
    state: &AppState,
    account: &PoolResolvedAccount,
    trace: &PoolUpstreamAttemptTraceContext,
    prompt_cache_key: Option<&str>,
    ordinal: u64,
    event: WsUsageEvent,
    request_contains_encrypted_content: bool,
    raw_event: &str,
) -> Result<()> {
    let proxy_settings = state.proxy_model_settings.read().await.clone();
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
        request_contains_encrypted_content,
        response_contains_encrypted_content: event.contains_encrypted_content,
        compaction_request_kind: None,
        compaction_response_kind: None,
        image_intent: None,
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
        prompt_cache_key: websocket_effective_prompt_cache_key(prompt_cache_key),
        prompt_cache_key_attribution_source: websocket_effective_prompt_cache_key(prompt_cache_key)
            .map(|_| "websocket_trace")
            .map(str::to_string)
            .as_deref(),
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
        stream_failure_origin: None,
        upstream_read_error_kind: None,
        content_encoding_chain: None,
        forwarded_chunk_count: None,
        forwarded_bytes: None,
        usage_observed: None,
        downstream_close_phase: None,
        downstream_write_error_kind: None,
        last_upstream_chunk_gap_ms: None,
        proxy_display_name: None,
        proxy_weight_delta: None,
        pool_attempt_count: None,
        pool_distinct_account_count: None,
        pool_attempt_terminal_reason: None,
    });
    let payload = mark_websocket_payload_transport(payload)?;
    let is_completed_terminal_event = ws_usage_event_is_completed_success(&event);
    let is_failed_terminal_event = !is_completed_terminal_event;
    let failure_kind = ws_terminal_event_failure_kind(&event);
    if is_completed_terminal_event
        && let Some(prompt_cache_key) = websocket_effective_prompt_cache_key(prompt_cache_key)
        && (request_contains_encrypted_content || event.contains_encrypted_content)
    {
        if confirm_prompt_cache_encrypted_session_owner_success_if_enabled(
            state,
            prompt_cache_key,
            account.account_id,
        )
        .await?
        {
            promote_prompt_cache_group_binding_to_upstream_account(
                &state.pool,
                prompt_cache_key,
                account.account_id,
            )
            .await?;
        }
    }
    persist_and_broadcast_proxy_capture_terminal_record(
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
            error_message: failure_kind
                .map(|value| format!("[{value}] response.failed"))
                .or_else(|| {
                    is_failed_terminal_event.then(|| {
                        let status = event.response_status.as_deref().unwrap_or("unknown");
                        format!("websocket response terminal status was {status}")
                    })
            }),
            failure_kind: failure_kind.map(str::to_string),
            payload: Some(payload),
            raw_response: raw_event.to_string(),
            response_body_preview_enabled: proxy_settings.response_body_logging_enabled,
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

fn mark_websocket_payload_transport(payload: String) -> Result<String> {
    let mut value = serde_json::from_str::<Value>(&payload)
        .context("failed to parse websocket proxy payload summary")?;
    if let Some(object) = value.as_object_mut() {
        object.insert(
            "transport".to_string(),
            Value::String("websocket".to_string()),
        );
    }
    serde_json::to_string(&value).context("failed to serialize websocket proxy payload summary")
}

fn ws_terminal_event_failure_kind(event: &WsUsageEvent) -> Option<&'static str> {
    if event.event_type == "response.failed" {
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
    force_responses_beta: bool,
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
            if force_responses_beta {
                request.headers_mut().insert(
                    HeaderName::from_static("openai-beta"),
                    HeaderValue::from_static("responses=experimental"),
                );
            }
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
) -> std::result::Result<
    (UpstreamWsStream, tungstenite::handshake::client::Response),
    tungstenite::Error,
> {
    let Some(forward_proxy_url) = forward_proxy_url else {
        let stream = connect_tcp_target(upstream_url).await?;
        return client_async_tls_with_config(request, Box::new(stream) as BoxedWsIo, None, None)
            .await;
    };

    let proxy_host = forward_proxy_url.host_str().ok_or_else(|| {
        tungstenite::Error::Io(io::Error::new(
            io::ErrorKind::InvalidInput,
            "forward proxy endpoint is missing host",
        ))
    })?;
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
            format!(
                "websocket proxy only supports HTTP CONNECT, HTTPS CONNECT, or SOCKS5 forward proxy endpoints, got {proxy_scheme}"
            ),
        )));
    }

    let mut stream = connect_http_forward_proxy(forward_proxy_url, proxy_host, proxy_port).await?;
    let mut connect_request =
        format!("CONNECT {target_authority} HTTP/1.1\r\nHost: {target_authority}\r\n");
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
        let read = stream
            .read(&mut buffer)
            .await
            .map_err(tungstenite::Error::Io)?;
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
        .ok_or_else(|| {
            tungstenite::Error::Io(io::Error::new(
                io::ErrorKind::InvalidData,
                "forward proxy CONNECT response missing status line",
            ))
        })?;
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

async fn connect_tcp_target(
    upstream_url: &Url,
) -> std::result::Result<TcpStream, tungstenite::Error> {
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
    let server_name =
        rustls_pki_types::ServerName::try_from(proxy_host.to_string()).map_err(|err| {
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
            single_account_rotation_enabled: false,
            upstream_429_retry_enabled: false,
            upstream_429_max_retries: 0,
            fast_mode_rewrite_mode: TagFastModeRewriteMode::default(),
            image_tool_rewrite_mode: ImageToolRewriteMode::KeepOriginal,
            image_tool_capability: ImageToolCapability::Unknown,
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
            single_account_rotation_enabled: false,
            upstream_429_retry_enabled: false,
            upstream_429_max_retries: 0,
            fast_mode_rewrite_mode: TagFastModeRewriteMode::default(),
            image_tool_rewrite_mode: ImageToolRewriteMode::KeepOriginal,
            image_tool_capability: ImageToolCapability::Unknown,
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
        let uri = "/v1/responses?model=gpt-5.5"
            .parse::<Uri>()
            .expect("valid uri");

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
    fn websocket_requested_model_extraction_reads_query_parameter() {
        let uri = "/v1/realtime?foo=1&model=gpt-5.5-preview&empty="
            .parse::<Uri>()
            .expect("valid uri");

        assert_eq!(
            extract_requested_model_from_websocket_uri(&uri).as_deref(),
            Some("gpt-5.5-preview")
        );
    }

    #[test]
    fn websocket_requested_model_extraction_ignores_blank_values() {
        let uri = "/v1/realtime?model=%20%20"
            .parse::<Uri>()
            .expect("valid uri");

        assert_eq!(extract_requested_model_from_websocket_uri(&uri), None);
    }

    #[test]
    fn websocket_header_prompt_cache_key_does_not_fallback_to_sticky_key() {
        let headers = HeaderMap::from_iter([(
            HeaderName::from_static("x-sticky-key"),
            HeaderValue::from_static("sticky-only-key"),
        )]);

        let (sticky_key, prompt_cache_key) = websocket_routing_keys_from_headers(&headers);

        assert_eq!(sticky_key.as_deref(), Some("sticky-only-key"));
        assert_eq!(prompt_cache_key, None);
    }

    #[test]
    fn upstream_ws_request_replaces_auth_and_drops_upgrade_hop_headers() {
        let account = api_key_account(Url::parse("https://api.example.test").expect("valid base"));
        let mut headers = HeaderMap::new();
        headers.insert(
            header::AUTHORIZATION,
            HeaderValue::from_static("Bearer downstream-pool-key"),
        );
        headers.insert(
            header::CONNECTION,
            HeaderValue::from_static("upgrade, x-drop-me"),
        );
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
            true,
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
            true,
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
    fn upstream_ws_request_preserves_oauth_realtime_beta_header() {
        let account = oauth_account(Url::parse("https://api.example.test").expect("valid base"));
        let mut headers = HeaderMap::new();
        headers.insert(
            HeaderName::from_static("openai-beta"),
            HeaderValue::from_static("realtime=v1"),
        );
        let request = build_upstream_ws_request(
            &Url::parse("wss://api.example.test/v1/realtime").expect("valid target"),
            &headers,
            &account,
            false,
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
            Some("realtime=v1")
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
        let proxy_url = Url::parse("http://user%2Bname:p%40ss%3Aword@proxy.example.test:8080")
            .expect("valid proxy url");

        assert_eq!(
            forward_proxy_basic_auth_credential(&proxy_url).as_deref(),
            Some("user+name:p@ss:word")
        );
    }

    #[tokio::test]
    async fn prefixed_io_replays_buffered_connect_bytes() {
        let (mut client, server) = tokio::io::duplex(64);
        client.write_all(b"inner").await.expect("write inner bytes");
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
        assert!(!event.contains_encrypted_content);
    }

    #[test]
    fn websocket_usage_event_detects_encrypted_content() {
        let event = parse_ws_usage_event(
            r#"{
                "type": "response.completed",
                "response": {
                    "id": "resp_2",
                    "model": "gpt-5.5",
                    "output": [{
                        "type": "encrypted_content",
                        "encrypted_content": "opaque"
                    }],
                    "usage": {
                        "input_tokens": 5,
                        "output_tokens": 2,
                        "total_tokens": 7
                    }
                }
            }"#,
        )
        .expect("usage event");

        assert!(event.contains_encrypted_content);
    }

    #[test]
    fn websocket_request_payload_detects_only_structured_encrypted_content() {
        assert!(ws_request_payload_contains_encrypted_content(
            br#"{"type":"message","content":[{"type":"encrypted_content","encrypted_content":"opaque"}]}"#
        ));
        assert!(!ws_request_payload_contains_encrypted_content(
            br#"{"type":"message","text":"literal encrypted_content token in plain text"}"#
        ));
    }

    #[test]
    fn websocket_request_payload_extracts_prompt_cache_key() {
        let inspection = inspect_ws_request_payload(
            br#"{"type":"conversation.item.create","metadata":{"promptCacheKey":"pck-ws"},"item":{"type":"message"}}"#,
        )
        .expect("payload inspection");

        assert_eq!(
            inspection.event_type.as_deref(),
            Some("conversation.item.create")
        );
        assert_eq!(inspection.prompt_cache_key.as_deref(), Some("pck-ws"));
        assert!(!inspection.contains_encrypted_content);
    }

    #[test]
    fn websocket_first_frame_requires_response_create_and_extracts_turn_fields() {
        let first_message = AxumWsMessage::Text(
            json!({
                "type": "response.create",
                "model": "gpt-5-realtime",
                "prompt_cache_key": "pck-ws-turn",
                "previous_response_id": "resp_prev"
            })
            .to_string(),
        );

        let inspection =
            inspect_ws_initial_response_create_message(&first_message).expect("first frame");

        assert_eq!(inspection.event_type.as_deref(), Some("response.create"));
        assert_eq!(inspection.requested_model.as_deref(), Some("gpt-5-realtime"));
        assert_eq!(inspection.prompt_cache_key.as_deref(), Some("pck-ws-turn"));
        assert_eq!(
            inspection.previous_response_id.as_deref(),
            Some("resp_prev")
        );

        let rejected = AxumWsMessage::Text(
            json!({
                "type": "conversation.item.create",
                "prompt_cache_key": "pck-ws-turn"
            })
            .to_string(),
        );
        assert_eq!(
            inspect_ws_initial_response_create_message(&rejected).unwrap_err(),
            "websocket first frame must be response.create"
        );
    }

    #[test]
    fn websocket_effective_prompt_cache_key_ignores_sticky_only_context() {
        let trace = PoolUpstreamAttemptTraceContext {
            invoke_id: "pool-ws-sticky-only".to_string(),
            occurred_at: shanghai_now_string(),
            endpoint: "/v1/realtime".to_string(),
            sticky_key: Some("sticky-only-key".to_string()),
            requester_ip: None,
        };
        let usage_tracker = WsUsageTracker::new(
            api_key_account(Url::parse("https://api.example.test").expect("valid base")),
            trace,
            None,
        );

        assert_eq!(
            websocket_effective_prompt_cache_key(usage_tracker.prompt_cache_key.as_deref()),
            None
        );
    }

    #[test]
    fn websocket_usage_payload_marks_transport() {
        let payload = mark_websocket_payload_transport(
            r#"{"endpoint":"/v1/responses","model":"gpt-5.5"}"#.to_string(),
        )
        .expect("marked payload");
        let value: Value = serde_json::from_str(&payload).expect("json payload");

        assert_eq!(
            value.get("transport").and_then(Value::as_str),
            Some("websocket")
        );
        assert_eq!(
            value.get("endpoint").and_then(Value::as_str),
            Some("/v1/responses")
        );
    }

    #[test]
    fn websocket_usage_event_rejects_non_terminal_or_partial_usage() {
        assert!(parse_ws_usage_event(
            r#"{"type":"response.in_progress","response":{"usage":{"input_tokens":7,"output_tokens":3}}}"#
        )
        .is_none());
        assert!(
            parse_ws_usage_event(
                r#"{"type":"response.completed","response":{"usage":{"input_tokens":7}}}"#
            )
            .is_none()
        );
        assert!(parse_ws_usage_event(
            r#"{"type":"response.completed","response":{"usage":{"input_tokens":"bad","output_tokens":3}}}"#
        )
        .is_none());
    }

    #[test]
    fn websocket_usage_event_requires_completed_done_before_success() {
        let completed = parse_ws_usage_event(
            r#"{"type":"response.done","response":{"status":"completed","usage":{"input_tokens":7,"output_tokens":3,"total_tokens":10}}}"#,
        )
        .expect("completed response.done should parse");
        assert!(ws_usage_event_is_completed_success(&completed));

        let incomplete = parse_ws_usage_event(
            r#"{"type":"response.done","response":{"status":"incomplete","usage":{"input_tokens":7,"output_tokens":3,"total_tokens":10}}}"#,
        )
        .expect("incomplete response.done should parse");
        assert!(!ws_usage_event_is_completed_success(&incomplete));
    }

    #[test]
    fn websocket_terminal_failure_kind_marks_response_failed() {
        assert_eq!(
            ws_terminal_event_failure_kind(&WsUsageEvent {
                event_type: "response.failed".to_string(),
                response_id: None,
                response_status: Some("failed".to_string()),
                model: None,
                service_tier: None,
                usage: ParsedUsage::default(),
                contains_encrypted_content: false,
            }),
            Some(PROXY_FAILURE_UPSTREAM_RESPONSE_FAILED)
        );
        assert_eq!(
            ws_terminal_event_failure_kind(&WsUsageEvent {
                event_type: "response.completed".to_string(),
                response_id: None,
                response_status: Some("completed".to_string()),
                model: None,
                service_tier: None,
                usage: ParsedUsage::default(),
                contains_encrypted_content: false,
            }),
            None
        );
    }

    #[test]
    fn websocket_upstream_close_requires_retry_until_terminal_event() {
        assert!(ws_upstream_close_requires_retry(true, false));
        assert!(!ws_upstream_close_requires_retry(false, false));
        assert!(!ws_upstream_close_requires_retry(true, true));
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

    #[test]
    fn websocket_post_upgrade_close_marks_only_responses_api_key_codex_no_ws() {
        let mut api_key_codex =
            api_key_account(Url::parse("https://api.example.test").expect("url"));
        api_key_codex.kind = API_KEYS_BILLING_ACCOUNT_KIND.to_string();
        assert!(websocket_post_upgrade_close_marks_account_ws_unsupported(
            true,
            &api_key_codex
        ));
        assert!(!websocket_post_upgrade_close_marks_account_ws_unsupported(
            false,
            &api_key_codex
        ));

        let generic_api_key = api_key_account(Url::parse("https://api.example.test").expect("url"));
        assert!(!websocket_post_upgrade_close_marks_account_ws_unsupported(
            true,
            &generic_api_key
        ));

        let mut oauth_codex = oauth_account(Url::parse("https://api.example.test").expect("url"));
        oauth_codex.kind = API_KEYS_BILLING_ACCOUNT_KIND.to_string();
        assert!(!websocket_post_upgrade_close_marks_account_ws_unsupported(
            true,
            &oauth_codex
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
