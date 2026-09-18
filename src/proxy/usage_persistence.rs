include!("usage_persistence/part_06.rs");
include!("usage_persistence/part_08.rs");
use super::*;

#[cfg(test)]
fn broadcast_test_record_payload(state: &AppState, record: &ApiInvocation) {
    if state.broadcaster.receiver_count() > 0 {
        let _ = state.broadcaster.send(BroadcastPayload::Records {
            records: vec![record.clone()],
        });
    }
}

include!("usage_persistence/part_09.rs");
include!("usage_persistence/part_10.rs");
include!("usage_persistence/part_11.rs");
include!("usage_persistence/part_12.rs");
include!("usage_persistence/part_13.rs");
include!("usage_persistence/part_14.rs");

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
        .map(|value| value.trim().to_string());
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
    state: Arc<AppState>,
    trace: &PoolUpstreamAttemptTraceContext,
    prompt_cache_key: Option<&str>,
    audit: &PoolRoutingNoCandidateAudit,
) -> Result<()> {
    persist_pool_routing_no_candidate_invocation_with_snapshot(
        state,
        PoolNoCandidateInvocationRequest {
            trace,
            prompt_cache_key,
            audit,
            request_body_snapshot: None,
            status: StatusCode::SERVICE_UNAVAILABLE,
            failure_kind: PROXY_FAILURE_POOL_NO_AVAILABLE_ACCOUNT,
            error_message: POOL_NO_AVAILABLE_ACCOUNT_MESSAGE,
            pool_attempt_count: 0,
            pool_distinct_account_count: 0,
            pool_attempt_terminal_reason: PROXY_FAILURE_POOL_NO_AVAILABLE_ACCOUNT,
        },
    )
    .await
}

pub(crate) struct PoolNoCandidateInvocationRequest<'a> {
    pub(crate) trace: &'a PoolUpstreamAttemptTraceContext,
    pub(crate) prompt_cache_key: Option<&'a str>,
    pub(crate) audit: &'a PoolRoutingNoCandidateAudit,
    pub(crate) request_body_snapshot: Option<PoolReplayBodySnapshot>,
    pub(crate) status: StatusCode,
    pub(crate) failure_kind: &'a str,
    pub(crate) error_message: &'a str,
    pub(crate) pool_attempt_count: usize,
    pub(crate) pool_distinct_account_count: usize,
    pub(crate) pool_attempt_terminal_reason: &'a str,
}

pub(crate) async fn persist_pool_routing_no_candidate_invocation_with_snapshot(
    state: Arc<AppState>,
    request: PoolNoCandidateInvocationRequest<'_>,
) -> Result<()> {
    let PoolNoCandidateInvocationRequest {
        trace,
        prompt_cache_key,
        audit,
        request_body_snapshot,
        status,
        failure_kind,
        error_message,
        pool_attempt_count,
        pool_distinct_account_count,
        pool_attempt_terminal_reason,
    } = request;
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
    mut request: PoolNoCandidateInvocationRequest<'_>,
) -> Result<()> {
    request.request_body_snapshot = None;
    persist_pool_routing_no_candidate_invocation_with_snapshot(state, request).await
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

#[derive(Default)]
pub(crate) struct PoolAttemptInsertRequest<'a> {
    pub(crate) group_name_snapshot: Option<&'a str>,
    pub(crate) proxy_binding_key_snapshot: Option<&'a str>,
    pub(crate) routing_source: Option<&'a str>,
    pub(crate) routing_selection_audit_json: Option<&'a str>,
    pub(crate) upstream_account_id: Option<i64>,
    pub(crate) upstream_route_key: Option<&'a str>,
    pub(crate) attempt_index: i64,
    pub(crate) distinct_account_index: i64,
    pub(crate) same_account_retry_index: i64,
    pub(crate) started_at: Option<&'a str>,
    pub(crate) finished_at: Option<&'a str>,
    pub(crate) status: &'a str,
    pub(crate) phase: Option<&'a str>,
    pub(crate) http_status: Option<StatusCode>,
    pub(crate) downstream_http_status: Option<StatusCode>,
    pub(crate) failure_kind: Option<&'a str>,
    pub(crate) error_message: Option<&'a str>,
    pub(crate) downstream_error_message: Option<&'a str>,
    pub(crate) connect_latency_ms: Option<f64>,
    pub(crate) first_byte_latency_ms: Option<f64>,
    pub(crate) stream_latency_ms: Option<f64>,
    pub(crate) upstream_request_id: Option<&'a str>,
    pub(crate) upstream_request_compression_algorithm: Option<&'a str>,
    pub(crate) upstream_request_compression_mode: Option<&'a str>,
    pub(crate) upstream_request_logical_body_bytes: Option<i64>,
    pub(crate) upstream_request_transmitted_body_bytes: Option<i64>,
    pub(crate) upstream_request_header_bytes_approx: Option<i64>,
    pub(crate) upstream_response_body_bytes: Option<i64>,
    pub(crate) upstream_response_header_bytes_approx: Option<i64>,
    pub(crate) compact_support_status: Option<&'a str>,
    pub(crate) compact_support_reason: Option<&'a str>,
}

