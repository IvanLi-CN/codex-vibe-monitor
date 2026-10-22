use super::*;

pub(crate) fn sticky_key_from_payload(payload: Option<&str>) -> Option<String> {
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
pub(crate) fn payload_text(payload: Option<&str>, key: &str) -> Option<String> {
    let payload = payload?;
    let value = serde_json::from_str::<Value>(payload).ok()?;
    value
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}
pub(crate) fn payload_i64(payload: Option<&str>, key: &str) -> Option<i64> {
    let payload = payload?;
    let value = serde_json::from_str::<Value>(payload).ok()?;
    value.get(key).and_then(Value::as_i64)
}

pub(crate) fn payload_f64(payload: Option<&str>, key: &str) -> Option<f64> {
    let payload = payload?;
    let value = serde_json::from_str::<Value>(payload).ok()?;
    value.get(key).and_then(Value::as_f64)
}

pub(crate) fn set_proxy_capture_record_request_compression_algorithm(
    record: &mut ProxyCaptureRecord,
    algorithm: Option<&str>,
) {
    let Some(algorithm) = algorithm.map(str::trim).filter(|value| !value.is_empty()) else {
        return;
    };
    let Some(payload) = record.payload.as_deref() else {
        return;
    };
    let Ok(mut value) = serde_json::from_str::<Value>(payload) else {
        return;
    };
    let Some(object) = value.as_object_mut() else {
        return;
    };
    object.insert(
        "requestCompressionAlgorithm".to_string(),
        Value::String(algorithm.to_string()),
    );
    if let Ok(payload) = serde_json::to_string(&value) {
        record.payload = Some(payload);
    }
}

pub(crate) fn set_proxy_capture_record_pool_routing_no_candidate_audit(
    record: &mut ProxyCaptureRecord,
    audit: Option<&PoolRoutingNoCandidateAudit>,
) {
    let Some(audit) = audit else {
        return;
    };
    let Some(payload) = record.payload.as_deref() else {
        return;
    };
    let Ok(mut value) = serde_json::from_str::<Value>(payload) else {
        return;
    };
    let Some(object) = value.as_object_mut() else {
        return;
    };
    object.insert("poolRoutingNoCandidateAudit".to_string(), json!(audit));
    if let Ok(payload) = serde_json::to_string(&value) {
        record.payload = Some(payload);
    }
}

pub(crate) async fn persist_pool_routing_no_candidate_invocation(
    state: Arc<AppState>,
    trace: &PoolUpstreamAttemptTraceContext,
    prompt_cache_key: Option<&str>,
    audit: &PoolRoutingNoCandidateAudit,
) -> Result<()> {
    persist_pool_routing_no_candidate_invocation_with_snapshot(
        state,
        trace,
        prompt_cache_key,
        audit,
        None,
        StatusCode::SERVICE_UNAVAILABLE,
        PROXY_FAILURE_POOL_NO_AVAILABLE_ACCOUNT,
        POOL_NO_AVAILABLE_ACCOUNT_MESSAGE,
        0,
        0,
        PROXY_FAILURE_POOL_NO_AVAILABLE_ACCOUNT,
    )
    .await
}

pub(crate) async fn persist_pool_routing_no_candidate_invocation_with_snapshot(
    state: Arc<AppState>,
    trace: &PoolUpstreamAttemptTraceContext,
    prompt_cache_key: Option<&str>,
    audit: &PoolRoutingNoCandidateAudit,
    request_body_snapshot: Option<PoolReplayBodySnapshot>,
    status: StatusCode,
    failure_kind: &str,
    error_message: &str,
    pool_attempt_count: usize,
    pool_distinct_account_count: usize,
    pool_attempt_terminal_reason: &str,
) -> Result<()> {
    let target = ProxyCaptureTarget::from_endpoint(&trace.endpoint);
    let mut record = build_admitted_proxy_capture_runtime_snapshot(
        &trace.invoke_id,
        &trace.occurred_at,
        target,
        trace.requester_ip.as_deref(),
        trace.sticky_key.as_deref(),
        prompt_cache_key,
    );
    let request_body_logging_enabled = state
        .proxy_model_settings
        .read()
        .await
        .request_body_logging_enabled;
    record.req_raw = match request_body_snapshot {
        Some(snapshot) => {
            spawn_raw_payload_snapshot_write(
                state.clone(),
                &trace.invoke_id,
                "request",
                snapshot,
                request_body_logging_enabled,
            )
            .finish()
            .await
        }
        None => RawPayloadMeta::default(),
    };
    record.model = trace.request_model.clone();
    record.status = format!("http_{}", status.as_u16());
    record.error_message = Some(format!("[{failure_kind}] {error_message}"));
    record.failure_kind = Some(failure_kind.to_string());
    let response_envelope = build_proxy_error_response_envelope(
        &ProxyErrorResponse {
            status,
            message: error_message.to_string(),
            cvm_id: None,
            retry_after_secs: retry_after_secs_for_proxy_error(status, error_message),
            code: Some(failure_kind.to_string()),
            blocked_binding: None,
        },
        &trace.invoke_id,
    );
    record.raw_response = response_envelope.body_text;
    record.response_body_preview_enabled = state
        .proxy_model_settings
        .read()
        .await
        .response_body_logging_enabled;
    record.resp_raw = RawPayloadMeta {
        size_bytes: record.raw_response.len() as i64,
        ..RawPayloadMeta::default()
    };
    if let Some(payload) = record.payload.as_deref()
        && let Ok(mut value) = serde_json::from_str::<Value>(payload)
        && let Some(object) = value.as_object_mut()
    {
        object.insert("statusCode".to_string(), json!(status.as_u16()));
        object.insert(
            "requestModel".to_string(),
            json!(trace.request_model.as_deref()),
        );
        object.insert("failureKind".to_string(), json!(failure_kind));
        object.insert("downstreamStatusCode".to_string(), json!(status.as_u16()));
        object.insert("downstreamErrorMessage".to_string(), json!(error_message));
        object.insert("poolAttemptCount".to_string(), json!(pool_attempt_count));
        object.insert(
            "poolDistinctAccountCount".to_string(),
            json!(pool_distinct_account_count),
        );
        object.insert(
            "poolAttemptTerminalReason".to_string(),
            json!(pool_attempt_terminal_reason),
        );
        record.payload = serde_json::to_string(&value).ok();
    }
    set_proxy_capture_record_pool_routing_no_candidate_audit(&mut record, Some(audit));
    persist_and_broadcast_proxy_capture_terminal_record(state.as_ref(), record).await
}

pub(crate) async fn persist_pool_routing_no_candidate_invocation_with_error(
    state: Arc<AppState>,
    trace: &PoolUpstreamAttemptTraceContext,
    prompt_cache_key: Option<&str>,
    audit: &PoolRoutingNoCandidateAudit,
    status: StatusCode,
    failure_kind: &str,
    error_message: &str,
    pool_attempt_count: usize,
    pool_distinct_account_count: usize,
    pool_attempt_terminal_reason: &str,
) -> Result<()> {
    persist_pool_routing_no_candidate_invocation_with_snapshot(
        state,
        trace,
        prompt_cache_key,
        audit,
        None,
        status,
        failure_kind,
        error_message,
        pool_attempt_count,
        pool_distinct_account_count,
        pool_attempt_terminal_reason,
    )
    .await
}

pub(crate) fn blocked_binding_json_from_payload(payload: Option<&str>) -> Option<String> {
    let payload = payload?;
    let value = serde_json::from_str::<Value>(payload).ok()?;
    let blocked_binding = value.get("blockedBinding")?;
    if !blocked_binding.is_object() {
        return None;
    }
    serde_json::to_string(blocked_binding).ok()
}

pub(crate) fn blocked_binding_from_payload(
    payload: Option<&str>,
) -> Option<BlockedBindingDiagnostic> {
    parse_blocked_binding_json(blocked_binding_json_from_payload(payload).as_deref())
}

pub(crate) fn hydrate_api_invocation_blocked_binding(record: &mut ApiInvocation) {
    if record.blocked_binding.is_none() {
        record.blocked_binding = parse_blocked_binding_json(record.blocked_binding_json.as_deref());
    }
}

pub(crate) fn shanghai_now_string() -> String {
    format_naive(Utc::now().with_timezone(&Shanghai).naive_local())
}

pub(crate) fn terminal_pool_upstream_request_attempt_phase(status: &str) -> &'static str {
    if status == POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS {
        POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_COMPLETED
    } else {
        POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_FAILED
    }
}

