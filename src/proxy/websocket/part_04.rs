struct WebSocketDownstreamMessageContext<'a> {
    state: &'a AppState,
    downstream_tx: &'a mut futures_util::stream::SplitSink<WebSocket, AxumWsMessage>,
    upstream_tx: &'a mut futures_util::stream::SplitSink<UpstreamWsStream, TungsteniteMessage>,
    usage_tracker: &'a mut WsUsageTracker,
    reservation_key: &'a str,
    model_mapping: &'a mut Option<ResolvedModelMapping>,
    pending_attempt_record: Option<&'a PendingPoolAttemptRecord>,
    active_turn_waiting_terminal: &'a mut bool,
    saw_terminal_upstream_event: &'a mut bool,
    drain_upstream_after_downstream_close: &'a mut bool,
    failure_kind_override: &'a mut Option<&'static str>,
    failure: &'a mut Option<String>,
    upstream_route_failure: &'a mut Option<String>,
}

struct PreparedDownstreamWebSocketMessage {
    message: AxumWsMessage,
    close_seen: bool,
}

async fn guard_downstream_websocket_message(
    context: &mut WebSocketDownstreamMessageContext<'_>,
    message: &AxumWsMessage,
) -> bool {
    let Some(payload_bytes) = ws_message_payload_bytes(message) else {
        return true;
    };
    match apply_ws_downstream_payload_guard(
        context.state,
        context.usage_tracker,
        context.reservation_key,
        payload_bytes,
    )
    .await
    {
        Ok(true) => {
            *context.failure_kind_override =
                Some(PROXY_FAILURE_ENCRYPTED_SESSION_OWNER_UNAVAILABLE);
            *context.failure = Some(ENCRYPTED_SESSION_OWNER_UNAVAILABLE_MESSAGE.to_string());
            let _ = context
                .downstream_tx
                .send(AxumWsMessage::Close(Some(axum::extract::ws::CloseFrame {
                    code: axum::extract::ws::close_code::AGAIN,
                    reason: "encrypted_session_owner_unavailable; retry".into(),
                })))
                .await;
            false
        }
        Ok(false) => true,
        Err(err) => {
            let message = err.to_string();
            if message.starts_with("websocket model route is ") {
                *context.failure = Some(message);
                let _ = context
                    .downstream_tx
                    .send(AxumWsMessage::Close(Some(axum::extract::ws::CloseFrame {
                        code: axum::extract::ws::close_code::AGAIN,
                        reason: "model_route_unavailable; retry".into(),
                    })))
                    .await;
            } else {
                *context.failure = Some(format!(
                    "failed to inspect websocket payload routing constraint: {message}"
                ));
            }
            false
        }
    }
}

async fn prepare_downstream_websocket_message(
    context: &mut WebSocketDownstreamMessageContext<'_>,
    timestamped_message: TimestampedWsDownstreamMessage,
) -> Option<PreparedDownstreamWebSocketMessage> {
    let TimestampedWsDownstreamMessage {
        mut message,
        received_at,
        received_at_rfc3339,
    } = timestamped_message;
    let close_seen = matches!(message, AxumWsMessage::Close(_));
    if ws_message_starts_response_create_turn(&message) {
        if *context.active_turn_waiting_terminal {
            context
                .usage_tracker
                .persist_interrupted_turn(
                    context.state,
                    "websocket turn superseded by a new response.create",
                )
                .await;
        }
        *context.active_turn_waiting_terminal = true;
        *context.saw_terminal_upstream_event = false;
        context
            .usage_tracker
            .start_turn_at(received_at, received_at_rfc3339);
    }
    if !guard_downstream_websocket_message(context, &message).await {
        return None;
    }
    match rewrite_ws_downstream_message_model(
        context.state,
        context.usage_tracker.account.account_id,
        message,
        context.model_mapping.as_ref(),
    )
    .await
    {
        Ok((rewritten, mapping)) => {
            message = rewritten;
            if let Some(mapping) = mapping {
                *context.model_mapping = Some(mapping.clone());
                if let Some(pending) = context.pending_attempt_record
                    && let Err(err) = annotate_pool_upstream_request_attempt_model_mapping(
                        &context.state.pool,
                        pending,
                        Some(&mapping.target_model),
                        Some(&mapping.source_model),
                    )
                    .await
                {
                    warn!(
                        invoke_id = %pending.invoke_id,
                        error = %err,
                        "failed to persist websocket frame model mapping audit"
                    );
                }
            }
        }
        Err(err) => {
            *context.failure = Some(format!("failed to rewrite websocket model mapping: {err}"));
            let _ = context
                .downstream_tx
                .send(AxumWsMessage::Close(Some(axum::extract::ws::CloseFrame {
                    code: axum::extract::ws::close_code::AGAIN,
                    reason: "model_mapping_rewrite_failed; retry".into(),
                })))
                .await;
            return None;
        }
    }
    Some(PreparedDownstreamWebSocketMessage {
        message,
        close_seen,
    })
}

