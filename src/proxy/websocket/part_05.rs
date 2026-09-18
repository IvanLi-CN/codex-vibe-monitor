async fn proxy_websocket_tunnel_deferred_prepare(
    context: WebSocketTunnelPreparationContext,
    mut downstream: WebSocket,
) {
    let Some(first_downstream_message) = read_first_websocket_message(
        &mut downstream,
        context.state.as_ref(),
        &context.trace,
        context.proxy_request_id,
        context.runtime_timeouts.request_read_timeout,
    )
    .await
    else {
        return;
    };
    if matches!(first_downstream_message.message, AxumWsMessage::Close(_)) {
        return;
    }

    let payload_inspection = match classify_deferred_websocket_first_message(
        &first_downstream_message.message,
        context.original_uri.path(),
    ) {
        DeferredWebSocketFirstMessage::ResponseCreate(inspection) => inspection,
        DeferredWebSocketFirstMessage::PrepareImmediately => {
            // Realtime clients commonly begin with session.update or conversation.item.create
            // before the first response.create. Prepare a model-agnostic upstream immediately so
            // setup acknowledgements are not blocked while preserving the first frame.
            proxy_websocket_tunnel_immediate_prepare(
                context,
                downstream,
                Some(first_downstream_message),
            )
            .await;
            return;
        }
        DeferredWebSocketFirstMessage::Rejected => {
            reject_deferred_websocket_first_message(
                &mut downstream,
                context.state.as_ref(),
                &context.trace,
                context.proxy_request_id,
            )
            .await;
            return;
        }
    };

    let WebSocketTunnelPreparationContext {
        state,
        proxy_request_id,
        original_uri,
        headers,
        runtime_timeouts,
        sticky_key,
        requested_model,
        header_prompt_cache_key,
        required_subprotocol,
        mut trace,
        proxy_request_permit,
    } = context;

    let Some(prepared) =
        prepare_deferred_websocket_upstream(DeferredWebSocketUpstreamPreparationRequest {
            state: &state,
            downstream: &mut downstream,
            proxy_request_id,
            original_uri: &original_uri,
            headers: &headers,
            runtime_timeouts: &runtime_timeouts,
            sticky_key: sticky_key.as_deref(),
            requested_model,
            header_prompt_cache_key,
            required_subprotocol: required_subprotocol.as_deref(),
            trace: &mut trace,
            payload_inspection,
        })
        .await
    else {
        return;
    };
    proxy_websocket_tunnel(
        state,
        downstream,
        prepared,
        proxy_request_permit,
        vec![first_downstream_message],
    )
    .await;
}

struct DeferredWebSocketUpstreamPreparationRequest<'a> {
    state: &'a Arc<AppState>,
    downstream: &'a mut WebSocket,
    proxy_request_id: u64,
    original_uri: &'a Uri,
    headers: &'a HeaderMap,
    runtime_timeouts: &'a PoolRoutingTimeoutSettingsResolved,
    sticky_key: Option<&'a str>,
    requested_model: Option<String>,
    header_prompt_cache_key: Option<String>,
    required_subprotocol: Option<&'a str>,
    trace: &'a mut PoolUpstreamAttemptTraceContext,
    payload_inspection: WsRequestPayloadInspection,
}