pub(crate) async fn insert_pool_upstream_request_attempt_with_scope(
    pool: &Pool<Sqlite>,
    trace: &PoolUpstreamAttemptTraceContext,
    group_name_snapshot: Option<&str>,
    proxy_binding_key_snapshot: Option<&str>,
    routing_source: Option<&str>,
    routing_selection_audit_json: Option<&str>,
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
    downstream_http_status: Option<StatusCode>,
    failure_kind: Option<&str>,
    error_message: Option<&str>,
    downstream_error_message: Option<&str>,
    connect_latency_ms: Option<f64>,
    first_byte_latency_ms: Option<f64>,
    stream_latency_ms: Option<f64>,
    upstream_request_id: Option<&str>,
    upstream_request_compression_algorithm: Option<&str>,
    upstream_request_compression_mode: Option<&str>,
    upstream_request_logical_body_bytes: Option<i64>,
    upstream_request_transmitted_body_bytes: Option<i64>,
    upstream_request_header_bytes_approx: Option<i64>,
    upstream_response_body_bytes: Option<i64>,
    upstream_response_header_bytes_approx: Option<i64>,
    compact_support_status: Option<&str>,
    compact_support_reason: Option<&str>,
) -> Result<i64> {
    for _ in 0..POOL_UPSTREAM_REQUEST_ATTEMPT_PUBLIC_ID_RETRY_LIMIT {
        let attempt_public_id = generate_pool_upstream_request_attempt_public_id();
        let result = sqlx::query(
            r#"
            INSERT INTO pool_upstream_request_attempts (
                attempt_public_id,
                invoke_id,
                occurred_at,
                endpoint,
                route_mode,
                sticky_key,
                routing_source,
                routing_selection_audit_json,
                upstream_base_url_host,
                group_name_snapshot,
                proxy_binding_key_snapshot,
                request_model,
                upstream_request_model,
                model_mapping_pattern,
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
                downstream_http_status,
                failure_kind,
                error_message,
                downstream_error_message,
                connect_latency_ms,
                first_byte_latency_ms,
                stream_latency_ms,
                upstream_request_id,
                upstream_request_compression_algorithm,
                upstream_request_compression_mode,
                upstream_request_logical_body_bytes,
                upstream_request_transmitted_body_bytes,
                upstream_request_header_bytes_approx,
                upstream_response_body_bytes,
                upstream_response_header_bytes_approx,
                compact_support_status,
                compact_support_reason
            )
            VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25, ?26, ?27, ?28, ?29, ?30, ?31, ?32, ?33, ?34, ?35, ?36, ?37, ?38, ?39, ?40, ?41, ?42
            )
            "#,
        )
        .bind(&attempt_public_id)
        .bind(&trace.invoke_id)
        .bind(&trace.occurred_at)
        .bind(&trace.endpoint)
        .bind(INVOCATION_ROUTE_MODE_POOL)
        .bind(trace.sticky_key.as_deref())
        .bind(routing_source)
        .bind(routing_selection_audit_json)
        .bind(trace.upstream_base_url_host.as_deref())
        .bind(group_name_snapshot)
        .bind(proxy_binding_key_snapshot)
        .bind(trace.request_model.as_deref())
        .bind(trace.request_model.as_deref())
        .bind(Option::<&str>::None)
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
        .bind(downstream_http_status.map(|value| i64::from(value.as_u16())))
        .bind(failure_kind)
        .bind(error_message)
        .bind(downstream_error_message)
        .bind(connect_latency_ms)
        .bind(first_byte_latency_ms)
        .bind(stream_latency_ms)
        .bind(upstream_request_id)
        .bind(upstream_request_compression_algorithm)
        .bind(upstream_request_compression_mode)
        .bind(upstream_request_logical_body_bytes)
        .bind(upstream_request_transmitted_body_bytes)
        .bind(upstream_request_header_bytes_approx)
        .bind(upstream_response_body_bytes)
        .bind(upstream_response_header_bytes_approx)
        .bind(compact_support_status)
        .bind(compact_support_reason)
        .execute(pool)
        .await;
        match result {
            Ok(result) => return Ok(result.last_insert_rowid()),
            Err(error) if is_pool_upstream_request_attempt_public_id_collision(&error) => continue,
            Err(error) => return Err(error.into()),
        }
    }

    bail!("failed to allocate unique attempt_public_id for pool_upstream_request_attempts insert")
}