async fn process_downstream_websocket_message(
    context: &mut WebSocketDownstreamMessageContext<'_>,
    timestamped_message: TimestampedWsDownstreamMessage,
) -> bool {
    let Some(PreparedDownstreamWebSocketMessage {
        message,
        close_seen,
    }) = prepare_downstream_websocket_message(context, timestamped_message).await
    else {
        return true;
    };
    if close_seen && *context.active_turn_waiting_terminal {
        *context.drain_upstream_after_downstream_close = true;
        return true;
    }
    if let Some(message) = axum_to_tungstenite_message(message)
        && let Err(err) = context.upstream_tx.send(message).await
    {
        let message = format!("failed to forward downstream websocket frame upstream: {err}");
        *context.upstream_route_failure = Some(message.clone());
        *context.failure = Some(message);
        return true;
    }
    close_seen
}

struct WebSocketUpstreamMessageContext<'a> {
    state: &'a AppState,
    downstream_tx: &'a mut futures_util::stream::SplitSink<WebSocket, AxumWsMessage>,
    usage_tracker: &'a mut WsUsageTracker,
    active_turn_waiting_terminal: &'a mut bool,
    saw_terminal_upstream_event: &'a mut bool,
    drain_upstream_after_downstream_close: &'a mut bool,
    failure: &'a mut Option<String>,
    upstream_route_failure: &'a mut Option<String>,
    mark_account_ws_unsupported_after_close: &'a mut bool,
    requires_response_create_first_frame: bool,
}

async fn handle_upstream_websocket_message(
    context: &mut WebSocketUpstreamMessageContext<'_>,
    message: TungsteniteMessage,
) -> bool {
    let close_frame = match &message {
        TungsteniteMessage::Close(frame) => Some(frame.as_ref()),
        _ => None,
    };
    let close_seen = close_frame.is_some();
    let upstream_text = match &message {
        TungsteniteMessage::Text(text) => Some(text.as_str().to_owned()),
        _ => None,
    };
    let terminal_for_message = upstream_text
        .as_deref()
        .is_some_and(ws_text_event_is_terminal);
    if let Some(text) = upstream_text.as_deref() {
        context
            .usage_tracker
            .observe_upstream_text(context.state, text)
            .await;
    }
    if terminal_for_message {
        *context.saw_terminal_upstream_event = true;
        *context.active_turn_waiting_terminal = false;
    }
    if close_seen
        && ws_upstream_close_requires_retry(
            *context.active_turn_waiting_terminal,
            *context.saw_terminal_upstream_event,
        )
    {
        let message = "upstream websocket closed before response.completed".to_string();
        *context.upstream_route_failure = Some(message.clone());
        *context.failure = Some(message);
        *context.mark_account_ws_unsupported_after_close =
            websocket_post_upgrade_close_marks_account_ws_unsupported(
                context.requires_response_create_first_frame,
                &context.usage_tracker.account,
                close_frame.flatten(),
            );
        let _ = context
            .downstream_tx
            .send(AxumWsMessage::Close(Some(axum::extract::ws::CloseFrame {
                code: axum::extract::ws::close_code::AGAIN,
                reason: "upstream_unavailable; retry".into(),
            })))
            .await;
        return true;
    }
    if let Some(message) = tungstenite_to_axum_message(message)
        && let Err(err) = context.downstream_tx.send(message).await
    {
        if terminal_for_message
            || (!*context.active_turn_waiting_terminal
                && *context.saw_terminal_upstream_event
                && close_seen)
        {
            return true;
        }
        if *context.active_turn_waiting_terminal {
            *context.drain_upstream_after_downstream_close = true;
            return true;
        }
        *context.failure = Some(format!(
            "failed to forward upstream websocket frame downstream: {err}"
        ));
        return true;
    }
    close_seen
}