async fn prepare_deferred_websocket_upstream(
    request: DeferredWebSocketUpstreamPreparationRequest<'_>,
) -> Option<PreparedUpstreamWebSocket> {
    let DeferredWebSocketUpstreamPreparationRequest {
        state,
        downstream,
        proxy_request_id,
        original_uri,
        headers,
        runtime_timeouts,
        sticky_key,
        requested_model,
        header_prompt_cache_key,
        required_subprotocol,
        trace,
        payload_inspection,
    } = request;
    let requested_model = payload_inspection.requested_model.or(requested_model);
    trace.request_model = requested_model.clone();
    let prompt_cache_key = payload_inspection
        .prompt_cache_key
        .or(header_prompt_cache_key);
    debug!(
        proxy_request_id,
        requested_model = ?requested_model,
        previous_response_id = ?payload_inspection.previous_response_id,
        prompt_cache_key_present = prompt_cache_key.is_some(),
        request_contains_encrypted_content = payload_inspection.contains_encrypted_content,
        "websocket first response.create inspected"
    );
    let routing = load_via_pool_effective_routing(
        state.as_ref(),
        prompt_cache_key.as_deref(),
        payload_inspection.contains_encrypted_content,
    )
    .await;
    let (binding_constraint, owner_auto_guard_active, conversation_override) = match routing {
        Ok(routing) => routing,
        Err((_status, message)) => {
            close_websocket_with_reason(downstream, axum::extract::ws::close_code::ERROR, message)
                .await;
            return None;
        }
    };
    match prepare_upstream_websocket(WebSocketUpstreamPreparationRequest {
        state: state.clone(),
        proxy_request_id,
        original_uri,
        headers,
        runtime_timeouts,
        sticky_key,
        requested_model: requested_model.as_deref(),
        prompt_cache_key: prompt_cache_key.as_deref(),
        binding_constraint,
        conversation_override,
        owner_auto_guard_active,
        trace,
        required_subprotocol,
    })
    .await
    {
        Ok(prepared) => Some(prepared),
        Err(err) => {
            let reason = if err.message == ENCRYPTED_SESSION_OWNER_UNAVAILABLE_MESSAGE {
                "encrypted_session_owner_unavailable; retry"
            } else {
                "upstream_unavailable; retry"
            };
            close_websocket_with_reason(downstream, axum::extract::ws::close_code::AGAIN, reason)
                .await;
            None
        }
    }
}

async fn close_websocket_with_reason(
    downstream: &mut WebSocket,
    code: u16,
    reason: impl Into<Cow<'static, str>>,
) {
    let _ = downstream
        .send(AxumWsMessage::Close(Some(axum::extract::ws::CloseFrame {
            code,
            reason: reason.into(),
        })))
        .await;
}

enum DeferredWebSocketFirstMessage {
    ResponseCreate(WsRequestPayloadInspection),
    PrepareImmediately,
    Rejected,
}

fn classify_deferred_websocket_first_message(
    message: &AxumWsMessage,
    path: &str,
) -> DeferredWebSocketFirstMessage {
    match inspect_ws_request_payload(ws_message_payload_bytes(message).unwrap_or_default()) {
        Some(inspection) if inspection.event_type.as_deref() == Some("response.create") => {
            DeferredWebSocketFirstMessage::ResponseCreate(inspection)
        }
        Some(_) | None if path == "/v1/realtime" => {
            DeferredWebSocketFirstMessage::PrepareImmediately
        }
        Some(_) | None => DeferredWebSocketFirstMessage::Rejected,
    }
}

