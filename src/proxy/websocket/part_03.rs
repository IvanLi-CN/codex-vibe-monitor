struct WebSocketSingleAttemptRequest<'a> {
    state: Arc<AppState>,
    original_uri: &'a Uri,
    headers: &'a HeaderMap,
    runtime_timeouts: &'a PoolRoutingTimeoutSettingsResolved,
    trace: &'a PoolUpstreamAttemptTraceContext,
    prompt_cache_key: Option<&'a str>,
    reservation_guard: PoolRoutingReservationGuard,
    account: PoolResolvedAccount,
    attempt_index: usize,
    required_subprotocol: Option<&'a str>,
}

fn websocket_attempt_failure(
    account: &PoolResolvedAccount,
    status: StatusCode,
    message: String,
    failure_kind: &'static str,
    retryable: bool,
) -> WsAttemptFailure {
    WsAttemptFailure {
        status,
        message,
        failure_kind,
        retryable,
        account_id: Some(account.account_id),
        upstream_route_key: Some(account.upstream_route_key()),
    }
}

struct WsRouteFailureRecordRequest<'a> {
    state: &'a AppState,
    account: &'a PoolResolvedAccount,
    trace: &'a PoolUpstreamAttemptTraceContext,
    pending: Option<&'a PendingPoolAttemptRecord>,
    reservation_guard: &'a mut PoolRoutingReservationGuard,
    message: &'a str,
    failure_kind: &'static str,
    log_message: &'static str,
}

struct PreparedWebSocketConnectionAttempt {
    forward_proxy_scope: ForwardProxyRouteScope,
    selected_proxy: SelectedForwardProxy,
    model_mapping: Option<ResolvedModelMapping>,
    upstream_url: Url,
    pending_attempt_record: Option<PendingPoolAttemptRecord>,
    deferred_cleanup_guard: Option<PoolEarlyPhaseOrphanCleanupGuard>,
    reservation_guard: PoolRoutingReservationGuard,
}

struct PreparedWebSocketConnectionRoute {
    forward_proxy_scope: ForwardProxyRouteScope,
    selected_proxy: SelectedForwardProxy,
    model_mapping: Option<ResolvedModelMapping>,
    upstream_url: Url,
    reservation_guard: PoolRoutingReservationGuard,
}

struct WebSocketConnectionPreparationRequest<'a> {
    state: Arc<AppState>,
    original_uri: &'a Uri,
    trace: &'a PoolUpstreamAttemptTraceContext,
    reservation_guard: PoolRoutingReservationGuard,
    account: &'a PoolResolvedAccount,
    attempt_index: usize,
}

struct ConnectedWebSocket {
    upstream: UpstreamWsStream,
    transport_flush_task: Option<UpstreamSocketFlushTask>,
    connect_latency_ms: f64,
    connect_started_at_utc: chrono::DateTime<Utc>,
}

struct WebSocketRequestBuildRequest<'a> {
    upstream_url: &'a Url,
    headers: &'a HeaderMap,
    account: &'a PoolResolvedAccount,
    original_uri: &'a Uri,
    state: &'a AppState,
    pending_attempt_record: Option<&'a PendingPoolAttemptRecord>,
    deferred_cleanup_guard: &'a mut Option<PoolEarlyPhaseOrphanCleanupGuard>,
    reservation_guard: &'a mut PoolRoutingReservationGuard,
}

struct WebSocketConnectAttemptRequest<'a> {
    state: Arc<AppState>,
    request: TungsteniteRequest<()>,
    upstream_url: &'a Url,
    selected_proxy: &'a SelectedForwardProxy,
    account: &'a PoolResolvedAccount,
    trace: &'a PoolUpstreamAttemptTraceContext,
    runtime_timeout: Duration,
    pending_attempt_record: Option<&'a PendingPoolAttemptRecord>,
    deferred_cleanup_guard: &'a mut Option<PoolEarlyPhaseOrphanCleanupGuard>,
    reservation_guard: &'a mut PoolRoutingReservationGuard,
    forward_proxy_scope: &'a ForwardProxyRouteScope,
    required_subprotocol: Option<&'a str>,
}