async fn process_upstream_websocket_event(
    context: &mut WebSocketUpstreamMessageContext<'_>,
    event: Option<Result<TungsteniteMessage, tungstenite::Error>>,
) -> bool {
    match event {
        Some(Ok(message)) => handle_upstream_websocket_message(context, message).await,
        Some(Err(err)) => {
            let message = format!("upstream websocket error: {err}");
            *context.upstream_route_failure = Some(message.clone());
            *context.failure = Some(message);
            let _ = context
                .downstream_tx
                .send(AxumWsMessage::Close(Some(axum::extract::ws::CloseFrame {
                    code: axum::extract::ws::close_code::AGAIN,
                    reason: "upstream_unavailable; retry".into(),
                })))
                .await;
            true
        }
        None => {
            if *context.active_turn_waiting_terminal {
                let message = "upstream websocket closed before response.completed".to_string();
                *context.upstream_route_failure = Some(message.clone());
                *context.failure = Some(message);
                *context.mark_account_ws_unsupported_after_close =
                    websocket_post_upgrade_close_marks_account_ws_unsupported(
                        context.requires_response_create_first_frame,
                        &context.usage_tracker.account,
                        None,
                    );
                let _ = context
                    .downstream_tx
                    .send(AxumWsMessage::Close(Some(axum::extract::ws::CloseFrame {
                        code: axum::extract::ws::close_code::AGAIN,
                        reason: "upstream_unavailable; retry".into(),
                    })))
                    .await;
            }
            true
        }
    }
}

async fn process_downstream_websocket_event<E: std::fmt::Display>(
    context: &mut WebSocketDownstreamMessageContext<'_>,
    event: Option<Result<AxumWsMessage, E>>,
) -> bool {
    match event {
        Some(Ok(message)) => {
            process_downstream_websocket_message(
                context,
                TimestampedWsDownstreamMessage::now(message),
            )
            .await
        }
        Some(Err(err)) => {
            if *context.active_turn_waiting_terminal {
                *context.drain_upstream_after_downstream_close = true;
            } else {
                *context.failure = Some(format!("downstream websocket error: {err}"));
            }
            true
        }
        None => {
            if *context.active_turn_waiting_terminal {
                *context.drain_upstream_after_downstream_close = true;
            }
            true
        }
    }
}

struct WebSocketDrainContext<'a> {
    state: &'a AppState,
    upstream_rx: &'a mut futures_util::stream::SplitStream<UpstreamWsStream>,
    usage_tracker: &'a mut WsUsageTracker,
    active_turn_waiting_terminal: &'a mut bool,
    saw_terminal_upstream_event: &'a mut bool,
    failure_kind_override: &'a mut Option<&'static str>,
    failure: &'a mut Option<String>,
    upstream_route_failure: &'a mut Option<String>,
}

