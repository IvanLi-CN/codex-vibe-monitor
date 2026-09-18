pub(crate) struct WsUsageEvent {
    event_type: String,
    response_id: Option<String>,
    response_status: Option<String>,
    model: Option<String>,
    service_tier: Option<String>,
    usage: ParsedUsage,
    contains_encrypted_content: bool,
}

#[derive(Debug)]
pub(crate) struct WsRequestPayloadInspection {
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

pub(crate) fn inspect_ws_initial_response_create_message(
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

pub(crate) fn ws_message_payload_bytes(message: &AxumWsMessage) -> Option<&[u8]> {
    match message {
        AxumWsMessage::Text(text) => Some(text.as_bytes()),
        AxumWsMessage::Binary(bytes) => Some(bytes.as_ref()),
        _ => None,
    }
}

pub(crate) fn ws_message_starts_response_create_turn(message: &AxumWsMessage) -> bool {
    match message {
        AxumWsMessage::Text(text) => std::str::from_utf8(text.as_bytes())
            .ok()
            .is_some_and(ws_text_event_is_response_create),
        _ => false,
    }
}

pub(crate) fn ws_text_event_is_response_create(text: &str) -> bool {
    let Ok(value) = serde_json::from_str::<Value>(text) else {
        return false;
    };
    value
        .get("type")
        .and_then(Value::as_str)
        .is_some_and(|event_type| event_type == "response.create")
}

pub(crate) fn extract_nonempty_json_string(value: &Value, pointers: &[&str]) -> Option<String> {
    pointers.iter().find_map(|pointer| {
        value
            .pointer(pointer)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    })
}

fn extract_json_string(value: &Value, pointers: &[&str]) -> Option<String> {
    pointers.iter().find_map(|pointer| {
        value
            .pointer(pointer)
            .and_then(Value::as_str)
            .map(str::trim)
            .map(str::to_string)
    })
}

fn ws_response_id_from_text(text: &str) -> Option<String> {
    let value = serde_json::from_str::<Value>(text).ok()?;
    extract_nonempty_json_string(&value, &["/response/id", "/response_id", "/id"])
}

pub(crate) fn parse_ws_usage_event(text: &str) -> Option<WsUsageEvent> {
    let value = serde_json::from_str::<Value>(text).ok()?;
    let event_type = value.get("type")?.as_str()?.trim().to_string();
    if !ws_event_type_has_billable_usage(event_type.as_str()) {
        return None;
    }
    let usage = value
        .pointer("/response/usage")
        .map(parse_usage_value)
        .unwrap_or_default();
    if (usage.input_tokens.is_none() || usage.output_tokens.is_none())
        && !ws_terminal_event_is_failure_without_usage(text)
    {
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

pub(crate) fn ws_event_type_has_billable_usage(event_type: &str) -> bool {
    matches!(
        event_type,
        "response.completed" | "response.done" | "response.failed"
    )
}

fn ws_terminal_event_is_failure_without_usage(text: &str) -> bool {
    let Ok(value) = serde_json::from_str::<Value>(text) else {
        return false;
    };
    let Some(event_type) = value.get("type").and_then(Value::as_str) else {
        return false;
    };
    if event_type == "response.failed" {
        return true;
    }
    event_type == "response.done"
        && !value
            .pointer("/response/status")
            .or_else(|| value.get("status"))
            .and_then(Value::as_str)
            .is_some_and(|status| status.eq_ignore_ascii_case("completed"))
}

fn ws_terminal_temporary_classification(text: &str) -> Option<(&'static str, StatusCode)> {
    let Ok(value) = serde_json::from_str::<Value>(text) else {
        return None;
    };
    let error_code = ["/response/error/code", "/error/code"]
        .into_iter()
        .find_map(|pointer| value.pointer(pointer).and_then(Value::as_str))
        .map(str::trim)
        .map(str::to_ascii_lowercase);
    match error_code.as_deref()? {
        "server_is_overloaded" => Some((
            UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_SERVER_OVERLOADED,
            StatusCode::BAD_GATEWAY,
        )),
        "rate_limit_exceeded" => Some((
            UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_429_RATE_LIMIT,
            StatusCode::TOO_MANY_REQUESTS,
        )),
        "server_error" | "upstream_error" => Some((
            UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_5XX,
            StatusCode::BAD_GATEWAY,
        )),
        code if code.starts_with("server_") => Some((
            UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_5XX,
            StatusCode::BAD_GATEWAY,
        )),
        _ => None,
    }
}

pub(crate) fn websocket_upstream_error_marks_account_ws_unsupported(
    err: &tungstenite::Error,
) -> bool {
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

pub(crate) fn websocket_post_upgrade_close_marks_account_ws_unsupported(
    requires_response_create_first_frame: bool,
    account: &PoolResolvedAccount,
    close_frame: Option<&tungstenite::protocol::CloseFrame>,
) -> bool {
    requires_response_create_first_frame
        && account
            .kind
            .eq_ignore_ascii_case(API_KEYS_BILLING_ACCOUNT_KIND)
        && !account.auth.is_oauth()
        && !websocket_account_uses_official_openai_base_url(account)
        && close_frame.is_none_or(|frame| {
            matches!(
                frame.code,
                tungstenite::protocol::frame::coding::CloseCode::Normal
                    | tungstenite::protocol::frame::coding::CloseCode::Away
            )
        })
}

pub(crate) fn websocket_account_uses_official_openai_base_url(
    account: &PoolResolvedAccount,
) -> bool {
    account
        .upstream_base_url
        .host_str()
        .is_some_and(|host| host.eq_ignore_ascii_case("api.openai.com"))
}

pub(crate) fn ws_text_event_is_terminal(event_type: &str) -> bool {
    let Ok(value) = serde_json::from_str::<Value>(event_type) else {
        return false;
    };
    value
        .get("type")
        .and_then(Value::as_str)
        .is_some_and(ws_event_type_has_billable_usage)
}

pub(crate) fn ws_upstream_close_requires_retry(
    saw_downstream_request: bool,
    saw_terminal_upstream_event: bool,
) -> bool {
    saw_downstream_request && !saw_terminal_upstream_event
}

pub(crate) fn inspect_ws_request_payload(bytes: &[u8]) -> Option<WsRequestPayloadInspection> {
    let value = serde_json::from_slice::<Value>(bytes).ok()?;
    Some(WsRequestPayloadInspection {
        event_type: extract_nonempty_json_string(&value, &["/type"]),
        requested_model: extract_json_string(
            &value,
            &["/model", "/response/model", "/session/model"],
        ),
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

pub(crate) fn ws_request_payload_contains_encrypted_content(bytes: &[u8]) -> bool {
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

pub(crate) async fn apply_ws_downstream_payload_guard(
    state: &AppState,
    usage_tracker: &mut WsUsageTracker,
    reservation_key: &str,
    payload_bytes: &[u8],
) -> Result<bool> {
    if let Some(inspection) = inspect_ws_request_payload(payload_bytes) {
        if inspection.event_type.as_deref() == Some("response.create") {
            apply_ws_response_create_model_guard(
                state,
                usage_tracker,
                reservation_key,
                inspection.requested_model.as_deref(),
            )
            .await?;
        } else if let Some(model) = inspection.requested_model.as_deref() {
            update_ws_usage_tracker_request_model(state, usage_tracker, model).await?;
        }
    }
    apply_ws_request_payload_guard_outcome(state, usage_tracker, payload_bytes).await
}

async fn apply_ws_response_create_model_guard(
    state: &AppState,
    usage_tracker: &mut WsUsageTracker,
    reservation_key: &str,
    requested_model: Option<&str>,
) -> Result<()> {
    let requested_model = requested_model
        .map(str::to_string)
        .or_else(|| usage_tracker.trace.request_model.clone());
    let Some(model) = requested_model.as_deref() else {
        return Ok(());
    };
    if model_route_penalty(&state.pool, usage_tracker.account.account_id, Some(model)).await?
        == ModelRoutePenalty::Excluded
    {
        bail!("websocket model route is cooling down for {model}; retry after cooldown");
    }
    let concurrency_limit =
        model_route_concurrency_limit(&state.pool, usage_tracker.account.account_id, Some(model))
            .await?;
    if !try_reserve_pool_routing_account_for_model(
        state,
        reservation_key,
        &usage_tracker.account,
        Some(model),
        concurrency_limit,
    ) {
        bail!("websocket model route is at its concurrency limit for {model}; retry later");
    }
    update_ws_usage_tracker_request_model(state, usage_tracker, model).await?;
    observe_model_route_seen(&state.pool, usage_tracker.account.account_id, Some(model)).await
}

async fn update_ws_usage_tracker_request_model(
    state: &AppState,
    usage_tracker: &mut WsUsageTracker,
    model: &str,
) -> Result<()> {
    usage_tracker.trace.request_model = Some(model.to_string());
    update_pool_upstream_request_attempt_model(&state.pool, usage_tracker.attempt_id, Some(model))
        .await
}

async fn apply_ws_request_payload_guard_outcome(
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

async fn rewrite_ws_downstream_message_model(
    state: &AppState,
    account_id: i64,
    message: AxumWsMessage,
    active_mapping: Option<&ResolvedModelMapping>,
) -> Result<(AxumWsMessage, Option<ResolvedModelMapping>)> {
    if let Some(error) = websocket_mapping_unsafe_frame_error(&message, active_mapping.is_some()) {
        return Err(anyhow!(error));
    }
    let text = match message {
        AxumWsMessage::Text(text) => text,
        AxumWsMessage::Binary(_) | AxumWsMessage::Ping(_) | AxumWsMessage::Pong(_) => {
            return Ok((message, None));
        }
        AxumWsMessage::Close(_) => return Ok((message, None)),
    };
    let Some(mut payload) = parse_websocket_mapping_payload(&text, active_mapping.is_some())?
    else {
        return Ok((AxumWsMessage::Text(text), None));
    };
    let Some(requested_model) =
        websocket_mapping_requested_model(&payload, active_mapping.is_some())?
    else {
        return Ok((AxumWsMessage::Text(text), None));
    };
    let Some(mapping) =
        load_model_mapping_for_account(state, account_id, Some(&requested_model)).await?
    else {
        return Ok((AxumWsMessage::Text(text), None));
    };
    rewrite_websocket_json_payload_model(&mut payload, &mapping.target_model)?;
    let rewritten = serde_json::to_string(&payload)
        .context("failed to serialize mapped websocket JSON frame")?;
    Ok((AxumWsMessage::Text(rewritten), Some(mapping)))
}

fn websocket_mapping_unsafe_frame_error(
    message: &AxumWsMessage,
    mapping_active: bool,
) -> Option<&'static str> {
    if !mapping_active {
        return None;
    }
    match message {
        AxumWsMessage::Binary(_) => {
            Some("websocket model mapping cannot safely rewrite a binary frame")
        }
        _ => None,
    }
}

fn parse_websocket_mapping_payload(text: &str, mapping_active: bool) -> Result<Option<Value>> {
    let Ok(payload) = serde_json::from_str::<Value>(text) else {
        if mapping_active {
            return Err(anyhow!(
                "websocket model mapping cannot safely parse a JSON frame"
            ));
        }
        return Ok(None);
    };
    if !payload.is_object() {
        if mapping_active {
            return Err(anyhow!(
                "websocket model mapping requires a top-level JSON object"
            ));
        }
        return Ok(None);
    }
    Ok(Some(payload))
}

fn websocket_mapping_requested_model(
    payload: &Value,
    mapping_active: bool,
) -> Result<Option<String>> {
    for pointer in ["/model", "/response/model", "/session/model"] {
        let Some(model) = payload.pointer(pointer) else {
            continue;
        };
        let Some(requested_model) = model.as_str() else {
            if mapping_active {
                return Err(anyhow!(
                    "websocket model mapping requires a string model field"
                ));
            }
            return Ok(None);
        };
        return Ok(Some(requested_model.to_string()));
    }
    Ok(None)
}

fn rewrite_websocket_json_payload_model(payload: &mut Value, target_model: &str) -> Result<()> {
    for pointer in ["/model", "/response/model", "/session/model"] {
        let Some(model) = payload.pointer_mut(pointer) else {
            continue;
        };
        if !model.is_string() {
            bail!("websocket model mapping requires a string model field");
        }
        *model = Value::String(target_model.to_string());
        return Ok(());
    }
    bail!("websocket model mapping requires a model field")
}

pub(crate) fn ws_usage_event_is_completed_success(event: &WsUsageEvent) -> bool {
    match event.event_type.as_str() {
        "response.completed" => true,
        "response.done" => event
            .response_status
            .as_deref()
            .is_some_and(|status| status.eq_ignore_ascii_case("completed")),
        _ => false,
    }
}

fn ws_usage_event_is_terminal(event: &WsUsageEvent) -> bool {
    ws_event_type_has_billable_usage(event.event_type.as_str())
}

struct WsUsagePersistenceRequest<'a> {
    state: &'a AppState,
    account: &'a PoolResolvedAccount,
    trace: &'a PoolUpstreamAttemptTraceContext,
    prompt_cache_key: Option<&'a str>,
    ordinal: u64,
    event: WsUsageEvent,
    request_contains_encrypted_content: bool,
    raw_event: &'a str,
    attempt_id: Option<i64>,
    request_started_at: Option<&'a str>,
    occurred_at_override: Option<&'a str>,
    invoke_id_override: Option<&'a str>,
    response_id_override: Option<&'a str>,
    first_token_ms: Option<f64>,
    stream_duration_ms: Option<f64>,
}

struct WsUsagePayloadRequest<'a> {
    account: &'a PoolResolvedAccount,
    trace: &'a PoolUpstreamAttemptTraceContext,
    prompt_cache_key: Option<&'a str>,
    event: &'a WsUsageEvent,
    request_contains_encrypted_content: bool,
    billing_service_tier: Option<&'a str>,
    response_id: Option<&'a str>,
}

fn build_ws_usage_payload(request: WsUsagePayloadRequest<'_>) -> Result<String> {
    let WsUsagePayloadRequest {
        account,
        trace,
        prompt_cache_key,
        event,
        request_contains_encrypted_content,
        billing_service_tier,
        response_id,
    } = request;
    let model = event.model.as_deref();
    let payload = build_proxy_payload_summary(ProxyPayloadSummary {
        target: ProxyCaptureTarget::Responses,
        status: StatusCode::OK,
        is_stream: true,
        request_contains_encrypted_content,
        response_contains_encrypted_content: event.contains_encrypted_content,
        compaction_request_kind: None,
        compaction_response_kind: None,
        image_intent: None,
        request_model: trace.request_model.as_deref(),
        requested_service_tier: None,
        billing_service_tier,
        reasoning_effort: None,
        response_model: model,
        usage_missing_reason: None,
        request_parse_error: None,
        request_compression_algorithm: None,
        request_compression_mode: None,
        request_compression_logical_body_bytes: None,
        request_compression_transmitted_body_bytes: None,
        request_compression_transmission_complete: None,
        failure_kind: None,
        requester_ip: trace.requester_ip.as_deref(),
        request_user_agent: None,
        request_x_forwarded_for: None,
        request_forwarded: None,
        request_x_real_ip: None,
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
        upstream_approx_upload_bytes: None,
        upstream_approx_download_bytes: None,
        proxy_display_name: None,
        proxy_weight_delta: None,
        pool_attempt_count: None,
        pool_distinct_account_count: None,
        pool_attempt_terminal_reason: None,
        blocked_binding: None,
    });
    mark_websocket_payload_transport(payload)
}

async fn record_ws_usage_route_outcome(
    state: &AppState,
    request: &WsUsagePersistenceRequest<'_>,
    is_completed_terminal_event: bool,
    failure_kind: Option<&str>,
) -> Result<()> {
    let Some(attempt_id) = request.attempt_id else {
        return Ok(());
    };
    let _write_permit = crate::proxy_sqlite_write_coordinator::proxy_sqlite_write_coordinator()
        .acquire(crate::proxy_sqlite_write_coordinator::ProxySqliteWriteClass::InteractiveProxy)
        .await;
    if is_completed_terminal_event {
        record_model_route_success_from_attempt_admitted(
            &state.pool,
            request.account.account_id,
            attempt_id,
            request.request_started_at,
        )
        .await?;
    } else if request.account.kind == UPSTREAM_ACCOUNT_KIND_API_KEY_CODEX {
        if let Some((reason_code, http_status)) =
            ws_terminal_temporary_classification(request.raw_event)
        {
            record_api_key_temporary_model_failure_or_diagnostic(
                &state.pool,
                request.account.account_id,
                request.trace.sticky_key.as_deref(),
                request.raw_event,
                failure_kind.unwrap_or(PROXY_FAILURE_UPSTREAM_RESPONSE_FAILED),
                reason_code,
                http_status,
                Some(request.trace.invoke_id.as_str()),
                Some(attempt_id),
            )
            .await?;
        } else {
            record_model_route_failure_from_attempt_with_start_admitted(
                &state.pool,
                request.account.account_id,
                attempt_id,
                StatusCode::BAD_REQUEST,
                Some(request.raw_event),
                failure_kind,
                request.request_started_at,
            )
            .await?;
        }
    }
    Ok(())
}

async fn promote_ws_usage_cache_binding_if_needed(
    state: &AppState,
    request: &WsUsagePersistenceRequest<'_>,
) -> Result<()> {
    if !ws_usage_event_is_completed_success(&request.event) {
        return Ok(());
    }
    let Some(prompt_cache_key) = websocket_effective_prompt_cache_key(request.prompt_cache_key)
    else {
        return Ok(());
    };
    if !(request.request_contains_encrypted_content || request.event.contains_encrypted_content) {
        return Ok(());
    }
    if confirm_prompt_cache_encrypted_session_owner_success_if_enabled(
        state,
        prompt_cache_key,
        request.account.account_id,
    )
    .await?
    {
        promote_prompt_cache_group_binding_to_upstream_account_and_broadcast(
            state,
            prompt_cache_key,
            request.account.account_id,
        )
        .await?;
    }
    Ok(())
}

async fn persist_ws_usage_route_and_binding(
    state: &AppState,
    request: &WsUsagePersistenceRequest<'_>,
    is_completed_terminal_event: bool,
    failure_kind: Option<&str>,
) -> Result<()> {
    record_ws_usage_route_outcome(state, request, is_completed_terminal_event, failure_kind)
        .await?;
    promote_ws_usage_cache_binding_if_needed(state, request).await
}

struct WsUsageCaptureRequest<'a> {
    state: &'a AppState,
    event: WsUsageEvent,
    invoke_id: String,
    occurred_at: String,
    cost: Option<f64>,
    cost_breakdown: Option<ProxyCostBreakdown>,
    cost_estimated: bool,
    price_version: Option<String>,
    payload: String,
    response_body_preview_enabled: bool,
    raw_event: &'a str,
    first_token_ms: Option<f64>,
    stream_duration_ms: Option<f64>,
}

async fn persist_ws_usage_capture(request: WsUsageCaptureRequest<'_>) -> Result<()> {
    let WsUsageCaptureRequest {
        state,
        event,
        invoke_id,
        occurred_at,
        cost,
        cost_breakdown,
        cost_estimated,
        price_version,
        payload,
        response_body_preview_enabled,
        raw_event,
        first_token_ms,
        stream_duration_ms,
    } = request;
    let failure_kind = ws_terminal_event_failure_kind(&event);
    let is_failed_terminal_event = !ws_usage_event_is_completed_success(&event);
    persist_and_broadcast_proxy_capture_terminal_record(
        state,
        ProxyCaptureRecord {
            invoke_id,
            occurred_at,
            model: event.model,
            usage: event.usage,
            cost,
            cost_breakdown,
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
            response_body_preview_enabled,
            req_raw: RawPayloadMeta::default(),
            resp_raw: RawPayloadMeta::default(),
            timings: StageTimings {
                t_total_ms: 0.0,
                t_req_read_ms: 0.0,
                t_req_parse_ms: 0.0,
                t_upstream_connect_ms: 0.0,
                t_upstream_ttfb_ms: 0.0,
                first_token_ms,
                t_upstream_stream_ms: stream_duration_ms.unwrap_or(0.0),
                t_resp_parse_ms: 0.0,
                t_persist_ms: 0.0,
            },
        },
    )
    .await
}

async fn persist_ws_usage_event(request: WsUsagePersistenceRequest<'_>) -> Result<()> {
    let WsUsagePersistenceRequest {
        state,
        account,
        trace,
        prompt_cache_key,
        ordinal,
        event,
        request_contains_encrypted_content,
        raw_event,
        attempt_id,
        request_started_at,
        occurred_at_override,
        invoke_id_override,
        response_id_override,
        first_token_ms,
        stream_duration_ms,
    } = request;
    let response_body_preview_enabled = state
        .proxy_model_settings
        .read()
        .await
        .response_body_logging_enabled;
    let model = event.model.as_deref();
    let (billing_service_tier, pricing_mode) =
        resolve_proxy_billing_service_tier_and_pricing_mode_for_account(
            None,
            None,
            event.service_tier.as_deref(),
            Some(account),
        );
    let (cost_breakdown, cost_estimated, price_version) =
        estimate_proxy_cost_breakdown_from_shared_catalog(
            &state.pricing_catalog,
            model,
            &event.usage,
            billing_service_tier.as_deref(),
            pricing_mode,
        )
        .await;
    let cost = cost_breakdown.map(ProxyCostBreakdown::total);
    let occurred_at = occurred_at_override
        .map(str::to_string)
        .unwrap_or_else(shanghai_now_string);
    let response_id = event.response_id.as_deref().or(response_id_override);
    let invoke_id = invoke_id_override.map(str::to_string).unwrap_or_else(|| {
        response_id
            .map(|id| format!("{}-{id}", trace.invoke_id))
            .unwrap_or_else(|| format!("{}-turn-{ordinal}", trace.invoke_id))
    });
    let payload = build_ws_usage_payload(WsUsagePayloadRequest {
        account,
        trace,
        prompt_cache_key,
        event: &event,
        request_contains_encrypted_content,
        billing_service_tier: billing_service_tier.as_deref(),
        response_id,
    })?;
    let usage_request = WsUsagePersistenceRequest {
        state,
        account,
        trace,
        prompt_cache_key,
        ordinal,
        event,
        request_contains_encrypted_content,
        raw_event,
        attempt_id,
        request_started_at,
        occurred_at_override,
        invoke_id_override,
        response_id_override,
        first_token_ms,
        stream_duration_ms,
    };
    persist_ws_usage_route_and_binding(
        state,
        &usage_request,
        ws_usage_event_is_completed_success(&usage_request.event),
        ws_terminal_event_failure_kind(&usage_request.event),
    )
    .await?;
    persist_ws_usage_capture(WsUsageCaptureRequest {
        state,
        event: usage_request.event,
        invoke_id,
        occurred_at,
        cost,
        cost_breakdown,
        cost_estimated,
        price_version,
        payload,
        response_body_preview_enabled,
        raw_event,
        first_token_ms,
        stream_duration_ms,
    })
    .await
}
struct WsAttemptFinalization<'a> {
    state: &'a AppState,
    pending: Option<&'a PendingPoolAttemptRecord>,
    status: &'a str,
    failure_kind: Option<&'a str>,
    error_message: Option<&'a str>,
    connect_latency_ms: Option<f64>,
    first_byte_latency_ms: Option<f64>,
    stream_latency_ms: Option<f64>,
}

async fn finalize_ws_attempt(finalization: WsAttemptFinalization<'_>) {
    let WsAttemptFinalization {
        state,
        pending,
        status,
        failure_kind,
        error_message,
        connect_latency_ms,
        first_byte_latency_ms,
        stream_latency_ms,
    } = finalization;
    let Some(pending) = pending else {
        return;
    };
    let finished_at = shanghai_now_string();
    if let Err(err) = finalize_pool_upstream_request_attempt(
        &state.pool,
        pending,
        PoolAttemptFinalization {
            finished_at: finished_at.as_str(),
            status,
            http_status: None,
            downstream_http_status: None,
            failure_kind,
            error_message,
            downstream_error_message: None,
            connect_latency_ms,
            first_byte_latency_ms,
            stream_latency_ms,
            upstream_request_id: None,
            compact_support_status: None,
            compact_support_reason: None,
        },
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