pub(crate) async fn insert_pool_upstream_request_attempt(
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
    downstream_http_status: Option<StatusCode>,
    failure_kind: Option<&str>,
    error_message: Option<&str>,
    downstream_error_message: Option<&str>,
    connect_latency_ms: Option<f64>,
    first_byte_latency_ms: Option<f64>,
    stream_latency_ms: Option<f64>,
    upstream_request_id: Option<&str>,
    compact_support_status: Option<&str>,
    compact_support_reason: Option<&str>,
) -> Result<i64> {
    insert_pool_upstream_request_attempt_with_scope(
        pool,
        trace,
        None,
        None,
        None,
        None,
        upstream_account_id,
        upstream_route_key,
        attempt_index,
        distinct_account_index,
        same_account_retry_index,
        started_at,
        finished_at,
        status,
        phase,
        http_status,
        downstream_http_status,
        failure_kind,
        error_message,
        downstream_error_message,
        connect_latency_ms,
        first_byte_latency_ms,
        stream_latency_ms,
        upstream_request_id,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        compact_support_status,
        compact_support_reason,
    )
    .await
}

pub(crate) async fn begin_pool_upstream_request_attempt(
    pool: &Pool<Sqlite>,
    trace: &PoolUpstreamAttemptTraceContext,
    upstream_account_id: i64,
    upstream_route_key: &str,
    attempt_index: i64,
    distinct_account_index: i64,
    same_account_retry_index: i64,
    started_at: &str,
) -> PendingPoolAttemptRecord {
    begin_pool_upstream_request_attempt_with_scope(
        pool,
        trace,
        None,
        None,
        upstream_account_id,
        upstream_route_key,
        attempt_index,
        distinct_account_index,
        same_account_retry_index,
        started_at,
    )
    .await
}

pub(crate) async fn begin_pool_upstream_request_attempt_in_memory(
    state: &AppState,
    trace: &PoolUpstreamAttemptTraceContext,
    group_name_snapshot: Option<&str>,
    proxy_binding_key_snapshot: Option<&str>,
    routing_source: Option<PoolRoutingSelectionSource>,
    routing_selection_audit: Option<&PoolRoutingSelectionAudit>,
    selected_control_fingerprint: Option<&str>,
    sticky_affinity_generation: Option<i64>,
    upstream_account_id: i64,
    upstream_route_key: &str,
    attempt_index: i64,
    distinct_account_index: i64,
    same_account_retry_index: i64,
    started_at: &str,
) -> Option<PendingPoolAttemptRecord> {
    let routing_source_value = routing_source.map(PoolRoutingSelectionSource::as_persisted_str);
    let routing_selection_audit_json =
        routing_selection_audit.and_then(|audit| serde_json::to_string(audit).ok());
    let attempt_public_id = generate_pool_upstream_request_attempt_public_id();
    let pending = PendingPoolAttemptRecord {
        attempt_id: None,
        attempt_public_id: Some(attempt_public_id),
        invoke_id: trace.invoke_id.clone(),
        occurred_at: trace.occurred_at.clone(),
        endpoint: trace.endpoint.clone(),
        sticky_key: trace.sticky_key.clone(),
        routing_source: routing_source_value.map(ToOwned::to_owned),
        routing_selection_audit_json,
        sticky_affinity_generation,
        requester_ip: trace.requester_ip.clone(),
        upstream_base_url_host: trace.upstream_base_url_host.clone(),
        group_name_snapshot: group_name_snapshot.map(ToOwned::to_owned),
        proxy_binding_key_snapshot: proxy_binding_key_snapshot.map(ToOwned::to_owned),
        request_model: trace.request_model.clone(),
        upstream_request_model: trace.request_model.clone(),
        model_mapping_pattern: None,
        request_summary_json: None,
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
        upstream_request_compression_algorithm: None,
        upstream_request_compression_mode: None,
        upstream_request_logical_body_bytes: None,
        upstream_request_transmitted_body_bytes: None,
        upstream_request_header_bytes_approx: None,
        upstream_response_body_bytes: None,
        upstream_response_header_bytes_approx: None,
        response_raw_path: None,
        response_raw_codec: None,
        response_raw_size: None,
        response_raw_truncated: false,
        response_raw_truncated_reason: None,
        response_content_encoding: None,
    };
    let mut runtime_snapshot = runtime_attempt_snapshot_from_pending(&pending);
    let account = match load_upstream_account_row(&state.pool, upstream_account_id).await {
        Ok(Some(account)) => account,
        Ok(None) => return None,
        Err(error) => {
            warn!(%error, "cannot fence proxy attempt before admission");
            return None;
        }
    };
    runtime_snapshot.routing_control_fingerprint = selected_control_fingerprint
        .map(ToOwned::to_owned)
        .or_else(|| Some(route_account_control_fingerprint(&account)));
    if !state
        .proxy_runtime_attempts
        .upsert(runtime_snapshot.clone())
    {
        warn!(invoke_id = %pending.invoke_id, "bounded attempt capacity exhausted before journal admission");
        return None;
    }
    let journal = state
        .sqlite_batch_writer
        .append_runtime_attempt_start(&runtime_snapshot);
    if !matches!(
        journal.durability_mode,
        TerminalJournalDurabilityMode::Journal
    ) {
        warn!(
            invoke_id = %pending.invoke_id,
            attempt_public_id = %runtime_snapshot.attempt_public_id,
            "proxy attempt start event was rejected by the terminal journal"
        );
        state.proxy_runtime_attempts.reject_unaccepted_start(
            &pending.invoke_id,
            &pending.occurred_at,
            &runtime_snapshot.attempt_public_id,
        );
        return None;
    }
    if let Err(error) = state
        .sqlite_batch_writer
        .wait_for_runtime_journal(journal.sequence)
        .await
    {
        warn!(error = %error, invoke_id = %pending.invoke_id, "attempt start durability confirmation failed");
        state.proxy_runtime_attempts.reject_unaccepted_start(
            &pending.invoke_id,
            &pending.occurred_at,
            &runtime_snapshot.attempt_public_id,
        );
        return None;
    }
    if routing_source == Some(PoolRoutingSelectionSource::PriorityHandoff) {
        remember_priority_handoff_attempt(
            None,
            Some(trace.invoke_id.as_str()),
            upstream_account_id,
            trace.request_model.as_deref(),
            pending.routing_selection_audit_json.as_deref(),
        );
    }
    Some(pending)
}