async fn drain_upstream_after_downstream_close(mut context: WebSocketDrainContext<'_>) {
    let WebSocketDrainContext {
        state,
        upstream_rx,
        usage_tracker,
        active_turn_waiting_terminal,
        saw_terminal_upstream_event,
        failure_kind_override,
        failure,
        upstream_route_failure,
    } = &mut context;
    let drain_started = Instant::now();
    loop {
        if !**active_turn_waiting_terminal {
            break;
        }
        let remaining = WS_UPSTREAM_DRAIN_AFTER_DOWNSTREAM_CLOSE_TIMEOUT
            .saturating_sub(drain_started.elapsed());
        if remaining.is_zero() {
            **failure_kind_override = Some(PROXY_STREAM_TERMINAL_DOWNSTREAM_CLOSED);
            **failure =
                Some("downstream websocket closed before upstream terminal event".to_string());
            break;
        }
        match timeout(remaining, upstream_rx.next()).await {
            Ok(Some(Ok(message))) => {
                let close_seen = matches!(message, TungsteniteMessage::Close(_));
                if let TungsteniteMessage::Text(text) = &message {
                    let text = text.as_str();
                    let terminal_for_message = ws_text_event_is_terminal(text);
                    usage_tracker.observe_upstream_text(state, text).await;
                    if terminal_for_message {
                        **saw_terminal_upstream_event = true;
                        **active_turn_waiting_terminal = false;
                        break;
                    }
                }
                if close_seen && **active_turn_waiting_terminal {
                    let message = "upstream websocket closed before response.completed".to_string();
                    **upstream_route_failure = Some(message.clone());
                    **failure = Some(message);
                    break;
                }
            }
            Ok(Some(Err(err))) => {
                let message = format!("upstream websocket error during downstream drain: {err}");
                **upstream_route_failure = Some(message.clone());
                **failure = Some(message);
                break;
            }
            Ok(None) => {
                let message = "upstream websocket closed before response.completed".to_string();
                **upstream_route_failure = Some(message.clone());
                **failure = Some(message);
                break;
            }
            Err(_) => {
                **failure_kind_override = Some(PROXY_STREAM_TERMINAL_DOWNSTREAM_CLOSED);
                **failure =
                    Some("downstream websocket closed before upstream terminal event".to_string());
                break;
            }
        }
    }
}

struct WebSocketTunnelFinalization<'a> {
    state: &'a AppState,
    pending_attempt_record: Option<&'a PendingPoolAttemptRecord>,
    deferred_cleanup_guard: &'a mut Option<PoolEarlyPhaseOrphanCleanupGuard>,
    reservation_guard: &'a mut PoolRoutingReservationGuard,
    usage_tracker: &'a WsUsageTracker,
    stream_started: Instant,
    connect_latency_ms: f64,
    failure: Option<&'a str>,
    failure_kind_override: Option<&'a str>,
    upstream_route_failure: Option<&'a str>,
    mark_account_ws_unsupported_after_close: bool,
}