struct WebSocketConnectFailureRequest<'a> {
    state: &'a AppState,
    account: &'a PoolResolvedAccount,
    trace: &'a PoolUpstreamAttemptTraceContext,
    pending_attempt_record: Option<&'a PendingPoolAttemptRecord>,
    deferred_cleanup_guard: &'a mut Option<PoolEarlyPhaseOrphanCleanupGuard>,
    reservation_guard: &'a mut PoolRoutingReservationGuard,
    forward_proxy_scope: &'a ForwardProxyRouteScope,
    selected_proxy: &'a SelectedForwardProxy,
    traffic_reporter: &'a UpstreamTrafficReporter,
    socket_meter: &'a UpstreamSocketByteMeter,
    connect_started: Instant,
    message: String,
    failure_kind: &'static str,
    retryable: bool,
    mark_ws_unsupported: bool,
}

struct WebSocketSubprotocolFailureRequest<'a> {
    state: &'a AppState,
    account: &'a PoolResolvedAccount,
    trace: &'a PoolUpstreamAttemptTraceContext,
    pending_attempt_record: Option<&'a PendingPoolAttemptRecord>,
    deferred_cleanup_guard: &'a mut Option<PoolEarlyPhaseOrphanCleanupGuard>,
    reservation_guard: &'a mut PoolRoutingReservationGuard,
    traffic_reporter: &'a UpstreamTrafficReporter,
    socket_meter: &'a UpstreamSocketByteMeter,
    connect_started: Instant,
    message: String,
}

struct WebSocketConnectedAttemptRequest<'a> {
    state: &'a AppState,
    account: &'a PoolResolvedAccount,
    trace: &'a PoolUpstreamAttemptTraceContext,
    pending_attempt_record: Option<&'a PendingPoolAttemptRecord>,
    deferred_cleanup_guard: &'a mut Option<PoolEarlyPhaseOrphanCleanupGuard>,
    reservation_guard: &'a mut PoolRoutingReservationGuard,
    traffic_reporter: &'a UpstreamTrafficReporter,
    socket_meter: &'a UpstreamSocketByteMeter,
    connect_started: Instant,
    connect_started_at_utc: chrono::DateTime<Utc>,
    upstream: UpstreamWsStream,
    response: tungstenite::handshake::client::Response,
    required_subprotocol: Option<&'a str>,
}

struct WebSocketAttemptSuccessRequest<'a> {
    state: &'a AppState,
    forward_proxy_scope: &'a ForwardProxyRouteScope,
    selected_proxy: &'a SelectedForwardProxy,
    account: &'a PoolResolvedAccount,
    trace: &'a PoolUpstreamAttemptTraceContext,
    prompt_cache_key: Option<&'a str>,
    pending_attempt_record: Option<&'a PendingPoolAttemptRecord>,
    reservation_guard: &'a mut PoolRoutingReservationGuard,
    connect_started_at_utc: chrono::DateTime<Utc>,
}

async fn record_ws_route_failure(request: WsRouteFailureRecordRequest<'_>) {
    let WsRouteFailureRecordRequest {
        state,
        account,
        trace,
        pending,
        reservation_guard,
        message,
        failure_kind,
        log_message,
    } = request;
    reservation_guard.suppress_availability_publish();
    let result = record_pool_route_transport_failure_for_attempt_with_kind(
        &state.pool,
        account.account_id,
        trace.sticky_key.as_deref(),
        message,
        failure_kind,
        Some(trace.invoke_id.as_str()),
        pending.and_then(|pending| pending.attempt_id),
    )
    .await;
    if let Err(err) = &result {
        warn!(invoke_id = %trace.invoke_id, account_id = account.account_id, error = %err, "{log_message}");
    }
    if result.is_ok() {
        reservation_guard.release_after_persisted_failure();
    }
}