pub(crate) async fn insert_pool_upstream_request_attempt_with_scope(
    pool: &Pool<Sqlite>,
    trace: &PoolUpstreamAttemptTraceContext,
    request: PoolAttemptInsertRequest<'_>,
) -> Result<i64> {
    for _ in 0..POOL_UPSTREAM_REQUEST_ATTEMPT_PUBLIC_ID_RETRY_LIMIT {
        let attempt_public_id = generate_pool_upstream_request_attempt_public_id();
        let result =
            insert_pool_upstream_request_attempt_once(pool, trace, &request, &attempt_public_id)
                .await;
        match result {
            Ok(result) => return Ok(result.last_insert_rowid()),
            Err(error) if is_pool_upstream_request_attempt_public_id_collision(&error) => continue,
            Err(error) => return Err(error.into()),
        }
    }

    bail!("failed to allocate unique attempt_public_id for pool_upstream_request_attempts insert")
}

async fn insert_pool_upstream_request_attempt_once(
    pool: &Pool<Sqlite>,
    trace: &PoolUpstreamAttemptTraceContext,
    request: &PoolAttemptInsertRequest<'_>,
    attempt_public_id: &str,
) -> std::result::Result<sqlx::sqlite::SqliteQueryResult, sqlx::Error> {
    let PoolAttemptInsertRequest {
        group_name_snapshot,
        proxy_binding_key_snapshot,
        routing_source,
        routing_selection_audit_json,
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
        upstream_request_compression_algorithm,
        upstream_request_compression_mode,
        upstream_request_logical_body_bytes,
        upstream_request_transmitted_body_bytes,
        upstream_request_header_bytes_approx,
        upstream_response_body_bytes,
        upstream_response_header_bytes_approx,
        compact_support_status,
        compact_support_reason,
    } = request;
    sqlx::query(include_str!(
        "usage_persistence/insert_pool_upstream_request_attempt.sql"
    ))
    .bind(attempt_public_id)
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
    .await
}

pub(crate) async fn insert_pool_upstream_request_attempt(
    pool: &Pool<Sqlite>,
    trace: &PoolUpstreamAttemptTraceContext,
    request: PoolAttemptInsertRequest<'_>,
) -> Result<i64> {
    insert_pool_upstream_request_attempt_with_scope(pool, trace, request).await
}

#[derive(Default)]
pub(crate) struct PoolAttemptStartScope<'a> {
    pub(crate) group_name_snapshot: Option<&'a str>,
    pub(crate) proxy_binding_key_snapshot: Option<&'a str>,
    pub(crate) upstream_account_id: i64,
    pub(crate) upstream_route_key: &'a str,
}

#[derive(Default)]
pub(crate) struct PoolAttemptStartIndexes {
    pub(crate) attempt_index: i64,
    pub(crate) distinct_account_index: i64,
    pub(crate) same_account_retry_index: i64,
}

