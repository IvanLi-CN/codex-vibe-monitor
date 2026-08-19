use super::*;

#[cfg(test)]
fn broadcast_test_record_payload(state: &AppState, record: &ApiInvocation) {
    if state.broadcaster.receiver_count() > 0 {
        let _ = state.broadcaster.send(BroadcastPayload::Records {
            records: vec![record.clone()],
        });
    }
}

pub(crate) fn upstream_account_name_from_payload(payload: Option<&str>) -> Option<String> {
    let payload = payload?;
    let value = serde_json::from_str::<Value>(payload).ok()?;
    value
        .get("upstreamAccountName")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

pub(crate) fn prompt_cache_key_from_payload(payload: Option<&str>) -> Option<String> {
    let payload = payload?;
    let value = serde_json::from_str::<Value>(payload).ok()?;
    value
        .get("promptCacheKey")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

#[derive(Debug, Default)]
pub(crate) struct TerminalPayloadMetadata {
    pub(crate) prompt_cache_key: Option<String>,
    pub(crate) upstream_account_id: Option<i64>,
    pub(crate) request_model: Option<String>,
}

pub(crate) fn terminal_payload_metadata(payload: Option<&str>) -> TerminalPayloadMetadata {
    let Some(payload) = payload else {
        return TerminalPayloadMetadata::default();
    };
    let Ok(value) = serde_json::from_str::<Value>(payload) else {
        return TerminalPayloadMetadata::default();
    };
    let prompt_cache_key = value
        .get("promptCacheKey")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);
    let upstream_account_id = value.get("upstreamAccountId").and_then(|value| {
        value
            .as_i64()
            .or_else(|| value.as_u64().and_then(|value| i64::try_from(value).ok()))
            .or_else(|| value.as_str().and_then(|value| value.parse::<i64>().ok()))
    });
    let request_model = value
        .get("requestModel")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);
    TerminalPayloadMetadata {
        prompt_cache_key,
        upstream_account_id,
        request_model,
    }
}

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
    state: &AppState,
    trace: &PoolUpstreamAttemptTraceContext,
    prompt_cache_key: Option<&str>,
    audit: &PoolRoutingNoCandidateAudit,
) -> Result<()> {
    persist_pool_routing_no_candidate_invocation_with_error(
        state,
        trace,
        prompt_cache_key,
        audit,
        StatusCode::SERVICE_UNAVAILABLE,
        PROXY_FAILURE_POOL_NO_AVAILABLE_ACCOUNT,
        POOL_NO_AVAILABLE_ACCOUNT_MESSAGE,
        0,
        0,
        PROXY_FAILURE_POOL_NO_AVAILABLE_ACCOUNT,
    )
    .await
}