async fn prepare_websocket_connection_route(
    request: WebSocketConnectionPreparationRequest<'_>,
) -> Result<PreparedWebSocketConnectionRoute, WsAttemptFailure> {
    let WebSocketConnectionPreparationRequest {
        state,
        original_uri,
        trace,
        mut reservation_guard,
        account,
        attempt_index: _,
    } = request;
    let (forward_proxy_scope, selected_proxy, _client) =
        match select_pool_account_forward_proxy_client(state.as_ref(), account).await {
            Ok(selection) => selection,
            Err(message) => {
                reservation_guard.release();
                return Err(websocket_attempt_failure(
                    account,
                    StatusCode::BAD_GATEWAY,
                    message,
                    PROXY_FAILURE_FAILED_CONTACT_UPSTREAM,
                    false,
                ));
            }
        };

    let model_mapping = match load_model_mapping_for_account(
        state.as_ref(),
        account.account_id,
        trace.request_model.as_deref(),
    )
    .await
    {
        Ok(mapping) => mapping,
        Err(err) => {
            reservation_guard.release();
            return Err(websocket_attempt_failure(
                account,
                StatusCode::BAD_GATEWAY,
                format!("failed to resolve websocket model mapping: {err}"),
                PROXY_FAILURE_FAILED_CONTACT_UPSTREAM,
                false,
            ));
        }
    };
    let mut upstream_url =
        match build_websocket_upstream_url(&account.upstream_base_url, original_uri) {
            Ok(url) => url,
            Err(err) => {
                reservation_guard.release();
                return Err(websocket_attempt_failure(
                    account,
                    StatusCode::BAD_GATEWAY,
                    format!("failed to build pool websocket upstream url: {err}"),
                    PROXY_FAILURE_FAILED_CONTACT_UPSTREAM,
                    false,
                ));
            }
        };
    if let Some(mapping) = model_mapping.as_ref() {
        rewrite_websocket_upstream_url_model(&mut upstream_url, &mapping.target_model);
    }

    Ok(PreparedWebSocketConnectionRoute {
        forward_proxy_scope,
        selected_proxy,
        model_mapping,
        upstream_url,
        reservation_guard,
    })
}

async fn record_websocket_attempt_start(
    state: Arc<AppState>,
    trace: &PoolUpstreamAttemptTraceContext,
    account: &PoolResolvedAccount,
    attempt_index: usize,
    route: PreparedWebSocketConnectionRoute,
) -> PreparedWebSocketConnectionAttempt {
    let PreparedWebSocketConnectionRoute {
        forward_proxy_scope,
        selected_proxy,
        model_mapping,
        upstream_url,
        reservation_guard,
    } = route;
    let proxy_binding_key_snapshot =
        canonical_pool_attempt_proxy_binding_key(state.as_ref(), selected_proxy.key.as_str()).await;
    let group_name_snapshot = normalize_pool_attempt_group_name(account.group_name.clone());
    let mut attempt_trace = trace.clone();
    attempt_trace.upstream_base_url_host = account
        .upstream_base_url
        .host_str()
        .and_then(normalize_upstream_base_url_host_value);
    let pending_attempt_record = Some(
        begin_pool_upstream_request_attempt_with_scope_and_routing_source_and_audit(
            &state.pool,
            &attempt_trace,
            PoolAttemptStartScope {
                group_name_snapshot: group_name_snapshot.as_deref(),
                proxy_binding_key_snapshot: proxy_binding_key_snapshot.as_deref(),
                upstream_account_id: account.account_id,
                upstream_route_key: account.upstream_route_key().as_str(),
            },
            Some(account.routing_source),
            account.routing_selection_audit.as_ref(),
            PoolAttemptStartIndexes {
                attempt_index: attempt_index as i64,
                distinct_account_index: attempt_index as i64,
                same_account_retry_index: 0,
            },
            format_naive_precise(Utc::now().with_timezone(&Shanghai).naive_local()).as_str(),
        )
        .await,
    );
    if let Some(pending) = pending_attempt_record.as_ref()
        && let Some(mapping) = model_mapping.as_ref()
        && let Err(err) = annotate_pool_upstream_request_attempt_model_mapping(
            &state.pool,
            pending,
            Some(&mapping.target_model),
            Some(&mapping.source_model),
        )
        .await
    {
        warn!(
            invoke_id = %pending.invoke_id,
            error = %err,
            "failed to persist websocket model mapping audit"
        );
    }
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
    if let Some(pending) = pending_attempt_record.as_ref()
        && let Err(err) =
            broadcast_pool_upstream_attempts_snapshot(state.as_ref(), &pending.invoke_id).await
    {
        warn!(
            invoke_id = %pending.invoke_id,
            error = %err,
            "failed to broadcast websocket pool attempt snapshot"
        );
    }
    let deferred_cleanup_guard = pending_attempt_record
        .as_ref()
        .map(|pending| PoolEarlyPhaseOrphanCleanupGuard::new(state, pending.clone()));

    PreparedWebSocketConnectionAttempt {
        forward_proxy_scope,
        selected_proxy,
        model_mapping,
        upstream_url,
        pending_attempt_record,
        deferred_cleanup_guard,
        reservation_guard,
    }
}