pub(crate) async fn begin_pool_upstream_request_attempt(
    pool: &Pool<Sqlite>,
    trace: &PoolUpstreamAttemptTraceContext,
    scope: PoolAttemptStartScope<'_>,
    indexes: PoolAttemptStartIndexes,
    started_at: &str,
) -> PendingPoolAttemptRecord {
    begin_pool_upstream_request_attempt_with_scope(pool, trace, scope, indexes, started_at).await
}

pub(crate) async fn update_pool_upstream_request_attempt_model(
    pool: &Pool<Sqlite>,
    attempt_id: Option<i64>,
    model: Option<&str>,
) -> Result<()> {
    let Some(attempt_id) = attempt_id else {
        return Ok(());
    };
    let _write_permit = crate::proxy_sqlite_write_coordinator::proxy_sqlite_write_coordinator()
        .acquire(crate::proxy_sqlite_write_coordinator::ProxySqliteWriteClass::InteractiveProxy)
        .await;
    let model = model.map(str::trim);
    sqlx::query(
        r#"
        UPDATE pool_upstream_request_attempts
        SET request_model = ?1,
            upstream_request_model = CASE
                WHEN model_mapping_pattern IS NULL THEN ?1
                ELSE upstream_request_model
            END
        WHERE id = ?2
        "#,
    )
    .bind(model)
    .bind(attempt_id)
    .execute(pool)
    .await?;
    Ok(())
}

pub(crate) async fn annotate_pool_upstream_request_attempt_model_mapping(
    pool: &Pool<Sqlite>,
    pending: &PendingPoolAttemptRecord,
    upstream_request_model: Option<&str>,
    model_mapping_pattern: Option<&str>,
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
        SET upstream_request_model = ?2,
            model_mapping_pattern = ?3
        WHERE id = ?1
        "#,
    )
    .bind(attempt_id)
    .bind(
        upstream_request_model
            .map(str::trim)
            .filter(|value| !value.is_empty()),
    )
    .bind(
        model_mapping_pattern
            .map(str::trim)
            .filter(|value| !value.is_empty()),
    )
    .execute(pool)
    .await?;
    Ok(())
}

pub(crate) async fn begin_pool_upstream_request_attempt_with_scope(
    pool: &Pool<Sqlite>,
    trace: &PoolUpstreamAttemptTraceContext,
    scope: PoolAttemptStartScope<'_>,
    indexes: PoolAttemptStartIndexes,
    started_at: &str,
) -> PendingPoolAttemptRecord {
    begin_pool_upstream_request_attempt_with_scope_and_routing_source(
        pool, trace, scope, None, indexes, started_at,
    )
    .await
}

pub(crate) async fn begin_pool_upstream_request_attempt_with_scope_and_routing_source(
    pool: &Pool<Sqlite>,
    trace: &PoolUpstreamAttemptTraceContext,
    scope: PoolAttemptStartScope<'_>,
    routing_source: Option<PoolRoutingSelectionSource>,
    indexes: PoolAttemptStartIndexes,
    started_at: &str,
) -> PendingPoolAttemptRecord {
    begin_pool_upstream_request_attempt_with_scope_and_routing_source_and_audit(
        pool,
        trace,
        scope,
        routing_source,
        None,
        indexes,
        started_at,
    )
    .await
}

pub(crate) async fn begin_pool_upstream_request_attempt_with_scope_and_routing_source_and_audit(
    pool: &Pool<Sqlite>,
    trace: &PoolUpstreamAttemptTraceContext,
    scope: PoolAttemptStartScope<'_>,
    routing_source: Option<PoolRoutingSelectionSource>,
    routing_selection_audit: Option<&PoolRoutingSelectionAudit>,
    indexes: PoolAttemptStartIndexes,
    started_at: &str,
) -> PendingPoolAttemptRecord {
    let persistence = persist_pool_attempt_start(
        pool,
        trace,
        &scope,
        routing_source,
        routing_selection_audit,
        &indexes,
        started_at,
    )
    .await;
    build_pending_pool_attempt_record(trace, scope, indexes, started_at, persistence)
}

struct PoolAttemptStartPersistence {
    attempt_id: Option<i64>,
    attempt_public_id: Option<String>,
    routing_source: Option<&'static str>,
    routing_selection_audit_json: Option<String>,
}