pub(crate) async fn persist_pool_routing_no_candidate_invocation_with_error(
    state: &AppState,
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
    let target = ProxyCaptureTarget::from_endpoint(&trace.endpoint);
    let mut record = build_admitted_proxy_capture_runtime_snapshot(
        &trace.invoke_id,
        &trace.occurred_at,
        target,
        trace.requester_ip.as_deref(),
        trace.sticky_key.as_deref(),
        prompt_cache_key,
    );
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
    persist_and_broadcast_proxy_capture_terminal_record(state, record).await
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

pub(crate) const POOL_UPSTREAM_REQUEST_ATTEMPT_PUBLIC_ID_LENGTH: usize = 8;
const POOL_UPSTREAM_REQUEST_ATTEMPT_PUBLIC_ID_RETRY_LIMIT: usize = 16;
const POOL_UPSTREAM_REQUEST_ATTEMPT_PUBLIC_ID_ALPHABET: [char; 58] = [
    '1', '2', '3', '4', '5', '6', '7', '8', '9', 'A', 'B', 'C', 'D', 'E', 'F', 'G', 'H', 'J', 'K',
    'L', 'M', 'N', 'P', 'Q', 'R', 'S', 'T', 'U', 'V', 'W', 'X', 'Y', 'Z', 'a', 'b', 'c', 'd', 'e',
    'f', 'g', 'h', 'i', 'j', 'k', 'm', 'n', 'o', 'p', 'q', 'r', 's', 't', 'u', 'v', 'w', 'x', 'y',
    'z',
];

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PoolAttemptPublicIdBackfillSummary {
    pub(crate) scanned: u64,
    pub(crate) updated: u64,
}

pub(crate) fn pool_upstream_request_attempt_public_id_has_alpha(value: &str) -> bool {
    value.chars().any(|char| char.is_ascii_alphabetic())
}

pub(crate) fn generate_pool_upstream_request_attempt_public_id() -> String {
    loop {
        let candidate = nanoid::nanoid!(
            POOL_UPSTREAM_REQUEST_ATTEMPT_PUBLIC_ID_LENGTH,
            &POOL_UPSTREAM_REQUEST_ATTEMPT_PUBLIC_ID_ALPHABET
        );
        if pool_upstream_request_attempt_public_id_has_alpha(&candidate) {
            return candidate;
        }
    }
}

fn is_pool_upstream_request_attempt_public_id_collision(error: &sqlx::Error) -> bool {
    let sqlx::Error::Database(database_error) = error else {
        return false;
    };
    let code_matches = database_error
        .code()
        .as_deref()
        .is_some_and(|code| code == "1555" || code == "2067");
    let message = database_error.message().to_ascii_lowercase();
    code_matches
        && (message.contains("attempt_public_id")
            || message.contains("idx_pool_upstream_request_attempts_public_id"))
}

pub(crate) async fn assign_pool_upstream_request_attempt_public_id_if_missing(
    conn: &mut SqliteConnection,
    attempt_row_id: i64,
) -> Result<bool> {
    for _ in 0..POOL_UPSTREAM_REQUEST_ATTEMPT_PUBLIC_ID_RETRY_LIMIT {
        let attempt_public_id = generate_pool_upstream_request_attempt_public_id();
        match sqlx::query(
            r#"
            UPDATE pool_upstream_request_attempts
            SET attempt_public_id = ?1
            WHERE id = ?2
              AND TRIM(COALESCE(attempt_public_id, '')) = ''
            "#,
        )
        .bind(&attempt_public_id)
        .bind(attempt_row_id)
        .execute(&mut *conn)
        .await
        {
            Ok(result) => return Ok(result.rows_affected() > 0),
            Err(error) if is_pool_upstream_request_attempt_public_id_collision(&error) => continue,
            Err(error) => return Err(error.into()),
        }
    }

    bail!(
        "failed to allocate unique attempt_public_id for pool_upstream_request_attempts row {}",
        attempt_row_id
    );
}

pub(crate) async fn backfill_pool_upstream_request_attempt_public_ids_on_connection(
    conn: &mut SqliteConnection,
    start_after_id: i64,
    scan_limit: Option<u64>,
    max_elapsed: Option<Duration>,
) -> Result<BackfillBatchOutcome<PoolAttemptPublicIdBackfillSummary>> {
    let started_at = Instant::now();
    let mut summary = PoolAttemptPublicIdBackfillSummary::default();
    let mut last_seen_id = start_after_id;
    let mut hit_budget = false;
    let mut samples = Vec::new();

    loop {
        if startup_backfill_budget_reached(started_at, summary.scanned, scan_limit, max_elapsed) {
            hit_budget = true;
            break;
        }

        let rows = sqlx::query_scalar::<_, i64>(
            r#"
            SELECT id
            FROM pool_upstream_request_attempts
            WHERE id > ?1
              AND TRIM(COALESCE(attempt_public_id, '')) = ''
            ORDER BY id ASC
            LIMIT ?2
            "#,
        )
        .bind(last_seen_id)
        .bind(startup_backfill_query_limit(summary.scanned, scan_limit))
        .fetch_all(&mut *conn)
        .await?;

        if rows.is_empty() {
            break;
        }

        if let Some(last) = rows.last() {
            last_seen_id = *last;
        }
        summary.scanned += rows.len() as u64;

        for row_id in rows {
            if assign_pool_upstream_request_attempt_public_id_if_missing(conn, row_id).await? {
                summary.updated += 1;
                push_backfill_sample(&mut samples, format!("id={row_id}"));
            }
        }
    }

    Ok(BackfillBatchOutcome {
        summary,
        next_cursor_id: last_seen_id,
        hit_budget,
        samples,
    })
}

pub(crate) async fn backfill_pool_upstream_request_attempt_public_ids_from_cursor(
    pool: &Pool<Sqlite>,
    start_after_id: i64,
    scan_limit: Option<u64>,
    max_elapsed: Option<Duration>,
) -> Result<BackfillBatchOutcome<PoolAttemptPublicIdBackfillSummary>> {
    let mut conn = pool.acquire().await?;
    backfill_pool_upstream_request_attempt_public_ids_on_connection(
        &mut conn,
        start_after_id,
        scan_limit,
        max_elapsed,
    )
    .await
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
                ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25, ?26, ?27, ?28, ?29, ?30, ?31, ?32, ?33, ?34, ?35, ?36, ?37, ?38, ?39, ?40
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

pub(crate) async fn update_pool_upstream_request_attempt_model(
    pool: &Pool<Sqlite>,
    attempt_id: Option<i64>,
    model: Option<&str>,
) -> Result<()> {
    let Some(attempt_id) = attempt_id else {
        return Ok(());
    };
    let model = model.map(str::trim).filter(|value| !value.is_empty());
    sqlx::query("UPDATE pool_upstream_request_attempts SET request_model = ?1 WHERE id = ?2")
        .bind(model)
        .bind(attempt_id)
        .execute(pool)
        .await?;
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
            if let Err(err) =
                observe_model_route_seen(pool, upstream_account_id, trace.request_model.as_deref())
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
        requester_ip: trace.requester_ip.clone(),
        upstream_base_url_host: trace.upstream_base_url_host.clone(),
        group_name_snapshot: group_name_snapshot.map(ToOwned::to_owned),
        proxy_binding_key_snapshot: proxy_binding_key_snapshot.map(ToOwned::to_owned),
        request_model: trace.request_model.clone(),
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

pub(crate) async fn persist_pool_upstream_request_attempt_response_capture(
    pool: &Pool<Sqlite>,
    pending: &PendingPoolAttemptRecord,
) -> Result<()> {
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
        return Ok(());
    };
    sqlx::query(
        r#"
        UPDATE pool_upstream_request_attempts
        SET
            response_raw_path = ?2,
            response_raw_codec = COALESCE(?3, response_raw_codec),
            response_raw_size = ?4,
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

pub(crate) async fn annotate_pool_upstream_request_attempt_request_compression(
    pool: &Pool<Sqlite>,
    pending: &mut PendingPoolAttemptRecord,
    algorithm: &str,
    mode: &str,
) -> Result<bool> {
    pending.upstream_request_compression_algorithm = Some(algorithm.to_string());
    pending.upstream_request_compression_mode = Some(mode.to_string());

    let Some(attempt_id) = pending.attempt_id else {
        return Ok(false);
    };

    let result = sqlx::query(
        r#"
        UPDATE pool_upstream_request_attempts
        SET
            upstream_request_compression_algorithm = ?2,
            upstream_request_compression_mode = ?3
        WHERE id = ?1
          AND (
                COALESCE(upstream_request_compression_algorithm, '') <> ?2
                OR COALESCE(upstream_request_compression_mode, '') <> ?3
              )
        "#,
    )
    .bind(attempt_id)
    .bind(algorithm)
    .bind(mode)
    .execute(pool)
    .await?;

    Ok(result.rows_affected() > 0)
}

/// Stores the bounded rewrite audit on the attempt that produced it. This keeps
/// failover timelines account-accurate instead of inheriting the final attempt's audit.
pub(crate) async fn annotate_pool_upstream_request_attempt_codex_imagegen_rewrite(
    pool: &Pool<Sqlite>,
    pending: &PendingPoolAttemptRecord,
    codex_imagegen_rewrite: Option<&Value>,
) -> Result<bool> {
    let Some(codex_imagegen_rewrite) = codex_imagegen_rewrite else {
        return Ok(false);
    };
    let Some(attempt_id) = pending.attempt_id else {
        return Ok(false);
    };

    let existing = sqlx::query_scalar::<_, Option<String>>(
        "SELECT request_summary_json FROM pool_upstream_request_attempts WHERE id = ?1",
    )
    .bind(attempt_id)
    .fetch_optional(pool)
    .await?
    .flatten();
    let mut summary = existing
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
    let request_summary_json = serde_json::to_string(&Value::Object(summary.clone()))?;
    let result = sqlx::query(
        "UPDATE pool_upstream_request_attempts SET request_summary_json = ?2 WHERE id = ?1",
    )
    .bind(attempt_id)
    .bind(request_summary_json)
    .execute(pool)
    .await?;

    Ok(result.rows_affected() > 0)
}

pub(crate) async fn update_pool_upstream_request_attempt_phase(
    pool: &Pool<Sqlite>,
    pending: &PendingPoolAttemptRecord,
    phase: &str,
) -> Result<bool> {
    update_pool_upstream_request_attempt_progress(pool, pending, phase, None, None, None, None)
        .await
}

pub(crate) async fn update_pool_upstream_request_attempt_progress(
    pool: &Pool<Sqlite>,
    pending: &PendingPoolAttemptRecord,
    phase: &str,
    connect_latency_ms: Option<f64>,
    first_byte_latency_ms: Option<f64>,
    compact_support_status: Option<&str>,
    compact_support_reason: Option<&str>,
) -> Result<bool> {
    let _write_permit = crate::proxy_sqlite_write_coordinator::proxy_sqlite_write_coordinator()
        .acquire(crate::proxy_sqlite_write_coordinator::ProxySqliteWriteClass::InteractiveProxy)
        .await;
    let Some(attempt_id) = pending.attempt_id else {
        return Ok(false);
    };

    let result = sqlx::query(
        r#"
        UPDATE pool_upstream_request_attempts
        SET
            phase = ?2,
            connect_latency_ms = CASE
                WHEN ?4 IS NULL THEN connect_latency_ms
                WHEN connect_latency_ms IS NULL OR connect_latency_ms < ?4 THEN ?4
                ELSE connect_latency_ms
            END,
            first_byte_latency_ms = CASE
                WHEN ?5 IS NULL THEN first_byte_latency_ms
                WHEN first_byte_latency_ms IS NULL OR first_byte_latency_ms < ?5 THEN ?5
                ELSE first_byte_latency_ms
            END,
            compact_support_status = COALESCE(?6, compact_support_status),
            compact_support_reason = COALESCE(?7, compact_support_reason)
        WHERE id = ?1
          AND status = ?3
          AND finished_at IS NULL
          AND (
                COALESCE(phase, '') <> ?2
                OR (?4 IS NOT NULL AND (connect_latency_ms IS NULL OR connect_latency_ms < ?4))
                OR (?5 IS NOT NULL AND (first_byte_latency_ms IS NULL OR first_byte_latency_ms < ?5))
                OR (?6 IS NOT NULL AND COALESCE(compact_support_status, '') <> ?6)
                OR (?7 IS NOT NULL AND COALESCE(compact_support_reason, '') <> ?7)
              )
        "#,
    )
    .bind(attempt_id)
    .bind(phase)
    .bind(POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_PENDING)
    .bind(connect_latency_ms)
    .bind(first_byte_latency_ms)
    .bind(compact_support_status)
    .bind(compact_support_reason)
    .execute(pool)
    .await?;

    Ok(result.rows_affected() > 0)
}

pub(crate) async fn persist_pool_upstream_request_attempt_first_byte_progress(
    pool: &Pool<Sqlite>,
    pending: &PendingPoolAttemptRecord,
    connect_latency_ms: f64,
    first_byte_latency_ms: f64,
) -> Result<bool> {
    let _write_permit = crate::proxy_sqlite_write_coordinator::proxy_sqlite_write_coordinator()
        .acquire(crate::proxy_sqlite_write_coordinator::ProxySqliteWriteClass::InteractiveProxy)
        .await;
    let Some(attempt_id) = pending.attempt_id else {
        return Ok(false);
    };

    let result = sqlx::query(
        r#"
        UPDATE pool_upstream_request_attempts
        SET
            connect_latency_ms = CASE
                WHEN connect_latency_ms IS NULL OR connect_latency_ms < ?2 THEN ?2
                ELSE connect_latency_ms
            END,
            first_byte_latency_ms = CASE
                WHEN first_byte_latency_ms IS NULL OR first_byte_latency_ms < ?3 THEN ?3
                ELSE first_byte_latency_ms
            END
        WHERE id = ?1
        "#,
    )
    .bind(attempt_id)
    .bind(connect_latency_ms)
    .bind(first_byte_latency_ms)
    .execute(pool)
    .await?;

    Ok(result.rows_affected() > 0)
}

pub(crate) async fn advance_pool_upstream_request_attempt_phase(
    state: &AppState,
    pending: &PendingPoolAttemptRecord,
    phase: &str,
) -> Result<()> {
    enqueue_pool_upstream_request_attempt_progress(state, pending, phase, None, None, None, None);
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
    let Some(attempt_id) = pending.attempt_id else {
        return false;
    };
    state
        .sqlite_batch_writer
        .enqueue(SqliteBatchWrite::AttemptProgress(BatchedAttemptProgress {
            attempt_id,
            pending_status: POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_PENDING,
            phase: phase.to_string(),
            connect_latency_ms,
            first_byte_latency_ms,
            compact_support_status: compact_support_status.map(ToOwned::to_owned),
            compact_support_reason: compact_support_reason.map(ToOwned::to_owned),
        }))
}

pub(crate) enum PoolAttemptRecoveryScope<'a> {
    AllPending,
    SpecificEarlyPhase {
        attempt_id: i64,
    },
    StaleEarlyPhase {
        responses_started_before: &'a str,
        compact_started_before: &'a str,
        default_started_before: &'a str,
    },
}

pub(crate) async fn recover_pool_upstream_request_attempts_with_scope(
    pool: &Pool<Sqlite>,
    scope: PoolAttemptRecoveryScope<'_>,
) -> Result<Vec<RecoveredPoolAttemptRow>> {
    let mut tx = pool.begin().await?;
    let recovered =
        recover_pool_upstream_request_attempts_with_scope_tx(tx.as_mut(), scope).await?;
    tx.commit().await?;
    Ok(recovered)
}

pub(crate) async fn recover_pool_upstream_request_attempts_with_scope_tx(
    tx: &mut SqliteConnection,
    scope: PoolAttemptRecoveryScope<'_>,
) -> Result<Vec<RecoveredPoolAttemptRow>> {
    let finished_at = shanghai_now_string();
    let recovered = match scope {
        PoolAttemptRecoveryScope::AllPending => {
            sqlx::query_as::<_, RecoveredPoolAttemptRow>(
                r#"
                UPDATE pool_upstream_request_attempts
                SET
                    finished_at = COALESCE(finished_at, ?1),
                    status = ?2,
                    phase = ?3,
                    failure_kind = COALESCE(failure_kind, ?4),
                    error_message = COALESCE(error_message, ?5)
                WHERE status = ?6
                  AND finished_at IS NULL
                RETURNING id, invoke_id, occurred_at, sticky_key, upstream_account_id
                "#,
            )
            .bind(finished_at)
            .bind(POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_TRANSPORT_FAILURE)
            .bind(POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_FAILED)
            .bind(PROXY_FAILURE_POOL_ATTEMPT_INTERRUPTED)
            .bind(POOL_ATTEMPT_INTERRUPTED_MESSAGE)
            .bind(POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_PENDING)
            .fetch_all(&mut *tx)
            .await?
        }
        PoolAttemptRecoveryScope::SpecificEarlyPhase { attempt_id } => {
            sqlx::query_as::<_, RecoveredPoolAttemptRow>(
                r#"
                UPDATE pool_upstream_request_attempts
                SET
                    finished_at = COALESCE(finished_at, ?1),
                    status = ?2,
                    phase = ?3,
                    failure_kind = COALESCE(failure_kind, ?4),
                    error_message = COALESCE(error_message, ?5)
                WHERE id = ?6
                  AND status = ?7
                  AND finished_at IS NULL
                  AND LOWER(TRIM(COALESCE(phase, ''))) IN ('connecting', 'sending_request', 'waiting_first_byte')
                RETURNING id, invoke_id, occurred_at, sticky_key, upstream_account_id
                "#,
            )
            .bind(finished_at)
            .bind(POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_TRANSPORT_FAILURE)
            .bind(POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_FAILED)
            .bind(PROXY_FAILURE_POOL_ATTEMPT_INTERRUPTED)
            .bind(POOL_ATTEMPT_INTERRUPTED_MESSAGE)
            .bind(attempt_id)
            .bind(POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_PENDING)
            .fetch_all(&mut *tx)
            .await?
        }
        PoolAttemptRecoveryScope::StaleEarlyPhase {
            responses_started_before,
            compact_started_before,
            default_started_before,
        } => {
            let candidates = load_stale_pool_upstream_request_attempt_candidate_rows_tx(
                tx,
                responses_started_before,
                compact_started_before,
                default_started_before,
            )
            .await?;
            if candidates.is_empty() {
                Vec::new()
            } else {
                let candidate_ids = candidates.iter().map(|row| row.id).collect::<Vec<_>>();
                recover_stale_pool_upstream_request_attempt_candidates_tx(
                    tx,
                    &candidate_ids,
                    &finished_at,
                    responses_started_before,
                    compact_started_before,
                    default_started_before,
                )
                .await?
            }
        }
    };

    Ok(recovered)
}

pub(crate) async fn load_stale_pool_upstream_request_attempt_candidate_rows_tx(
    tx: &mut SqliteConnection,
    responses_started_before: &str,
    compact_started_before: &str,
    default_started_before: &str,
) -> Result<Vec<RecoveredPoolAttemptRow>> {
    sqlx::query_as::<_, RecoveredPoolAttemptRow>(
        r#"
        SELECT id, invoke_id, occurred_at, sticky_key, upstream_account_id
        FROM pool_upstream_request_attempts
        WHERE status = ?1
          AND finished_at IS NULL
          AND LOWER(TRIM(COALESCE(phase, ''))) IN ('connecting', 'sending_request', 'waiting_first_byte')
          AND COALESCE(first_byte_latency_ms, 0) <= 0
          AND NOT EXISTS (
                SELECT 1
                FROM codex_invocations inv
                WHERE inv.source = ?2
                  AND inv.invoke_id = pool_upstream_request_attempts.invoke_id
                  AND inv.occurred_at = pool_upstream_request_attempts.occurred_at
                  AND COALESCE(inv.t_upstream_ttfb_ms, 0) > 0
          )
          AND (
                started_at IS NULL
                OR (
                    endpoint = '/v1/responses'
                    AND started_at <= ?3
                )
                OR (
                    endpoint = '/v1/responses/compact'
                    AND started_at <= ?4
                )
                OR (
                    COALESCE(endpoint, '') NOT IN ('/v1/responses', '/v1/responses/compact')
                    AND started_at <= ?5
                )
          )
        "#,
    )
    .bind(POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_PENDING)
    .bind(SOURCE_PROXY)
    .bind(responses_started_before)
    .bind(compact_started_before)
    .bind(default_started_before)
    .fetch_all(&mut *tx)
    .await
    .map_err(Into::into)
}

pub(crate) async fn recover_stale_pool_upstream_request_attempt_candidates_tx(
    tx: &mut SqliteConnection,
    candidate_ids: &[i64],
    finished_at: &str,
    responses_started_before: &str,
    compact_started_before: &str,
    default_started_before: &str,
) -> Result<Vec<RecoveredPoolAttemptRow>> {
    if candidate_ids.is_empty() {
        return Ok(Vec::new());
    }

    let mut recovered = Vec::new();
    for chunk in candidate_ids.chunks(POOL_ATTEMPT_RECOVERY_SELECTOR_BATCH_SIZE) {
        let mut query = QueryBuilder::<Sqlite>::new(
            r#"
            UPDATE pool_upstream_request_attempts
            SET
                finished_at = COALESCE(finished_at, "#,
        );
        query.push_bind(finished_at);
        query.push(
            r#"),
                status = "#,
        );
        query.push_bind(POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_TRANSPORT_FAILURE);
        query.push(
            r#",
                phase = "#,
        );
        query.push_bind(POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_FAILED);
        query.push(
            r#",
                failure_kind = COALESCE(failure_kind, "#,
        );
        query.push_bind(PROXY_FAILURE_POOL_ATTEMPT_INTERRUPTED);
        query.push(
            r#"),
                error_message = COALESCE(error_message, "#,
        );
        query.push_bind(POOL_ATTEMPT_INTERRUPTED_MESSAGE);
        query.push(
            r#")
            WHERE id IN ("#,
        );
        let mut separated = query.separated(", ");
        for id in chunk {
            separated.push_bind(id);
        }
        separated.push_unseparated(")");
        query.push(
            r#"
              AND status = "#,
        );
        query.push_bind(POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_PENDING);
        query.push(
            r#"
              AND finished_at IS NULL
              AND LOWER(TRIM(COALESCE(phase, ''))) IN ('connecting', 'sending_request', 'waiting_first_byte')
              AND COALESCE(first_byte_latency_ms, 0) <= 0
              AND NOT EXISTS (
                    SELECT 1
                    FROM codex_invocations inv
                    WHERE inv.source = "#,
        );
        query.push_bind(SOURCE_PROXY);
        query.push(
            r#"
                      AND inv.invoke_id = pool_upstream_request_attempts.invoke_id
                      AND inv.occurred_at = pool_upstream_request_attempts.occurred_at
                      AND COALESCE(inv.t_upstream_ttfb_ms, 0) > 0
              )
              AND (
                    started_at IS NULL
                    OR (
                        endpoint = '/v1/responses'
                        AND started_at <= "#,
        );
        query.push_bind(responses_started_before);
        query.push(
            r#"
                    )
                    OR (
                        endpoint = '/v1/responses/compact'
                        AND started_at <= "#,
        );
        query.push_bind(compact_started_before);
        query.push(
            r#"
                    )
                    OR (
                        COALESCE(endpoint, '') NOT IN ('/v1/responses', '/v1/responses/compact')
                        AND started_at <= "#,
        );
        query.push_bind(default_started_before);
        query.push(
            r#"
                    )
              )
            RETURNING id, invoke_id, occurred_at, sticky_key, upstream_account_id
            "#,
        );
        recovered.extend(
            query
                .build_query_as::<RecoveredPoolAttemptRow>()
                .fetch_all(&mut *tx)
                .await?,
        );
    }

    Ok(recovered)
}

#[cfg(test)]
pub(crate) async fn recover_stale_pool_upstream_request_attempt_candidates(
    pool: &Pool<Sqlite>,
    candidate_ids: &[i64],
    finished_at: &str,
    responses_started_before: &str,
    compact_started_before: &str,
    default_started_before: &str,
) -> Result<Vec<RecoveredPoolAttemptRow>> {
    let mut tx = pool.begin().await?;
    let recovered = recover_stale_pool_upstream_request_attempt_candidates_tx(
        tx.as_mut(),
        candidate_ids,
        finished_at,
        responses_started_before,
        compact_started_before,
        default_started_before,
    )
    .await?;
    tx.commit().await?;
    Ok(recovered)
}

pub(crate) async fn recover_orphaned_pool_upstream_request_attempts(
    pool: &Pool<Sqlite>,
) -> Result<u64> {
    Ok(
        recover_pool_upstream_request_attempts_with_scope(
            pool,
            PoolAttemptRecoveryScope::AllPending,
        )
        .await?
        .len() as u64,
    )
}

pub(crate) enum ProxyInvocationRecoveryScope<'a> {
    AllInFlight,
    Selectors(&'a [InvocationRecoverySelector]),
}

pub(crate) async fn recover_proxy_invocations_with_scope(
    pool: &Pool<Sqlite>,
    scope: ProxyInvocationRecoveryScope<'_>,
) -> Result<Vec<RecoveredInvocationRow>> {
    let mut tx = pool.begin().await?;
    let rows = recover_proxy_invocations_with_scope_tx(tx.as_mut(), scope).await?;
    tx.commit().await?;
    Ok(rows)
}

pub(crate) async fn recover_proxy_invocations_with_scope_tx(
    tx: &mut SqliteConnection,
    scope: ProxyInvocationRecoveryScope<'_>,
) -> Result<Vec<RecoveredInvocationRow>> {
    let rows = match scope {
        ProxyInvocationRecoveryScope::AllInFlight => {
            sqlx::query_as::<_, RecoveredInvocationRow>(
                r#"
                UPDATE codex_invocations
                SET status = ?1,
                    error_message = ?2,
                    failure_kind = ?3,
                    failure_class = ?4,
                    is_actionable = 1
                WHERE source = ?5
                  AND LOWER(TRIM(COALESCE(status, ''))) IN ('running', 'pending')
                RETURNING id, invoke_id, occurred_at
                "#,
            )
            .bind(INVOCATION_STATUS_INTERRUPTED)
            .bind(INVOCATION_INTERRUPTED_MESSAGE)
            .bind(PROXY_FAILURE_INVOCATION_INTERRUPTED)
            .bind(FAILURE_CLASS_SERVICE)
            .bind(SOURCE_PROXY)
            .fetch_all(&mut *tx)
            .await?
        }
        ProxyInvocationRecoveryScope::Selectors(selectors) => {
            let selectors: Vec<_> = selectors
                .iter()
                .cloned()
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect();
            if selectors.is_empty() {
                return Ok(Vec::new());
            }

            let mut recovered = Vec::new();
            for chunk in selectors.chunks(PROXY_INVOCATION_RECOVERY_SELECTOR_BATCH_SIZE) {
                let mut query = QueryBuilder::<Sqlite>::new(
                    r#"
                    UPDATE codex_invocations
                    SET status = "#,
                );
                query.push_bind(INVOCATION_STATUS_INTERRUPTED);
                query.push(
                    r#",
                        error_message = "#,
                );
                query.push_bind(INVOCATION_INTERRUPTED_MESSAGE);
                query.push(
                    r#",
                        failure_kind = "#,
                );
                query.push_bind(PROXY_FAILURE_INVOCATION_INTERRUPTED);
                query.push(
                    r#",
                        failure_class = "#,
                );
                query.push_bind(FAILURE_CLASS_SERVICE);
                query.push(
                    r#",
                        is_actionable = 1
                    WHERE source = "#,
                );
                query.push_bind(SOURCE_PROXY);
                query.push(
                    r#"
                      AND LOWER(TRIM(COALESCE(status, ''))) IN ('running', 'pending')
                      AND (
                    "#,
                );
                let mut first = true;
                for selector in chunk {
                    if !first {
                        query.push(" OR ");
                    }
                    first = false;
                    query.push("(");
                    query.push("invoke_id = ");
                    query.push_bind(&selector.invoke_id);
                    query.push(" AND occurred_at = ");
                    query.push_bind(&selector.occurred_at);
                    query.push(")");
                }
                query.push(
                    r#"
                      )
                    RETURNING id, invoke_id, occurred_at
                    "#,
                );
                recovered.extend(
                    query
                        .build_query_as::<RecoveredInvocationRow>()
                        .fetch_all(&mut *tx)
                        .await?,
                );
            }
            recovered
        }
    };

    if !rows.is_empty() {
        let updated_ids: Vec<i64> = rows.iter().map(|row| row.id).collect();
        recompute_invocation_hourly_rollups_for_ids_tx(&mut *tx, &updated_ids).await?;
        if let Some(max_id) = updated_ids.iter().copied().max() {
            save_hourly_rollup_live_progress_tx(
                &mut *tx,
                HOURLY_ROLLUP_DATASET_INVOCATIONS,
                max_id,
            )
            .await?;
        }
    }

    Ok(rows)
}

pub(crate) async fn recover_orphaned_proxy_invocations(pool: &Pool<Sqlite>) -> Result<u64> {
    Ok(
        recover_proxy_invocations_with_scope(pool, ProxyInvocationRecoveryScope::AllInFlight)
            .await?
            .len() as u64,
    )
}

pub(crate) fn stale_started_before_string(timeout: Duration, grace: Duration) -> String {
    let cutoff = Utc::now().with_timezone(&Shanghai).naive_local()
        - ChronoDuration::from_std(timeout + grace)
            .expect("pool orphan recovery cutoff should fit chrono duration");
    format_naive(cutoff)
}

pub(crate) async fn load_persisted_api_invocation(
    pool: &Pool<Sqlite>,
    invoke_id: &str,
    occurred_at: &str,
) -> Result<ApiInvocation> {
    let mut tx = pool.begin().await?;
    let invocation = load_persisted_api_invocation_tx(tx.as_mut(), invoke_id, occurred_at).await?;
    tx.commit().await?;
    Ok(invocation)
}

pub(crate) async fn broadcast_recovered_proxy_invocations(
    state: &AppState,
    recovered: &[RecoveredInvocationRow],
) -> Result<()> {
    if recovered.is_empty() {
        return Ok(());
    }

    let selectors: Vec<_> = recovered
        .iter()
        .map(|row| InvocationRecoverySelector::new(row.invoke_id.clone(), row.occurred_at.clone()))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let mut records = Vec::new();
    for selector in selectors {
        match load_persisted_api_invocation(&state.pool, &selector.invoke_id, &selector.occurred_at)
            .await
        {
            Ok(record) => records.push(record),
            Err(err) => {
                warn!(
                    invoke_id = %selector.invoke_id,
                    occurred_at = %selector.occurred_at,
                    error = %err,
                    "failed to load recovered proxy invocation for runtime broadcast"
                );
            }
        }
    }

    if records.is_empty() {
        return Ok(());
    }

    invalidate_dashboard_activity_baselines_for_recovery(state).await;

    for record in &records {
        let delta = apply_dashboard_activity_terminal_record(state, record).await;
        debug!(
            invoke_id = %record.invoke_id,
            terminal_delta_applied_selection_count = delta.applied_selection_count,
            terminal_delta_duplicate = delta.duplicate,
            response_source = "memory",
            "applied recovered terminal record to dashboard activity read model"
        );
    }

    if records.iter().any(|record| {
        record
            .prompt_cache_key
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty())
    }) {
        invalidate_prompt_cache_conversations_cache(&state.prompt_cache_conversation_cache).await;
    }

    let summary_invoke_id = records[0].invoke_id.clone();
    for record in &records {
        state
            .subscription_hub
            .publish_runtime_mutation(RuntimeMutation::invocation(
                record,
                RuntimeMutationKind::Recovery,
            ));
        #[cfg(test)]
        broadcast_test_record_payload(state, record);
    }
    schedule_dashboard_activity_live_snapshot(state);
    schedule_proxy_capture_follow_up_worker(state, &summary_invoke_id).await?;

    Ok(())
}

pub(crate) fn pool_routing_reservation_key_for_invoke_id(invoke_id: &str) -> Option<String> {
    let request_id = invoke_id
        .strip_prefix("proxy-")
        .or_else(|| invoke_id.strip_prefix("pool-ws-"))?
        .split('-')
        .next()?;
    request_id
        .parse::<u64>()
        .ok()
        .map(build_pool_routing_reservation_key)
}

pub(crate) async fn observe_proxy_cache_hit_if_success(
    state: &AppState,
    record: &ProxyCaptureRecord,
) -> Result<ModelRouteCacheObservationOutcome> {
    if record.status != "success" {
        return Ok(ModelRouteCacheObservationOutcome::default());
    }
    let metadata = terminal_payload_metadata(record.payload.as_deref());
    let Some(account_id) = metadata.upstream_account_id else {
        return Ok(ModelRouteCacheObservationOutcome::default());
    };
    let model = metadata
        .request_model
        .as_deref()
        .or(record.model.as_deref())
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let Some(model) = model else {
        return Ok(ModelRouteCacheObservationOutcome::default());
    };
    let reservation_held = pool_routing_reservation_key_for_invoke_id(&record.invoke_id)
        .is_some_and(|reservation_key| {
            pool_routing_reservation_matches_model(state, &reservation_key, account_id, Some(model))
        });
    let active_concurrency = pool_routing_model_reservation_count(state, account_id, Some(model))
        + if reservation_held { 0 } else { 1 };
    observe_model_route_cache_hit(
        &state.pool,
        account_id,
        Some(model),
        record.usage.input_tokens,
        record.usage.cache_input_tokens,
        active_concurrency,
    )
    .await
}

pub(crate) fn pool_route_orphan_recovery_failure_message(recovery_trigger: &str) -> String {
    format!("pool request was interrupted before completion and recovered via {recovery_trigger}")
}

pub(crate) async fn clean_up_pool_route_after_orphan_recovery(
    state: &AppState,
    invoke_id: &str,
    sticky_key: Option<&str>,
    upstream_account_id: Option<i64>,
    recovery_trigger: &'static str,
    record_route_failure: bool,
) {
    let reservation_key = pool_routing_reservation_key_for_invoke_id(invoke_id);
    let mut reservation_released_after_failure = false;
    if record_route_failure && let Some(account_id) = upstream_account_id {
        let error_message = pool_route_orphan_recovery_failure_message(recovery_trigger);
        let result = if let Some(reservation_key) = reservation_key.as_deref() {
            reservation_released_after_failure = true;
            persist_pool_route_failure_then_release(
                state,
                reservation_key,
                record_pool_route_transport_failure(
                    &state.pool,
                    account_id,
                    sticky_key,
                    &error_message,
                    Some(invoke_id),
                ),
            )
            .await
        } else {
            record_pool_route_transport_failure(
                &state.pool,
                account_id,
                sticky_key,
                &error_message,
                Some(invoke_id),
            )
            .await
        };
        if let Err(err) = result {
            warn!(
                invoke_id,
                account_id,
                recovery_trigger,
                error = %err,
                "failed to record pool route transport failure during orphan recovery cleanup"
            );
        }
    }

    if !reservation_released_after_failure && let Some(reservation_key) = reservation_key {
        release_pool_routing_reservation(state, &reservation_key);
    }
}

pub(crate) async fn should_record_route_failure_after_attempt_recovery(
    state: &AppState,
    invoke_id: &str,
    occurred_at: &str,
    recovered_invocation: bool,
) -> bool {
    if state
        .proxy_runtime_invocations
        .contains_terminal(invoke_id, occurred_at)
    {
        debug!(
            invoke_id,
            occurred_at,
            "skipping route failure cleanup because terminal runtime overlay already exists"
        );
        return false;
    }

    if recovered_invocation {
        return true;
    }

    let latest_status = sqlx::query_scalar::<_, String>(
        r#"
        SELECT status
        FROM codex_invocations
        WHERE invoke_id = ?1 AND occurred_at = ?2
        ORDER BY id DESC
        LIMIT 1
        "#,
    )
    .bind(invoke_id)
    .bind(occurred_at)
    .fetch_optional(&state.pool)
    .await;

    match latest_status {
        Ok(Some(status)) => matches!(
            status.as_str(),
            INVOCATION_STATUS_RUNNING | INVOCATION_STATUS_PENDING
        ),
        Ok(None) => true,
        Err(err) => {
            warn!(
                invoke_id,
                occurred_at,
                error = %err,
                "failed to inspect invocation terminal state during pool orphan cleanup"
            );
            true
        }
    }
}

#[cfg(test)]
pub(crate) async fn should_record_route_failure_after_attempt_recovery_for_test(
    state: &AppState,
    invoke_id: &str,
    occurred_at: &str,
    recovered_invocation: bool,
) -> bool {
    should_record_route_failure_after_attempt_recovery(
        state,
        invoke_id,
        occurred_at,
        recovered_invocation,
    )
    .await
}

pub(crate) async fn clean_up_recovered_pool_routes(
    state: &AppState,
    recovered_attempts: &[RecoveredPoolAttemptRow],
    recovered_invocations: &[RecoveredInvocationRow],
    recovery_trigger: &'static str,
) {
    let recovered_invocation_keys = recovered_invocations
        .iter()
        .map(|row| (row.invoke_id.as_str(), row.occurred_at.as_str()))
        .collect::<BTreeSet<_>>();
    for row in recovered_attempts {
        let recovered_invocation =
            recovered_invocation_keys.contains(&(row.invoke_id.as_str(), row.occurred_at.as_str()));
        let record_route_failure = should_record_route_failure_after_attempt_recovery(
            state,
            &row.invoke_id,
            &row.occurred_at,
            recovered_invocation,
        )
        .await;
        clean_up_pool_route_after_orphan_recovery(
            state,
            &row.invoke_id,
            row.sticky_key.as_deref(),
            row.upstream_account_id,
            recovery_trigger,
            record_route_failure,
        )
        .await;
    }
}

pub(crate) async fn recover_guard_dropped_pool_early_phase_orphan(
    state: &AppState,
    pending_attempt_record: PendingPoolAttemptRecord,
    first_byte_observed: bool,
    terminal_outcome_observed: bool,
) -> Result<()> {
    state.sqlite_batch_writer.flush_now(&state.pool).await?;

    if first_byte_observed && terminal_outcome_observed {
        info!(
            invoke_id = %pending_attempt_record.invoke_id,
            attempt_id = pending_attempt_record.attempt_id,
            first_byte_latency_ms = pending_attempt_record.first_byte_latency_ms,
            recovery_trigger = "drop_guard",
            "skipping guard-based orphan recovery because a terminal post-first-byte outcome was already observed"
        );
        return Ok(());
    }
    if first_byte_observed {
        info!(
            invoke_id = %pending_attempt_record.invoke_id,
            attempt_id = pending_attempt_record.attempt_id,
            first_byte_latency_ms = pending_attempt_record.first_byte_latency_ms,
            recovery_trigger = "drop_guard",
            "recovering post-first-byte orphan because the stream task ended before any terminal outcome was observed"
        );
    }

    let dashboard_reconcile_gate = state.sqlite_batch_writer.dashboard_reconcile_gate();
    let _dashboard_reconcile_guard = dashboard_reconcile_gate.lock().await;
    let mut tx = state.pool.begin().await?;
    let recovered_attempts = match pending_attempt_record.attempt_id {
        Some(attempt_id) => {
            recover_pool_upstream_request_attempts_with_scope_tx(
                tx.as_mut(),
                PoolAttemptRecoveryScope::SpecificEarlyPhase { attempt_id },
            )
            .await?
        }
        None => Vec::new(),
    };

    let recovered_invocations =
        if pending_attempt_record.attempt_id.is_none() || !recovered_attempts.is_empty() {
            let selector = InvocationRecoverySelector::from(&pending_attempt_record);
            recover_proxy_invocations_with_scope_tx(
                tx.as_mut(),
                ProxyInvocationRecoveryScope::Selectors(std::slice::from_ref(&selector)),
            )
            .await?
        } else {
            Vec::new()
        };
    tx.commit().await?;

    let should_clean_up_route = pending_attempt_record.attempt_id.is_none()
        || !recovered_attempts.is_empty()
        || !recovered_invocations.is_empty();
    let record_route_failure = (pending_attempt_record.attempt_id.is_none()
        || !recovered_attempts.is_empty())
        && should_record_route_failure_after_attempt_recovery(
            state,
            &pending_attempt_record.invoke_id,
            &pending_attempt_record.occurred_at,
            !recovered_invocations.is_empty(),
        )
        .await;

    if recovered_invocations.is_empty() {
        terminalize_proxy_runtime_snapshot_by_key(
            state,
            &pending_attempt_record.invoke_id,
            &pending_attempt_record.occurred_at,
            "drop_guard",
        );
        schedule_dashboard_activity_live_snapshot(state);
    } else {
        remove_proxy_runtime_snapshot_by_key(
            state,
            &pending_attempt_record.invoke_id,
            &pending_attempt_record.occurred_at,
            "drop_guard",
        );
        schedule_dashboard_activity_live_snapshot(state);
    }

    if should_clean_up_route {
        clean_up_pool_route_after_orphan_recovery(
            state,
            &pending_attempt_record.invoke_id,
            pending_attempt_record.sticky_key.as_deref(),
            Some(pending_attempt_record.upstream_account_id),
            "drop_guard",
            record_route_failure,
        )
        .await;
    }

    if recovered_attempts.is_empty() && recovered_invocations.is_empty() {
        return Ok(());
    }

    if !recovered_attempts.is_empty()
        && let Err(err) =
            broadcast_pool_upstream_attempts_snapshot(state, &pending_attempt_record.invoke_id)
                .await
    {
        warn!(
            invoke_id = %pending_attempt_record.invoke_id,
            error = %err,
            "failed to broadcast guard-recovered pool attempt snapshot"
        );
    }
    broadcast_recovered_proxy_invocations(state, &recovered_invocations).await?;

    info!(
        invoke_id = %pending_attempt_record.invoke_id,
        attempt_id = pending_attempt_record.attempt_id,
        recovered_attempts = recovered_attempts.len(),
        recovered_invocations = recovered_invocations.len(),
        recovery_trigger = "drop_guard",
        "recovered pool early-phase orphan after request future dropped"
    );

    Ok(())
}

pub(crate) async fn recover_guard_dropped_pool_invocation_orphan(
    state: &AppState,
    selector: InvocationRecoverySelector,
    recovery_trigger: &'static str,
) -> Result<()> {
    state.sqlite_batch_writer.flush_now(&state.pool).await?;

    let dashboard_reconcile_gate = state.sqlite_batch_writer.dashboard_reconcile_gate();
    let _dashboard_reconcile_guard = dashboard_reconcile_gate.lock().await;
    let recovered_invocations = recover_proxy_invocations_with_scope(
        &state.pool,
        ProxyInvocationRecoveryScope::Selectors(std::slice::from_ref(&selector)),
    )
    .await?;

    if recovered_invocations.is_empty() {
        terminalize_proxy_runtime_snapshot_by_key(
            state,
            &selector.invoke_id,
            &selector.occurred_at,
            recovery_trigger,
        );
        schedule_dashboard_activity_live_snapshot(state);
        return Ok(());
    }

    info!(
        invoke_id = %selector.invoke_id,
        occurred_at = %selector.occurred_at,
        recovered_invocations = recovered_invocations.len(),
        recovery_trigger,
        "recovered pool invocation orphan after request future dropped"
    );

    broadcast_recovered_proxy_invocations(state, &recovered_invocations).await
}

pub(crate) async fn recover_guard_dropped_pool_terminal_invocation_orphan(
    state: &AppState,
    selector: InvocationRecoverySelector,
) -> Result<()> {
    recover_guard_dropped_pool_invocation_orphan(state, selector, "terminal_invocation_drop_guard")
        .await
}

pub(crate) async fn recover_stale_pool_early_phase_orphans_runtime(
    state: &AppState,
) -> Result<PoolOrphanRecoveryOutcome> {
    state.sqlite_batch_writer.flush_now(&state.pool).await?;

    let timeouts = resolve_pool_routing_timeouts(&state.pool, &state.config).await?;
    let responses_started_before = stale_started_before_string(
        timeouts.responses_first_byte_timeout,
        POOL_EARLY_PHASE_ORPHAN_RECOVERY_GRACE,
    );
    let compact_started_before = stale_started_before_string(
        timeouts.compact_first_byte_timeout,
        POOL_EARLY_PHASE_ORPHAN_RECOVERY_GRACE,
    );
    let default_started_before = stale_started_before_string(
        timeouts.default_first_byte_timeout,
        POOL_EARLY_PHASE_ORPHAN_RECOVERY_GRACE,
    );
    let active_attempt_ids = state
        .pool_live_attempt_ids
        .lock()
        .unwrap_or_else(|err| err.into_inner())
        .clone();
    let dashboard_reconcile_gate = state.sqlite_batch_writer.dashboard_reconcile_gate();
    let _dashboard_reconcile_guard = dashboard_reconcile_gate.lock().await;
    let mut tx = state.pool.begin().await?;
    let stale_candidates = load_stale_pool_upstream_request_attempt_candidate_rows_tx(
        tx.as_mut(),
        &responses_started_before,
        &compact_started_before,
        &default_started_before,
    )
    .await?;
    let candidate_ids = stale_candidates
        .into_iter()
        .filter(|row| !active_attempt_ids.contains(&row.id))
        .map(|row| row.id)
        .collect::<Vec<_>>();
    let finished_at = shanghai_now_string();
    let recovered_attempts = recover_stale_pool_upstream_request_attempt_candidates_tx(
        tx.as_mut(),
        &candidate_ids,
        finished_at.as_str(),
        &responses_started_before,
        &compact_started_before,
        &default_started_before,
    )
    .await?;
    if recovered_attempts.is_empty() {
        tx.commit().await?;
        return Ok(PoolOrphanRecoveryOutcome::default());
    }

    let selectors: Vec<_> = recovered_attempts
        .iter()
        .map(|row| InvocationRecoverySelector::new(row.invoke_id.clone(), row.occurred_at.clone()))
        .collect();
    let recovered_invocations = recover_proxy_invocations_with_scope_tx(
        tx.as_mut(),
        ProxyInvocationRecoveryScope::Selectors(&selectors),
    )
    .await?;
    tx.commit().await?;

    clean_up_recovered_pool_routes(
        state,
        &recovered_attempts,
        &recovered_invocations,
        "runtime_sweeper",
    )
    .await;

    for invoke_id in recovered_attempts
        .iter()
        .map(|row| row.invoke_id.as_str())
        .collect::<BTreeSet<_>>()
    {
        if let Err(err) = broadcast_pool_upstream_attempts_snapshot(state, invoke_id).await {
            warn!(
                invoke_id,
                error = %err,
                "failed to broadcast stale pool orphan recovery snapshot"
            );
        }
    }
    broadcast_recovered_proxy_invocations(state, &recovered_invocations).await?;

    let outcome = PoolOrphanRecoveryOutcome {
        recovered_attempts: recovered_attempts.len(),
        recovered_invocations: recovered_invocations.len(),
    };
    info!(
        recovered_attempts = outcome.recovered_attempts,
        recovered_invocations = outcome.recovered_invocations,
        recovery_trigger = "runtime_sweeper",
        "recovered stale pool early-phase orphans at runtime"
    );

    Ok(outcome)
}

pub(crate) async fn broadcast_pool_upstream_attempts_snapshot(
    state: &AppState,
    invoke_id: &str,
) -> Result<()> {
    state
        .subscription_hub
        .publish_runtime_mutation(RuntimeMutation::AttemptChanged {
            invoke_id: invoke_id.to_string(),
        });
    state
        .subscription_hub
        .publish_runtime_mutation(RuntimeMutation::ModelRoutingChanged);
    #[cfg(test)]
    if state.broadcaster.receiver_count() > 0 {
        let attempts = query_pool_attempt_records_from_live(&state.pool, invoke_id)
            .await
            .map_err(|err| anyhow!("failed to load test pool attempt snapshot: {err:?}"))?;
        let _ = state.broadcaster.send(BroadcastPayload::PoolAttempts {
            invoke_id: invoke_id.to_string(),
            attempts,
        });
    }
    Ok(())
}

pub(crate) async fn broadcast_pool_attempt_started_runtime_snapshot(
    state: &AppState,
    trace: &PoolUpstreamAttemptTraceContext,
    runtime_snapshot: &PoolAttemptRuntimeSnapshotContext,
    account: &PoolResolvedAccount,
    attempt_count: usize,
    distinct_account_count: usize,
    request_compression_algorithm: Option<&str>,
) {
    let mut running_record = build_running_proxy_capture_record(
        &trace.invoke_id,
        &trace.occurred_at,
        runtime_snapshot.capture_target,
        &runtime_snapshot.request_info,
        trace.requester_ip.as_deref(),
        trace.sticky_key.as_deref(),
        runtime_snapshot.prompt_cache_key.as_deref(),
        true,
        Some(account.account_id),
        Some(account.display_name.as_str()),
        payload_summary_upstream_account_kind(Some(account)),
        payload_summary_upstream_base_url_host(Some(account)),
        None,
        Some(attempt_count),
        Some(distinct_account_count),
        None,
        None,
        runtime_snapshot.t_req_read_ms,
        runtime_snapshot.t_req_parse_ms,
        0.0,
        0.0,
    );
    set_proxy_capture_record_request_compression_algorithm(
        &mut running_record,
        request_compression_algorithm,
    );
    if let Err(err) =
        persist_and_broadcast_proxy_capture_runtime_snapshot(state, running_record).await
    {
        warn!(
            ?err,
            invoke_id = %trace.invoke_id,
            "failed to broadcast pool attempt start runtime snapshot"
        );
    }
    if let Err(err) = broadcast_pool_upstream_attempts_snapshot(state, &trace.invoke_id).await {
        warn!(
            invoke_id = %trace.invoke_id,
            error = %err,
            "failed to broadcast pool attempt start snapshot"
        );
    }
}

pub(crate) async fn finalize_pool_upstream_request_attempt(
    pool: &Pool<Sqlite>,
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
    let _write_permit = crate::proxy_sqlite_write_coordinator::proxy_sqlite_write_coordinator()
        .acquire(crate::proxy_sqlite_write_coordinator::ProxySqliteWriteClass::InteractiveProxy)
        .await;
    let terminal_phase = terminal_pool_upstream_request_attempt_phase(status);
    let compact_support_status =
        compact_support_status.or(pending.compact_support_status.as_deref());
    let compact_support_reason =
        compact_support_reason.or(pending.compact_support_reason.as_deref());
    let upstream_request_compression_algorithm =
        pending.upstream_request_compression_algorithm.as_deref();
    let upstream_request_compression_mode = pending.upstream_request_compression_mode.as_deref();
    let upstream_request_logical_body_bytes = pending.upstream_request_logical_body_bytes;
    let upstream_request_transmitted_body_bytes = pending.upstream_request_transmitted_body_bytes;
    let upstream_request_header_bytes_approx = pending.upstream_request_header_bytes_approx;
    let upstream_response_body_bytes = pending.upstream_response_body_bytes;
    let upstream_response_header_bytes_approx = pending.upstream_response_header_bytes_approx;
    let response_raw_capture_present = pending.response_raw_path.is_some()
        || pending.response_raw_size.is_some()
        || pending.response_raw_truncated
        || pending.response_raw_truncated_reason.is_some()
        || pending.response_content_encoding.is_some();
    let trace = PoolUpstreamAttemptTraceContext {
        invoke_id: pending.invoke_id.clone(),
        occurred_at: pending.occurred_at.clone(),
        endpoint: pending.endpoint.clone(),
        sticky_key: pending.sticky_key.clone(),
        requester_ip: pending.requester_ip.clone(),
        upstream_base_url_host: pending.upstream_base_url_host.clone(),
        request_model: pending.request_model.clone(),
    };
    if let Some(attempt_id) = pending.attempt_id {
        let result = sqlx::query(
            r#"
            UPDATE pool_upstream_request_attempts
            SET
                finished_at = ?2,
                status = ?3,
                phase = ?4,
                http_status = ?5,
                downstream_http_status = ?6,
                failure_kind = ?7,
                error_message = ?8,
                downstream_error_message = ?9,
                connect_latency_ms = ?10,
                first_byte_latency_ms = ?11,
                stream_latency_ms = ?12,
                upstream_request_id = ?13,
                compact_support_status = ?14,
                compact_support_reason = ?15,
                upstream_request_compression_algorithm = COALESCE(?16, upstream_request_compression_algorithm),
                upstream_request_compression_mode = COALESCE(?17, upstream_request_compression_mode),
                upstream_request_logical_body_bytes = COALESCE(?18, upstream_request_logical_body_bytes),
                upstream_request_transmitted_body_bytes = COALESCE(?19, upstream_request_transmitted_body_bytes),
                upstream_request_header_bytes_approx = COALESCE(?20, upstream_request_header_bytes_approx),
                upstream_response_body_bytes = COALESCE(?21, upstream_response_body_bytes),
                upstream_response_header_bytes_approx = COALESCE(?22, upstream_response_header_bytes_approx),
                upstream_base_url_host = COALESCE(?23, upstream_base_url_host),
                response_raw_path = COALESCE(?24, response_raw_path),
                response_raw_codec = COALESCE(?25, response_raw_codec),
                response_raw_size = COALESCE(?26, response_raw_size),
                response_raw_truncated = COALESCE(?27, response_raw_truncated),
                response_raw_truncated_reason = COALESCE(?28, response_raw_truncated_reason),
                response_content_encoding = COALESCE(?29, response_content_encoding)
            WHERE id = ?1
            "#,
        )
        .bind(attempt_id)
        .bind(finished_at)
        .bind(status)
        .bind(terminal_phase)
        .bind(http_status.map(|value| i64::from(value.as_u16())))
        .bind(downstream_http_status.map(|value| i64::from(value.as_u16())))
        .bind(failure_kind)
        .bind(error_message)
        .bind(downstream_error_message)
        .bind(connect_latency_ms)
        .bind(first_byte_latency_ms)
        .bind(stream_latency_ms)
        .bind(upstream_request_id)
        .bind(compact_support_status)
        .bind(compact_support_reason)
        .bind(upstream_request_compression_algorithm)
        .bind(upstream_request_compression_mode)
        .bind(upstream_request_logical_body_bytes)
        .bind(upstream_request_transmitted_body_bytes)
        .bind(upstream_request_header_bytes_approx)
        .bind(upstream_response_body_bytes)
        .bind(upstream_response_header_bytes_approx)
        .bind(pending.upstream_base_url_host.as_deref())
        .bind(pending.response_raw_path.as_deref())
        .bind(pending.response_raw_codec.as_deref())
        .bind(pending.response_raw_size)
        .bind(response_raw_capture_present.then_some(if pending.response_raw_truncated {
            1_i64
        } else {
            0_i64
        }))
        .bind(pending.response_raw_truncated_reason.as_deref())
        .bind(pending.response_content_encoding.as_deref())
        .execute(pool)
        .await?;

        if result.rows_affected() > 0 {
            return Ok(());
        }
    }

    let inserted_attempt_id = insert_pool_upstream_request_attempt_with_scope(
        pool,
        &trace,
        pending.group_name_snapshot.as_deref(),
        pending.proxy_binding_key_snapshot.as_deref(),
        pending.routing_source.as_deref(),
        pending.routing_selection_audit_json.as_deref(),
        Some(pending.upstream_account_id),
        Some(pending.upstream_route_key.as_str()),
        pending.attempt_index,
        pending.distinct_account_index,
        pending.same_account_retry_index,
        Some(pending.started_at.as_str()),
        Some(finished_at),
        status,
        Some(terminal_phase),
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
        compact_support_reason,
    )
    .await?;
    sqlx::query(
        r#"
        UPDATE pool_upstream_request_attempts
        SET
            response_raw_path = ?2,
            response_raw_codec = COALESCE(?3, response_raw_codec),
            response_raw_size = ?4,
            response_raw_truncated = ?5,
            response_raw_truncated_reason = ?6,
            response_content_encoding = ?7
        WHERE id = ?1
        "#,
    )
    .bind(inserted_attempt_id)
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

pub(crate) async fn insert_pool_upstream_terminal_attempt(
    pool: &Pool<Sqlite>,
    trace: &PoolUpstreamAttemptTraceContext,
    final_error: &PoolUpstreamError,
    attempt_index: i64,
    distinct_account_index: i64,
    failure_kind: &'static str,
) -> Result<()> {
    let _ = (
        pool,
        trace,
        final_error,
        attempt_index,
        distinct_account_index,
        failure_kind,
    );
    // Terminal pool adjudications do not represent a real upstream dispatch and must not create
    // pseudo-attempt rows going forward. Historical rows remain readable via workflow
    // reconstruction, but new data should rely on invocation-level route + final failure.
    Ok(())
}

pub(crate) async fn insert_and_broadcast_pool_upstream_terminal_attempt(
    state: &AppState,
    trace: &PoolUpstreamAttemptTraceContext,
    final_error: &PoolUpstreamError,
    attempt_index: i64,
    distinct_account_index: i64,
    failure_kind: &'static str,
) -> Result<()> {
    let _ = (
        state,
        trace,
        final_error,
        attempt_index,
        distinct_account_index,
        failure_kind,
    );
    Ok(())
}

pub(crate) fn prompt_cache_upstream_account_rollup_key(
    upstream_account_id: Option<i64>,
    upstream_account_name: Option<&str>,
) -> String {
    let normalized_name = upstream_account_name
        .map(str::trim)
        .filter(|value| !value.is_empty());
    match (upstream_account_id, normalized_name) {
        (Some(account_id), Some(account_name)) => format!("id:{account_id}|name:{account_name}"),
        (Some(account_id), None) => format!("id:{account_id}"),
        (None, Some(account_name)) => format!("name:{account_name}"),
        (None, None) => "unknown".to_string(),
    }
}

pub(crate) async fn load_hourly_rollup_live_progress(
    pool: &Pool<Sqlite>,
    dataset: &str,
) -> Result<i64> {
    Ok(sqlx::query_scalar::<_, i64>(
        "SELECT cursor_id FROM hourly_rollup_live_progress WHERE dataset = ?1",
    )
    .bind(dataset)
    .fetch_optional(pool)
    .await?
    .unwrap_or(0))
}

pub(crate) async fn load_hourly_rollup_live_progress_tx(
    tx: &mut SqliteConnection,
    dataset: &str,
) -> Result<i64> {
    Ok(sqlx::query_scalar::<_, i64>(
        "SELECT cursor_id FROM hourly_rollup_live_progress WHERE dataset = ?1",
    )
    .bind(dataset)
    .fetch_optional(&mut *tx)
    .await?
    .unwrap_or(0))
}

pub(crate) async fn save_hourly_rollup_live_progress_tx(
    tx: &mut SqliteConnection,
    dataset: &str,
    cursor_id: i64,
) -> Result<()> {
    sqlx::query(
        r#"
        INSERT INTO hourly_rollup_live_progress (dataset, cursor_id, updated_at)
        VALUES (?1, ?2, datetime('now'))
        ON CONFLICT(dataset) DO UPDATE SET
            cursor_id = MAX(hourly_rollup_live_progress.cursor_id, excluded.cursor_id),
            updated_at = datetime('now')
        "#,
    )
    .bind(dataset)
    .bind(cursor_id)
    .execute(&mut *tx)
    .await?;
    Ok(())
}

pub(crate) async fn mark_hourly_rollup_archive_replayed_tx(
    tx: &mut SqliteConnection,
    target: &str,
    dataset: &str,
    file_path: &str,
) -> Result<()> {
    sqlx::query(
        r#"
        INSERT OR IGNORE INTO hourly_rollup_archive_replay (
            target,
            dataset,
            file_path,
            replayed_at
        )
        VALUES (?1, ?2, ?3, datetime('now'))
        "#,
    )
    .bind(target)
    .bind(dataset)
    .bind(file_path)
    .execute(&mut *tx)
    .await?;
    Ok(())
}

pub(crate) async fn hourly_rollup_archive_replayed_tx(
    tx: &mut SqliteConnection,
    target: &str,
    dataset: &str,
    file_path: &str,
) -> Result<bool> {
    Ok(
        sqlx::query_scalar::<_, i64>(
            "SELECT 1 FROM hourly_rollup_archive_replay WHERE target = ?1 AND dataset = ?2 AND file_path = ?3 LIMIT 1",
        )
        .bind(target)
        .bind(dataset)
        .bind(file_path)
        .fetch_optional(&mut *tx)
        .await?
        .is_some(),
    )
}

pub(crate) fn normalized_oauth_account_id(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|value| !value.is_empty())
}

pub(crate) fn looks_like_uuid_shape(value: &str) -> bool {
    let bytes = value.as_bytes();
    if bytes.len() != 36 {
        return false;
    }
    for (idx, byte) in bytes.iter().enumerate() {
        let is_hyphen = matches!(idx, 8 | 13 | 18 | 23);
        if is_hyphen {
            if *byte != b'-' {
                return false;
            }
        } else if !byte.is_ascii_hexdigit() {
            return false;
        }
    }
    true
}

pub(crate) fn oauth_account_id_shape(value: Option<&str>) -> &'static str {
    match normalized_oauth_account_id(value) {
        None => "empty",
        Some(value) if value.starts_with("org_") => "org",
        Some(value) if looks_like_uuid_shape(value) => "uuid",
        Some(_) => "other",
    }
}

pub(crate) fn oauth_account_header_attached_for_account(
    account: Option<&PoolResolvedAccount>,
) -> Option<bool> {
    let PoolResolvedAuth::Oauth {
        chatgpt_account_id, ..
    } = &account?.auth
    else {
        return None;
    };

    Some(normalized_oauth_account_id(chatgpt_account_id.as_deref()).is_some())
}

pub(crate) fn oauth_account_id_shape_for_account(
    account: Option<&PoolResolvedAccount>,
) -> Option<&'static str> {
    let PoolResolvedAuth::Oauth {
        chatgpt_account_id, ..
    } = &account?.auth
    else {
        return None;
    };

    Some(oauth_account_id_shape(chatgpt_account_id.as_deref()))
}

pub(crate) struct ProxyPayloadSummary<'a> {
    pub(crate) target: ProxyCaptureTarget,
    pub(crate) status: StatusCode,
    pub(crate) is_stream: bool,
    pub(crate) request_contains_encrypted_content: bool,
    pub(crate) response_contains_encrypted_content: bool,
    pub(crate) compaction_request_kind: Option<CompactionKind>,
    pub(crate) compaction_response_kind: Option<CompactionKind>,
    pub(crate) image_intent: Option<&'a str>,
    pub(crate) request_model: Option<&'a str>,
    pub(crate) requested_service_tier: Option<&'a str>,
    pub(crate) billing_service_tier: Option<&'a str>,
    pub(crate) reasoning_effort: Option<&'a str>,
    pub(crate) response_model: Option<&'a str>,
    pub(crate) usage_missing_reason: Option<&'a str>,
    pub(crate) request_parse_error: Option<&'a str>,
    pub(crate) request_compression_algorithm: Option<&'a str>,
    pub(crate) request_compression_mode: Option<&'a str>,
    pub(crate) request_compression_logical_body_bytes: Option<usize>,
    pub(crate) request_compression_transmitted_body_bytes: Option<usize>,
    pub(crate) request_compression_transmission_complete: Option<bool>,
    pub(crate) failure_kind: Option<&'a str>,
    pub(crate) requester_ip: Option<&'a str>,
    pub(crate) request_user_agent: Option<&'a str>,
    pub(crate) request_x_forwarded_for: Option<&'a str>,
    pub(crate) request_forwarded: Option<&'a str>,
    pub(crate) request_x_real_ip: Option<&'a str>,
    pub(crate) upstream_scope: &'a str,
    pub(crate) route_mode: &'a str,
    pub(crate) sticky_key: Option<&'a str>,
    pub(crate) prompt_cache_key: Option<&'a str>,
    pub(crate) prompt_cache_key_attribution_source: Option<&'a str>,
    pub(crate) client_fingerprint: Option<&'a str>,
    pub(crate) client_header_fingerprints: Option<&'a BTreeMap<String, String>>,
    pub(crate) upstream_account_id: Option<i64>,
    pub(crate) upstream_account_name: Option<&'a str>,
    pub(crate) upstream_account_kind: Option<&'a str>,
    pub(crate) upstream_base_url_host: Option<&'a str>,
    pub(crate) oauth_account_header_attached: Option<bool>,
    pub(crate) oauth_account_id_shape: Option<&'a str>,
    pub(crate) oauth_forwarded_header_count: Option<usize>,
    pub(crate) oauth_forwarded_header_names: Option<&'a [String]>,
    pub(crate) oauth_fingerprint_version: Option<&'a str>,
    pub(crate) oauth_forwarded_header_fingerprints: Option<&'a BTreeMap<String, String>>,
    pub(crate) oauth_prompt_cache_header_forwarded: Option<bool>,
    pub(crate) oauth_request_body_prefix_fingerprint: Option<&'a str>,
    pub(crate) oauth_request_body_prefix_bytes: Option<usize>,
    pub(crate) oauth_request_body_snapshot_kind: Option<&'a str>,
    pub(crate) oauth_responses_body_mode: Option<&'a str>,
    pub(crate) oauth_responses_rewrite: Option<&'a oauth_bridge::OauthResponsesRewriteSummary>,
    pub(crate) service_tier: Option<&'a str>,
    pub(crate) stream_terminal_event: Option<&'a str>,
    pub(crate) upstream_error_code: Option<&'a str>,
    pub(crate) upstream_error_message: Option<&'a str>,
    pub(crate) downstream_status_code: Option<StatusCode>,
    pub(crate) downstream_error_message: Option<&'a str>,
    pub(crate) upstream_request_id: Option<&'a str>,
    pub(crate) response_content_encoding: Option<&'a str>,
    pub(crate) stream_failure_origin: Option<&'a str>,
    pub(crate) upstream_read_error_kind: Option<&'a str>,
    pub(crate) content_encoding_chain: Option<&'a str>,
    pub(crate) forwarded_chunk_count: Option<usize>,
    pub(crate) forwarded_bytes: Option<usize>,
    pub(crate) usage_observed: Option<bool>,
    pub(crate) downstream_close_phase: Option<&'a str>,
    pub(crate) downstream_write_error_kind: Option<&'a str>,
    pub(crate) last_upstream_chunk_gap_ms: Option<u64>,
    pub(crate) upstream_approx_upload_bytes: Option<usize>,
    pub(crate) upstream_approx_download_bytes: Option<usize>,
    pub(crate) proxy_display_name: Option<&'a str>,
    pub(crate) proxy_weight_delta: Option<f64>,
    pub(crate) pool_attempt_count: Option<usize>,
    pub(crate) pool_distinct_account_count: Option<usize>,
    pub(crate) pool_attempt_terminal_reason: Option<&'a str>,
    pub(crate) blocked_binding: Option<&'a BlockedBindingDiagnostic>,
}

pub(crate) fn build_proxy_payload_summary(summary: ProxyPayloadSummary<'_>) -> String {
    let ProxyPayloadSummary {
        target,
        status,
        is_stream,
        request_contains_encrypted_content,
        response_contains_encrypted_content,
        compaction_request_kind,
        compaction_response_kind,
        image_intent,
        request_model,
        requested_service_tier,
        billing_service_tier,
        reasoning_effort,
        response_model,
        usage_missing_reason,
        request_parse_error,
        request_compression_algorithm,
        request_compression_mode,
        request_compression_logical_body_bytes,
        request_compression_transmitted_body_bytes,
        request_compression_transmission_complete,
        failure_kind,
        requester_ip,
        request_user_agent,
        request_x_forwarded_for,
        request_forwarded,
        request_x_real_ip,
        upstream_scope,
        route_mode,
        sticky_key,
        prompt_cache_key,
        prompt_cache_key_attribution_source,
        client_fingerprint,
        client_header_fingerprints,
        upstream_account_id,
        upstream_account_name,
        upstream_account_kind,
        upstream_base_url_host,
        oauth_account_header_attached,
        oauth_account_id_shape,
        oauth_forwarded_header_count,
        oauth_forwarded_header_names,
        oauth_fingerprint_version,
        oauth_forwarded_header_fingerprints,
        oauth_prompt_cache_header_forwarded,
        oauth_request_body_prefix_fingerprint,
        oauth_request_body_prefix_bytes,
        oauth_request_body_snapshot_kind,
        oauth_responses_body_mode,
        oauth_responses_rewrite,
        service_tier,
        stream_terminal_event,
        upstream_error_code,
        upstream_error_message,
        downstream_status_code,
        downstream_error_message,
        upstream_request_id,
        response_content_encoding,
        stream_failure_origin,
        upstream_read_error_kind,
        content_encoding_chain,
        forwarded_chunk_count,
        forwarded_bytes,
        usage_observed,
        downstream_close_phase,
        downstream_write_error_kind,
        last_upstream_chunk_gap_ms,
        upstream_approx_upload_bytes,
        upstream_approx_download_bytes,
        proxy_display_name,
        proxy_weight_delta,
        pool_attempt_count,
        pool_distinct_account_count,
        pool_attempt_terminal_reason,
        blocked_binding,
    } = summary;
    let payload = json!({
        "endpoint": target.endpoint(),
        "statusCode": status.as_u16(),
        "isStream": is_stream,
        "requestContainsEncryptedContent": request_contains_encrypted_content,
        "responseContainsEncryptedContent": response_contains_encrypted_content,
        "compactionRequestKind": compaction_request_kind.map(CompactionKind::as_payload_str),
        "compactionResponseKind": compaction_response_kind.map(CompactionKind::as_payload_str),
        "imageIntent": image_intent,
        "requestModel": request_model,
        "requestedServiceTier": requested_service_tier,
        "billingServiceTier": billing_service_tier,
        "reasoningEffort": reasoning_effort,
        "responseModel": response_model,
        "usageMissingReason": usage_missing_reason,
        "requestParseError": request_parse_error,
        "requestCompressionAlgorithm": request_compression_algorithm,
        "requestCompressionMode": request_compression_mode,
        "requestCompressionLogicalBodyBytes": request_compression_logical_body_bytes,
        "requestCompressionTransmittedBodyBytes": request_compression_transmitted_body_bytes,
        "requestCompressionTransmissionComplete": request_compression_transmission_complete,
        "failureKind": failure_kind,
        "requesterIp": requester_ip,
        "requestUserAgent": request_user_agent,
        "requestXForwardedFor": request_x_forwarded_for,
        "requestForwarded": request_forwarded,
        "requestXRealIp": request_x_real_ip,
        "upstreamScope": upstream_scope,
        "routeMode": route_mode,
        "stickyKey": sticky_key,
        "promptCacheKey": prompt_cache_key,
        "promptCacheKeyAttributionSource": prompt_cache_key_attribution_source,
        "clientFingerprint": client_fingerprint,
        "clientHeaderFingerprints": client_header_fingerprints,
        "upstreamAccountId": upstream_account_id,
        "upstreamAccountName": upstream_account_name,
        "upstreamAccountKind": upstream_account_kind,
        "upstreamBaseUrlHost": upstream_base_url_host,
        "oauthAccountHeaderAttached": oauth_account_header_attached,
        "oauthAccountIdShape": oauth_account_id_shape,
        "oauthForwardedHeaderCount": oauth_forwarded_header_count,
        "oauthForwardedHeaderNames": oauth_forwarded_header_names,
        "oauthFingerprintVersion": oauth_fingerprint_version,
        "oauthForwardedHeaderFingerprints": oauth_forwarded_header_fingerprints,
        "oauthPromptCacheHeaderForwarded": oauth_prompt_cache_header_forwarded,
        "oauthRequestBodyPrefixFingerprint": oauth_request_body_prefix_fingerprint,
        "oauthRequestBodyPrefixBytes": oauth_request_body_prefix_bytes,
        "oauthRequestBodySnapshotKind": oauth_request_body_snapshot_kind,
        "oauthResponsesBodyMode": oauth_responses_body_mode,
        "oauthResponsesRewrite": oauth_responses_rewrite,
        "serviceTier": service_tier,
        "streamTerminalEvent": stream_terminal_event,
        "upstreamErrorCode": upstream_error_code,
        "upstreamErrorMessage": upstream_error_message,
        "downstreamStatusCode": downstream_status_code.map(|value| value.as_u16()),
        "downstreamErrorMessage": downstream_error_message,
        "upstreamRequestId": upstream_request_id,
        "responseContentEncoding": response_content_encoding,
        "streamFailureOrigin": stream_failure_origin,
        "upstreamReadErrorKind": upstream_read_error_kind,
        "contentEncodingChain": content_encoding_chain,
        "forwardedChunkCount": forwarded_chunk_count,
        "forwardedBytes": forwarded_bytes,
        "usageObserved": usage_observed,
        "downstreamClosePhase": downstream_close_phase,
        "downstreamWriteErrorKind": downstream_write_error_kind,
        "lastUpstreamChunkGapMs": last_upstream_chunk_gap_ms,
        "upstreamApproxUploadBytes": upstream_approx_upload_bytes,
        "upstreamApproxDownloadBytes": upstream_approx_download_bytes,
        "proxyDisplayName": proxy_display_name,
        "proxyWeightDelta": proxy_weight_delta,
        "poolAttemptCount": pool_attempt_count,
        "poolDistinctAccountCount": pool_distinct_account_count,
        "poolAttemptTerminalReason": pool_attempt_terminal_reason,
        "blockedBinding": blocked_binding,
    });
    serde_json::to_string(&payload).unwrap_or_else(|_| "{}".to_string())
}

pub(crate) fn with_proxy_stream_terminal_diagnostics(
    payload: String,
    upstream_outcome: Option<&str>,
    post_terminal_upstream_read_error_kind: Option<&str>,
    post_terminal_upstream_read_error_message: Option<&str>,
    post_terminal_downstream_write_error_kind: Option<&str>,
    post_terminal_downstream_write_error_message: Option<&str>,
) -> String {
    let Ok(mut value) = serde_json::from_str::<Value>(&payload) else {
        return payload;
    };
    let Some(object) = value.as_object_mut() else {
        return payload;
    };
    for (key, entry) in [
        ("upstreamOutcome", upstream_outcome),
        (
            "postTerminalUpstreamReadErrorKind",
            post_terminal_upstream_read_error_kind,
        ),
        (
            "postTerminalUpstreamReadErrorMessage",
            post_terminal_upstream_read_error_message,
        ),
        (
            "postTerminalDownstreamWriteErrorKind",
            post_terminal_downstream_write_error_kind,
        ),
        (
            "postTerminalDownstreamWriteErrorMessage",
            post_terminal_downstream_write_error_message,
        ),
    ] {
        if let Some(entry) = entry {
            object.insert(key.to_string(), Value::String(entry.to_string()));
        }
    }
    serde_json::to_string(&value).unwrap_or(payload)
}

/// Adds transport-only request-streaming observations to the invocation audit
/// payload. Request content is intentionally never copied into this payload.
pub(crate) fn with_live_request_streaming_payload_summary(
    payload: String,
    decision: &LiveRequestStreamingDecision,
    measurement: &LiveRequestStreamingMeasurement,
) -> String {
    let Ok(mut value) = serde_json::from_str::<Value>(&payload) else {
        return payload;
    };
    let Some(object) = value.as_object_mut() else {
        return payload;
    };
    object.insert(
        "requestBodyTransportMode".to_string(),
        Value::String(decision.transport_mode.as_str().to_string()),
    );
    object.insert(
        "liveFirstEligibility".to_string(),
        Value::Bool(decision.eligible),
    );
    object.insert(
        "liveFirstReason".to_string(),
        Value::String(decision.reason.to_string()),
    );
    insert_payload_optional_text(object, "liveFirstRevision", decision.revision);
    insert_payload_optional_text(
        object,
        "liveFirstExperimentVariant",
        decision
            .variant
            .map(LiveRequestStreamingExperimentVariant::as_str),
    );
    insert_payload_optional_text(
        object,
        "upstreamAccountGroup",
        measurement.upstream_account_group.as_deref(),
    );
    insert_payload_optional_text(
        object,
        "liveFirstAccountGroup",
        measurement.experiment_account_group.as_deref(),
    );
    insert_payload_optional_usize(object, "requestBodyRawBytes", measurement.raw_body_bytes);
    insert_payload_optional_usize(
        object,
        "requestBodyLogicalBytes",
        measurement.logical_body_bytes,
    );
    insert_payload_optional_f64(
        object,
        "upstreamRequestFirstByteMs",
        measurement.upstream_request_first_byte_ms,
    );
    insert_payload_optional_f64(
        object,
        "requestBodyCaptureCompleteMs",
        measurement.request_body_capture_complete_ms,
    );
    insert_payload_optional_f64(
        object,
        "requestUpstreamOverlapMs",
        measurement.request_upstream_overlap_ms,
    );
    insert_payload_optional_f64(
        object,
        "firstResponseByteTotalMs",
        measurement.first_response_byte_total_ms,
    );
    insert_payload_optional_f64(object, "firstTokenTotalMs", measurement.first_token_ms);
    object.insert(
        "liveFirstAttemptFailed".to_string(),
        Value::Bool(measurement.first_attempt_failed),
    );
    object.insert(
        "liveFirstFallbackOrRetry".to_string(),
        Value::Bool(measurement.fallback_or_retry),
    );
    object.insert(
        "liveFirstCaptureFailed".to_string(),
        Value::Bool(measurement.capture_failed),
    );
    object.insert(
        "ambiguousUpstreamDelivery".to_string(),
        Value::Bool(measurement.ambiguous_upstream_delivery),
    );
    serde_json::to_string(&value).unwrap_or(payload)
}

fn insert_payload_optional_text(
    object: &mut serde_json::Map<String, Value>,
    key: &str,
    value: Option<&str>,
) {
    object.insert(
        key.to_string(),
        value
            .map(|value| Value::String(value.to_string()))
            .unwrap_or(Value::Null),
    );
}

fn insert_payload_optional_usize(
    object: &mut serde_json::Map<String, Value>,
    key: &str,
    value: Option<usize>,
) {
    object.insert(
        key.to_string(),
        value
            .map(|value| Value::Number(serde_json::Number::from(value)))
            .unwrap_or(Value::Null),
    );
}

fn insert_payload_optional_f64(
    object: &mut serde_json::Map<String, Value>,
    key: &str,
    value: Option<f64>,
) {
    object.insert(
        key.to_string(),
        value
            .and_then(serde_json::Number::from_f64)
            .map(Value::Number)
            .unwrap_or(Value::Null),
    );
}

pub(crate) fn with_image_tool_rewrite_payload_summary(
    payload: String,
    image_tool_rewrite: Option<&Value>,
) -> String {
    let Some(image_tool_rewrite) = image_tool_rewrite else {
        return payload;
    };
    let Ok(mut value) = serde_json::from_str::<Value>(&payload) else {
        return payload;
    };
    let Some(object) = value.as_object_mut() else {
        return payload;
    };
    object.insert("imageToolRewrite".to_string(), image_tool_rewrite.clone());
    serde_json::to_string(&value).unwrap_or(payload)
}

pub(crate) fn with_codex_imagegen_rewrite_payload_summary(
    payload: String,
    codex_imagegen_rewrite: Option<&Value>,
) -> String {
    let Some(codex_imagegen_rewrite) = codex_imagegen_rewrite else {
        return payload;
    };
    let Ok(mut value) = serde_json::from_str::<Value>(&payload) else {
        return payload;
    };
    let Some(object) = value.as_object_mut() else {
        return payload;
    };
    object.insert(
        "codexImagegenRewrite".to_string(),
        codex_imagegen_rewrite.clone(),
    );
    serde_json::to_string(&value).unwrap_or(payload)
}

pub(crate) fn invocation_status_is_in_flight(status: Option<&str>) -> bool {
    matches!(
        status
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase()
            .as_str(),
        INVOCATION_STATUS_RUNNING | INVOCATION_STATUS_PENDING
    )
}

pub(crate) fn invocation_status_is_recoverable_proxy_interrupted(
    status: Option<&str>,
    failure_kind: Option<&str>,
) -> bool {
    status
        .unwrap_or_default()
        .trim()
        .eq_ignore_ascii_case(INVOCATION_STATUS_INTERRUPTED)
        && failure_kind
            .unwrap_or_default()
            .trim()
            .eq_ignore_ascii_case(PROXY_FAILURE_INVOCATION_INTERRUPTED)
}

pub(crate) fn nullable_runtime_timing_value(value: f64) -> Option<f64> {
    (value.is_finite() && value > 0.0).then_some(value)
}

#[derive(Debug, FromRow)]
pub(crate) struct PersistedInvocationIdentityRow {
    pub(crate) id: i64,
    pub(crate) status: Option<String>,
    pub(crate) failure_kind: Option<String>,
}

pub(crate) async fn load_persisted_invocation_identity_tx(
    tx: &mut SqliteConnection,
    invoke_id: &str,
    occurred_at: &str,
) -> Result<Option<PersistedInvocationIdentityRow>> {
    sqlx::query_as::<_, PersistedInvocationIdentityRow>(
        r#"
        SELECT id, status, failure_kind
        FROM codex_invocations
        WHERE invoke_id = ?1 AND occurred_at = ?2
        ORDER BY id DESC
        LIMIT 1
        "#,
    )
    .bind(invoke_id)
    .bind(occurred_at)
    .fetch_optional(&mut *tx)
    .await
    .map_err(Into::into)
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn update_existing_proxy_invocation_record_tx(
    tx: &mut SqliteConnection,
    id: i64,
    record: &ProxyCaptureRecord,
    raw_response: &str,
    resp_raw: &RawPayloadMeta,
    failure_kind: Option<&str>,
    failure_class: &str,
    is_actionable: bool,
    t_total_ms: Option<f64>,
    t_req_read_ms: Option<f64>,
    t_req_parse_ms: Option<f64>,
    t_upstream_connect_ms: Option<f64>,
    t_upstream_ttfb_ms: Option<f64>,
    first_token_ms: Option<f64>,
    t_upstream_stream_ms: Option<f64>,
    t_resp_parse_ms: Option<f64>,
    t_persist_ms: Option<f64>,
) -> Result<bool> {
    let result = sqlx::query(
        r#"
        UPDATE codex_invocations
        SET
            source = ?2,
            model = ?3,
            input_tokens = ?4,
            output_tokens = ?5,
            cache_input_tokens = ?6,
            reasoning_tokens = ?7,
            total_tokens = ?8,
            cost = ?9,
            cost_input = ?10,
            cost_cache_write = ?11,
            cost_cache_read = ?12,
            cost_output = ?13,
            cost_reasoning = ?14,
            cost_estimated = ?15,
            price_version = ?16,
            status = ?17,
            error_message = ?18,
            failure_kind = ?19,
            failure_class = ?20,
            is_actionable = ?21,
            payload = ?22,
            raw_response = ?23,
            request_raw_path = ?24,
            request_raw_codec = ?25,
            request_raw_size = ?26,
            request_raw_truncated = ?27,
            request_raw_truncated_reason = ?28,
            response_raw_path = ?29,
            response_raw_codec = ?30,
            response_raw_size = ?31,
            response_raw_truncated = ?32,
            response_raw_truncated_reason = ?33,
            t_total_ms = ?34,
            t_req_read_ms = ?35,
            t_req_parse_ms = ?36,
            t_upstream_connect_ms = ?37,
            t_upstream_ttfb_ms = ?38,
            first_token_ms = ?39,
            t_upstream_stream_ms = ?40,
            t_resp_parse_ms = ?41,
            t_persist_ms = ?42
        WHERE id = ?1
          AND (
                LOWER(TRIM(COALESCE(status, ''))) IN ('running', 'pending')
                OR (
                    LOWER(TRIM(COALESCE(status, ''))) = 'interrupted'
                    AND LOWER(TRIM(COALESCE(failure_kind, ''))) = 'proxy_interrupted'
                )
          )
        "#,
    )
    .bind(id)
    .bind(SOURCE_PROXY)
    .bind(&record.model)
    .bind(record.usage.input_tokens)
    .bind(record.usage.output_tokens)
    .bind(record.usage.cache_input_tokens)
    .bind(record.usage.reasoning_tokens)
    .bind(record.usage.total_tokens)
    .bind(record.cost)
    .bind(record.cost_breakdown.map(|value| value.input))
    .bind(record.cost_breakdown.map(|value| value.cache_write))
    .bind(record.cost_breakdown.map(|value| value.cache_read))
    .bind(record.cost_breakdown.map(|value| value.output))
    .bind(record.cost_breakdown.map(|value| value.reasoning))
    .bind(record.cost_estimated as i64)
    .bind(record.price_version.as_deref())
    .bind(&record.status)
    .bind(record.error_message.as_deref())
    .bind(failure_kind)
    .bind(failure_class)
    .bind(is_actionable as i64)
    .bind(record.payload.as_deref())
    .bind(raw_response)
    .bind(record.req_raw.path.as_deref())
    .bind(raw_payload_meta_codec(&record.req_raw))
    .bind(record.req_raw.size_bytes)
    .bind(record.req_raw.truncated as i64)
    .bind(record.req_raw.truncated_reason.as_deref())
    .bind(resp_raw.path.as_deref())
    .bind(raw_payload_meta_codec(resp_raw))
    .bind(resp_raw.size_bytes)
    .bind(resp_raw.truncated as i64)
    .bind(resp_raw.truncated_reason.as_deref())
    .bind(t_total_ms)
    .bind(t_req_read_ms)
    .bind(t_req_parse_ms)
    .bind(t_upstream_connect_ms)
    .bind(t_upstream_ttfb_ms)
    .bind(first_token_ms)
    .bind(t_upstream_stream_ms)
    .bind(t_resp_parse_ms)
    .bind(t_persist_ms)
    .execute(&mut *tx)
    .await?;

    Ok(result.rows_affected() > 0)
}

pub(crate) fn api_invocation_from_runtime_record(record: &ProxyCaptureRecord) -> ApiInvocation {
    let payload = record
        .payload
        .as_deref()
        .and_then(|payload| serde_json::from_str::<Value>(payload).ok());
    let payload_text = |key: &str| {
        payload
            .as_ref()
            .and_then(|value| value.get(key))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned)
    };
    let payload_i64 = |key: &str| {
        payload
            .as_ref()
            .and_then(|value| value.get(key))
            .and_then(crate::proxy::json_value_to_i64)
    };
    let payload_f64 = |key: &str| {
        payload
            .as_ref()
            .and_then(|value| value.get(key))
            .and_then(Value::as_f64)
    };
    let prompt_cache_key = payload_text("promptCacheKey");
    let sticky_key = payload
        .as_ref()
        .and_then(|value| {
            value
                .get("stickyKey")
                .or_else(|| value.get("promptCacheKey"))
        })
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);
    let upstream_account_id = payload
        .as_ref()
        .and_then(|value| value.get("upstreamAccountId"))
        .and_then(crate::proxy::json_value_to_i64);
    let upstream_account_name = payload_text("upstreamAccountName");
    let blocked_binding_json = payload
        .as_ref()
        .and_then(|value| value.get("blockedBinding"))
        .filter(|value| value.is_object())
        .and_then(|value| serde_json::to_string(value).ok());
    let blocked_binding = parse_blocked_binding_json(blocked_binding_json.as_deref());
    let failure = resolve_failure_classification(
        Some(record.status.as_str()),
        record.error_message.as_deref(),
        record.failure_kind.as_deref(),
        None,
        None,
    );
    ApiInvocation {
        id: 0,
        invoke_id: record.invoke_id.clone(),
        occurred_at: record.occurred_at.clone(),
        source: SOURCE_PROXY.to_string(),
        proxy_display_name: payload_text("proxyDisplayName"),
        model: record.model.clone(),
        request_model: payload_text("requestModel"),
        response_model: payload_text("responseModel"),
        input_tokens: record.usage.input_tokens,
        output_tokens: record.usage.output_tokens,
        cache_input_tokens: record.usage.cache_input_tokens,
        reasoning_tokens: record.usage.reasoning_tokens,
        reasoning_effort: payload_text("reasoningEffort"),
        total_tokens: record.usage.total_tokens,
        cost: record.cost,
        cost_input: record.cost_breakdown.map(|value| value.input),
        cost_cache_write: record.cost_breakdown.map(|value| value.cache_write),
        cost_cache_read: record.cost_breakdown.map(|value| value.cache_read),
        cost_output: record.cost_breakdown.map(|value| value.output),
        cost_reasoning: record.cost_breakdown.map(|value| value.reasoning),
        cache_write_tokens: record.usage.input_tokens.map(|input| {
            input.saturating_sub(record.usage.cache_input_tokens.unwrap_or_default().max(0))
        }),
        status: Some(record.status.clone()),
        live_phase: None,
        error_message: record.error_message.clone(),
        downstream_status_code: payload_i64("downstreamStatusCode"),
        failure_kind: failure
            .failure_kind
            .clone()
            .or_else(|| record.failure_kind.clone()),
        blocked_binding,
        blocked_binding_json,
        stream_terminal_event: payload_text("streamTerminalEvent"),
        upstream_error_code: payload_text("upstreamErrorCode"),
        upstream_error_message: payload_text("upstreamErrorMessage"),
        downstream_error_message: payload_text("downstreamErrorMessage"),
        upstream_request_id: payload_text("upstreamRequestId"),
        failure_class: Some(failure.failure_class.as_str().to_string()),
        is_actionable: Some(failure.is_actionable),
        endpoint: payload_text("endpoint"),
        compaction_request_kind: payload_text("compactionRequestKind"),
        compaction_response_kind: payload_text("compactionResponseKind"),
        image_intent: payload_text("imageIntent"),
        requester_ip: payload_text("requesterIp"),
        prompt_cache_key,
        sticky_key,
        route_mode: payload_text("routeMode"),
        upstream_account_id,
        upstream_account_name,
        response_content_encoding: payload_text("responseContentEncoding"),
        request_compression_algorithm: payload_text("requestCompressionAlgorithm"),
        transport: None,
        pool_attempt_count: payload_i64("poolAttemptCount"),
        pool_distinct_account_count: payload_i64("poolDistinctAccountCount"),
        pool_attempt_terminal_reason: payload_text("poolAttemptTerminalReason"),
        requested_service_tier: payload_text("requestedServiceTier"),
        service_tier: payload_text("serviceTier"),
        billing_service_tier: payload_text("billingServiceTier"),
        proxy_weight_delta: payload_f64("proxyWeightDelta"),
        cost_estimated: Some(record.cost_estimated as i64),
        price_version: record.price_version.clone(),
        cost_audit: None,
        request_raw_path: record.req_raw.path.clone(),
        request_raw_size: Some(record.req_raw.size_bytes),
        request_raw_truncated: Some(record.req_raw.truncated as i64),
        request_raw_truncated_reason: record.req_raw.truncated_reason.clone(),
        response_raw_path: record.resp_raw.path.clone(),
        response_raw_size: Some(record.resp_raw.size_bytes),
        response_raw_truncated: Some(record.resp_raw.truncated as i64),
        response_raw_truncated_reason: record.resp_raw.truncated_reason.clone(),
        detail_level: DETAIL_LEVEL_FULL.to_string(),
        detail_pruned_at: None,
        detail_prune_reason: None,
        t_total_ms: nullable_runtime_timing_value(record.timings.t_total_ms),
        t_req_read_ms: nullable_runtime_timing_value(record.timings.t_req_read_ms),
        t_req_parse_ms: nullable_runtime_timing_value(record.timings.t_req_parse_ms),
        t_upstream_connect_ms: nullable_runtime_timing_value(record.timings.t_upstream_connect_ms),
        t_upstream_ttfb_ms: nullable_runtime_timing_value(record.timings.t_upstream_ttfb_ms),
        first_token_ms: record
            .timings
            .first_token_ms
            .filter(|value| value.is_finite() && *value >= 0.0),
        t_upstream_stream_ms: nullable_runtime_timing_value(record.timings.t_upstream_stream_ms),
        t_resp_parse_ms: nullable_runtime_timing_value(record.timings.t_resp_parse_ms),
        t_persist_ms: nullable_runtime_timing_value(record.timings.t_persist_ms),
        created_at: format_utc_iso_millis(Utc::now()),
    }
}

pub(crate) fn persisted_invocation_allows_proxy_record_update(
    existing_status: Option<&str>,
    existing_failure_kind: Option<&str>,
    incoming_status: &str,
) -> bool {
    invocation_status_is_in_flight(existing_status)
        || (!invocation_status_is_in_flight(Some(incoming_status))
            && invocation_status_is_recoverable_proxy_interrupted(
                existing_status,
                existing_failure_kind,
            ))
}

pub(crate) async fn load_persisted_api_invocation_tx(
    tx: &mut SqliteConnection,
    invoke_id: &str,
    occurred_at: &str,
) -> Result<ApiInvocation> {
    let mut record = sqlx::query_as::<_, ApiInvocation>(
        r#"
        SELECT
            id,
            invoke_id,
            occurred_at,
            source,
            CASE WHEN json_valid(payload) THEN json_extract(payload, '$.proxyDisplayName') END AS proxy_display_name,
            model,
            CASE WHEN json_valid(payload) THEN json_extract(payload, '$.requestModel') END AS request_model,
            CASE WHEN json_valid(payload) THEN json_extract(payload, '$.responseModel') END AS response_model,
            input_tokens,
            output_tokens,
            cache_input_tokens,
            reasoning_tokens,
            CASE WHEN json_valid(payload) THEN json_extract(payload, '$.reasoningEffort') END AS reasoning_effort,
        total_tokens,
        cost,
        cost_input,
        cost_cache_write,
        cost_cache_read,
        cost_output,
        cost_reasoning,
        MAX(COALESCE(input_tokens, 0) - COALESCE(cache_input_tokens, 0), 0) AS cache_write_tokens,
        status,
            error_message,
            CASE WHEN json_valid(payload) THEN json_extract(payload, '$.downstreamStatusCode') END AS downstream_status_code,
            CASE WHEN json_valid(payload) THEN json_extract(payload, '$.endpoint') END AS endpoint,
            CASE WHEN json_valid(payload) THEN json_extract(payload, '$.compactionRequestKind') END AS compaction_request_kind,
            CASE WHEN json_valid(payload) THEN json_extract(payload, '$.compactionResponseKind') END AS compaction_response_kind,
            COALESCE(CASE WHEN json_valid(payload) THEN json_extract(payload, '$.failureKind') END, failure_kind) AS failure_kind,
            CASE
              WHEN json_valid(payload) AND json_type(payload, '$.blockedBinding') = 'object'
                THEN json_extract(payload, '$.blockedBinding')
            END AS blocked_binding_json,
            CASE WHEN json_valid(payload) THEN json_extract(payload, '$.streamTerminalEvent') END AS stream_terminal_event,
            CASE WHEN json_valid(payload) THEN json_extract(payload, '$.upstreamErrorCode') END AS upstream_error_code,
            CASE WHEN json_valid(payload) THEN json_extract(payload, '$.upstreamErrorMessage') END AS upstream_error_message,
            CASE WHEN json_valid(payload) THEN json_extract(payload, '$.downstreamErrorMessage') END AS downstream_error_message,
            CASE WHEN json_valid(payload) THEN json_extract(payload, '$.upstreamRequestId') END AS upstream_request_id,
            failure_class,
            is_actionable,
            CASE WHEN json_valid(payload) THEN json_extract(payload, '$.requesterIp') END AS requester_ip,
            CASE WHEN json_valid(payload) THEN json_extract(payload, '$.promptCacheKey') END AS prompt_cache_key,
            CASE WHEN json_valid(payload) THEN TRIM(COALESCE(CAST(json_extract(payload, '$.stickyKey') AS TEXT), CAST(json_extract(payload, '$.promptCacheKey') AS TEXT))) END AS sticky_key,
            CASE WHEN json_valid(payload) THEN json_extract(payload, '$.routeMode') END AS route_mode,
            CASE WHEN json_valid(payload) THEN json_extract(payload, '$.upstreamAccountId') END AS upstream_account_id,
            CASE WHEN json_valid(payload) THEN json_extract(payload, '$.upstreamAccountName') END AS upstream_account_name,
            CASE WHEN json_valid(payload) THEN json_extract(payload, '$.responseContentEncoding') END AS response_content_encoding,
            CASE
              WHEN json_valid(payload) AND json_type(payload, '$.poolAttemptCount') IN ('integer', 'real')
                THEN json_extract(payload, '$.poolAttemptCount')
            END AS pool_attempt_count,
            CASE
              WHEN json_valid(payload) AND json_type(payload, '$.poolDistinctAccountCount') IN ('integer', 'real')
                THEN json_extract(payload, '$.poolDistinctAccountCount')
            END AS pool_distinct_account_count,
            CASE
              WHEN json_valid(payload) AND json_type(payload, '$.poolAttemptTerminalReason') = 'text'
                THEN json_extract(payload, '$.poolAttemptTerminalReason')
            END AS pool_attempt_terminal_reason,
            CASE
              WHEN json_valid(payload) AND json_type(payload, '$.requestedServiceTier') = 'text'
                THEN json_extract(payload, '$.requestedServiceTier')
              WHEN json_valid(payload) AND json_type(payload, '$.requested_service_tier') = 'text'
                THEN json_extract(payload, '$.requested_service_tier') END AS requested_service_tier,
            CASE
              WHEN json_valid(payload) AND json_type(payload, '$.serviceTier') = 'text'
                THEN json_extract(payload, '$.serviceTier')
              WHEN json_valid(payload) AND json_type(payload, '$.service_tier') = 'text'
                THEN json_extract(payload, '$.service_tier') END AS service_tier,
            CASE
              WHEN json_valid(payload) AND json_type(payload, '$.billingServiceTier') = 'text'
                THEN json_extract(payload, '$.billingServiceTier')
              WHEN json_valid(payload) AND json_type(payload, '$.billing_service_tier') = 'text'
                THEN json_extract(payload, '$.billing_service_tier') END AS billing_service_tier,
            CASE WHEN json_valid(payload)
              AND json_type(payload, '$.proxyWeightDelta') IN ('integer', 'real')
              THEN json_extract(payload, '$.proxyWeightDelta') END AS proxy_weight_delta,
            cost_estimated,
            price_version,
            request_raw_path,
            request_raw_size,
            request_raw_truncated,
            request_raw_truncated_reason,
            response_raw_path,
            response_raw_size,
            response_raw_truncated,
            response_raw_truncated_reason,
            detail_level,
            detail_pruned_at,
            detail_prune_reason,
            t_total_ms,
            t_req_read_ms,
            t_req_parse_ms,
            t_upstream_connect_ms,
            t_upstream_ttfb_ms,
            first_token_ms,
            t_upstream_stream_ms,
            t_resp_parse_ms,
            t_persist_ms,
            created_at
        FROM codex_invocations
        WHERE invoke_id = ?1 AND occurred_at = ?2
        ORDER BY id DESC
        LIMIT 1
        "#,
    )
    .bind(invoke_id)
    .bind(occurred_at)
    .fetch_one(&mut *tx)
    .await?;
    hydrate_api_invocation_blocked_binding(&mut record);
    Ok(record)
}

pub(crate) async fn touch_invocation_upstream_account_last_activity_tx(
    tx: &mut SqliteConnection,
    occurred_at: &str,
    payload: Option<&str>,
) -> Result<()> {
    touch_upstream_account_last_activity_tx(
        tx,
        occurred_at,
        upstream_account_id_from_payload(payload),
    )
    .await
}

pub(crate) async fn touch_upstream_account_last_activity_tx(
    tx: &mut SqliteConnection,
    occurred_at: &str,
    upstream_account_id: Option<i64>,
) -> Result<()> {
    if let Some(upstream_account_id) = upstream_account_id {
        sqlx::query(
            r#"
            UPDATE pool_upstream_accounts
            SET last_activity_at = CASE
                WHEN last_activity_at IS NULL OR last_activity_at < ?1 THEN ?1
                ELSE last_activity_at
            END
            WHERE id = ?2
            "#,
        )
        .bind(occurred_at)
        .bind(upstream_account_id)
        .execute(&mut *tx)
        .await?;
    }

    Ok(())
}

pub(crate) async fn persist_and_broadcast_proxy_capture_runtime_snapshot(
    state: &AppState,
    record: ProxyCaptureRecord,
) -> Result<()> {
    let started = Instant::now();
    let persisted_record = api_invocation_from_runtime_record(&record);
    let invoke_id = persisted_record.invoke_id.clone();
    let occurred_at = persisted_record.occurred_at.clone();
    let store_outcome = state
        .proxy_runtime_invocations
        .upsert(persisted_record.clone());
    if store_outcome.skipped_terminal {
        let elapsed_ms = started.elapsed().as_millis() as u64;
        debug!(
            invoke_id = %invoke_id,
            occurred_at = %occurred_at,
            elapsed_ms,
            runtime_store_running_count = store_outcome.running_count,
            runtime_store_pruned_count = store_outcome.pruned_count,
            running_snapshot_db_write_skipped = true,
            running_snapshot_skipped_after_terminal = true,
            "stale running proxy capture snapshot skipped after terminal persistence"
        );
        return Ok(());
    }
    state
        .dashboard_network_speed_cache
        .observe_dashboard_activity_runtime_snapshot(&persisted_record, Utc::now());
    state
        .subscription_hub
        .publish_runtime_mutation(RuntimeMutation::invocation(
            &persisted_record,
            RuntimeMutationKind::RuntimeUpsert,
        ));
    #[cfg(test)]
    broadcast_test_record_payload(state, &persisted_record);
    schedule_dashboard_activity_live_snapshot(state);

    let elapsed_ms = started.elapsed().as_millis() as u64;
    debug!(
        invoke_id = %invoke_id,
        occurred_at = %occurred_at,
        elapsed_ms,
        runtime_store_running_count = store_outcome.running_count,
        runtime_store_pruned_count = store_outcome.pruned_count,
        running_snapshot_db_write_skipped = true,
        running_snapshot_recovery_placeholder_enqueued = false,
        "running proxy capture snapshot stored in memory and broadcast"
    );

    Ok(())
}

pub(crate) fn broadcast_proxy_capture_first_token_runtime_snapshot(
    state: &AppState,
    invoke_id: &str,
    occurred_at: &str,
    first_token_ms: f64,
) {
    if !first_token_ms.is_finite() || first_token_ms < 0.0 {
        return;
    }
    let Some(mut record) = state
        .proxy_runtime_invocations
        .snapshot()
        .into_iter()
        .find(|record| record.invoke_id == invoke_id && record.occurred_at == occurred_at)
    else {
        return;
    };
    if record.first_token_ms.is_some() {
        return;
    }
    record.first_token_ms = Some(first_token_ms);
    let outcome = state.proxy_runtime_invocations.upsert(record.clone());
    if outcome.skipped_terminal {
        return;
    }
    state
        .dashboard_network_speed_cache
        .observe_dashboard_activity_runtime_snapshot(&record, Utc::now());
    state
        .subscription_hub
        .publish_runtime_mutation(RuntimeMutation::invocation(
            &record,
            RuntimeMutationKind::LifecyclePhase,
        ));
    #[cfg(test)]
    broadcast_test_record_payload(state, &record);
    schedule_dashboard_activity_live_snapshot(state);
}

pub(crate) fn remove_proxy_runtime_snapshot_for_terminal(
    state: &AppState,
    record: &ApiInvocation,
) -> bool {
    state
        .dashboard_network_speed_cache
        .finalize_dashboard_activity_invocation(record, Utc::now());
    state
        .dashboard_network_speed_cache
        .finish_invocation(&record.invoke_id, &record.occurred_at);
    let remove_outcome = state
        .proxy_runtime_invocations
        .upsert_terminal(record.clone());
    debug!(
        invoke_id = %record.invoke_id,
        occurred_at = %record.occurred_at,
        terminal_overlay_emitted = true,
        terminal_removed_runtime_snapshot = remove_outcome.removed,
        terminal_already_tombstoned = remove_outcome.already_terminal,
        "terminal proxy capture record stored in memory runtime overlay"
    );
    remove_outcome.already_terminal
}

pub(crate) fn remove_proxy_runtime_snapshot_by_key(
    state: &AppState,
    invoke_id: &str,
    occurred_at: &str,
    reason: &'static str,
) -> bool {
    state
        .dashboard_network_speed_cache
        .drop_dashboard_activity_invocation(invoke_id, occurred_at);
    state
        .dashboard_network_speed_cache
        .finish_invocation(invoke_id, occurred_at);
    let removed_runtime_snapshot = state
        .proxy_runtime_invocations
        .remove_non_terminal(invoke_id, occurred_at);
    if let Some(record) = &removed_runtime_snapshot {
        state
            .subscription_hub
            .publish_runtime_mutation(RuntimeMutation::invocation(
                record,
                RuntimeMutationKind::RuntimeRemoved,
            ));
    }
    debug!(
        invoke_id,
        occurred_at,
        reason,
        terminal_removed_runtime_snapshot = removed_runtime_snapshot.is_some(),
        terminal_already_tombstoned = false,
        "non-terminal proxy runtime snapshot removed by key"
    );
    removed_runtime_snapshot.is_some()
}

pub(crate) fn terminalize_proxy_runtime_snapshot_by_key(
    state: &AppState,
    invoke_id: &str,
    occurred_at: &str,
    reason: &'static str,
) -> bool {
    let Some(mut record) = state
        .proxy_runtime_invocations
        .remove_non_terminal(invoke_id, occurred_at)
    else {
        debug!(
            invoke_id,
            occurred_at,
            reason,
            terminal_removed_runtime_snapshot = false,
            terminal_already_tombstoned = false,
            "no non-terminal proxy runtime snapshot found for terminal cleanup"
        );
        return false;
    };

    record.status = Some(INVOCATION_STATUS_INTERRUPTED.to_string());
    record.error_message = Some(format!(
        "[{PROXY_FAILURE_INVOCATION_INTERRUPTED}] proxy request ended before a terminal record was written"
    ));
    record.failure_kind = Some(PROXY_FAILURE_INVOCATION_INTERRUPTED.to_string());
    record.failure_class = Some(FAILURE_CLASS_SERVICE.to_string());
    record.is_actionable = Some(true);
    record.pool_attempt_terminal_reason = Some(PROXY_FAILURE_INVOCATION_INTERRUPTED.to_string());
    state
        .dashboard_network_speed_cache
        .finalize_dashboard_activity_invocation(&record, Utc::now());
    state
        .dashboard_network_speed_cache
        .finish_invocation(invoke_id, occurred_at);

    let remove_outcome = state
        .proxy_runtime_invocations
        .upsert_terminal(record.clone());
    debug!(
        invoke_id,
        occurred_at,
        reason,
        terminal_removed_runtime_snapshot = true,
        terminal_already_tombstoned = remove_outcome.already_terminal,
        terminal_delta_skipped_runtime_only = true,
        "non-terminal proxy runtime snapshot terminalized by key"
    );
    state
        .subscription_hub
        .publish_runtime_mutation(RuntimeMutation::invocation(
            &record,
            RuntimeMutationKind::TerminalCommitted,
        ));
    #[cfg(test)]
    broadcast_test_record_payload(state, &record);
    true
}

pub(crate) fn terminalize_proxy_runtime_snapshot_with_error(
    state: &AppState,
    invoke_id: &str,
    occurred_at: &str,
    status: StatusCode,
    failure_kind: &'static str,
    error_message: &str,
    reason: &'static str,
) -> bool {
    let Some(mut record) = state
        .proxy_runtime_invocations
        .remove_non_terminal(invoke_id, occurred_at)
    else {
        debug!(
            invoke_id,
            occurred_at,
            reason,
            terminal_overlay_emitted = false,
            terminal_removed_runtime_snapshot = false,
            "no non-terminal proxy runtime snapshot found for terminal error overlay"
        );
        return false;
    };

    record.status = Some(if status.is_server_error() {
        format!("http_{}", status.as_u16())
    } else {
        "failed".to_string()
    });
    record.error_message = Some(format!("[{failure_kind}] {error_message}"));
    record.failure_kind = Some(failure_kind.to_string());
    record.failure_class = Some(
        if status.is_client_error() {
            FAILURE_CLASS_CLIENT
        } else {
            FAILURE_CLASS_SERVICE
        }
        .to_string(),
    );
    record.is_actionable = Some(true);
    record.pool_attempt_terminal_reason = Some(failure_kind.to_string());
    state
        .dashboard_network_speed_cache
        .finalize_dashboard_activity_invocation(&record, Utc::now());
    state
        .dashboard_network_speed_cache
        .finish_invocation(invoke_id, occurred_at);

    let remove_outcome = state
        .proxy_runtime_invocations
        .upsert_terminal(record.clone());
    debug!(
        invoke_id,
        occurred_at,
        reason,
        status = %status,
        failure_kind,
        terminal_overlay_emitted = true,
        terminal_removed_runtime_snapshot = true,
        terminal_already_tombstoned = remove_outcome.already_terminal,
        terminal_delta_skipped_runtime_only = true,
        "non-terminal proxy runtime snapshot terminalized with error overlay"
    );
    state
        .subscription_hub
        .publish_runtime_mutation(RuntimeMutation::invocation(
            &record,
            RuntimeMutationKind::TerminalCommitted,
        ));
    #[cfg(test)]
    broadcast_test_record_payload(state, &record);
    true
}

pub(crate) async fn observe_successful_proxy_capture_model_route_cache(
    state: &AppState,
    record: &ProxyCaptureRecord,
) {
    if record.status != "success" {
        return;
    }

    let metadata = terminal_payload_metadata(record.payload.as_deref());
    match observe_proxy_cache_hit_if_success(state, record).await {
        Ok(outcome) => {
            if outcome.observed {
                state
                    .subscription_hub
                    .publish_runtime_mutation(RuntimeMutation::ModelRoutingChanged);
            }
            if outcome.availability_increased {
                let account_allows_publish = match metadata.upstream_account_id {
                    Some(account_id) => match pool_account_allows_model_route_availability_publish(
                        &state.pool,
                        account_id,
                    )
                    .await
                    {
                        Ok(allowed) => allowed,
                        Err(err) => {
                            warn!(
                                invoke_id = %record.invoke_id,
                                account_id,
                                error = %err,
                                "failed to verify account fence before publishing model route availability"
                            );
                            false
                        }
                    },
                    None => false,
                };
                if account_allows_publish {
                    publish_pool_routing_availability(state);
                } else {
                    debug!(
                        invoke_id = %record.invoke_id,
                        upstream_account_id = metadata.upstream_account_id,
                        "model cache observation increased capacity without publishing because the account remains fenced"
                    );
                }
            }
        }
        Err(err) => {
            warn!(
                invoke_id = %record.invoke_id,
                error = %err,
                "failed to observe model route cache hit"
            );
        }
    }
}

pub(crate) async fn persist_and_broadcast_proxy_capture_terminal_record(
    state: &AppState,
    record: ProxyCaptureRecord,
) -> Result<()> {
    let enqueue_started = Instant::now();
    let persisted_record = api_invocation_from_runtime_record(&record);
    let invoke_id = persisted_record.invoke_id.clone();
    let duplicate_terminal = remove_proxy_runtime_snapshot_for_terminal(state, &persisted_record);
    if duplicate_terminal {
        debug!(
            invoke_id = %invoke_id,
            occurred_at = %persisted_record.occurred_at,
            business_unblocked_record_write = true,
            "duplicate terminal proxy capture record skipped before sqlite enqueue"
        );
        schedule_proxy_capture_follow_up_after_terminal_enqueue(
            state,
            &invoke_id,
            "duplicate_runtime_terminal",
        );
        return Ok(());
    }
    observe_successful_proxy_capture_model_route_cache(state, &record).await;
    let projection = register_terminal_projection_before_enqueue(state, &persisted_record).await;
    let delta = &projection.dashboard;
    let startup_backfill_tasks = startup_backfill_tasks_for_terminal(&persisted_record);
    debug!(
        invoke_id = %invoke_id,
        terminal_delta_applied_selection_count = delta.applied_selection_count,
        terminal_delta_duplicate = delta.duplicate,
        terminal_delta_skipped_out_of_range_count = delta.skipped_out_of_range_count,
        response_source = "memory",
        "registered terminal record in dashboard activity read model before sqlite enqueue"
    );
    let terminal_enqueue =
        state
            .sqlite_batch_writer
            .enqueue_terminal(BatchedTerminalInvocationWrite {
                record,
                capture_started: None,
                raw_capture: false,
                dashboard_terminal_sequence: delta.terminal_sequence,
                terminal_projection_event_ids: projection.event_id.into_iter().collect(),
                startup_backfill_tasks,
            });
    let terminal_enqueued = terminal_enqueue.enqueued;
    if !terminal_enqueued {
        rollback_terminal_projection_before_enqueue(state, &persisted_record, &projection).await;
        let terminal_tombstone_cleared = state
            .proxy_runtime_invocations
            .clear_terminal_tombstone(&persisted_record.invoke_id, &persisted_record.occurred_at);
        warn!(
            invoke_id = %invoke_id,
            occurred_at = %persisted_record.occurred_at,
            enqueue_failed_by_class = "terminal_invocation",
            terminal_tombstone_cleared,
            durability_mode = terminal_enqueue.durability_mode.as_str(),
            journal_sequence = ?terminal_enqueue.journal_sequence,
            business_unblocked_record_write = true,
            record_flush_deferred_or_failed = "terminal_invocation_enqueue_failed",
            "terminal proxy capture record dropped by sqlite write controller"
        );
    } else {
        debug!(
            invoke_id = %invoke_id,
            terminal_record_enqueue_elapsed = enqueue_started.elapsed().as_millis() as u64,
            durability_mode = terminal_enqueue.durability_mode.as_str(),
            journal_sequence = ?terminal_enqueue.journal_sequence,
            journal_pending_records = terminal_enqueue.journal_pending_records,
            journal_pending_bytes = terminal_enqueue.journal_pending_bytes,
            business_unblocked_record_write = true,
            record_flush_deferred_or_failed = "terminal_invocation_enqueued_async",
            "terminal proxy capture record queued for sqlite write controller"
        );
    }
    #[cfg(test)]
    if terminal_enqueued && state.sqlite_batch_writer.auto_flush_terminal_for_test() {
        state
            .sqlite_batch_writer
            .flush_buffered_for_test(&state.pool)
            .await;
    }
    if terminal_enqueued {
        state
            .subscription_hub
            .publish_runtime_mutation(RuntimeMutation::invocation(
                &persisted_record,
                RuntimeMutationKind::TerminalCommitted,
            ));
        #[cfg(test)]
        broadcast_test_record_payload(state, &persisted_record);
    }
    if terminal_enqueued {
        schedule_dashboard_activity_live_snapshot(state);
        schedule_proxy_capture_follow_up_after_terminal_enqueue(
            state,
            &invoke_id,
            "runtime_terminal",
        );
    }

    Ok(())
}

pub(crate) async fn persist_proxy_capture_runtime_record(
    pool: &Pool<Sqlite>,
    record: ProxyCaptureRecord,
) -> Result<Option<ApiInvocation>> {
    let _write_permit = crate::proxy_sqlite_write_coordinator::proxy_sqlite_write_coordinator()
        .acquire(crate::proxy_sqlite_write_coordinator::ProxySqliteWriteClass::P1Terminal)
        .await;
    persist_proxy_capture_runtime_record_core(pool, record, true).await
}

pub(crate) async fn persist_proxy_capture_runtime_record_core(
    pool: &Pool<Sqlite>,
    record: ProxyCaptureRecord,
    write_derived_inline: bool,
) -> Result<Option<ApiInvocation>> {
    let mut tx = pool.begin().await?;
    let persisted =
        persist_proxy_capture_runtime_record_tx(tx.as_mut(), record, write_derived_inline).await?;
    tx.commit().await?;
    Ok(persisted)
}

pub(crate) async fn persist_proxy_capture_runtime_record_tx(
    tx: &mut SqliteConnection,
    record: ProxyCaptureRecord,
    write_derived_inline: bool,
) -> Result<Option<ApiInvocation>> {
    let raw_response = if record.response_body_preview_enabled {
        record.raw_response.clone()
    } else {
        String::new()
    };
    let resp_raw = if record.response_body_preview_enabled {
        record.resp_raw.clone()
    } else {
        RawPayloadMeta {
            path: None,
            size_bytes: record.resp_raw.size_bytes,
            truncated: record.resp_raw.truncated,
            truncated_reason: record.resp_raw.truncated_reason.clone(),
        }
    };
    let failure = resolve_failure_classification(
        Some(record.status.as_str()),
        record.error_message.as_deref(),
        record.failure_kind.as_deref(),
        None,
        None,
    );
    let failure_kind = failure.failure_kind.clone();
    let t_req_read_ms = nullable_runtime_timing_value(record.timings.t_req_read_ms);
    let t_req_parse_ms = nullable_runtime_timing_value(record.timings.t_req_parse_ms);
    let t_upstream_connect_ms = nullable_runtime_timing_value(record.timings.t_upstream_connect_ms);
    let t_upstream_ttfb_ms = nullable_runtime_timing_value(record.timings.t_upstream_ttfb_ms);
    let first_token_ms = record
        .timings
        .first_token_ms
        .filter(|value| value.is_finite() && *value >= 0.0);
    let core_write_started = Instant::now();
    let created_at = format_utc_iso_millis(Utc::now());
    let mut core_write_path = "insert_missing";
    let existing_identity =
        load_persisted_invocation_identity_tx(&mut *tx, &record.invoke_id, &record.occurred_at)
            .await?;
    if let Some(existing) = existing_identity.as_ref()
        && !persisted_invocation_allows_proxy_record_update(
            existing.status.as_deref(),
            existing.failure_kind.as_deref(),
            &record.status,
        )
    {
        return Ok(None);
    }

    if let Some(existing) = existing_identity.as_ref() {
        let updated = update_existing_proxy_invocation_record_tx(
            &mut *tx,
            existing.id,
            &record,
            &raw_response,
            &resp_raw,
            failure_kind.as_deref(),
            failure.failure_class.as_str(),
            failure.is_actionable,
            None,
            t_req_read_ms,
            t_req_parse_ms,
            t_upstream_connect_ms,
            t_upstream_ttfb_ms,
            first_token_ms,
            None,
            None,
            None,
        )
        .await?;
        if !updated {
            return Ok(None);
        }
        core_write_path = "update_existing";
    } else {
        let insert_result = sqlx::query(
            r#"
            INSERT OR IGNORE INTO codex_invocations (
                invoke_id,
                occurred_at,
                source,
                model,
                input_tokens,
                output_tokens,
                cache_input_tokens,
                reasoning_tokens,
                total_tokens,
                cost,
                cost_input,
                cost_cache_write,
                cost_cache_read,
                cost_output,
                cost_reasoning,
                cost_estimated,
                price_version,
                status,
                error_message,
                failure_kind,
                failure_class,
                is_actionable,
                payload,
                raw_response,
                request_raw_path,
                request_raw_codec,
                request_raw_size,
                request_raw_truncated,
                request_raw_truncated_reason,
                response_raw_path,
                response_raw_codec,
                response_raw_size,
                response_raw_truncated,
                response_raw_truncated_reason,
                t_total_ms,
                t_req_read_ms,
                t_req_parse_ms,
                t_upstream_connect_ms,
                t_upstream_ttfb_ms,
                first_token_ms,
                t_upstream_stream_ms,
                t_resp_parse_ms,
                t_persist_ms,
                created_at
            )
            VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19,
                ?20, ?21, ?22, ?23, ?24, ?25, ?26, ?27, ?28, ?29, ?30, ?31, ?32, ?33, ?34, ?35, ?36,
                ?37, ?38, ?39, ?40, ?41, ?42, ?43, ?44
            )
            "#,
        )
        .bind(&record.invoke_id)
        .bind(&record.occurred_at)
        .bind(SOURCE_PROXY)
        .bind(&record.model)
        .bind(record.usage.input_tokens)
        .bind(record.usage.output_tokens)
        .bind(record.usage.cache_input_tokens)
        .bind(record.usage.reasoning_tokens)
        .bind(record.usage.total_tokens)
        .bind(record.cost)
        .bind(record.cost_breakdown.map(|value| value.input))
        .bind(record.cost_breakdown.map(|value| value.cache_write))
        .bind(record.cost_breakdown.map(|value| value.cache_read))
        .bind(record.cost_breakdown.map(|value| value.output))
        .bind(record.cost_breakdown.map(|value| value.reasoning))
        .bind(record.cost_estimated as i64)
        .bind(record.price_version.as_deref())
        .bind(&record.status)
        .bind(record.error_message.as_deref())
        .bind(failure_kind.as_deref())
        .bind(failure.failure_class.as_str())
        .bind(failure.is_actionable as i64)
        .bind(record.payload.as_deref())
        .bind(&raw_response)
        .bind(record.req_raw.path.as_deref())
        .bind(raw_payload_meta_codec(&record.req_raw))
        .bind(record.req_raw.size_bytes)
        .bind(record.req_raw.truncated as i64)
        .bind(record.req_raw.truncated_reason.as_deref())
        .bind(resp_raw.path.as_deref())
        .bind(raw_payload_meta_codec(&resp_raw))
        .bind(resp_raw.size_bytes)
        .bind(resp_raw.truncated as i64)
        .bind(resp_raw.truncated_reason.as_deref())
        .bind(None::<f64>)
        .bind(t_req_read_ms)
        .bind(t_req_parse_ms)
        .bind(t_upstream_connect_ms)
        .bind(t_upstream_ttfb_ms)
        .bind(first_token_ms)
        .bind(None::<f64>)
        .bind(None::<f64>)
        .bind(None::<f64>)
        .bind(created_at)
        .execute(&mut *tx)
        .await?;
        if insert_result.rows_affected() == 0 {
            let Some(existing) = load_persisted_invocation_identity_tx(
                &mut *tx,
                &record.invoke_id,
                &record.occurred_at,
            )
            .await?
            else {
                return Ok(None);
            };
            if !persisted_invocation_allows_proxy_record_update(
                existing.status.as_deref(),
                existing.failure_kind.as_deref(),
                &record.status,
            ) {
                return Ok(None);
            }
            let updated = update_existing_proxy_invocation_record_tx(
                &mut *tx,
                existing.id,
                &record,
                &raw_response,
                &resp_raw,
                failure_kind.as_deref(),
                failure.failure_class.as_str(),
                failure.is_actionable,
                None,
                t_req_read_ms,
                t_req_parse_ms,
                t_upstream_connect_ms,
                t_upstream_ttfb_ms,
                first_token_ms,
                None,
                None,
                None,
            )
            .await?;
            if !updated {
                return Ok(None);
            }
            core_write_path = "update_race";
        }
    }

    let persisted_identity =
        load_persisted_invocation_identity_tx(&mut *tx, &record.invoke_id, &record.occurred_at)
            .await?
            .ok_or_else(|| {
                anyhow!("persisted proxy runtime invocation row disappeared after upsert")
            })?;
    if write_derived_inline {
        upsert_invocation_hourly_rollups_tx(
            &mut *tx,
            &[InvocationHourlySourceRecord {
                id: persisted_identity.id,
                occurred_at: record.occurred_at.clone(),
                source: SOURCE_PROXY.to_string(),
                status: Some(record.status.clone()),
                detail_level: DETAIL_LEVEL_FULL.to_string(),
                model: record.model.clone(),
                input_tokens: record.usage.input_tokens,
                output_tokens: record.usage.output_tokens,
                cache_input_tokens: record.usage.cache_input_tokens,
                reasoning_tokens: record.usage.reasoning_tokens,
                total_tokens: record.usage.total_tokens,
                cost: record.cost,
                upstream_account_id: crate::proxy::upstream_account_id_from_payload(
                    record.payload.as_deref(),
                ),
                cost_input: record.cost_breakdown.map(|value| value.input),
                cost_cache_write: record.cost_breakdown.map(|value| value.cache_write),
                cost_cache_read: record.cost_breakdown.map(|value| value.cache_read),
                cost_output: record.cost_breakdown.map(|value| value.output),
                cost_reasoning: record.cost_breakdown.map(|value| value.reasoning),
                error_message: record.error_message.clone(),
                failure_kind: failure_kind.clone(),
                failure_class: Some(failure.failure_class.as_str().to_string()),
                is_actionable: Some(failure.is_actionable as i64),
                payload: record.payload.clone(),
                t_total_ms: None,
                t_req_read_ms,
                t_req_parse_ms,
                t_upstream_connect_ms,
                t_upstream_ttfb_ms,
                first_token_ms,
                t_upstream_stream_ms: None,
                t_resp_parse_ms: None,
                t_persist_ms: None,
            }],
            &INVOCATION_HOURLY_ROLLUP_TARGETS,
        )
        .await?;
        save_hourly_rollup_live_progress_tx(
            &mut *tx,
            HOURLY_ROLLUP_DATASET_INVOCATIONS,
            persisted_identity.id,
        )
        .await?;
        touch_invocation_upstream_account_last_activity_tx(
            &mut *tx,
            &record.occurred_at,
            record.payload.as_deref(),
        )
        .await?;
    }

    let persisted =
        load_persisted_api_invocation_tx(&mut *tx, &record.invoke_id, &record.occurred_at).await?;

    let core_write_elapsed_ms = core_write_started.elapsed().as_millis() as u64;
    if core_write_elapsed_ms >= 1_000 {
        warn!(
            invoke_id = %record.invoke_id,
            status = %record.status,
            core_write_path,
            request_raw_bytes = record.req_raw.size_bytes,
            response_raw_bytes = resp_raw.size_bytes,
            has_request_raw_path = record.req_raw.path.is_some(),
            has_response_raw_path = resp_raw.path.is_some(),
            elapsed_ms = core_write_elapsed_ms,
            "proxy capture core invocation write was slow"
        );
    } else {
        debug!(
            invoke_id = %record.invoke_id,
            status = %record.status,
            core_write_path,
            request_raw_bytes = record.req_raw.size_bytes,
            response_raw_bytes = resp_raw.size_bytes,
            has_request_raw_path = record.req_raw.path.is_some(),
            has_response_raw_path = resp_raw.path.is_some(),
            elapsed_ms = core_write_elapsed_ms,
            "proxy capture core invocation write completed"
        );
    }

    Ok(Some(persisted))
}

pub(crate) fn build_running_proxy_capture_record(
    invoke_id: &str,
    occurred_at: &str,
    target: ProxyCaptureTarget,
    request_info: &RequestCaptureInfo,
    requester_ip: Option<&str>,
    sticky_key: Option<&str>,
    prompt_cache_key: Option<&str>,
    pool_route_active: bool,
    upstream_account_id: Option<i64>,
    upstream_account_name: Option<&str>,
    upstream_account_kind: Option<&str>,
    upstream_base_url_host: Option<&str>,
    proxy_display_name: Option<&str>,
    pool_attempt_count: Option<usize>,
    pool_distinct_account_count: Option<usize>,
    pool_attempt_terminal_reason: Option<&str>,
    response_content_encoding: Option<&str>,
    t_req_read_ms: f64,
    t_req_parse_ms: f64,
    t_upstream_connect_ms: f64,
    t_upstream_ttfb_ms: f64,
) -> ProxyCaptureRecord {
    ProxyCaptureRecord {
        invoke_id: invoke_id.to_string(),
        occurred_at: occurred_at.to_string(),
        model: request_info.model.clone(),
        usage: ParsedUsage::default(),
        cost: None,
        cost_breakdown: None,
        cost_estimated: false,
        price_version: None,
        status: "running".to_string(),
        error_message: None,
        failure_kind: None,
        payload: Some(build_proxy_payload_summary(ProxyPayloadSummary {
            target,
            status: StatusCode::OK,
            is_stream: request_info.is_stream,
            request_contains_encrypted_content: request_info.contains_encrypted_content,
            response_contains_encrypted_content: false,
            compaction_request_kind: request_info.compaction_request_kind,
            compaction_response_kind: None,
            image_intent: request_info.image_intent.as_deref(),
            request_model: request_info.model.as_deref(),
            requested_service_tier: request_info.requested_service_tier.as_deref(),
            billing_service_tier: None,
            reasoning_effort: request_info.reasoning_effort.as_deref(),
            response_model: None,
            usage_missing_reason: None,
            request_parse_error: request_info.parse_error.as_deref(),
            request_compression_algorithm: None,
            request_compression_mode: None,
            request_compression_logical_body_bytes: None,
            request_compression_transmitted_body_bytes: None,
            request_compression_transmission_complete: None,
            failure_kind: None,
            requester_ip,
            request_user_agent: None,
            request_x_forwarded_for: None,
            request_forwarded: None,
            request_x_real_ip: None,
            upstream_scope: if pool_route_active {
                INVOCATION_UPSTREAM_SCOPE_INTERNAL
            } else {
                INVOCATION_UPSTREAM_SCOPE_EXTERNAL
            },
            route_mode: if pool_route_active {
                INVOCATION_ROUTE_MODE_POOL
            } else {
                INVOCATION_ROUTE_MODE_FORWARD_PROXY
            },
            sticky_key,
            prompt_cache_key,
            prompt_cache_key_attribution_source: request_info
                .prompt_cache_key_attribution_source
                .as_deref(),
            client_fingerprint: None,
            client_header_fingerprints: None,
            upstream_account_id,
            upstream_account_name,
            upstream_account_kind,
            upstream_base_url_host,
            oauth_account_header_attached: None,
            oauth_account_id_shape: None,
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
            service_tier: None,
            stream_terminal_event: None,
            upstream_error_code: None,
            upstream_error_message: None,
            downstream_status_code: None,
            downstream_error_message: None,
            upstream_request_id: None,
            response_content_encoding,
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
            proxy_display_name,
            proxy_weight_delta: None,
            pool_attempt_count,
            pool_distinct_account_count,
            pool_attempt_terminal_reason,
            blocked_binding: None,
        })),
        raw_response: "{}".to_string(),
        response_body_preview_enabled: false,
        req_raw: RawPayloadMeta::default(),
        resp_raw: RawPayloadMeta::default(),
        timings: StageTimings {
            t_total_ms: 0.0,
            t_req_read_ms,
            t_req_parse_ms,
            t_upstream_connect_ms,
            t_upstream_ttfb_ms,
            first_token_ms: None,
            t_upstream_stream_ms: 0.0,
            t_resp_parse_ms: 0.0,
            t_persist_ms: 0.0,
        },
    }
}

pub(crate) fn build_admitted_proxy_capture_runtime_snapshot(
    invoke_id: &str,
    occurred_at: &str,
    target: ProxyCaptureTarget,
    requester_ip: Option<&str>,
    sticky_key: Option<&str>,
    prompt_cache_key: Option<&str>,
) -> ProxyCaptureRecord {
    let request_info = RequestCaptureInfo {
        sticky_key: sticky_key.map(ToOwned::to_owned),
        prompt_cache_key: prompt_cache_key.map(ToOwned::to_owned),
        prompt_cache_key_attribution_source: prompt_cache_key.map(|_| "request".to_string()),
        ..RequestCaptureInfo::default()
    };
    build_running_proxy_capture_record(
        invoke_id,
        occurred_at,
        target,
        &request_info,
        requester_ip,
        sticky_key,
        prompt_cache_key,
        true,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        0.0,
        0.0,
        0.0,
        0.0,
    )
}

pub(crate) fn resolve_invocation_proxy_display_name(
    selected_proxy: Option<&SelectedForwardProxy>,
) -> Option<String> {
    selected_proxy.map(|proxy| proxy.display_name.clone())
}

pub(crate) fn summarize_response_content_encoding(content_encoding: Option<&str>) -> String {
    let encodings = parse_content_encodings(content_encoding);
    if encodings.is_empty() {
        "identity".to_string()
    } else {
        encodings.join(", ")
    }
}

#[derive(Default)]
pub(crate) struct RawResponsePreviewBuffer {
    bytes: Vec<u8>,
}

impl RawResponsePreviewBuffer {
    pub(crate) fn append(&mut self, chunk: &[u8]) {
        let remaining = RAW_RESPONSE_PREVIEW_LIMIT.saturating_sub(self.bytes.len());
        if remaining == 0 || chunk.is_empty() {
            return;
        }
        self.bytes
            .extend_from_slice(&chunk[..chunk.len().min(remaining)]);
    }

    pub(crate) fn as_slice(&self) -> &[u8] {
        &self.bytes
    }

    pub(crate) fn into_preview(self) -> String {
        build_raw_response_preview(&self.bytes)
    }
}

pub(crate) struct BoundedResponseParseBuffer {
    bytes: Vec<u8>,
    limit: usize,
    exceeded_limit: bool,
}

impl BoundedResponseParseBuffer {
    pub(crate) fn new(limit: usize) -> Self {
        Self {
            bytes: Vec::new(),
            limit,
            exceeded_limit: false,
        }
    }

    pub(crate) fn append(&mut self, chunk: &[u8]) {
        if self.exceeded_limit || chunk.is_empty() {
            return;
        }

        let remaining = self.limit.saturating_sub(self.bytes.len());
        let take_len = remaining.min(chunk.len());
        if take_len > 0 {
            self.bytes.extend_from_slice(&chunk[..take_len]);
        }
        if take_len < chunk.len() {
            self.exceeded_limit = true;
        }
    }

    pub(crate) fn into_response_info(
        self,
        target: ProxyCaptureTarget,
        content_encoding: Option<&str>,
    ) -> ResponseCaptureInfo {
        let mut response_info =
            parse_target_response_payload(target, &self.bytes, false, content_encoding);
        if self.exceeded_limit {
            merge_response_capture_reason(
                &mut response_info,
                PROXY_USAGE_MISSING_NON_STREAM_PARSE_SKIPPED,
            );
        }
        response_info
    }
}

pub(crate) enum PendingRawPayloadWrite {
    Ready(RawPayloadMeta),
    Task(JoinHandle<RawPayloadMeta>),
}

impl PendingRawPayloadWrite {
    pub(crate) async fn finish(self) -> RawPayloadMeta {
        match self {
            Self::Ready(meta) => meta,
            Self::Task(handle) => match handle.await {
                Ok(meta) => meta,
                Err(err) => RawPayloadMeta {
                    path: None,
                    size_bytes: 0,
                    truncated: true,
                    truncated_reason: Some(format!("write_failed:{err}")),
                },
            },
        }
    }
}

pub(crate) fn spawn_raw_payload_file_write(
    state: &AppState,
    invoke_id: &str,
    kind: &'static str,
    bytes: Bytes,
    enabled: bool,
) -> PendingRawPayloadWrite {
    if bytes.is_empty() {
        return PendingRawPayloadWrite::Ready(RawPayloadMeta::default());
    }
    if !enabled {
        return PendingRawPayloadWrite::Ready(RawPayloadMeta {
            path: None,
            size_bytes: bytes.len() as i64,
            truncated: false,
            truncated_reason: None,
        });
    }

    let semaphore = state.proxy_raw_async_semaphore.clone();
    let invoke_id = invoke_id.to_string();
    let kind_for_spool = kind;
    let bytes_for_spool = bytes.clone();
    if semaphore.available_permits() == 0 {
        let codec = state.config.proxy_raw_compression;
        let spool = match RawOverflowSpool::create(state, &invoke_id, kind_for_spool, codec) {
            Ok(spool) => spool,
            Err(err) => {
                warn!(
                    capture_path = "capture_unavailable",
                    capture_unavailable_reason = "spool_capacity",
                    error = %err,
                    "raw capture unavailable because the durable spool cannot accept it"
                );
                return PendingRawPayloadWrite::Ready(RawPayloadMeta {
                    path: None,
                    size_bytes: bytes_for_spool.len() as i64,
                    truncated: true,
                    truncated_reason: Some("capture_unavailable:spool_capacity".to_string()),
                });
            }
        };
        return PendingRawPayloadWrite::Task(tokio::spawn(async move {
            let mut spool = spool;
            if let Err(err) = spool.append(&bytes_for_spool) {
                return RawPayloadMeta {
                    path: None,
                    size_bytes: bytes_for_spool.len() as i64,
                    truncated: true,
                    truncated_reason: Some(if err.to_string().contains("capacity") {
                        "capture_unavailable:spool_capacity".to_string()
                    } else {
                        "capture_unavailable:spool_write_failed".to_string()
                    }),
                };
            }
            spool.finish(bytes_for_spool.len() as i64).await
        }));
    }

    let config = state.config.clone();
    PendingRawPayloadWrite::Task(tokio::spawn(async move {
        // Queue behind the bounded CPU writer pool instead of dropping an enabled capture.
        let permit = semaphore
            .acquire_owned()
            .await
            .expect("raw writer semaphore is live");
        let _permit = permit;
        store_raw_payload_file(&config, &invoke_id, kind, bytes).await
    }))
}

pub(crate) fn spawn_raw_payload_snapshot_write(
    state: Arc<AppState>,
    invoke_id: &str,
    kind: &'static str,
    snapshot: PoolReplayBodySnapshot,
    enabled: bool,
) -> PendingRawPayloadWrite {
    match snapshot {
        PoolReplayBodySnapshot::Empty => PendingRawPayloadWrite::Ready(RawPayloadMeta::default()),
        PoolReplayBodySnapshot::Memory(bytes) => {
            spawn_raw_payload_file_write(state.as_ref(), invoke_id, kind, bytes, enabled)
        }
        PoolReplayBodySnapshot::File { size, .. } if !enabled => {
            PendingRawPayloadWrite::Ready(RawPayloadMeta {
                path: None,
                size_bytes: size as i64,
                truncated: false,
                truncated_reason: None,
            })
        }
        PoolReplayBodySnapshot::File { temp_file, size } => {
            let config = state.config.clone();
            let semaphore = state.proxy_raw_async_semaphore.clone();
            let invoke_id = invoke_id.to_string();
            let source_path = temp_file.path.clone();
            PendingRawPayloadWrite::Task(tokio::spawn(async move {
                let _temp_file_guard = temp_file;
                let _permit = semaphore
                    .acquire_owned()
                    .await
                    .expect("raw writer semaphore is live");
                store_raw_payload_snapshot_file(&config, &invoke_id, kind, source_path, size).await
            }))
        }
    }
}

pub(crate) fn raw_payload_path_for_kind(
    raw_dir: &Path,
    invoke_id: &str,
    kind: &str,
    gzip: bool,
) -> PathBuf {
    let filename = if gzip {
        format!("{invoke_id}-{kind}.bin.gz")
    } else {
        format!("{invoke_id}-{kind}.bin")
    };
    raw_dir.join(filename)
}

pub(crate) fn raw_payload_zstd_path(path: &Path) -> PathBuf {
    PathBuf::from(format!("{}.zst", path.display()))
}

pub(crate) fn raw_payload_path_is_gzip(path: Option<&str>) -> bool {
    path.is_some_and(|value| value.ends_with(".gz"))
}

pub(crate) fn raw_payload_meta_codec(meta: &RawPayloadMeta) -> &'static str {
    if raw_payload_path_is_gzip(meta.path.as_deref()) {
        RAW_CODEC_GZIP
    } else if meta
        .path
        .as_deref()
        .is_some_and(|path| path.ends_with(".zst"))
    {
        RAW_CODEC_ZSTD
    } else {
        RAW_CODEC_IDENTITY
    }
}