pub(crate) fn runtime_attempt_snapshot_from_pending(
    pending: &PendingPoolAttemptRecord,
) -> RuntimeAttemptSnapshot {
    RuntimeAttemptSnapshot {
        routing_control_fingerprint: None,
        attempt_public_id: pending
            .attempt_public_id
            .clone()
            .unwrap_or_else(|| pool_attempt_response_capture_key(pending)),
        invoke_id: pending.invoke_id.clone(),
        occurred_at: pending.occurred_at.clone(),
        endpoint: pending.endpoint.clone(),
        sticky_key: pending.sticky_key.clone(),
        routing_source: pending.routing_source.clone(),
        routing_selection_audit_json: pending.routing_selection_audit_json.clone(),
        requester_ip: pending.requester_ip.clone(),
        upstream_base_url_host: pending.upstream_base_url_host.clone(),
        group_name_snapshot: pending.group_name_snapshot.clone(),
        proxy_binding_key_snapshot: pending.proxy_binding_key_snapshot.clone(),
        request_model: pending.request_model.clone(),
        upstream_request_model: pending.upstream_request_model.clone(),
        model_mapping_pattern: pending.model_mapping_pattern.clone(),
        request_summary_json: pending.request_summary_json.clone(),
        upstream_account_id: pending.upstream_account_id,
        upstream_route_key: pending.upstream_route_key.clone(),
        attempt_index: pending.attempt_index,
        distinct_account_index: pending.distinct_account_index,
        same_account_retry_index: pending.same_account_retry_index,
        started_at: pending.started_at.clone(),
        finished_at: None,
        status: POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_PENDING.to_string(),
        phase: Some(POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_CONNECTING.to_string()),
        http_status: None,
        downstream_http_status: None,
        failure_kind: None,
        error_message: None,
        downstream_error_message: None,
        connect_latency_ms: (pending.connect_latency_ms > 0.0)
            .then_some(pending.connect_latency_ms),
        first_byte_latency_ms: (pending.first_byte_latency_ms > 0.0)
            .then_some(pending.first_byte_latency_ms),
        stream_latency_ms: None,
        upstream_request_id: None,
        compact_support_status: pending.compact_support_status.clone(),
        compact_support_reason: pending.compact_support_reason.clone(),
        upstream_request_compression_algorithm: pending
            .upstream_request_compression_algorithm
            .clone(),
        upstream_request_compression_mode: pending.upstream_request_compression_mode.clone(),
        upstream_request_logical_body_bytes: pending.upstream_request_logical_body_bytes,
        upstream_request_transmitted_body_bytes: pending.upstream_request_transmitted_body_bytes,
        upstream_request_header_bytes_approx: pending.upstream_request_header_bytes_approx,
        upstream_response_body_bytes: pending.upstream_response_body_bytes,
        upstream_response_header_bytes_approx: pending.upstream_response_header_bytes_approx,
        response_raw_path: pending.response_raw_path.clone(),
        response_raw_codec: pending.response_raw_codec.clone(),
        response_raw_size: pending.response_raw_size,
        response_raw_truncated: pending.response_raw_truncated,
        response_raw_truncated_reason: pending.response_raw_truncated_reason.clone(),
        response_content_encoding: pending.response_content_encoding.clone(),
    }
}

pub(crate) fn update_runtime_attempt_from_pending(
    state: &AppState,
    pending: &PendingPoolAttemptRecord,
) -> bool {
    upsert_runtime_attempt_with_completion_capacity(
        state,
        runtime_attempt_snapshot_from_pending(pending),
        false,
    )
}

fn upsert_runtime_attempt_with_completion_capacity(
    state: &AppState,
    snapshot: RuntimeAttemptSnapshot,
    terminal: bool,
) -> bool {
    state
        .proxy_runtime_attempts
        .upsert_with_admission(snapshot, terminal, |candidate| {
            state
                .sqlite_batch_writer
                .reserve_runtime_attempt_completion_growth(candidate)
        })
}

pub(crate) fn discard_runtime_attempt_for_drop(
    state: &AppState,
    pending: &PendingPoolAttemptRecord,
) -> bool {
    let Some(attempt_public_id) = pending.attempt_public_id.as_deref() else {
        return false;
    };
    let marked = state.proxy_runtime_attempts.mark_interrupted(
        &pending.invoke_id,
        &pending.occurred_at,
        attempt_public_id,
        &shanghai_now_string(),
    );
    let discarded = state.proxy_runtime_attempts.discard_for_attempt(
        &pending.invoke_id,
        &pending.occurred_at,
        attempt_public_id,
    );
    marked || discarded
}

#[expect(
    clippy::too_many_arguments,
    reason = "Runtime attempt terminal adapter mirrors the established lifecycle contract."
)]
pub(crate) async fn finalize_pool_upstream_request_attempt_in_memory(
    state: &AppState,
    pending: &PendingPoolAttemptRecord,
    finished_at: &str,
    status: &str,
    http_status: Option<StatusCode>,
    downstream_http_status: Option<StatusCode>,
    failure_kind: Option<&str>,
    error_message: Option<&str>,
    downstream_error_message: Option<&str>,
    connect_latency_ms: Option<f64>,
    first_byte_latency_ms: Option<f64>,
    stream_latency_ms: Option<f64>,
    upstream_request_id: Option<&str>,
    compact_support_status: Option<&str>,
    compact_support_reason: Option<&str>,
) -> Result<()> {
    let mut snapshot = runtime_attempt_snapshot_from_pending(pending);
    snapshot.finished_at = Some(finished_at.to_string());
    snapshot.status = status.to_string();
    snapshot.phase = Some(terminal_pool_upstream_request_attempt_phase(status).to_string());
    snapshot.http_status = http_status.map(|value| i64::from(value.as_u16()));
    snapshot.downstream_http_status = downstream_http_status.map(|value| i64::from(value.as_u16()));
    snapshot.failure_kind = failure_kind.map(ToOwned::to_owned);
    snapshot.error_message = error_message.map(ToOwned::to_owned);
    snapshot.downstream_error_message = downstream_error_message.map(ToOwned::to_owned);
    snapshot.connect_latency_ms = connect_latency_ms.or(snapshot.connect_latency_ms);
    snapshot.first_byte_latency_ms = first_byte_latency_ms.or(snapshot.first_byte_latency_ms);
    snapshot.stream_latency_ms = stream_latency_ms;
    snapshot.upstream_request_id = upstream_request_id.map(ToOwned::to_owned);
    snapshot.compact_support_status = compact_support_status
        .map(ToOwned::to_owned)
        .or(snapshot.compact_support_status);
    snapshot.compact_support_reason = compact_support_reason
        .map(ToOwned::to_owned)
        .or(snapshot.compact_support_reason);
    if !upsert_runtime_attempt_with_completion_capacity(state, snapshot, true) {
        bail!("proxy runtime attempt store rejected terminal snapshot")
    }
    Ok(())
}

pub(crate) fn annotate_pool_upstream_request_attempt_model_mapping(
    pending: &mut PendingPoolAttemptRecord,
    upstream_request_model: Option<&str>,
    model_mapping_pattern: Option<&str>,
) -> Result<()> {
    pending.upstream_request_model = upstream_request_model
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);
    pending.model_mapping_pattern = model_mapping_pattern
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);
    Ok(())
}