async fn finalize_proxy_websocket_tunnel(finalization: WebSocketTunnelFinalization<'_>) {
    let WebSocketTunnelFinalization {
        state,
        pending_attempt_record,
        deferred_cleanup_guard,
        reservation_guard,
        usage_tracker,
        stream_started,
        connect_latency_ms,
        failure,
        failure_kind_override,
        upstream_route_failure,
        mark_account_ws_unsupported_after_close,
    } = finalization;
    let status = if failure.is_some() {
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_TRANSPORT_FAILURE
    } else {
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS
    };
    finalize_ws_attempt(WsAttemptFinalization {
        state,
        pending: pending_attempt_record,
        status,
        failure_kind: failure
            .map(|_| failure_kind_override.unwrap_or(PROXY_FAILURE_UPSTREAM_STREAM_ERROR)),
        error_message: failure,
        connect_latency_ms: Some(connect_latency_ms),
        first_byte_latency_ms: None,
        stream_latency_ms: Some(elapsed_ms(stream_started)),
    })
    .await;
    if let Some(message) = upstream_route_failure {
        reservation_guard.suppress_availability_publish();
        let failure_recorded = record_pool_route_transport_failure_for_attempt_with_kind(
            &state.pool,
            usage_tracker.account.account_id,
            usage_tracker.trace.sticky_key.as_deref(),
            message,
            failure_kind_override.unwrap_or(PROXY_FAILURE_UPSTREAM_STREAM_ERROR),
            Some(usage_tracker.trace.invoke_id.as_str()),
            pending_attempt_record.and_then(|pending| pending.attempt_id),
        )
        .await;
        if let Err(ref err) = failure_recorded {
            warn!(
                invoke_id = %usage_tracker.trace.invoke_id,
                account_id = usage_tracker.account.account_id,
                error = %err,
                "failed to record post-upgrade websocket pool route transport failure"
            );
        }
        if failure_recorded.is_ok() {
            reservation_guard.release_after_persisted_failure();
        }
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
    complete_deferred_pool_early_phase_cleanup_guard(deferred_cleanup_guard);
    reservation_guard.release();
}

struct WebSocketTunnelLoopContext<'a> {
    state: &'a AppState,
    downstream_tx: &'a mut futures_util::stream::SplitSink<WebSocket, AxumWsMessage>,
    downstream_rx: &'a mut futures_util::stream::SplitStream<WebSocket>,
    upstream_tx: &'a mut futures_util::stream::SplitSink<UpstreamWsStream, TungsteniteMessage>,
    upstream_rx: &'a mut futures_util::stream::SplitStream<UpstreamWsStream>,
    usage_tracker: &'a mut WsUsageTracker,
    reservation_key: &'a str,
    model_mapping: &'a mut Option<ResolvedModelMapping>,
    pending_attempt_record: Option<&'a PendingPoolAttemptRecord>,
    active_turn_waiting_terminal: &'a mut bool,
    saw_terminal_upstream_event: &'a mut bool,
    drain_upstream_after_downstream_close: &'a mut bool,
    failure_kind_override: &'a mut Option<&'static str>,
    failure: &'a mut Option<String>,
    upstream_route_failure: &'a mut Option<String>,
    mark_account_ws_unsupported_after_close: &'a mut bool,
    requires_response_create_first_frame: bool,
    initial_downstream_messages: Vec<TimestampedWsDownstreamMessage>,
}

fn downstream_websocket_message_context<'context, 'state>(
    context: &'context mut WebSocketTunnelLoopContext<'state>,
) -> WebSocketDownstreamMessageContext<'context> {
    WebSocketDownstreamMessageContext {
        state: context.state,
        downstream_tx: context.downstream_tx,
        upstream_tx: context.upstream_tx,
        usage_tracker: context.usage_tracker,
        reservation_key: context.reservation_key,
        model_mapping: context.model_mapping,
        pending_attempt_record: context.pending_attempt_record,
        active_turn_waiting_terminal: context.active_turn_waiting_terminal,
        saw_terminal_upstream_event: context.saw_terminal_upstream_event,
        drain_upstream_after_downstream_close: context.drain_upstream_after_downstream_close,
        failure_kind_override: context.failure_kind_override,
        failure: context.failure,
        upstream_route_failure: context.upstream_route_failure,
    }
}

fn upstream_websocket_message_context<'context, 'state>(
    context: &'context mut WebSocketTunnelLoopContext<'state>,
) -> WebSocketUpstreamMessageContext<'context> {
    WebSocketUpstreamMessageContext {
        state: context.state,
        downstream_tx: context.downstream_tx,
        usage_tracker: context.usage_tracker,
        active_turn_waiting_terminal: context.active_turn_waiting_terminal,
        saw_terminal_upstream_event: context.saw_terminal_upstream_event,
        drain_upstream_after_downstream_close: context.drain_upstream_after_downstream_close,
        failure: context.failure,
        upstream_route_failure: context.upstream_route_failure,
        mark_account_ws_unsupported_after_close: context.mark_account_ws_unsupported_after_close,
        requires_response_create_first_frame: context.requires_response_create_first_frame,
    }
}