pub(crate) fn compress_raw_payload_bytes_to_gzip(bytes: &[u8]) -> io::Result<Vec<u8>> {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(bytes)?;
    encoder.finish()
}

pub(crate) fn raw_payload_gzip_path(path: &Path) -> PathBuf {
    PathBuf::from(format!("{}.gz", path.display()))
}

pub(crate) enum StreamingRawPayloadWriterState {
    Buffer(Vec<u8>),
    Plain {
        path: PathBuf,
        file: fs::File,
    },
    Gzip {
        path: PathBuf,
        encoder: GzEncoder<io::BufWriter<fs::File>>,
    },
    Zstd {
        path: PathBuf,
        encoder: zstd::stream::write::Encoder<'static, io::BufWriter<fs::File>>,
    },
}

impl StreamingRawPayloadWriterState {
    fn current_path(&self) -> Option<&Path> {
        match self {
            Self::Buffer(_) => None,
            Self::Plain { path, .. } | Self::Gzip { path, .. } | Self::Zstd { path, .. } => {
                Some(path.as_path())
            }
        }
    }
}

pub(crate) fn prepare_streaming_raw_parent(path: &Path) -> io::Result<&Path> {
    let parent = path.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("raw payload path has no parent: {}", path.display()),
        )
    })?;
    fs::create_dir_all(parent)?;
    Ok(parent)
}