async fn persist_pool_attempt_start(
    pool: &Pool<Sqlite>,
    trace: &PoolUpstreamAttemptTraceContext,
    scope: &PoolAttemptStartScope<'_>,
    routing_source: Option<PoolRoutingSelectionSource>,
    routing_selection_audit: Option<&PoolRoutingSelectionAudit>,
    indexes: &PoolAttemptStartIndexes,
    started_at: &str,
) -> PoolAttemptStartPersistence {
    let _write_permit = crate::proxy_sqlite_write_coordinator::proxy_sqlite_write_coordinator()
        .acquire(crate::proxy_sqlite_write_coordinator::ProxySqliteWriteClass::InteractiveProxy)
        .await;
    let routing_source_value = routing_source.map(PoolRoutingSelectionSource::as_persisted_str);
    let routing_selection_audit_json =
        routing_selection_audit.and_then(|audit| serde_json::to_string(audit).ok());
    let attempt_id = match insert_pool_upstream_request_attempt_with_scope(
        pool,
        trace,
        PoolAttemptInsertRequest {
            group_name_snapshot: scope.group_name_snapshot,
            proxy_binding_key_snapshot: scope.proxy_binding_key_snapshot,
            routing_source: routing_source_value,
            routing_selection_audit_json: routing_selection_audit_json.as_deref(),
            upstream_account_id: Some(scope.upstream_account_id),
            upstream_route_key: Some(scope.upstream_route_key),
            attempt_index: indexes.attempt_index,
            distinct_account_index: indexes.distinct_account_index,
            same_account_retry_index: indexes.same_account_retry_index,
            started_at: Some(started_at),
            status: POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_PENDING,
            phase: Some(POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_CONNECTING),
            ..PoolAttemptInsertRequest::default()
        },
    )
    .await
    {
        Ok(attempt_id) => {
            if let Err(err) = observe_model_route_seen_admitted(
                pool,
                scope.upstream_account_id,
                trace.request_model.as_deref(),
            )
            .await
            {
                warn!(
                    account_id = scope.upstream_account_id,
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
            scope.upstream_account_id,
            trace.request_model.as_deref(),
            routing_selection_audit_json.as_deref(),
        );
    }

    let attempt_public_id = load_pool_attempt_public_id(pool, attempt_id).await;
    PoolAttemptStartPersistence {
        attempt_id,
        attempt_public_id,
        routing_source: routing_source_value,
        routing_selection_audit_json,
    }
}

async fn load_pool_attempt_public_id(
    pool: &Pool<Sqlite>,
    attempt_id: Option<i64>,
) -> Option<String> {
    let Some(attempt_id) = attempt_id else {
        return None;
    };
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
}

fn build_pending_pool_attempt_record(
    trace: &PoolUpstreamAttemptTraceContext,
    scope: PoolAttemptStartScope<'_>,
    indexes: PoolAttemptStartIndexes,
    started_at: &str,
    persistence: PoolAttemptStartPersistence,
) -> PendingPoolAttemptRecord {
    PendingPoolAttemptRecord {
        attempt_id: persistence.attempt_id,
        attempt_public_id: persistence.attempt_public_id,
        invoke_id: trace.invoke_id.clone(),
        occurred_at: trace.occurred_at.clone(),
        endpoint: trace.endpoint.clone(),
        sticky_key: trace.sticky_key.clone(),
        routing_source: persistence.routing_source.map(ToOwned::to_owned),
        routing_selection_audit_json: persistence.routing_selection_audit_json,
        requester_ip: trace.requester_ip.clone(),
        upstream_base_url_host: trace.upstream_base_url_host.clone(),
        group_name_snapshot: scope.group_name_snapshot.map(ToOwned::to_owned),
        proxy_binding_key_snapshot: scope.proxy_binding_key_snapshot.map(ToOwned::to_owned),
        request_model: trace.request_model.clone(),
        upstream_account_id: scope.upstream_account_id,
        upstream_route_key: scope.upstream_route_key.to_string(),
        attempt_index: indexes.attempt_index,
        distinct_account_index: indexes.distinct_account_index,
        same_account_retry_index: indexes.same_account_retry_index,
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