async fn run_websocket_tunnel_loop(mut context: WebSocketTunnelLoopContext<'_>) {
    let mut pending_downstream_messages =
        std::collections::VecDeque::from(std::mem::take(&mut context.initial_downstream_messages));

    loop {
        if let Some(timestamped_message) = pending_downstream_messages.pop_front() {
            if process_downstream_websocket_message(
                &mut downstream_websocket_message_context(&mut context),
                timestamped_message,
            )
            .await
            {
                break;
            }
            continue;
        }
        tokio::select! {
            downstream_msg = context.downstream_rx.next() => {
                if process_downstream_websocket_event(
                    &mut downstream_websocket_message_context(&mut context),
                    downstream_msg,
                )
                .await
                {
                    break;
                }
            }
            upstream_msg = context.upstream_rx.next() => {
                if process_upstream_websocket_event(
                    &mut upstream_websocket_message_context(&mut context),
                    upstream_msg,
                )
                .await
                {
                    break;
                }
            }
        }
    }

    if *context.drain_upstream_after_downstream_close {
        drain_upstream_after_downstream_close(WebSocketDrainContext {
            state: context.state,
            upstream_rx: context.upstream_rx,
            usage_tracker: context.usage_tracker,
            active_turn_waiting_terminal: context.active_turn_waiting_terminal,
            saw_terminal_upstream_event: context.saw_terminal_upstream_event,
            failure_kind_override: context.failure_kind_override,
            failure: context.failure,
            upstream_route_failure: context.upstream_route_failure,
        })
        .await;
    }
}

pub(crate) async fn proxy_websocket_tunnel(
    state: Arc<AppState>,
    downstream: WebSocket,
    prepared: PreparedUpstreamWebSocket,
    _proxy_request_permit: ProxyRequestConcurrencyPermit,
    initial_downstream_messages: Vec<TimestampedWsDownstreamMessage>,
) {
    let PreparedUpstreamWebSocket {
        upstream,
        transport_flush_task: _transport_flush_task,
        pending_attempt_record,
        mut model_mapping,
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
    let mut usage_tracker = WsUsageTracker::new(
        account,
        trace,
        prompt_cache_key,
        pending_attempt_record
            .as_ref()
            .and_then(|pending| pending.attempt_id),
        pending_attempt_record.as_ref().and_then(|pending| {
            parse_to_utc_datetime(&pending.started_at).map(|started| started.to_rfc3339())
        }),
    );
    let mut active_turn_waiting_terminal = false;
    let mut saw_terminal_upstream_event = false;
    let mut drain_upstream_after_downstream_close = false;

    run_websocket_tunnel_loop(WebSocketTunnelLoopContext {
        state: state.as_ref(),
        downstream_tx: &mut downstream_tx,
        downstream_rx: &mut downstream_rx,
        upstream_tx: &mut upstream_tx,
        upstream_rx: &mut upstream_rx,
        usage_tracker: &mut usage_tracker,
        reservation_key: &reservation_guard.reservation_key,
        model_mapping: &mut model_mapping,
        pending_attempt_record: pending_attempt_record.as_ref(),
        active_turn_waiting_terminal: &mut active_turn_waiting_terminal,
        saw_terminal_upstream_event: &mut saw_terminal_upstream_event,
        drain_upstream_after_downstream_close: &mut drain_upstream_after_downstream_close,
        failure_kind_override: &mut failure_kind_override,
        failure: &mut failure,
        upstream_route_failure: &mut upstream_route_failure,
        mark_account_ws_unsupported_after_close: &mut mark_account_ws_unsupported_after_close,
        requires_response_create_first_frame,
        initial_downstream_messages,
    })
    .await;

    if active_turn_waiting_terminal && !saw_terminal_upstream_event {
        usage_tracker
            .persist_interrupted_turn(
                state.as_ref(),
                failure
                    .as_deref()
                    .unwrap_or("websocket turn ended before terminal event"),
            )
            .await;
    }

    finalize_proxy_websocket_tunnel(WebSocketTunnelFinalization {
        state: state.as_ref(),
        pending_attempt_record: pending_attempt_record.as_ref(),
        deferred_cleanup_guard: &mut deferred_cleanup_guard,
        reservation_guard: &mut reservation_guard,
        usage_tracker: &usage_tracker,
        stream_started,
        connect_latency_ms,
        failure: failure.as_deref(),
        failure_kind_override,
        upstream_route_failure: upstream_route_failure.as_deref(),
        mark_account_ws_unsupported_after_close,
    })
    .await;
}