pub(crate) fn create_plain_streaming_raw_file(path: &Path) -> io::Result<fs::File> {
    prepare_streaming_raw_parent(path)?;
    fs::File::create(path)
}

pub(crate) fn create_gzip_streaming_raw_encoder(
    path: &Path,
) -> io::Result<GzEncoder<io::BufWriter<fs::File>>> {
    prepare_streaming_raw_parent(path)?;
    let file = fs::File::create(path)?;
    Ok(GzEncoder::new(
        io::BufWriter::new(file),
        Compression::default(),
    ))
}

pub(crate) fn create_zstd_streaming_raw_encoder(
    path: &Path,
) -> io::Result<zstd::stream::write::Encoder<'static, io::BufWriter<fs::File>>> {
    prepare_streaming_raw_parent(path)?;
    let file = fs::File::create(path)?;
    zstd::stream::write::Encoder::new(io::BufWriter::new(file), 0)
}

pub(crate) async fn run_blocking_raw_writer_io<T, F>(op: F) -> io::Result<T>
where
    T: Send + 'static,
    F: FnOnce() -> io::Result<T> + Send + 'static,
{
    tokio::task::spawn_blocking(op)
        .await
        .map_err(|err| io::Error::other(format!("raw writer task join failed: {err}")))?
}

const RAW_OVERFLOW_SPOOL_DIR: &str = ".spool";
const RAW_OVERFLOW_SPOOL_MAGIC: &[u8] = b"CVM_RAW_SPOOL_V1\n";
pub(crate) const RAW_OVERFLOW_SPOOL_SEGMENT_BYTES: u64 = 16 * 1024 * 1024;
pub(crate) const RAW_OVERFLOW_SPOOL_MAX_BYTES: u64 = 512 * 1024 * 1024;
const RAW_OVERFLOW_SPOOL_FRAME_OVERHEAD_BYTES: u64 = 8;