async fn prepare_websocket_connection_attempt(
    request: WebSocketConnectionPreparationRequest<'_>,
) -> Result<PreparedWebSocketConnectionAttempt, WsAttemptFailure> {
    let WebSocketConnectionPreparationRequest {
        state,
        original_uri,
        trace,
        reservation_guard,
        account,
        attempt_index,
    } = request;
    let route = prepare_websocket_connection_route(WebSocketConnectionPreparationRequest {
        state: state.clone(),
        original_uri,
        trace,
        reservation_guard,
        account,
        attempt_index,
    })
    .await?;
    Ok(record_websocket_attempt_start(state, trace, account, attempt_index, route).await)
}

async fn build_websocket_attempt_request(
    request: WebSocketRequestBuildRequest<'_>,
) -> Result<TungsteniteRequest<()>, WsAttemptFailure> {
    let WebSocketRequestBuildRequest {
        upstream_url,
        headers,
        account,
        original_uri,
        state,
        pending_attempt_record,
        deferred_cleanup_guard,
        reservation_guard,
    } = request;
    match build_upstream_ws_request(
        upstream_url,
        headers,
        account,
        websocket_requires_response_create_first_frame(original_uri.path()),
    ) {
        Ok(request) => Ok(request),
        Err(err) => {
            let message = format!("failed to build upstream websocket request: {err}");
            finalize_ws_attempt(WsAttemptFinalization {
                state,
                pending: pending_attempt_record,
                status: POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_TRANSPORT_FAILURE,
                failure_kind: Some(PROXY_FAILURE_FAILED_CONTACT_UPSTREAM),
                error_message: Some(message.as_str()),
                connect_latency_ms: None,
                first_byte_latency_ms: None,
                stream_latency_ms: None,
            })
            .await;
            complete_deferred_pool_early_phase_cleanup_guard(deferred_cleanup_guard);
            reservation_guard.release();
            Err(websocket_attempt_failure(
                account,
                StatusCode::BAD_GATEWAY,
                message,
                PROXY_FAILURE_FAILED_CONTACT_UPSTREAM,
                false,
            ))
        }
    }
}

async fn fail_websocket_connect_attempt(
    request: WebSocketConnectFailureRequest<'_>,
) -> WsAttemptFailure {
    let WebSocketConnectFailureRequest {
        state,
        account,
        trace,
        pending_attempt_record,
        deferred_cleanup_guard,
        reservation_guard,
        forward_proxy_scope,
        selected_proxy,
        traffic_reporter,
        socket_meter,
        connect_started,
        message,
        failure_kind,
        retryable,
        mark_ws_unsupported,
    } = request;
    traffic_reporter.record_delta(socket_meter.snapshot(), Utc::now());
    finalize_ws_attempt(WsAttemptFinalization {
        state,
        pending: pending_attempt_record,
        status: POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_TRANSPORT_FAILURE,
        failure_kind: Some(failure_kind),
        error_message: Some(message.as_str()),
        connect_latency_ms: Some(elapsed_ms(connect_started)),
        first_byte_latency_ms: None,
        stream_latency_ms: None,
    })
    .await;
    complete_deferred_pool_early_phase_cleanup_guard(deferred_cleanup_guard);
    record_pool_account_forward_proxy_result(
        state,
        forward_proxy_scope,
        selected_proxy,
        ForwardProxyRouteResultKind::NetworkFailure,
    )
    .await;
    record_ws_route_failure(WsRouteFailureRecordRequest {
        state,
        account,
        trace,
        pending: pending_attempt_record,
        reservation_guard,
        message: &message,
        failure_kind,
        log_message: "failed to record websocket connection attempt failure",
    })
    .await;
    if mark_ws_unsupported
        && let Err(err) =
            ensure_account_has_websocket_unsupported_tag(&state.pool, account.account_id).await
    {
        warn!(
            invoke_id = %trace.invoke_id,
            account_id = account.account_id,
            error = %err,
            "failed to mark upstream account as websocket unsupported"
        );
    }
    reservation_guard.release();
    websocket_attempt_failure(
        account,
        StatusCode::BAD_GATEWAY,
        message,
        failure_kind,
        retryable,
    )
}