async fn reject_deferred_websocket_first_message(
    downstream: &mut WebSocket,
    state: &AppState,
    trace: &PoolUpstreamAttemptTraceContext,
    proxy_request_id: u64,
) {
    let reason = "websocket first frame must be response.create";
    warn!(
        proxy_request_id,
        reason, "websocket first downstream frame rejected"
    );
    record_ws_pre_upstream_failure(
        state,
        trace,
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
}

async fn read_first_websocket_message(
    downstream: &mut WebSocket,
    state: &AppState,
    trace: &PoolUpstreamAttemptTraceContext,
    proxy_request_id: u64,
    request_read_timeout: Duration,
) -> Option<TimestampedWsDownstreamMessage> {
    match timeout(request_read_timeout, downstream.next()).await {
        Ok(Some(Ok(message))) => Some(TimestampedWsDownstreamMessage::now(message)),
        Ok(Some(Err(err))) => {
            let message = format!("failed to read websocket first response.create frame: {err}");
            warn!(
                proxy_request_id,
                error = %message,
                "downstream websocket closed before deferred upstream prepare"
            );
            record_ws_pre_upstream_failure(
                state,
                trace,
                PROXY_FAILURE_REQUEST_BODY_READ_TIMEOUT,
                message.as_str(),
            )
            .await;
            None
        }
        Ok(None) => {
            debug!(
                proxy_request_id,
                "downstream websocket closed before first response.create frame"
            );
            None
        }
        Err(_) => {
            let message = "websocket first response.create timed out";
            warn!(
                proxy_request_id,
                timeout_secs = request_read_timeout.as_secs_f64(),
                "websocket first response.create timed out"
            );
            record_ws_pre_upstream_failure(
                state,
                trace,
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
            None
        }
    }
}

pub(crate) async fn record_ws_pre_upstream_failure(
    state: &AppState,
    trace: &PoolUpstreamAttemptTraceContext,
    failure_kind: &'static str,
    message: &str,
) {
    let _ = (state, trace, failure_kind, message);
    // Pre-upstream WebSocket failures never started a real upstream dispatch. They should surface
    // through invocation-level adjudication only, not as synthetic attempt rows.
}

async fn proxy_websocket_tunnel_immediate_prepare(
    context: WebSocketTunnelPreparationContext,
    mut downstream: WebSocket,
    initial_downstream_message: Option<TimestampedWsDownstreamMessage>,
) {
    let WebSocketTunnelPreparationContext {
        state,
        proxy_request_id,
        original_uri,
        headers,
        runtime_timeouts,
        sticky_key,
        requested_model,
        header_prompt_cache_key,
        required_subprotocol,
        mut trace,
        proxy_request_permit,
    } = context;
    let initial_values = initial_websocket_prepare_values(initial_downstream_message.as_ref());
    let requested_model = initial_values.requested_model.or(requested_model);
    let prompt_cache_key = initial_values.prompt_cache_key.or(header_prompt_cache_key);
    let request_contains_encrypted_content = initial_values.contains_encrypted_content;
    trace.request_model = requested_model.clone();
    debug!(
        proxy_request_id,
        requested_model = ?requested_model,
        prompt_cache_key_present = prompt_cache_key.is_some(),
        path = %original_uri.path(),
        "websocket passthrough prepare without response.create first frame"
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
    let prepared = match prepare_upstream_websocket(WebSocketUpstreamPreparationRequest {
        state: state.clone(),
        proxy_request_id,
        original_uri: &original_uri,
        headers: &headers,
        runtime_timeouts: &runtime_timeouts,
        sticky_key: sticky_key.as_deref(),
        requested_model: requested_model.as_deref(),
        prompt_cache_key: prompt_cache_key.as_deref(),
        binding_constraint,
        conversation_override,
        owner_auto_guard_active,
        trace: &trace,
        required_subprotocol: required_subprotocol.as_deref(),
    })
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
            let _ = downstream
                .send(AxumWsMessage::Close(Some(close_frame)))
                .await;
            return;
        }
    };
    proxy_websocket_tunnel(
        state,
        downstream,
        prepared,
        proxy_request_permit,
        initial_downstream_message.into_iter().collect(),
    )
    .await;
}

struct InitialWebSocketPrepareValues {
    requested_model: Option<String>,
    prompt_cache_key: Option<String>,
    contains_encrypted_content: bool,
}

fn initial_websocket_prepare_values(
    initial_downstream_message: Option<&TimestampedWsDownstreamMessage>,
) -> InitialWebSocketPrepareValues {
    let inspection = initial_downstream_message
        .and_then(|message| ws_message_payload_bytes(&message.message))
        .and_then(inspect_ws_request_payload);
    InitialWebSocketPrepareValues {
        requested_model: inspection
            .as_ref()
            .and_then(|value| value.requested_model.clone()),
        prompt_cache_key: inspection
            .as_ref()
            .and_then(|value| value.prompt_cache_key.clone()),
        contains_encrypted_content: inspection
            .is_some_and(|value| value.contains_encrypted_content),
    }
}
pub(crate) struct WsUsageTracker {
    account: PoolResolvedAccount,
    trace: PoolUpstreamAttemptTraceContext,
    prompt_cache_key: Option<String>,
    ordinal: u64,
    request_contains_encrypted_content: bool,
    attempt_id: Option<i64>,
    request_started_at: Option<String>,
    turn_started_at: Option<Instant>,
    turn_stream_started_at: Option<Instant>,
    turn_occurred_at: Option<String>,
    turn_index: u64,
    response_id: Option<String>,
    runtime_snapshot_published: bool,
    runtime_snapshot_invoke_id: Option<String>,
    first_token_ms: Option<f64>,
}

impl WsUsageTracker {
    fn new(
        account: PoolResolvedAccount,
        trace: PoolUpstreamAttemptTraceContext,
        prompt_cache_key: Option<String>,
        attempt_id: Option<i64>,
        request_started_at: Option<String>,
    ) -> Self {
        Self {
            account,
            trace,
            prompt_cache_key,
            ordinal: 0,
            request_contains_encrypted_content: false,
            attempt_id,
            request_started_at,
            turn_started_at: None,
            turn_stream_started_at: None,
            turn_occurred_at: None,
            turn_index: 0,
            response_id: None,
            runtime_snapshot_published: false,
            runtime_snapshot_invoke_id: None,
            first_token_ms: None,
        }
    }

    fn start_turn_at(&mut self, received_at: Instant, received_at_rfc3339: String) {
        self.request_started_at = Some(received_at_rfc3339);
        self.turn_started_at = Some(received_at);
        self.turn_stream_started_at = None;
        self.turn_occurred_at = Some(shanghai_now_string());
        self.turn_index = self.ordinal.saturating_add(1);
        self.response_id = None;
        self.runtime_snapshot_published = false;
        self.runtime_snapshot_invoke_id = None;
        self.first_token_ms = None;
    }

    fn stream_duration_ms(&self) -> Option<f64> {
        self.turn_stream_started_at.map(elapsed_ms)
    }

    fn turn_invoke_id(&self) -> String {
        self.response_id
            .as_deref()
            .map(|response_id| format!("{}-{response_id}", self.trace.invoke_id))
            .unwrap_or_else(|| format!("{}-turn-{}", self.trace.invoke_id, self.turn_index))
    }

    fn observe_first_token_text(&mut self, text: &str) -> bool {
        if self.first_token_ms.is_some() {
            return false;
        }
        let Some(turn_started_at) = self.turn_started_at else {
            return false;
        };
        let Ok(value) = serde_json::from_str::<Value>(text) else {
            return false;
        };
        if !stream_payload_contains_nonempty_model_delta(None, &value) {
            return false;
        }
        self.first_token_ms = Some(elapsed_ms(turn_started_at));
        true
    }

    async fn observe_upstream_text(&mut self, state: &AppState, text: &str) {
        if self.turn_stream_started_at.is_none() {
            self.turn_stream_started_at = Some(Instant::now());
        }
        if let Some(response_id) = ws_response_id_from_text(text) {
            self.response_id = Some(response_id);
        }
        if self.observe_first_token_text(text) {
            self.publish_first_token_runtime_snapshot(state).await;
        }
        let Some(event) = parse_ws_usage_event(text) else {
            if ws_terminal_event_is_failure_without_usage(text)
                && let Some((reason_code, http_status)) = ws_terminal_temporary_classification(text)
                && let Some(attempt_id) = self.attempt_id
                && self.account.kind == UPSTREAM_ACCOUNT_KIND_API_KEY_CODEX
            {
                let _write_permit =
                    crate::proxy_sqlite_write_coordinator::proxy_sqlite_write_coordinator()
                        .acquire(
                            crate::proxy_sqlite_write_coordinator::ProxySqliteWriteClass::InteractiveProxy,
                        )
                        .await;
                if let Err(err) = record_api_key_temporary_model_failure_or_diagnostic(
                    &state.pool,
                    self.account.account_id,
                    self.trace.sticky_key.as_deref(),
                    text,
                    PROXY_FAILURE_UPSTREAM_RESPONSE_FAILED,
                    reason_code,
                    http_status,
                    Some(self.trace.invoke_id.as_str()),
                    Some(attempt_id),
                )
                .await
                {
                    warn!(
                        invoke_id = %self.trace.invoke_id,
                        account_id = self.account.account_id,
                        error = %err,
                        "failed to record websocket model route terminal failure"
                    );
                }
            }
            return;
        };
        self.ordinal = self.ordinal.saturating_add(1);
        let response_id_override = self.response_id.clone();
        let invoke_id_override = self.runtime_snapshot_invoke_id.clone();
        let stream_duration_ms = ws_usage_event_is_terminal(&event)
            .then(|| self.stream_duration_ms())
            .flatten();
        if let Err(err) = persist_ws_usage_event(WsUsagePersistenceRequest {
            state,
            account: &self.account,
            trace: &self.trace,
            prompt_cache_key: self.prompt_cache_key.as_deref(),
            ordinal: self.ordinal,
            event,
            request_contains_encrypted_content: self.request_contains_encrypted_content,
            raw_event: text,
            attempt_id: self.attempt_id,
            request_started_at: self.request_started_at.as_deref(),
            occurred_at_override: self.turn_occurred_at.as_deref(),
            invoke_id_override: invoke_id_override.as_deref(),
            response_id_override: response_id_override.as_deref(),
            first_token_ms: self.first_token_ms,
            stream_duration_ms,
        })
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

    async fn publish_first_token_runtime_snapshot(&mut self, state: &AppState) {
        if self.runtime_snapshot_published {
            return;
        }
        let Some(first_token_ms) = self.first_token_ms else {
            return;
        };
        let Some(occurred_at) = self.turn_occurred_at.as_deref() else {
            return;
        };

        let mut request_info = RequestCaptureInfo {
            model: self.trace.request_model.clone(),
            is_stream: true,
            ..RequestCaptureInfo::default()
        };
        request_info.prompt_cache_key = self.prompt_cache_key.clone();
        request_info.prompt_cache_key_attribution_source = self
            .prompt_cache_key
            .as_ref()
            .map(|_| "websocket_trace".to_string());
        let invoke_id = self.turn_invoke_id();
        let mut record = build_running_proxy_capture_record(
            &invoke_id,
            occurred_at,
            ProxyCaptureTarget::Responses,
            &request_info,
            self.trace.requester_ip.as_deref(),
            self.trace.sticky_key.as_deref(),
            self.prompt_cache_key.as_deref(),
            true,
            Some(self.account.account_id),
            Some(self.account.display_name.as_str()),
            Some(self.account.kind.as_str()),
            self.account.upstream_base_url.host_str(),
            None,
            None,
            None,
            None,
            None,
            0.0,
            0.0,
            0.0,
            0.0,
        );
        record.timings.first_token_ms = Some(first_token_ms);
        self.runtime_snapshot_invoke_id = Some(invoke_id);
        self.runtime_snapshot_published = true;
        if let Err(err) = persist_and_broadcast_proxy_capture_runtime_snapshot(state, record).await
        {
            warn!(
                invoke_id = %self.trace.invoke_id,
                error = %err,
                "failed to publish websocket first-token runtime snapshot"
            );
        }
    }

    async fn persist_interrupted_turn(&mut self, state: &AppState, reason: &str) {
        let Some(first_token_ms) = self.first_token_ms else {
            return;
        };
        let event = WsUsageEvent {
            event_type: "response.failed".to_string(),
            response_id: None,
            response_status: Some("incomplete".to_string()),
            model: self.trace.request_model.clone(),
            service_tier: None,
            usage: ParsedUsage::default(),
            contains_encrypted_content: self.request_contains_encrypted_content,
        };
        let raw_event = serde_json::json!({
            "type": "response.failed",
            "response": {"status": "incomplete"},
            "error": reason,
        })
        .to_string();
        let ordinal = self.ordinal.saturating_add(1);
        if let Err(err) = persist_ws_usage_event(WsUsagePersistenceRequest {
            state,
            account: &self.account,
            trace: &self.trace,
            prompt_cache_key: self.prompt_cache_key.as_deref(),
            ordinal,
            event,
            request_contains_encrypted_content: self.request_contains_encrypted_content,
            raw_event: &raw_event,
            attempt_id: self.attempt_id,
            request_started_at: self.request_started_at.as_deref(),
            occurred_at_override: self.turn_occurred_at.as_deref(),
            invoke_id_override: self.runtime_snapshot_invoke_id.as_deref(),
            response_id_override: self.response_id.as_deref(),
            first_token_ms: Some(first_token_ms),
            stream_duration_ms: self.stream_duration_ms(),
        })
        .await
        {
            warn!(
                invoke_id = %self.trace.invoke_id,
                error = %err,
                "failed to persist websocket interrupted turn with observed first token"
            );
        }
        self.first_token_ms = None;
    }
}