static RAW_OVERFLOW_SPOOL_RESERVATIONS: std::sync::LazyLock<
    std::sync::Mutex<std::collections::HashMap<PathBuf, u64>>,
> = std::sync::LazyLock::new(|| std::sync::Mutex::new(std::collections::HashMap::new()));

struct StreamingRawPayloadChunk {
    bytes: Bytes,
}

impl StreamingRawPayloadChunk {
    fn unreserved(bytes: Bytes) -> Self {
        Self { bytes }
    }
}

impl std::ops::Deref for StreamingRawPayloadChunk {
    type Target = Bytes;

    fn deref(&self) -> &Self::Target {
        &self.bytes
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct RawOverflowSpoolHeader {
    invoke_id: String,
    kind: String,
    codec: RawCompressionCodec,
    #[serde(default)]
    capture_id: Option<String>,
    #[serde(default)]
    segment_index: u32,
}

struct RawOverflowSpool {
    directory: PathBuf,
    capture_id: String,
    segment_index: u32,
    segment_payload_bytes: u64,
    paths: Vec<PathBuf>,
    file: fs::File,
    config: AppConfig,
    invoke_id: String,
    kind: &'static str,
    semaphore: Arc<Semaphore>,
    pending_records: u64,
    pending_bytes: u64,
    payload_limit_bytes: u64,
    payload_bytes: u64,
    exceeded_payload_limit: bool,
    truncation_reason: Option<&'static str>,
}

impl RawOverflowSpool {
    fn create(
        state: &AppState,
        invoke_id: &str,
        kind: &'static str,
        codec: RawCompressionCodec,
    ) -> io::Result<Self> {
        let directory = state
            .config
            .resolved_proxy_raw_dir()
            .join(RAW_OVERFLOW_SPOOL_DIR);
        fs::create_dir_all(&directory)?;
        let payload_limit_bytes = state
            .config
            .proxy_raw_max_bytes
            .map(|limit| u64::try_from(limit).unwrap_or(u64::MAX))
            .unwrap_or(u64::MAX);
        let capture_id = nanoid::nanoid!();
        let (path, file) =
            create_raw_overflow_spool_segment(&directory, invoke_id, kind, codec, &capture_id, 0)?;
        let mut config = state.config.clone();
        config.proxy_raw_compression = codec;
        Ok(Self {
            directory,
            capture_id,
            segment_index: 0,
            segment_payload_bytes: 0,
            paths: vec![path],
            file,
            config,
            invoke_id: invoke_id.to_string(),
            kind,
            semaphore: state.proxy_raw_async_semaphore.clone(),
            pending_records: 0,
            pending_bytes: 0,
            payload_limit_bytes,
            payload_bytes: 0,
            exceeded_payload_limit: false,
            truncation_reason: None,
        })
    }

    fn append(&mut self, bytes: &[u8]) -> io::Result<()> {
        let remaining_capacity = self.payload_limit_bytes.saturating_sub(self.payload_bytes);
        let accepted_len = remaining_capacity.min(bytes.len() as u64) as usize;
        if accepted_len < bytes.len() {
            self.exceeded_payload_limit = true;
            self.truncation_reason = Some("max_bytes_exceeded");
        }
        let mut remaining = &bytes[..accepted_len];
        while !remaining.is_empty() {
            if self.segment_payload_bytes == RAW_OVERFLOW_SPOOL_SEGMENT_BYTES
                && let Err(error) = self.rotate_segment()
            {
                if error.kind() == io::ErrorKind::WouldBlock {
                    self.mark_spool_capacity_exceeded();
                    break;
                }
                return Err(error);
            }
            let writable = (RAW_OVERFLOW_SPOOL_SEGMENT_BYTES - self.segment_payload_bytes) as usize;
            let frame = &remaining[..remaining.len().min(writable)];
            let reservation_bytes =
                (frame.len() as u64).saturating_add(RAW_OVERFLOW_SPOOL_FRAME_OVERHEAD_BYTES);
            if let Err(error) = reserve_raw_overflow_spool_bytes(&self.directory, reservation_bytes)
            {
                if error.kind() == io::ErrorKind::WouldBlock {
                    self.mark_spool_capacity_exceeded();
                    break;
                }
                return Err(error);
            }
            let write_result = write_raw_overflow_spool_frame(&mut self.file, frame);
            release_raw_overflow_spool_bytes(&self.directory, reservation_bytes);
            write_result?;
            self.segment_payload_bytes = self
                .segment_payload_bytes
                .saturating_add(frame.len() as u64);
            self.pending_records = self.pending_records.saturating_add(1);
            self.pending_bytes = self.pending_bytes.saturating_add(frame.len() as u64);
            self.payload_bytes = self.payload_bytes.saturating_add(frame.len() as u64);
            remaining = &remaining[frame.len()..];
        }
        Ok(())
    }

    fn mark_spool_capacity_exceeded(&mut self) {
        self.exceeded_payload_limit = true;
        self.truncation_reason = Some("spool_capacity_exceeded");
        // A raw capture is useful only when it is complete. Leave an invalid trailing
        // frame marker so restart recovery retains this evidence instead of publishing
        // the durable prefix as if it were a complete response.
        let _ = self.file.write_all(&[0]);
        let _ = self.file.flush();
    }

    fn rotate_segment(&mut self) -> io::Result<()> {
        self.file.flush()?;
        self.segment_index = self.segment_index.saturating_add(1);
        let (path, file) = create_raw_overflow_spool_segment(
            &self.directory,
            &self.invoke_id,
            self.kind,
            self.config.proxy_raw_compression,
            &self.capture_id,
            self.segment_index,
        )?;
        self.paths.push(path);
        self.file = file;
        self.segment_payload_bytes = 0;
        Ok(())
    }

    async fn finish(mut self, observed_size_bytes: i64) -> RawPayloadMeta {
        if self.truncation_reason == Some("spool_capacity_exceeded") {
            warn!(
                capture_path = "overflow_spool",
                durability_mode = "rejected_at_capacity",
                spool_pending_bytes = self.pending_bytes,
                spool_segment_count = self.paths.len(),
                "raw capture exceeded the durable overflow spool budget; retaining evidence"
            );
            return RawPayloadMeta {
                path: None,
                size_bytes: observed_size_bytes,
                truncated: true,
                truncated_reason: Some("spool_capacity_exceeded".to_string()),
            };
        }
        if let Err(err) = self.file.flush() {
            return RawPayloadMeta {
                path: None,
                size_bytes: observed_size_bytes,
                truncated: true,
                truncated_reason: Some(format!("spool_write_failed:{err}")),
            };
        }
        let mut meta = replay_raw_overflow_spool_segments(
            &self.config,
            self.semaphore.clone(),
            self.paths.clone(),
        )
        .await;
        debug!(
            capture_path = "overflow_spool",
            storage_codec = ?self.config.proxy_raw_compression,
            spool_pending_records = self.pending_records,
            spool_pending_bytes = self.pending_bytes,
            spool_segment_count = self.paths.len(),
            "raw capture overflow spool finished"
        );
        if meta.path.is_some() || meta.truncated_reason.as_deref() == Some("max_bytes_exceeded") {
            remove_raw_overflow_spool_segments(&self.paths);
        }
        if self.exceeded_payload_limit {
            meta.truncated = true;
            meta.truncated_reason.get_or_insert_with(|| {
                self.truncation_reason
                    .unwrap_or_else(|| {
                        raw_overflow_payload_limit_reason(
                            self.config.proxy_raw_max_bytes,
                            self.payload_limit_bytes,
                        )
                    })
                    .to_string()
            });
        }
        meta
    }
}

fn raw_overflow_payload_limit_reason(
    configured_max_bytes: Option<usize>,
    payload_limit_bytes: u64,
) -> &'static str {
    if configured_max_bytes
        .is_some_and(|limit| u64::try_from(limit).unwrap_or(u64::MAX) <= payload_limit_bytes)
    {
        "max_bytes_exceeded"
    } else {
        "spool_capacity_exceeded"
    }
}

fn raw_overflow_spool_directory_bytes(directory: &Path) -> io::Result<u64> {
    let mut total = 0_u64;
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        if entry.file_type()?.is_file()
            && entry.path().extension().and_then(|value| value.to_str()) == Some("frames")
        {
            total = total.saturating_add(entry.metadata()?.len());
        }
    }
    Ok(total)
}