pub(crate) async fn begin_pool_upstream_request_attempt_with_scope(
    pool: &Pool<Sqlite>,
    trace: &PoolUpstreamAttemptTraceContext,
    group_name_snapshot: Option<&str>,
    proxy_binding_key_snapshot: Option<&str>,
    upstream_account_id: i64,
    upstream_route_key: &str,
    attempt_index: i64,
    distinct_account_index: i64,
    same_account_retry_index: i64,
    started_at: &str,
) -> PendingPoolAttemptRecord {
    begin_pool_upstream_request_attempt_with_scope_and_routing_source(
        pool,
        trace,
        group_name_snapshot,
        proxy_binding_key_snapshot,
        None,
        upstream_account_id,
        upstream_route_key,
        attempt_index,
        distinct_account_index,
        same_account_retry_index,
        started_at,
    )
    .await
}

pub(crate) async fn begin_pool_upstream_request_attempt_with_scope_and_routing_source(
    pool: &Pool<Sqlite>,
    trace: &PoolUpstreamAttemptTraceContext,
    group_name_snapshot: Option<&str>,
    proxy_binding_key_snapshot: Option<&str>,
    routing_source: Option<PoolRoutingSelectionSource>,
    upstream_account_id: i64,
    upstream_route_key: &str,
    attempt_index: i64,
    distinct_account_index: i64,
    same_account_retry_index: i64,
    started_at: &str,
) -> PendingPoolAttemptRecord {
    begin_pool_upstream_request_attempt_with_scope_and_routing_source_and_audit(
        pool,
        trace,
        group_name_snapshot,
        proxy_binding_key_snapshot,
        routing_source,
        None,
        upstream_account_id,
        upstream_route_key,
        attempt_index,
        distinct_account_index,
        same_account_retry_index,
        started_at,
    )
    .await
}

pub(crate) async fn begin_pool_upstream_request_attempt_with_scope_and_routing_source_and_audit(
    pool: &Pool<Sqlite>,
    trace: &PoolUpstreamAttemptTraceContext,
    group_name_snapshot: Option<&str>,
    proxy_binding_key_snapshot: Option<&str>,
    routing_source: Option<PoolRoutingSelectionSource>,
    routing_selection_audit: Option<&PoolRoutingSelectionAudit>,
    upstream_account_id: i64,
    upstream_route_key: &str,
    attempt_index: i64,
    distinct_account_index: i64,
    same_account_retry_index: i64,
    started_at: &str,
) -> PendingPoolAttemptRecord {
    let _write_permit = crate::proxy_sqlite_write_coordinator::proxy_sqlite_write_coordinator()
        .acquire(crate::proxy_sqlite_write_coordinator::ProxySqliteWriteClass::InteractiveProxy)
        .await;
    let routing_source_value = routing_source.map(PoolRoutingSelectionSource::as_persisted_str);
    let routing_selection_audit_json =
        routing_selection_audit.and_then(|audit| serde_json::to_string(audit).ok());
    let attempt_id = match insert_pool_upstream_request_attempt_with_scope(
        pool,
        trace,
        group_name_snapshot,
        proxy_binding_key_snapshot,
        routing_source_value,
        routing_selection_audit_json.as_deref(),
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
        Ok(attempt_id) => {
            if let Err(err) = observe_model_route_seen_admitted(
                pool,
                upstream_account_id,
                trace.request_model.as_deref(),
            )
            .await
            {
                warn!(
                    account_id = upstream_account_id,
                    model = ?trace.request_model,
                    error = %err,
                    "failed to observe model route at attempt start"
                );
            }
            Some(attempt_id)
        }
        Err(err) => {
            warn!(
                invoke_id = trace.invoke_id,
                error = %err,
                "failed to persist pending pool attempt"
            );
            None
        }
    };

    if routing_source == Some(PoolRoutingSelectionSource::PriorityHandoff) {
        remember_priority_handoff_attempt(
            attempt_id,
            Some(trace.invoke_id.as_str()),
            upstream_account_id,
            trace.request_model.as_deref(),
            routing_selection_audit_json.as_deref(),
        );
    }

    let attempt_public_id = if let Some(attempt_id) = attempt_id {
        match sqlx::query_scalar::<_, Option<String>>(
            "SELECT attempt_public_id FROM pool_upstream_request_attempts WHERE id = ?1",
        )
        .bind(attempt_id)
        .fetch_optional(pool)
        .await
        {
            Ok(value) => value.flatten(),
            Err(err) => {
                warn!(
                    attempt_id,
                    error = %err,
                    "failed to load pool attempt public id after insert"
                );
                None
            }
        }
    } else {
        None
    };

    PendingPoolAttemptRecord {
        attempt_id,
        attempt_public_id,
        invoke_id: trace.invoke_id.clone(),
        occurred_at: trace.occurred_at.clone(),
        endpoint: trace.endpoint.clone(),
        sticky_key: trace.sticky_key.clone(),
        routing_source: routing_source_value.map(ToOwned::to_owned),
        routing_selection_audit_json,
        sticky_affinity_generation: None,
        requester_ip: trace.requester_ip.clone(),
        upstream_base_url_host: trace.upstream_base_url_host.clone(),
        group_name_snapshot: group_name_snapshot.map(ToOwned::to_owned),
        proxy_binding_key_snapshot: proxy_binding_key_snapshot.map(ToOwned::to_owned),
        request_model: trace.request_model.clone(),
        upstream_request_model: trace.request_model.clone(),
        model_mapping_pattern: None,
        request_summary_json: None,
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
        upstream_request_compression_algorithm: None,
        upstream_request_compression_mode: None,
        upstream_request_logical_body_bytes: None,
        upstream_request_transmitted_body_bytes: None,
        upstream_request_header_bytes_approx: None,
        upstream_response_body_bytes: None,
        upstream_response_header_bytes_approx: None,
        response_raw_path: None,
        response_raw_codec: None,
        response_raw_size: None,
        response_raw_truncated: false,
        response_raw_truncated_reason: None,
        response_content_encoding: None,
    }
}

pub(crate) fn set_pending_pool_upstream_request_attempt_response_capture(
    pending: &mut PendingPoolAttemptRecord,
    meta: &RawPayloadMeta,
    content_encoding: Option<&str>,
) {
    pending.response_raw_path = meta.path.clone();
    pending.response_raw_codec = Some(raw_payload_meta_codec(meta).to_string());
    pending.response_raw_size = Some(meta.size_bytes);
    pending.response_raw_truncated = meta.truncated;
    pending.response_raw_truncated_reason = meta.truncated_reason.clone();
    pending.response_content_encoding = content_encoding
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);
}

pub(crate) fn pool_attempt_response_capture_key(pending: &PendingPoolAttemptRecord) -> String {
    pending.attempt_public_id.clone().unwrap_or_else(|| {
        format!(
            "{}-attempt-{}-{}-{}",
            pending.invoke_id,
            pending.attempt_index,
            pending.distinct_account_index,
            pending.same_account_retry_index,
        )
    })
}