async fn fail_websocket_subprotocol_attempt(
    request: WebSocketSubprotocolFailureRequest<'_>,
) -> WsAttemptFailure {
    let WebSocketSubprotocolFailureRequest {
        state,
        account,
        trace,
        pending_attempt_record,
        deferred_cleanup_guard,
        reservation_guard,
        traffic_reporter,
        socket_meter,
        connect_started,
        message,
    } = request;
    traffic_reporter.record_delta(socket_meter.snapshot(), Utc::now());
    finalize_ws_attempt(WsAttemptFinalization {
        state,
        pending: pending_attempt_record,
        status: POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_TRANSPORT_FAILURE,
        failure_kind: Some(PROXY_FAILURE_FAILED_CONTACT_UPSTREAM),
        error_message: Some(message.as_str()),
        connect_latency_ms: Some(elapsed_ms(connect_started)),
        first_byte_latency_ms: None,
        stream_latency_ms: None,
    })
    .await;
    complete_deferred_pool_early_phase_cleanup_guard(deferred_cleanup_guard);
    record_ws_route_failure(WsRouteFailureRecordRequest {
        state,
        account,
        trace,
        pending: pending_attempt_record,
        reservation_guard,
        message: &message,
        failure_kind: PROXY_FAILURE_FAILED_CONTACT_UPSTREAM,
        log_message: "failed to record websocket subprotocol mismatch route failure",
    })
    .await;
    reservation_guard.release();
    websocket_attempt_failure(
        account,
        StatusCode::BAD_GATEWAY,
        message,
        PROXY_FAILURE_FAILED_CONTACT_UPSTREAM,
        true,
    )
}

async fn accept_connected_websocket(
    request: WebSocketConnectedAttemptRequest<'_>,
) -> Result<ConnectedWebSocket, WsAttemptFailure> {
    let WebSocketConnectedAttemptRequest {
        state,
        account,
        trace,
        pending_attempt_record,
        deferred_cleanup_guard,
        reservation_guard,
        traffic_reporter,
        socket_meter,
        connect_started,
        connect_started_at_utc,
        upstream,
        response,
        required_subprotocol,
    } = request;
    traffic_reporter.record_delta(socket_meter.snapshot(), Utc::now());
    let transport_flush_task = Some(UpstreamSocketFlushTask::spawn(
        socket_meter.clone(),
        traffic_reporter.clone(),
    ));
    let selected_subprotocol = response
        .headers()
        .get(HeaderName::from_static("sec-websocket-protocol"))
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
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
        return Err(
            fail_websocket_subprotocol_attempt(WebSocketSubprotocolFailureRequest {
                state,
                account,
                trace,
                pending_attempt_record,
                deferred_cleanup_guard,
                reservation_guard,
                traffic_reporter,
                socket_meter,
                connect_started,
                message,
            })
            .await,
        );
    }
    Ok(ConnectedWebSocket {
        upstream,
        transport_flush_task,
        connect_latency_ms: elapsed_ms(connect_started),
        connect_started_at_utc,
    })
}

async fn connect_websocket_with_timeout(
    request: TungsteniteRequest<()>,
    upstream_url: &Url,
    forward_proxy_url: Option<&Url>,
    socket_meter: UpstreamSocketByteMeter,
    runtime_timeout: Duration,
) -> Result<
    std::result::Result<
        (UpstreamWsStream, tungstenite::handshake::client::Response),
        tungstenite::Error,
    >,
    tokio::time::error::Elapsed,
> {
    timeout(
        runtime_timeout,
        connect_upstream_websocket(request, upstream_url, forward_proxy_url, socket_meter),
    )
    .await
}

struct WebSocketConnectResultContext<'a> {
    state: &'a AppState,
    account: &'a PoolResolvedAccount,
    trace: &'a PoolUpstreamAttemptTraceContext,
    pending_attempt_record: Option<&'a PendingPoolAttemptRecord>,
    deferred_cleanup_guard: &'a mut Option<PoolEarlyPhaseOrphanCleanupGuard>,
    reservation_guard: &'a mut PoolRoutingReservationGuard,
    forward_proxy_scope: &'a ForwardProxyRouteScope,
    selected_proxy: &'a SelectedForwardProxy,
    traffic_reporter: &'a UpstreamTrafficReporter,
    socket_meter: &'a UpstreamSocketByteMeter,
    connect_started: Instant,
    connect_started_at_utc: chrono::DateTime<Utc>,
    required_subprotocol: Option<&'a str>,
    runtime_timeout: Duration,
}