fn reserve_raw_overflow_spool_bytes(directory: &Path, bytes: u64) -> io::Result<()> {
    let mut reservations = RAW_OVERFLOW_SPOOL_RESERVATIONS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    // Keep the disk sample and the in-process reservation in one critical section. A
    // concurrent writer may otherwise observe a stale directory size after another
    // writer commits and releases its temporary reservation.
    let on_disk_bytes = raw_overflow_spool_directory_bytes(directory)?;
    let reserved_bytes = reservations.get(directory).copied().unwrap_or_default();
    if on_disk_bytes
        .saturating_add(reserved_bytes)
        .saturating_add(bytes)
        > RAW_OVERFLOW_SPOOL_MAX_BYTES
    {
        return Err(io::Error::new(
            io::ErrorKind::WouldBlock,
            "raw overflow spool capacity reached",
        ));
    }
    reservations.insert(
        directory.to_path_buf(),
        reserved_bytes.saturating_add(bytes),
    );
    Ok(())
}

fn release_raw_overflow_spool_bytes(directory: &Path, bytes: u64) {
    if bytes == 0 {
        return;
    }
    let mut reservations = RAW_OVERFLOW_SPOOL_RESERVATIONS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let Some(reserved_bytes) = reservations.get_mut(directory) else {
        return;
    };
    *reserved_bytes = reserved_bytes.saturating_sub(bytes);
    if *reserved_bytes == 0 {
        reservations.remove(directory);
    }
}