pub(crate) async fn persist_pool_upstream_request_attempt_response_capture_fallback(
    pool: &Pool<Sqlite>,
    pending: &PendingPoolAttemptRecord,
) -> Result<()> {
    // Runtime attempts are authoritative in memory until their terminal batch is
    // materialized. A capture update must not turn a rejected in-memory update
    // into a synchronous SQLite write on the request path.
    if pending.attempt_id.is_none() {
        bail!("runtime proxy attempt response capture metadata was not retained");
    }
    let _write_permit = crate::proxy_sqlite_write_coordinator::proxy_sqlite_write_coordinator()
        .acquire(crate::proxy_sqlite_write_coordinator::ProxySqliteWriteClass::InteractiveProxy)
        .await;
    let attempt_id = match pending.attempt_id {
        Some(attempt_id) => Some(attempt_id),
        None => sqlx::query_scalar::<_, Option<i64>>(
            r#"
            SELECT id
            FROM pool_upstream_request_attempts
            WHERE invoke_id = ?1
              AND occurred_at = ?2
              AND attempt_index = ?3
            ORDER BY id DESC
            LIMIT 1
            "#,
        )
        .bind(&pending.invoke_id)
        .bind(&pending.occurred_at)
        .bind(pending.attempt_index)
        .fetch_optional(pool)
        .await?
        .flatten(),
    };
    let Some(attempt_id) = attempt_id else {
        return Err(anyhow!(
            "pool attempt row was not available for response capture metadata"
        ));
    };
    let current = sqlx::query_as::<
        _,
        (
            Option<String>,
            Option<String>,
            Option<i64>,
            Option<i64>,
            Option<String>,
            Option<String>,
        ),
    >(
        r#"
        SELECT
            response_raw_path,
            response_raw_codec,
            response_raw_size,
            response_raw_truncated,
            response_raw_truncated_reason,
            response_content_encoding
        FROM pool_upstream_request_attempts
        WHERE id = ?1
        "#,
    )
    .bind(attempt_id)
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| anyhow!("pool attempt row disappeared during response capture"))?;
    let expected_truncated = i64::from(pending.response_raw_truncated);
    if current
        == (
            pending.response_raw_path.clone(),
            pending.response_raw_codec.clone(),
            pending.response_raw_size,
            Some(expected_truncated),
            pending.response_raw_truncated_reason.clone(),
            pending.response_content_encoding.clone(),
        )
    {
        return Ok(());
    }
    sqlx::query(
        r#"
        UPDATE pool_upstream_request_attempts
        SET
            response_raw_path = COALESCE(?2, response_raw_path),
            response_raw_codec = COALESCE(?3, response_raw_codec),
            response_raw_size = COALESCE(?4, response_raw_size),
            response_raw_truncated = ?5,
            response_raw_truncated_reason = ?6,
            response_content_encoding = ?7
        WHERE id = ?1
        "#,
    )
    .bind(attempt_id)
    .bind(pending.response_raw_path.as_deref())
    .bind(pending.response_raw_codec.as_deref())
    .bind(pending.response_raw_size)
    .bind(if pending.response_raw_truncated {
        1_i64
    } else {
        0_i64
    })
    .bind(pending.response_raw_truncated_reason.as_deref())
    .bind(pending.response_content_encoding.as_deref())
    .execute(pool)
    .await?;
    Ok(())
}

pub(crate) async fn persist_pool_upstream_request_attempt_response_capture_in_memory(
    state: &AppState,
    pending: &PendingPoolAttemptRecord,
) -> Result<()> {
    let attempt_public_id = pending
        .attempt_public_id
        .clone()
        .unwrap_or_else(|| pool_attempt_response_capture_key(pending));
    if state.proxy_runtime_attempts.update_response_capture(
        &pending.invoke_id,
        &pending.occurred_at,
        &attempt_public_id,
        RuntimeAttemptResponseCapture {
            raw_path: pending.response_raw_path.clone(),
            raw_codec: pending.response_raw_codec.clone(),
            raw_size: pending.response_raw_size,
            raw_truncated: pending.response_raw_truncated,
            raw_truncated_reason: pending.response_raw_truncated_reason.clone(),
            content_encoding: pending.response_content_encoding.clone(),
        },
        |candidate| {
            state
                .sqlite_batch_writer
                .reserve_runtime_attempt_completion_growth(candidate)
        },
    ) {
        Ok(())
    } else {
        bail!("proxy runtime attempt store rejected response capture metadata")
    }
}

pub(crate) async fn persist_pool_upstream_request_attempt_metadata_before_recovery(
    pool: &Pool<Sqlite>,
    pending: &PendingPoolAttemptRecord,
) -> Result<()> {
    let Some(attempt_id) = pending.attempt_id else {
        return Ok(());
    };
    let _write_permit = crate::proxy_sqlite_write_coordinator::proxy_sqlite_write_coordinator()
        .acquire(crate::proxy_sqlite_write_coordinator::ProxySqliteWriteClass::InteractiveProxy)
        .await;
    sqlx::query(
        r#"
        UPDATE pool_upstream_request_attempts
        SET
            compact_support_status = COALESCE(?2, compact_support_status),
            compact_support_reason = COALESCE(?3, compact_support_reason),
            request_model = COALESCE(?4, request_model),
            upstream_request_model = COALESCE(?5, upstream_request_model),
            model_mapping_pattern = COALESCE(?6, model_mapping_pattern),
            request_summary_json = COALESCE(?7, request_summary_json),
            upstream_request_compression_algorithm = COALESCE(?8, upstream_request_compression_algorithm),
            upstream_request_compression_mode = COALESCE(?9, upstream_request_compression_mode),
            upstream_request_logical_body_bytes = COALESCE(?10, upstream_request_logical_body_bytes),
            upstream_request_transmitted_body_bytes = COALESCE(?11, upstream_request_transmitted_body_bytes),
            upstream_request_header_bytes_approx = COALESCE(?12, upstream_request_header_bytes_approx),
            upstream_response_body_bytes = COALESCE(?13, upstream_response_body_bytes),
            upstream_response_header_bytes_approx = COALESCE(?14, upstream_response_header_bytes_approx)
        WHERE id = ?1
          AND status = ?15
          AND finished_at IS NULL
        "#,
    )
    .bind(attempt_id)
    .bind(pending.compact_support_status.as_deref())
    .bind(pending.compact_support_reason.as_deref())
    .bind(pending.request_model.as_deref())
    .bind(pending.upstream_request_model.as_deref())
    .bind(pending.model_mapping_pattern.as_deref())
    .bind(pending.request_summary_json.as_deref())
    .bind(pending.upstream_request_compression_algorithm.as_deref())
    .bind(pending.upstream_request_compression_mode.as_deref())
    .bind(pending.upstream_request_logical_body_bytes)
    .bind(pending.upstream_request_transmitted_body_bytes)
    .bind(pending.upstream_request_header_bytes_approx)
    .bind(pending.upstream_response_body_bytes)
    .bind(pending.upstream_response_header_bytes_approx)
    .bind(POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_PENDING)
    .execute(pool)
    .await?;
    Ok(())
}