async fn process_websocket_connect_result(
    context: WebSocketConnectResultContext<'_>,
    connect_result: Result<
        std::result::Result<
            (UpstreamWsStream, tungstenite::handshake::client::Response),
            tungstenite::Error,
        >,
        tokio::time::error::Elapsed,
    >,
) -> Result<ConnectedWebSocket, WsAttemptFailure> {
    let WebSocketConnectResultContext {
        state,
        account,
        trace,
        pending_attempt_record,
        deferred_cleanup_guard,
        reservation_guard,
        forward_proxy_scope,
        selected_proxy,
        traffic_reporter,
        socket_meter,
        connect_started,
        connect_started_at_utc,
        required_subprotocol,
        runtime_timeout,
    } = context;
    match connect_result {
        Ok(Ok((upstream, response))) => {
            accept_connected_websocket(WebSocketConnectedAttemptRequest {
                state,
                account,
                trace,
                pending_attempt_record,
                deferred_cleanup_guard,
                reservation_guard,
                traffic_reporter,
                socket_meter,
                connect_started,
                connect_started_at_utc,
                upstream,
                response,
                required_subprotocol,
            })
            .await
        }
        Ok(Err(err)) => {
            let message = format!("failed to contact websocket upstream: {err}");
            let mark_ws_unsupported = websocket_upstream_error_marks_account_ws_unsupported(&err);
            Err(
                fail_websocket_connect_attempt(WebSocketConnectFailureRequest {
                    state,
                    account,
                    trace,
                    pending_attempt_record,
                    deferred_cleanup_guard,
                    reservation_guard,
                    forward_proxy_scope,
                    selected_proxy,
                    traffic_reporter,
                    socket_meter,
                    connect_started,
                    message,
                    failure_kind: PROXY_FAILURE_FAILED_CONTACT_UPSTREAM,
                    retryable: true,
                    mark_ws_unsupported,
                })
                .await,
            )
        }
        Err(_) => {
            let message = proxy_request_send_timeout_message(None, runtime_timeout);
            Err(
                fail_websocket_connect_attempt(WebSocketConnectFailureRequest {
                    state,
                    account,
                    trace,
                    pending_attempt_record,
                    deferred_cleanup_guard,
                    reservation_guard,
                    forward_proxy_scope,
                    selected_proxy,
                    traffic_reporter,
                    socket_meter,
                    connect_started,
                    message,
                    failure_kind: PROXY_FAILURE_UPSTREAM_HANDSHAKE_TIMEOUT,
                    retryable: true,
                    mark_ws_unsupported: false,
                })
                .await,
            )
        }
    }
}

async fn connect_websocket_attempt(
    request: WebSocketConnectAttemptRequest<'_>,
) -> Result<ConnectedWebSocket, WsAttemptFailure> {
    let WebSocketConnectAttemptRequest {
        state,
        request,
        upstream_url,
        selected_proxy,
        account,
        trace,
        runtime_timeout,
        pending_attempt_record,
        deferred_cleanup_guard,
        reservation_guard,
        forward_proxy_scope,
        required_subprotocol,
    } = request;
    let socket_meter = UpstreamSocketByteMeter::default();
    let traffic_reporter = UpstreamTrafficReporter::new(
        state.clone(),
        trace.invoke_id.clone(),
        trace.occurred_at.clone(),
        Some(account.account_id),
        upstream_url.host_str(),
    );
    let (connect_started, connect_started_at_utc) = (Instant::now(), Utc::now());
    let connect_result = connect_websocket_with_timeout(
        request,
        upstream_url,
        selected_proxy.endpoint_url.as_ref(),
        socket_meter.clone(),
        runtime_timeout,
    )
    .await;
    process_websocket_connect_result(
        WebSocketConnectResultContext {
            state: state.as_ref(),
            account,
            trace,
            pending_attempt_record,
            deferred_cleanup_guard,
            reservation_guard,
            forward_proxy_scope,
            selected_proxy,
            traffic_reporter: &traffic_reporter,
            socket_meter: &socket_meter,
            connect_started,
            connect_started_at_utc,
            required_subprotocol,
            runtime_timeout,
        },
        connect_result,
    )
    .await
}