fn create_raw_overflow_spool_segment(
    directory: &Path,
    invoke_id: &str,
    kind: &str,
    codec: RawCompressionCodec,
    capture_id: &str,
    segment_index: u32,
) -> io::Result<(PathBuf, fs::File)> {
    let path = directory.join(format!("{capture_id}-{segment_index:06}.frames"));
    let header = RawOverflowSpoolHeader {
        invoke_id: invoke_id.to_string(),
        kind: kind.to_string(),
        codec,
        capture_id: Some(capture_id.to_string()),
        segment_index,
    };
    let header_bytes = raw_overflow_spool_header_bytes(&header)?;
    let reserved_bytes = (RAW_OVERFLOW_SPOOL_MAGIC.len() as u64)
        .saturating_add(4)
        .saturating_add(header_bytes.len() as u64);
    reserve_raw_overflow_spool_bytes(directory, reserved_bytes)?;
    let mut file = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&path);
    let write_result = match &mut file {
        Ok(file) => write_raw_overflow_spool_header_bytes(file, &header_bytes),
        Err(error) => Err(io::Error::new(error.kind(), error.to_string())),
    };
    release_raw_overflow_spool_bytes(directory, reserved_bytes);
    if let Err(error) = write_result {
        let _ = fs::remove_file(&path);
        return Err(error);
    }
    let file = file?;
    Ok((path, file))
}

fn raw_overflow_spool_header_bytes(header: &RawOverflowSpoolHeader) -> io::Result<Vec<u8>> {
    serde_json::to_vec(header).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
}

fn write_raw_overflow_spool_header_bytes(file: &mut fs::File, header: &[u8]) -> io::Result<()> {
    let header_len = u32::try_from(header.len())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "raw spool header too large"))?;
    file.write_all(RAW_OVERFLOW_SPOOL_MAGIC)?;
    file.write_all(&header_len.to_le_bytes())?;
    file.write_all(header)
}

fn write_raw_overflow_spool_frame(file: &mut fs::File, bytes: &[u8]) -> io::Result<()> {
    let length = u32::try_from(bytes.len())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "raw spool frame too large"))?;
    let mut crc = Crc32Hasher::new();
    crc.update(bytes);
    file.write_all(&length.to_le_bytes())?;
    file.write_all(&crc.finalize().to_le_bytes())?;
    file.write_all(bytes)
}

fn read_raw_overflow_spool_segment(path: &Path) -> io::Result<(RawOverflowSpoolHeader, Vec<u8>)> {
    let bytes = fs::read(path)?;
    if !bytes.starts_with(RAW_OVERFLOW_SPOOL_MAGIC) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "raw spool has invalid header magic",
        ));
    }
    let mut offset = RAW_OVERFLOW_SPOOL_MAGIC.len();
    if bytes.len().saturating_sub(offset) < 4 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "raw spool has partial header length",
        ));
    }
    let header_len =
        u32::from_le_bytes(bytes[offset..offset + 4].try_into().expect("header length")) as usize;
    offset += 4;
    let header_end = offset.saturating_add(header_len);
    if header_end > bytes.len() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "raw spool has partial header",
        ));
    }
    let header = serde_json::from_slice(&bytes[offset..header_end])
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
    offset = header_end;
    let mut payload = Vec::new();
    while offset < bytes.len() {
        if bytes.len().saturating_sub(offset) < 8 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "raw spool has partial frame header",
            ));
        }
        let length = u32::from_le_bytes(bytes[offset..offset + 4].try_into().expect("frame length"))
            as usize;
        let expected_crc =
            u32::from_le_bytes(bytes[offset + 4..offset + 8].try_into().expect("frame crc"));
        offset += 8;
        let end = offset.saturating_add(length);
        if end > bytes.len() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "raw spool has partial frame payload",
            ));
        }
        let frame = &bytes[offset..end];
        let mut crc = Crc32Hasher::new();
        crc.update(frame);
        if crc.finalize() != expected_crc {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "raw spool frame checksum mismatch",
            ));
        }
        payload.extend_from_slice(frame);
        offset = end;
    }
    Ok((header, payload))
}

fn raw_overflow_spool_capture_key(path: &Path, header: &RawOverflowSpoolHeader) -> String {
    header
        .capture_id
        .clone()
        .unwrap_or_else(|| format!("legacy:{}", path.display()))
}

fn raw_overflow_spool_capture_key_from_path(path: &Path) -> Option<String> {
    let stem = path.file_stem()?.to_str()?;
    let (capture_id, segment_index) = stem.rsplit_once('-')?;
    (!capture_id.is_empty()
        && segment_index.len() == 6
        && segment_index.bytes().all(|byte| byte.is_ascii_digit()))
    .then(|| capture_id.to_string())
}

fn validate_raw_overflow_spool_segments(
    segments: &[(PathBuf, RawOverflowSpoolHeader)],
) -> io::Result<RawOverflowSpoolHeader> {
    let Some((_, first)) = segments.first() else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "raw spool capture has no segments",
        ));
    };
    let capture_id = raw_overflow_spool_capture_key(&segments[0].0, first);
    for (expected_index, (path, header)) in segments.iter().enumerate() {
        if header.invoke_id != first.invoke_id
            || header.kind != first.kind
            || header.codec != first.codec
            || raw_overflow_spool_capture_key(path, header) != capture_id
            || header
                .capture_id
                .as_ref()
                .is_some_and(|_| header.segment_index != expected_index as u32)
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "raw spool capture segments are inconsistent",
            ));
        }
    }
    Ok(first.clone())
}

async fn replay_raw_overflow_spool_segments(
    config: &AppConfig,
    semaphore: Arc<Semaphore>,
    paths: Vec<PathBuf>,
) -> RawPayloadMeta {
    let inspected = match run_blocking_raw_writer_io({
        let paths = paths.clone();
        move || {
            let mut segments = Vec::with_capacity(paths.len());
            for path in paths {
                let (header, _) = read_raw_overflow_spool_segment(&path)?;
                segments.push((path, header));
            }
            validate_raw_overflow_spool_segments(&segments)
        }
    })
    .await
    {
        Ok(header) => header,
        Err(err) => {
            return RawPayloadMeta {
                path: None,
                size_bytes: 0,
                truncated: true,
                truncated_reason: Some(format!("spool_replay_failed:{err}")),
            };
        }
    };
    let permit = semaphore
        .acquire_owned()
        .await
        .expect("raw writer semaphore is live");
    let _permit = permit;
    let path = raw_payload_path_for_kind(
        &config.resolved_proxy_raw_dir(),
        &inspected.invoke_id,
        &inspected.kind,
        false,
    );
    // One segment at a time is enough to keep disk replay busy. A bounded channel
    // prevents recovery from copying the entire durable spool back into RAM.
    let (tx, mut rx) = mpsc::channel::<Bytes>(1);
    let mut replay_config = config.clone();
    replay_config.proxy_raw_compression = inspected.codec;
    let writer = tokio::spawn(async move {
        write_bounded_streaming_raw_payload_to_file(
            path,
            replay_config.proxy_raw_max_bytes,
            replay_config.proxy_raw_immediate_compression_threshold(),
            replay_config.proxy_raw_compression,
            &mut rx,
        )
        .await
    });
    for spool_path in paths {
        let payload = match run_blocking_raw_writer_io(move || {
            read_raw_overflow_spool_segment(&spool_path).map(|(_, payload)| payload)
        })
        .await
        {
            Ok(payload) => payload,
            Err(err) => {
                drop(tx);
                let _ = writer.await;
                return RawPayloadMeta {
                    path: None,
                    size_bytes: 0,
                    truncated: true,
                    truncated_reason: Some(format!("spool_replay_failed:{err}")),
                };
            }
        };
        if tx.send(Bytes::from(payload)).await.is_err() {
            return RawPayloadMeta {
                path: None,
                size_bytes: 0,
                truncated: true,
                truncated_reason: Some("spool_replay_failed:raw writer closed".to_string()),
            };
        }
    }
    drop(tx);
    match writer.await {
        Ok(meta) => meta,
        Err(err) => RawPayloadMeta {
            path: None,
            size_bytes: 0,
            truncated: true,
            truncated_reason: Some(format!("spool_replay_failed:{err}")),
        },
    }
}

fn remove_raw_overflow_spool_segments(paths: &[PathBuf]) {
    for path in paths {
        let _ = fs::remove_file(path);
    }
}

pub(crate) async fn recover_raw_overflow_spools(config: &AppConfig) {
    let directory = config.resolved_proxy_raw_dir().join(RAW_OVERFLOW_SPOOL_DIR);
    let entries = match fs::read_dir(&directory) {
        Ok(entries) => entries,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return,
        Err(err) => {
            warn!(path = %directory.display(), error = %err, "failed to scan raw overflow spool directory");
            return;
        }
    };

    let mut captures = HashMap::<String, Vec<(PathBuf, RawOverflowSpoolHeader)>>::new();
    let mut corrupt_captures = std::collections::HashSet::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|value| value.to_str()) != Some("frames") {
            continue;
        }
        let header = match run_blocking_raw_writer_io({
            let path = path.clone();
            move || read_raw_overflow_spool_segment(&path).map(|(header, _)| header)
        })
        .await
        {
            Ok(header) => header,
            Err(err) => {
                warn!(path = %path.display(), error = %err, "raw overflow spool is incomplete or corrupt; retaining for inspection");
                if let Some(capture_key) = raw_overflow_spool_capture_key_from_path(&path) {
                    corrupt_captures.insert(capture_key);
                }
                continue;
            }
        };
        let capture_key = raw_overflow_spool_capture_key(&path, &header);
        captures
            .entry(capture_key)
            .or_default()
            .push((path, header));
    }

    let semaphore = Arc::new(Semaphore::new(proxy_raw_async_writer_limit(config)));
    for (capture_key, mut segments) in captures {
        if corrupt_captures.contains(&capture_key) {
            warn!(
                capture_key,
                spool_segment_count = segments.len(),
                "raw overflow spool capture has a corrupt segment; retaining all segments"
            );
            continue;
        }
        segments.sort_by_key(|(_, header)| header.segment_index);
        let header = match validate_raw_overflow_spool_segments(&segments) {
            Ok(header) => header,
            Err(err) => {
                warn!(error = %err, spool_segment_count = segments.len(), "raw overflow spool capture is incomplete or inconsistent; retaining for inspection");
                continue;
            }
        };
        let paths = segments
            .into_iter()
            .map(|(path, _)| path)
            .collect::<Vec<_>>();
        let meta =
            replay_raw_overflow_spool_segments(config, semaphore.clone(), paths.clone()).await;
        if meta.path.is_some() || meta.truncated_reason.as_deref() == Some("max_bytes_exceeded") {
            remove_raw_overflow_spool_segments(&paths);
            info!(
                invoke_id = %header.invoke_id,
                kind = %header.kind,
                replay_count = 1,
                spool_segment_count = paths.len(),
                "recovered raw overflow spool"
            );
        } else {
            warn!(
                invoke_id = %header.invoke_id,
                kind = %header.kind,
                spool_segment_count = paths.len(),
                reason = ?meta.truncated_reason,
                "raw overflow spool recovery did not publish a payload; retaining spool"
            );
        }
    }
}

pub(crate) struct AsyncStreamingRawPayloadWriter {
    tx: Option<std::sync::mpsc::SyncSender<TrackedRawPayloadChunk>>,
    meta_rx: Option<oneshot::Receiver<RawPayloadMeta>>,
    observed_size_bytes: i64,
    local_truncated_reason: Option<String>,
    local_truncated: bool,
    spool: Option<RawOverflowSpool>,
}

static RAW_ASYNC_WRITER_QUEUED_BYTES: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);

pub(crate) fn proxy_raw_async_writer_queued_bytes() -> usize {
    RAW_ASYNC_WRITER_QUEUED_BYTES.load(std::sync::atomic::Ordering::Relaxed)
}

struct TrackedRawPayloadChunk {
    bytes: Option<Bytes>,
}

impl TrackedRawPayloadChunk {
    fn new(bytes: Bytes) -> Self {
        Self { bytes: Some(bytes) }
    }

    fn into_bytes(mut self) -> Bytes {
        let bytes = self.bytes.take().unwrap_or_default();
        release_raw_async_writer_queued_bytes(bytes.len());
        bytes
    }
}

impl Drop for TrackedRawPayloadChunk {
    fn drop(&mut self) {
        if let Some(bytes) = self.bytes.as_ref() {
            release_raw_async_writer_queued_bytes(bytes.len());
        }
    }
}

trait RawPayloadChunk {
    fn into_bytes(self) -> Bytes;
}

impl RawPayloadChunk for Bytes {
    fn into_bytes(self) -> Bytes {
        self
    }
}

impl RawPayloadChunk for TrackedRawPayloadChunk {
    fn into_bytes(self) -> Bytes {
        TrackedRawPayloadChunk::into_bytes(self)
    }
}