pub(crate) fn update_pending_pool_upstream_request_attempt_http_bytes(
    pending: &mut PendingPoolAttemptRecord,
    logical_body_bytes: Option<usize>,
    transmitted_body_bytes: Option<usize>,
    request_header_bytes_approx: Option<usize>,
    response_body_bytes: Option<usize>,
    response_header_bytes_approx: Option<usize>,
) {
    pending.upstream_request_logical_body_bytes =
        logical_body_bytes.and_then(|value| i64::try_from(value).ok());
    pending.upstream_request_transmitted_body_bytes =
        transmitted_body_bytes.and_then(|value| i64::try_from(value).ok());
    pending.upstream_request_header_bytes_approx =
        request_header_bytes_approx.and_then(|value| i64::try_from(value).ok());
    pending.upstream_response_body_bytes =
        response_body_bytes.and_then(|value| i64::try_from(value).ok());
    pending.upstream_response_header_bytes_approx =
        response_header_bytes_approx.and_then(|value| i64::try_from(value).ok());
}

pub(crate) fn annotate_pool_upstream_request_attempt_request_compression(
    pending: &mut PendingPoolAttemptRecord,
    algorithm: &str,
    mode: &str,
) -> Result<bool> {
    pending.upstream_request_compression_algorithm = Some(algorithm.to_string());
    pending.upstream_request_compression_mode = Some(mode.to_string());
    Ok(pending.attempt_id.is_some())
}

/// Stores the bounded rewrite audit on the attempt that produced it. This keeps
/// failover timelines account-accurate instead of inheriting the final attempt's audit.
pub(crate) fn annotate_pool_upstream_request_attempt_codex_imagegen_rewrite(
    pending: &mut PendingPoolAttemptRecord,
    codex_imagegen_rewrite: Option<&Value>,
) -> Result<bool> {
    let Some(codex_imagegen_rewrite) = codex_imagegen_rewrite else {
        return Ok(false);
    };
    let mut summary = pending
        .request_summary_json
        .as_deref()
        .and_then(|raw| serde_json::from_str::<Value>(raw).ok())
        .filter(Value::is_object)
        .unwrap_or_else(|| Value::Object(Default::default()));
    let Some(summary) = summary.as_object_mut() else {
        return Ok(false);
    };
    if summary.get("codexImagegenRewrite") == Some(codex_imagegen_rewrite) {
        return Ok(false);
    }
    summary.insert(
        "codexImagegenRewrite".to_string(),
        codex_imagegen_rewrite.clone(),
    );
    pending.request_summary_json = Some(serde_json::to_string(&Value::Object(summary.clone()))?);
    Ok(pending.attempt_id.is_some())
}

pub(crate) async fn advance_pool_upstream_request_attempt_phase(
    state: &AppState,
    pending: &PendingPoolAttemptRecord,
    phase: &str,
) -> Result<()> {
    let phase_enqueued = enqueue_pool_upstream_request_attempt_progress_reliably(
        state, pending, phase, None, None, None, None,
    );
    if !phase_enqueued {
        warn!(
            invoke_id = %pending.invoke_id,
            attempt_id = pending.attempt_id,
            phase,
            "failed to retain pool attempt phase progress"
        );
    }
    Ok(())
}

pub(crate) fn enqueue_pool_upstream_request_attempt_progress(
    state: &AppState,
    pending: &PendingPoolAttemptRecord,
    phase: &str,
    connect_latency_ms: Option<f64>,
    first_byte_latency_ms: Option<f64>,
    compact_support_status: Option<&str>,
    compact_support_reason: Option<&str>,
) -> bool {
    enqueue_pool_upstream_request_attempt_progress_with_phase(
        state,
        pending,
        Some(phase),
        connect_latency_ms,
        first_byte_latency_ms,
        compact_support_status,
        compact_support_reason,
    )
}

pub(crate) fn enqueue_pool_upstream_request_attempt_snapshot(
    state: &AppState,
    pending: &PendingPoolAttemptRecord,
) -> bool {
    enqueue_pool_upstream_request_attempt_progress_with_phase(
        state,
        pending,
        None,
        (pending.connect_latency_ms > 0.0).then_some(pending.connect_latency_ms),
        (pending.first_byte_latency_ms > 0.0).then_some(pending.first_byte_latency_ms),
        pending.compact_support_status.as_deref(),
        pending.compact_support_reason.as_deref(),
    )
}

pub(crate) fn enqueue_pool_upstream_request_attempt_progress_reliably(
    state: &AppState,
    pending: &PendingPoolAttemptRecord,
    phase: &str,
    connect_latency_ms: Option<f64>,
    first_byte_latency_ms: Option<f64>,
    compact_support_status: Option<&str>,
    compact_support_reason: Option<&str>,
) -> bool {
    if pending.attempt_id.is_some() {
        let Some(progress) = build_pool_upstream_request_attempt_progress(
            pending,
            Some(phase),
            connect_latency_ms,
            first_byte_latency_ms,
            compact_support_status,
            compact_support_reason,
        ) else {
            return false;
        };
        return state
            .sqlite_batch_writer
            .enqueue_attempt_progress_reliably(progress);
    }
    update_runtime_attempt_progress(
        state,
        pending,
        Some(phase),
        connect_latency_ms,
        first_byte_latency_ms,
        compact_support_status,
        compact_support_reason,
    )
}

pub(crate) fn enqueue_pool_upstream_request_attempt_snapshot_reliably(
    state: &AppState,
    pending: &PendingPoolAttemptRecord,
) -> bool {
    if pending.attempt_id.is_some() {
        let Some(progress) = build_pool_upstream_request_attempt_progress(
            pending,
            None,
            (pending.connect_latency_ms > 0.0).then_some(pending.connect_latency_ms),
            (pending.first_byte_latency_ms > 0.0).then_some(pending.first_byte_latency_ms),
            pending.compact_support_status.as_deref(),
            pending.compact_support_reason.as_deref(),
        ) else {
            return false;
        };
        return state
            .sqlite_batch_writer
            .enqueue_attempt_progress_reliably(progress);
    }
    update_runtime_attempt_progress(
        state,
        pending,
        None,
        (pending.connect_latency_ms > 0.0).then_some(pending.connect_latency_ms),
        (pending.first_byte_latency_ms > 0.0).then_some(pending.first_byte_latency_ms),
        pending.compact_support_status.as_deref(),
        pending.compact_support_reason.as_deref(),
    )
}