async fn record_websocket_attempt_success(request: WebSocketAttemptSuccessRequest<'_>) {
    let WebSocketAttemptSuccessRequest {
        state,
        forward_proxy_scope,
        selected_proxy,
        account,
        trace,
        prompt_cache_key,
        pending_attempt_record,
        reservation_guard,
        connect_started_at_utc,
    } = request;
    if let Some(pending) = pending_attempt_record
        && let Err(err) = advance_pool_upstream_request_attempt_phase(
            state,
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
        state,
        forward_proxy_scope,
        selected_proxy,
        ForwardProxyRouteResultKind::CompletedRequest,
    )
    .await;
    match record_pool_route_success_with_affinity_generation_and_broadcast(
        state,
        account.account_id,
        connect_started_at_utc,
        trace.sticky_key.as_deref(),
        websocket_effective_prompt_cache_key(prompt_cache_key),
        Some(trace.invoke_id.as_str()),
        pending_attempt_record.and_then(|pending| pending.attempt_id),
        account.sticky_affinity_generation,
    )
    .await
    {
        Ok(publish_availability) => {
            reservation_guard.set_availability_publish(publish_availability);
        }
        Err(err) => {
            reservation_guard.suppress_availability_publish();
            warn!(
                invoke_id = %trace.invoke_id,
                account_id = account.account_id,
                error = %err,
                "failed to record websocket pool route success"
            );
        }
    }
}

async fn prepare_single_upstream_websocket_attempt(
    request: WebSocketSingleAttemptRequest<'_>,
) -> Result<PreparedUpstreamWebSocket, WsAttemptFailure> {
    let WebSocketSingleAttemptRequest {
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
    } = request;
    let PreparedWebSocketConnectionAttempt {
        forward_proxy_scope,
        selected_proxy,
        model_mapping,
        upstream_url,
        pending_attempt_record,
        mut deferred_cleanup_guard,
        mut reservation_guard,
    } = prepare_websocket_connection_attempt(WebSocketConnectionPreparationRequest {
        state: state.clone(),
        original_uri,
        trace,
        reservation_guard,
        account: &account,
        attempt_index,
    })
    .await?;

    let request = build_websocket_attempt_request(WebSocketRequestBuildRequest {
        upstream_url: &upstream_url,
        headers,
        account: &account,
        original_uri,
        state: state.as_ref(),
        pending_attempt_record: pending_attempt_record.as_ref(),
        deferred_cleanup_guard: &mut deferred_cleanup_guard,
        reservation_guard: &mut reservation_guard,
    })
    .await?;
    let ConnectedWebSocket {
        upstream,
        transport_flush_task,
        connect_latency_ms,
        connect_started_at_utc,
    } = connect_websocket_attempt(WebSocketConnectAttemptRequest {
        state: state.clone(),
        request,
        upstream_url: &upstream_url,
        selected_proxy: &selected_proxy,
        account: &account,
        trace,
        runtime_timeout: runtime_timeouts.default_send_timeout,
        pending_attempt_record: pending_attempt_record.as_ref(),
        deferred_cleanup_guard: &mut deferred_cleanup_guard,
        reservation_guard: &mut reservation_guard,
        forward_proxy_scope: &forward_proxy_scope,
        required_subprotocol,
    })
    .await?;

    record_websocket_attempt_success(WebSocketAttemptSuccessRequest {
        state: state.as_ref(),
        forward_proxy_scope: &forward_proxy_scope,
        selected_proxy: &selected_proxy,
        account: &account,
        trace,
        prompt_cache_key,
        pending_attempt_record: pending_attempt_record.as_ref(),
        reservation_guard: &mut reservation_guard,
        connect_started_at_utc,
    })
    .await;

    Ok(PreparedUpstreamWebSocket {
        upstream,
        transport_flush_task,
        pending_attempt_record,
        model_mapping,
        deferred_cleanup_guard,
        reservation_guard,
        account,
        trace: trace.clone(),
        prompt_cache_key: prompt_cache_key.map(str::to_string),
        connect_latency_ms,
        requires_response_create_first_frame: websocket_requires_response_create_first_frame(
            original_uri.path(),
        ),
    })
}
pub(crate) struct TimestampedWsDownstreamMessage {
    message: AxumWsMessage,
    received_at: Instant,
    received_at_rfc3339: String,
}

impl TimestampedWsDownstreamMessage {
    fn now(message: AxumWsMessage) -> Self {
        Self {
            message,
            received_at: Instant::now(),
            received_at_rfc3339: Utc::now().to_rfc3339(),
        }
    }
}