impl AsyncStreamingRawPayloadWriter {
    pub(crate) fn new(
        state: &AppState,
        invoke_id: &str,
        kind: &'static str,
        enabled: bool,
        wire_content_encoding: Option<&str>,
    ) -> Self {
        if !enabled {
            return Self {
                tx: None,
                meta_rx: None,
                observed_size_bytes: 0,
                local_truncated_reason: None,
                local_truncated: false,
                spool: None,
            };
        }

        let path = raw_payload_path_for_kind(
            &state.config.resolved_proxy_raw_dir(),
            invoke_id,
            kind,
            false,
        );
        let max_bytes = state.config.proxy_raw_max_bytes;
        let immediate_gzip_bytes = state.config.proxy_raw_immediate_compression_threshold();
        // A wire-compressed response is already compressed. Preserve its bytes so the
        // capture path never spends CPU decoding and recompressing gzip/deflate/zstd.
        let codec = wire_content_encoding
            .map(str::trim)
            .filter(|encoding| !encoding.is_empty() && !encoding.eq_ignore_ascii_case("identity"))
            .map(|_| RawCompressionCodec::None)
            .unwrap_or(state.config.proxy_raw_compression);
        let semaphore = state.proxy_raw_async_semaphore.clone();
        let writer_max = proxy_raw_async_writer_limit(&state.config);
        let writer_active = writer_max.saturating_sub(semaphore.available_permits());
        let permit = semaphore.clone().try_acquire_owned();
        if permit.is_err() {
            return match RawOverflowSpool::create(state, invoke_id, kind, codec) {
                Ok(spool) => {
                    debug!(
                        capture_path = "overflow_spool",
                        storage_codec = ?codec,
                        writer_active,
                        writer_max,
                        spool_pending_records = 0,
                        spool_pending_bytes = 0,
                        "raw capture queued to durable overflow spool"
                    );
                    Self {
                        tx: None,
                        meta_rx: None,
                        observed_size_bytes: 0,
                        local_truncated_reason: None,
                        local_truncated: false,
                        spool: Some(spool),
                    }
                }
                Err(err) => {
                    warn!(
                        capture_path = "capture_unavailable",
                        capture_unavailable_reason = "spool_capacity",
                        storage_codec = ?codec,
                        writer_active,
                        writer_max,
                        error = %err,
                        "raw streaming capture unavailable because the durable spool cannot accept it"
                    );
                    Self {
                        tx: None,
                        meta_rx: None,
                        observed_size_bytes: 0,
                        local_truncated_reason: Some(
                            "capture_unavailable:spool_capacity".to_string(),
                        ),
                        local_truncated: true,
                        spool: None,
                    }
                }
            };
        }
        let permit = permit.expect("checked above");
        let (tx, rx) = std::sync::mpsc::sync_channel::<TrackedRawPayloadChunk>(64);
        let (meta_tx, meta_rx) = oneshot::channel();
        debug!(
            capture_path = "direct_writer",
            storage_codec = ?codec,
            writer_active,
            writer_max,
            "raw capture assigned to compression writer"
        );
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            let meta = write_direct_streaming_raw_payload_to_file_tracked(
                path,
                max_bytes,
                immediate_gzip_bytes,
                codec,
                rx,
            );
            let _ = meta_tx.send(meta);
        });

        Self {
            tx: Some(tx),
            meta_rx: Some(meta_rx),
            observed_size_bytes: 0,
            local_truncated_reason: None,
            local_truncated: false,
            spool: None,
        }
    }

    fn mark_writer_closed(&mut self, message: String) {
        self.local_truncated = true;
        self.local_truncated_reason.get_or_insert_with(|| {
            if message.starts_with("capture_unavailable:") {
                message
            } else {
                format!("write_failed:{message}")
            }
        });
        self.tx = None;
    }

    pub(crate) fn append(&mut self, bytes: &[u8]) {
        if bytes.is_empty() {
            return;
        }
        self.observed_size_bytes = self.observed_size_bytes.saturating_add(bytes.len() as i64);
        if let Some(spool) = self.spool.as_mut() {
            if let Err(err) = spool.append(bytes) {
                self.spool = None;
                let reason = if err.to_string().contains("capacity") {
                    "capture_unavailable:spool_capacity"
                } else {
                    "capture_unavailable:spool_write_failed"
                };
                self.mark_writer_closed(reason.to_string());
            }
            return;
        }
        let Some(tx) = self.tx.as_ref() else {
            return;
        };
        RAW_ASYNC_WRITER_QUEUED_BYTES.fetch_add(bytes.len(), std::sync::atomic::Ordering::Relaxed);
        match tx.try_send(TrackedRawPayloadChunk::new(Bytes::copy_from_slice(bytes))) {
            Ok(()) => {}
            Err(std::sync::mpsc::TrySendError::Full(_)) => {
                self.mark_writer_closed("capture_unavailable:ingress_queue_full".to_string());
            }
            Err(std::sync::mpsc::TrySendError::Disconnected(_)) => {
                self.mark_writer_closed("capture_unavailable:writer_closed".to_string());
            }
        }
    }

    pub(crate) async fn finish(mut self) -> RawPayloadMeta {
        self.tx.take();
        let mut meta = if let Some(spool) = self.spool.take() {
            spool.finish(self.observed_size_bytes).await
        } else {
            match self.meta_rx.take() {
                Some(meta_rx) => match meta_rx.await {
                    Ok(meta) => meta,
                    Err(err) => RawPayloadMeta {
                        path: None,
                        size_bytes: self.observed_size_bytes,
                        truncated: true,
                        truncated_reason: Some(format!("write_failed:{err}")),
                    },
                },
                None => RawPayloadMeta::default(),
            }
        };
        meta.size_bytes = self.observed_size_bytes;
        if self.local_truncated {
            meta.truncated = true;
            if meta.truncated_reason.is_none() {
                meta.truncated_reason = self.local_truncated_reason.clone();
            }
            if self
                .local_truncated_reason
                .as_deref()
                .is_some_and(|reason| reason.starts_with("capture_unavailable:"))
                && let Some(path) = meta.path.take()
            {
                let _ = fs::remove_file(path);
            }
        }
        meta
    }
}

pub(crate) async fn write_streaming_raw_payload_to_file(
    path: PathBuf,
    max_bytes: Option<usize>,
    immediate_gzip_bytes: Option<usize>,
    codec: RawCompressionCodec,
    rx: &mut mpsc::UnboundedReceiver<Bytes>,
) -> RawPayloadMeta {
    write_streaming_raw_payload_to_file_from_receiver(
        path,
        max_bytes,
        immediate_gzip_bytes,
        codec,
        StreamingRawPayloadReceiver::Unbounded(rx),
    )
    .await
}

async fn write_reserved_streaming_raw_payload_to_file(
    path: PathBuf,
    max_bytes: Option<usize>,
    immediate_gzip_bytes: Option<usize>,
    codec: RawCompressionCodec,
    rx: &mut mpsc::UnboundedReceiver<StreamingRawPayloadChunk>,
) -> RawPayloadMeta {
    write_streaming_raw_payload_to_file_from_receiver(
        path,
        max_bytes,
        immediate_gzip_bytes,
        codec,
        StreamingRawPayloadReceiver::ReservedUnbounded(rx),
    )
    .await
}

pub(crate) fn write_direct_streaming_raw_payload_to_file(
    path: PathBuf,
    max_bytes: Option<usize>,
    immediate_gzip_bytes: Option<usize>,
    codec: RawCompressionCodec,
    rx: std::sync::mpsc::Receiver<Bytes>,
) -> RawPayloadMeta {
    write_direct_streaming_raw_payload_to_file_inner(
        path,
        max_bytes,
        immediate_gzip_bytes,
        codec,
        rx,
    )
}

fn write_direct_streaming_raw_payload_to_file_tracked(
    path: PathBuf,
    max_bytes: Option<usize>,
    immediate_gzip_bytes: Option<usize>,
    codec: RawCompressionCodec,
    rx: std::sync::mpsc::Receiver<TrackedRawPayloadChunk>,
) -> RawPayloadMeta {
    write_direct_streaming_raw_payload_to_file_inner(
        path,
        max_bytes,
        immediate_gzip_bytes,
        codec,
        rx,
    )
}

fn release_raw_async_writer_queued_bytes(bytes: usize) {
    let _ = RAW_ASYNC_WRITER_QUEUED_BYTES.fetch_update(
        std::sync::atomic::Ordering::Relaxed,
        std::sync::atomic::Ordering::Relaxed,
        |queued| Some(queued.saturating_sub(bytes)),
    );
}

fn write_direct_streaming_raw_payload_to_file_inner(
    path: PathBuf,
    max_bytes: Option<usize>,
    immediate_gzip_bytes: Option<usize>,
    codec: RawCompressionCodec,
    rx: std::sync::mpsc::Receiver<impl RawPayloadChunk>,
) -> RawPayloadMeta {
    enum DirectWriter {
        Buffer(Vec<u8>),
        Plain(fs::File),
        Gzip(flate2::write::GzEncoder<io::BufWriter<fs::File>>),
        Zstd(zstd::stream::write::Encoder<'static, io::BufWriter<fs::File>>),
    }

    let gzip_path = raw_payload_gzip_path(&path);
    let zstd_path = raw_payload_zstd_path(&path);
    let mut writer = match codec {
        // Gzip retains the existing hot-plaintext threshold. Zstd intentionally starts at
        // the first byte: it is the current identity-response storage format.
        RawCompressionCodec::Gzip if immediate_gzip_bytes.is_some() => {
            Ok(DirectWriter::Buffer(Vec::new()))
        }
        RawCompressionCodec::Gzip => {
            create_plain_streaming_raw_file(&path).map(DirectWriter::Plain)
        }
        RawCompressionCodec::Zstd => {
            create_zstd_streaming_raw_encoder(&zstd_path).map(DirectWriter::Zstd)
        }
        RawCompressionCodec::None => {
            create_plain_streaming_raw_file(&path).map(DirectWriter::Plain)
        }
    };
    let mut meta = RawPayloadMeta::default();
    let mut written_bytes = 0usize;
    let mut active_path = match codec {
        RawCompressionCodec::Zstd => Some(zstd_path.clone()),
        RawCompressionCodec::Gzip if immediate_gzip_bytes.is_none() => Some(path.clone()),
        RawCompressionCodec::None => Some(path.clone()),
        RawCompressionCodec::Gzip => None,
    };
    while let Ok(chunk) = rx.recv() {
        let bytes = chunk.into_bytes();
        meta.size_bytes = meta.size_bytes.saturating_add(bytes.len() as i64);
        let write_len = max_bytes
            .map(|limit| limit.saturating_sub(written_bytes).min(bytes.len()))
            .unwrap_or(bytes.len());
        if write_len < bytes.len() {
            meta.truncated = true;
            meta.truncated_reason
                .get_or_insert_with(|| "max_bytes_exceeded".to_string());
        }
        if write_len == 0 {
            continue;
        }
        let current_writer = std::mem::replace(&mut writer, Ok(DirectWriter::Buffer(Vec::new())));
        let result = match current_writer {
            Ok(DirectWriter::Buffer(mut buffer)) => {
                buffer.extend_from_slice(&bytes[..write_len]);
                if buffer.len()
                    >= immediate_gzip_bytes.expect("buffer only used for gzip threshold")
                {
                    match create_gzip_streaming_raw_encoder(&gzip_path).and_then(|mut encoder| {
                        encoder.write_all(&buffer)?;
                        Ok(encoder)
                    }) {
                        Ok(encoder) => {
                            active_path = Some(gzip_path.clone());
                            writer = Ok(DirectWriter::Gzip(encoder));
                            Ok(())
                        }
                        Err(error) => Err(error),
                    }
                } else {
                    writer = Ok(DirectWriter::Buffer(buffer));
                    Ok(())
                }
            }
            Ok(DirectWriter::Plain(mut file)) => match file.write_all(&bytes[..write_len]) {
                Ok(()) => {
                    writer = Ok(DirectWriter::Plain(file));
                    Ok(())
                }
                Err(error) => Err(error),
            },
            Ok(DirectWriter::Gzip(mut encoder)) => match encoder.write_all(&bytes[..write_len]) {
                Ok(()) => {
                    writer = Ok(DirectWriter::Gzip(encoder));
                    Ok(())
                }
                Err(error) => Err(error),
            },
            Ok(DirectWriter::Zstd(mut encoder)) => match encoder.write_all(&bytes[..write_len]) {
                Ok(()) => {
                    writer = Ok(DirectWriter::Zstd(encoder));
                    Ok(())
                }
                Err(error) => Err(error),
            },
            Err(error) => Err(error),
        };
        if let Err(error) = result {
            if let Some(active_path) = active_path.as_deref() {
                let _ = fs::remove_file(active_path);
            }
            meta.truncated = true;
            meta.truncated_reason = Some(format!("write_failed:{error}"));
            return meta;
        }
        written_bytes = written_bytes.saturating_add(write_len);
    }
    if written_bytes == 0 {
        if let Some(active_path) = active_path.as_deref() {
            let _ = fs::remove_file(active_path);
        }
        return meta;
    }
    let result = match writer {
        Ok(DirectWriter::Buffer(buffer)) => {
            let mut file = match create_plain_streaming_raw_file(&path) {
                Ok(file) => file,
                Err(error) => {
                    return RawPayloadMeta {
                        path: None,
                        size_bytes: meta.size_bytes,
                        truncated: true,
                        truncated_reason: Some(format!("write_failed:{error}")),
                    };
                }
            };
            active_path = Some(path.clone());
            file.write_all(&buffer).and_then(|()| file.flush())
        }
        Ok(DirectWriter::Plain(mut file)) => file.flush(),
        Ok(DirectWriter::Gzip(encoder)) => encoder.finish().and_then(|mut file| file.flush()),
        Ok(DirectWriter::Zstd(encoder)) => encoder.finish().and_then(|mut file| file.flush()),
        Err(error) => Err(error),
    };
    match result {
        Ok(()) => {
            if let Some(active_path) = active_path {
                meta.path = Some(active_path.to_string_lossy().to_string());
            }
        }
        Err(error) => {
            if let Some(active_path) = active_path.as_deref() {
                let _ = fs::remove_file(active_path);
            }
            meta.truncated = true;
            meta.truncated_reason = Some(format!("write_failed:{error}"));
        }
    }
    meta
}

async fn write_bounded_streaming_raw_payload_to_file(
    path: PathBuf,
    max_bytes: Option<usize>,
    immediate_gzip_bytes: Option<usize>,
    codec: RawCompressionCodec,
    rx: &mut mpsc::Receiver<Bytes>,
) -> RawPayloadMeta {
    write_streaming_raw_payload_to_file_from_receiver(
        path,
        max_bytes,
        immediate_gzip_bytes,
        codec,
        StreamingRawPayloadReceiver::Bounded(rx),
    )
    .await
}

enum StreamingRawPayloadReceiver<'a> {
    Bounded(&'a mut mpsc::Receiver<Bytes>),
    Unbounded(&'a mut mpsc::UnboundedReceiver<Bytes>),
    ReservedUnbounded(&'a mut mpsc::UnboundedReceiver<StreamingRawPayloadChunk>),
}

impl StreamingRawPayloadReceiver<'_> {
    async fn recv(&mut self) -> Option<StreamingRawPayloadChunk> {
        match self {
            Self::Bounded(receiver) => receiver
                .recv()
                .await
                .map(StreamingRawPayloadChunk::unreserved),
            Self::Unbounded(receiver) => receiver
                .recv()
                .await
                .map(StreamingRawPayloadChunk::unreserved),
            Self::ReservedUnbounded(receiver) => receiver.recv().await,
        }
    }
}

async fn write_streaming_raw_payload_to_file_from_receiver(
    path: PathBuf,
    max_bytes: Option<usize>,
    immediate_gzip_bytes: Option<usize>,
    codec: RawCompressionCodec,
    mut rx: StreamingRawPayloadReceiver<'_>,
) -> RawPayloadMeta {
    let mut meta = RawPayloadMeta::default();
    let gzip_path = raw_payload_gzip_path(&path);
    let zstd_path = raw_payload_zstd_path(&path);
    let mut writer = StreamingRawPayloadWriterState::Buffer(Vec::new());
    let mut written_bytes = 0usize;
    while let Some(bytes) = rx.recv().await {
        if bytes.is_empty() {
            continue;
        }
        meta.size_bytes = meta.size_bytes.saturating_add(bytes.len() as i64);

        let write_len = if let Some(limit) = max_bytes {
            let remaining = limit.saturating_sub(written_bytes);
            if remaining == 0 {
                meta.truncated = true;
                meta.truncated_reason
                    .get_or_insert_with(|| "max_bytes_exceeded".to_string());
                continue;
            }
            let write_len = remaining.min(bytes.len());
            if write_len < bytes.len() {
                meta.truncated = true;
                meta.truncated_reason
                    .get_or_insert_with(|| "max_bytes_exceeded".to_string());
            }
            write_len
        } else {
            bytes.len()
        };

        if write_len == 0 {
            continue;
        }

        let current_writer = std::mem::replace(
            &mut writer,
            StreamingRawPayloadWriterState::Buffer(Vec::new()),
        );
        let mut failed_path: Option<PathBuf> = None;
        let result = match current_writer {
            StreamingRawPayloadWriterState::Buffer(mut buffer) => {
                buffer.extend_from_slice(&bytes[..write_len]);
                match immediate_gzip_bytes {
                    Some(threshold)
                        if buffer.len() >= threshold && codec == RawCompressionCodec::Gzip =>
                    {
                        let write_path = gzip_path.clone();
                        match run_blocking_raw_writer_io(move || {
                            let mut encoder = create_gzip_streaming_raw_encoder(&write_path)?;
                            encoder.write_all(&buffer)?;
                            Ok(encoder)
                        })
                        .await
                        {
                            Ok(encoder) => {
                                meta.path = Some(gzip_path.to_string_lossy().to_string());
                                writer = StreamingRawPayloadWriterState::Gzip {
                                    path: gzip_path.clone(),
                                    encoder,
                                };
                                Ok(())
                            }
                            Err(err) => {
                                failed_path = Some(gzip_path.clone());
                                Err(err)
                            }
                        }
                    }
                    Some(threshold)
                        if buffer.len() >= threshold && codec == RawCompressionCodec::Zstd =>
                    {
                        let write_path = zstd_path.clone();
                        match run_blocking_raw_writer_io(move || {
                            let mut encoder = create_zstd_streaming_raw_encoder(&write_path)?;
                            encoder.write_all(&buffer)?;
                            Ok(encoder)
                        })
                        .await
                        {
                            Ok(encoder) => {
                                meta.path = Some(zstd_path.to_string_lossy().to_string());
                                writer = StreamingRawPayloadWriterState::Zstd {
                                    path: zstd_path.clone(),
                                    encoder,
                                };
                                Ok(())
                            }
                            Err(err) => {
                                failed_path = Some(zstd_path.clone());
                                Err(err)
                            }
                        }
                    }
                    Some(_) => {
                        writer = StreamingRawPayloadWriterState::Buffer(buffer);
                        Ok(())
                    }
                    None => {
                        let write_path = path.clone();
                        match run_blocking_raw_writer_io(move || {
                            let mut file = create_plain_streaming_raw_file(&write_path)?;
                            file.write_all(&buffer)?;
                            Ok(file)
                        })
                        .await
                        {
                            Ok(file) => {
                                meta.path = Some(path.to_string_lossy().to_string());
                                writer = StreamingRawPayloadWriterState::Plain {
                                    path: path.clone(),
                                    file,
                                };
                                Ok(())
                            }
                            Err(err) => {
                                failed_path = Some(path.clone());
                                Err(err)
                            }
                        }
                    }
                }
            }
            StreamingRawPayloadWriterState::Plain { path, mut file } => {
                let chunk = bytes.slice(..write_len);
                match run_blocking_raw_writer_io(move || {
                    file.write_all(chunk.as_ref())?;
                    Ok(file)
                })
                .await
                {
                    Ok(file) => {
                        writer = StreamingRawPayloadWriterState::Plain { path, file };
                        Ok(())
                    }
                    Err(err) => {
                        failed_path = Some(path.clone());
                        Err(err)
                    }
                }
            }
            StreamingRawPayloadWriterState::Gzip { path, mut encoder } => {
                let chunk = bytes.slice(..write_len);
                match run_blocking_raw_writer_io(move || {
                    encoder.write_all(chunk.as_ref())?;
                    Ok(encoder)
                })
                .await
                {
                    Ok(encoder) => {
                        writer = StreamingRawPayloadWriterState::Gzip { path, encoder };
                        Ok(())
                    }
                    Err(err) => {
                        failed_path = Some(path.clone());
                        Err(err)
                    }
                }
            }
            StreamingRawPayloadWriterState::Zstd { path, mut encoder } => {
                let chunk = bytes.slice(..write_len);
                match run_blocking_raw_writer_io(move || {
                    encoder.write_all(chunk.as_ref())?;
                    Ok(encoder)
                })
                .await
                {
                    Ok(encoder) => {
                        writer = StreamingRawPayloadWriterState::Zstd { path, encoder };
                        Ok(())
                    }
                    Err(err) => {
                        failed_path = Some(path.clone());
                        Err(err)
                    }
                }
            }
        };

        if let Err(err) = result {
            meta.truncated = true;
            meta.truncated_reason = Some(format!("write_failed:{err}"));
            if let Some(current_path) = failed_path.as_deref() {
                let _ = fs::remove_file(current_path);
            }
            meta.path = None;
            return meta;
        }
        written_bytes = written_bytes.saturating_add(write_len);
    }

    let final_path = writer
        .current_path()
        .map(|value| value.to_path_buf())
        .or_else(|| meta.path.as_ref().map(PathBuf::from));
    let finish_result = match writer {
        StreamingRawPayloadWriterState::Buffer(buffer) => {
            if buffer.is_empty() {
                Ok(())
            } else if codec == RawCompressionCodec::Zstd {
                let final_zstd_path = zstd_path.clone();
                let write_result = run_blocking_raw_writer_io(move || {
                    let mut encoder = create_zstd_streaming_raw_encoder(&final_zstd_path)?;
                    encoder.write_all(&buffer)?;
                    let mut writer = encoder.finish()?;
                    writer.flush()
                })
                .await;
                if write_result.is_ok() {
                    meta.path = Some(zstd_path.to_string_lossy().to_string());
                }
                write_result
            } else {
                let final_plain_path = path.clone();
                let write_result = run_blocking_raw_writer_io(move || {
                    let mut file = create_plain_streaming_raw_file(&final_plain_path)?;
                    file.write_all(&buffer)?;
                    file.flush()
                })
                .await;
                if write_result.is_ok() {
                    meta.path = Some(path.to_string_lossy().to_string());
                }
                write_result
            }
        }
        StreamingRawPayloadWriterState::Plain { path, mut file } => {
            let flush_result = run_blocking_raw_writer_io(move || file.flush()).await;
            if flush_result.is_ok() {
                meta.path = Some(path.to_string_lossy().to_string());
            }
            flush_result
        }
        StreamingRawPayloadWriterState::Gzip { path, encoder } => {
            let finish_result = run_blocking_raw_writer_io(move || {
                let mut writer = encoder.finish()?;
                writer.flush()
            })
            .await;
            if finish_result.is_ok() {
                meta.path = Some(path.to_string_lossy().to_string());
            }
            finish_result
        }
        StreamingRawPayloadWriterState::Zstd { path, encoder } => {
            let finish_result = run_blocking_raw_writer_io(move || {
                let mut writer = encoder.finish()?;
                writer.flush()
            })
            .await;
            if finish_result.is_ok() {
                meta.path = Some(path.to_string_lossy().to_string());
            }
            finish_result
        }
    };

    if let Err(err) = finish_result {
        meta.truncated = true;
        meta.truncated_reason = Some(format!("write_failed:{err}"));
        if let Some(path) = final_path.as_deref() {
            let _ = fs::remove_file(path);
        }
        meta.path = None;
    }

    meta
}

pub(crate) fn build_raw_response_preview(bytes: &[u8]) -> String {
    if bytes.is_empty() {
        return "{}".to_string();
    }
    let preview = if bytes.len() > RAW_RESPONSE_PREVIEW_LIMIT {
        &bytes[..RAW_RESPONSE_PREVIEW_LIMIT]
    } else {
        bytes
    };
    String::from_utf8_lossy(preview).to_string()
}

pub(crate) fn extract_error_message_from_response(bytes: &[u8]) -> Option<String> {
    let value = serde_json::from_slice::<Value>(bytes).ok()?;
    value
        .pointer("/error/message")
        .and_then(|v| v.as_str())
        .map(|v| v.to_string())
        .or_else(|| {
            value
                .get("message")
                .and_then(|v| v.as_str())
                .map(|v| v.to_string())
        })
}

pub(crate) fn summarize_plaintext_upstream_error(bytes: &[u8]) -> Option<String> {
    let text = std::str::from_utf8(bytes).ok()?.trim();
    if text.is_empty() {
        return None;
    }
    let lower = text.to_ascii_lowercase();
    if text.starts_with('<')
        || lower.starts_with("<!doctype")
        || lower.starts_with("<html")
        || lower.starts_with("<body")
    {
        return None;
    }
    Some(text.chars().take(240).collect())
}

pub(crate) fn extract_error_message_from_response_preview(bytes: &[u8]) -> Option<String> {
    extract_error_message_from_response(bytes).or_else(|| summarize_plaintext_upstream_error(bytes))
}

pub(crate) fn merge_response_capture_reason(
    response_info: &mut ResponseCaptureInfo,
    reason: impl Into<String>,
) {
    let reason = reason.into();
    let combined_reason = if let Some(existing) = response_info.usage_missing_reason.take() {
        format!("{reason};{existing}")
    } else {
        reason
    };
    response_info.usage_missing_reason = Some(combined_reason);
}

#[cfg(test)]
mod raw_overflow_spool_tests {
    use super::*;

    #[test]
    fn bounded_streaming_ingress_marks_capture_unavailable_without_blocking() {
        let (tx, _rx) = std::sync::mpsc::sync_channel(1);
        let mut writer = AsyncStreamingRawPayloadWriter {
            tx: Some(tx),
            meta_rx: None,
            observed_size_bytes: 0,
            local_truncated_reason: None,
            local_truncated: false,
            spool: None,
        };

        writer.append(b"first");
        writer.append(b"second");

        assert!(writer.local_truncated);
        assert_eq!(
            writer.local_truncated_reason.as_deref(),
            Some("capture_unavailable:ingress_queue_full")
        );
    }

    #[test]
    fn overflow_spool_preserves_configured_max_bytes_reason() {
        assert_eq!(
            raw_overflow_payload_limit_reason(Some(1024), 1024),
            "max_bytes_exceeded"
        );
        assert_eq!(
            raw_overflow_payload_limit_reason(Some(2048), 1024),
            "spool_capacity_exceeded"
        );
    }
}