pub(crate) async fn enqueue_pool_streaming_phase_and_defer_guard(
    state: &AppState,
    pending_attempt_record: &PendingPoolAttemptRecord,
    early_phase_cleanup_guard: &mut Option<PoolEarlyPhaseOrphanCleanupGuard>,
    connect_latency_ms: f64,
    first_byte_latency_ms: f64,
) -> Option<PoolEarlyPhaseOrphanCleanupGuard> {
    let mut deferred_guard = if pending_attempt_record.attempt_id.is_none() {
        early_phase_cleanup_guard.take()
    } else {
        None
    };
    let phase_enqueued = enqueue_pool_upstream_request_attempt_progress_reliably(
        state,
        pending_attempt_record,
        POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_STREAMING_RESPONSE,
        Some(connect_latency_ms),
        Some(first_byte_latency_ms),
        None,
        None,
    );
    if phase_enqueued {
        debug!(
            invoke_id = %pending_attempt_record.invoke_id,
            attempt_id = pending_attempt_record.attempt_id,
            "queued pool attempt streaming phase progress"
        );
    } else {
        info!(
            invoke_id = %pending_attempt_record.invoke_id,
            attempt_id = pending_attempt_record.attempt_id,
            "streaming phase was not enqueued; relying on invocation cleanup guards for post-first-byte recovery"
        );
    }
    if pending_attempt_record.attempt_id.is_some() {
        deferred_guard = early_phase_cleanup_guard.take();
    }
    deferred_guard
}

fn enqueue_pool_upstream_request_attempt_progress_with_phase(
    state: &AppState,
    pending: &PendingPoolAttemptRecord,
    phase: Option<&str>,
    connect_latency_ms: Option<f64>,
    first_byte_latency_ms: Option<f64>,
    compact_support_status: Option<&str>,
    compact_support_reason: Option<&str>,
) -> bool {
    if pending.attempt_id.is_some() {
        let Some(progress) = build_pool_upstream_request_attempt_progress(
            pending,
            phase,
            connect_latency_ms,
            first_byte_latency_ms,
            compact_support_status,
            compact_support_reason,
        ) else {
            return false;
        };
        return state
            .sqlite_batch_writer
            .enqueue(SqliteBatchWrite::AttemptProgress(progress));
    }
    update_runtime_attempt_progress(
        state,
        pending,
        phase,
        connect_latency_ms,
        first_byte_latency_ms,
        compact_support_status,
        compact_support_reason,
    )
}

fn update_runtime_attempt_progress(
    state: &AppState,
    pending: &PendingPoolAttemptRecord,
    phase: Option<&str>,
    connect_latency_ms: Option<f64>,
    first_byte_latency_ms: Option<f64>,
    compact_support_status: Option<&str>,
    compact_support_reason: Option<&str>,
) -> bool {
    let mut snapshot = runtime_attempt_snapshot_from_pending(pending);
    let attempt_public_id = snapshot.attempt_public_id.clone();
    let current = state.proxy_runtime_attempts.get(
        &snapshot.invoke_id,
        &snapshot.occurred_at,
        &attempt_public_id,
    );
    let terminal_snapshot = current
        .as_ref()
        .filter(|current| current.finished_at.is_some());
    if let Some(phase) = phase
        && terminal_snapshot.is_none()
    {
        snapshot.phase = Some(phase.to_string());
    }
    if terminal_snapshot.is_none() {
        if phase.is_none()
            && let Some(current) = current.as_ref()
        {
            snapshot.phase = current.phase.clone();
        }
        snapshot.connect_latency_ms = connect_latency_ms.or(snapshot.connect_latency_ms);
        snapshot.first_byte_latency_ms = first_byte_latency_ms.or(snapshot.first_byte_latency_ms);
        snapshot.compact_support_status = compact_support_status
            .map(ToOwned::to_owned)
            .or(snapshot.compact_support_status);
        snapshot.compact_support_reason = compact_support_reason
            .map(ToOwned::to_owned)
            .or(snapshot.compact_support_reason);
    } else if let Some(current) = current {
        snapshot = current;
    }
    upsert_runtime_attempt_with_completion_capacity(state, snapshot, false)
}

fn build_pool_upstream_request_attempt_progress(
    pending: &PendingPoolAttemptRecord,
    phase: Option<&str>,
    connect_latency_ms: Option<f64>,
    first_byte_latency_ms: Option<f64>,
    compact_support_status: Option<&str>,
    compact_support_reason: Option<&str>,
) -> Option<BatchedAttemptProgress> {
    let attempt_id = pending.attempt_id?;
    let response_raw_capture_present = pending.response_raw_path.is_some()
        || pending.response_raw_size.is_some()
        || pending.response_raw_truncated
        || pending.response_raw_truncated_reason.is_some()
        || pending.response_content_encoding.is_some();
    Some(BatchedAttemptProgress {
        attempt_id,
        pending_status: POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_PENDING,
        enqueue_sequence: 0,
        field_sequences: Default::default(),
        phase: phase.map(ToOwned::to_owned),
        connect_latency_ms,
        first_byte_latency_ms,
        compact_support_status: compact_support_status.map(ToOwned::to_owned),
        compact_support_reason: compact_support_reason.map(ToOwned::to_owned),
        request_model: pending.request_model.clone(),
        upstream_request_model: pending.upstream_request_model.clone(),
        model_mapping_pattern: pending.model_mapping_pattern.clone(),
        request_summary_json: pending.request_summary_json.clone(),
        upstream_request_compression_algorithm: pending
            .upstream_request_compression_algorithm
            .clone(),
        upstream_request_compression_mode: pending.upstream_request_compression_mode.clone(),
        upstream_request_logical_body_bytes: pending.upstream_request_logical_body_bytes,
        upstream_request_transmitted_body_bytes: pending.upstream_request_transmitted_body_bytes,
        upstream_request_header_bytes_approx: pending.upstream_request_header_bytes_approx,
        upstream_response_body_bytes: pending.upstream_response_body_bytes,
        upstream_response_header_bytes_approx: pending.upstream_response_header_bytes_approx,
        response_raw_path: pending.response_raw_path.clone(),
        response_raw_codec: pending.response_raw_codec.clone(),
        response_raw_size: pending.response_raw_size,
        response_raw_truncated: response_raw_capture_present
            .then_some(pending.response_raw_truncated),
        response_raw_truncated_reason: pending.response_raw_truncated_reason.clone(),
        response_content_encoding: pending.response_content_encoding.clone(),
    })
}
