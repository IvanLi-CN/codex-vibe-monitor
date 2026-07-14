use super::*;
use anyhow::anyhow;
use serde::Serialize;
use serde_json::json;
use sqlx::FromRow;
use std::sync::Mutex as StdMutex;
use std::time::{Duration, Instant};
use tracing::debug;

pub(crate) const INVOCATION_PROXY_DISPLAY_SQL: &str = "NULLIF(TRIM(CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.proxyDisplayName') AS TEXT) END), '')";
pub(crate) const INVOCATION_ENDPOINT_SQL: &str =
    "CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.endpoint') AS TEXT) END";
pub(crate) const INVOCATION_COMPACTION_REQUEST_KIND_SQL: &str = "CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.compactionRequestKind') AS TEXT) END";
pub(crate) const INVOCATION_COMPACTION_RESPONSE_KIND_SQL: &str = "CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.compactionResponseKind') AS TEXT) END";
pub(crate) const INVOCATION_IMAGE_INTENT_SQL: &str =
    "CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.imageIntent') AS TEXT) END";
pub(crate) const INVOCATION_FAILURE_KIND_SQL: &str = "COALESCE(CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.failureKind') AS TEXT) END, failure_kind)";
pub(crate) const INVOCATION_REQUESTER_IP_SQL: &str =
    "CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.requesterIp') AS TEXT) END";
pub(crate) const INVOCATION_PROMPT_CACHE_KEY_SQL: &str = "CASE WHEN json_valid(payload) THEN TRIM(CAST(json_extract(payload, '$.promptCacheKey') AS TEXT)) END";
pub(crate) const INVOCATION_STICKY_KEY_SQL: &str = "CASE WHEN json_valid(payload) THEN TRIM(COALESCE(CAST(json_extract(payload, '$.stickyKey') AS TEXT), CAST(json_extract(payload, '$.promptCacheKey') AS TEXT))) END";
pub(crate) const INVOCATION_UPSTREAM_SCOPE_SQL: &str = "COALESCE(CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.upstreamScope') AS TEXT) END, 'external')";
pub(crate) const INVOCATION_ROUTE_MODE_SQL: &str =
    "CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.routeMode') AS TEXT) END";
pub(crate) const INVOCATION_REQUEST_MODEL_SQL: &str =
    "CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.requestModel') AS TEXT) END";
pub(crate) const INVOCATION_RESPONSE_MODEL_SQL: &str =
    "CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.responseModel') AS TEXT) END";
pub(crate) const INVOCATION_UPSTREAM_ACCOUNT_ID_SQL: &str = "CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.upstreamAccountId') AS INTEGER) END";
pub(crate) const INVOCATION_UPSTREAM_ACCOUNT_NAME_SQL: &str = "CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.upstreamAccountName') AS TEXT) END";
pub(crate) const INVOCATION_UPSTREAM_ACCOUNT_PLAN_TYPE_SQL: &str = "COALESCE((SELECT NULLIF(TRIM(sample.plan_type), '') FROM pool_upstream_account_limit_samples sample WHERE sample.account_id = CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.upstreamAccountId') AS INTEGER) END AND sample.plan_type IS NOT NULL AND TRIM(sample.plan_type) <> '' ORDER BY sample.captured_at DESC, sample.id DESC LIMIT 1), (SELECT NULLIF(TRIM(account.plan_type), '') FROM pool_upstream_accounts account WHERE account.id = CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.upstreamAccountId') AS INTEGER) END))";
pub(crate) const INVOCATION_REASONING_EFFORT_SQL: &str = "CASE WHEN json_valid(payload) AND json_type(payload, '$.reasoningEffort') = 'text' THEN json_extract(payload, '$.reasoningEffort') END";
pub(crate) const INVOCATION_RESPONSE_CONTENT_ENCODING_SQL: &str = "CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.responseContentEncoding') AS TEXT) END";
pub(crate) const INVOCATION_DOWNSTREAM_STATUS_CODE_SQL: &str = "CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.downstreamStatusCode') AS INTEGER) END";
pub(crate) const INVOCATION_DOWNSTREAM_ERROR_MESSAGE_SQL: &str = "CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.downstreamErrorMessage') AS TEXT) END";
pub(crate) const INVOCATION_TRANSPORT_SQL: &str = "CASE WHEN json_valid(payload) AND json_type(payload, '$.transport') = 'text' THEN json_extract(payload, '$.transport') END";
pub(crate) const INVOCATION_BILLING_SERVICE_TIER_SQL: &str = "CASE   WHEN json_valid(payload) AND json_type(payload, '$.billingServiceTier') = 'text'     THEN json_extract(payload, '$.billingServiceTier')   WHEN json_valid(payload) AND json_type(payload, '$.billing_service_tier') = 'text'     THEN json_extract(payload, '$.billing_service_tier') END";
pub(crate) const INVOCATION_POOL_ATTEMPT_COUNT_SQL: &str = "CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.poolAttemptCount') AS INTEGER) END";
pub(crate) const INVOCATION_POOL_DISTINCT_ACCOUNT_COUNT_SQL: &str = "CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.poolDistinctAccountCount') AS INTEGER) END";
pub(crate) const INVOCATION_POOL_ATTEMPT_TERMINAL_REASON_SQL: &str = "CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.poolAttemptTerminalReason') AS TEXT) END";
pub(crate) const PROMPT_CACHE_CONVERSATION_UPSTREAM_ACCOUNT_LIMIT: usize = 3;
pub(crate) const PROMPT_CACHE_CONVERSATION_INVOCATION_PREVIEW_LIMIT: usize = 5;
pub(crate) const INVOCATION_STATUS_NORMALIZED_SQL: &str = "LOWER(TRIM(COALESCE(status, '')))";
pub(crate) const INVOCATION_RESPONSE_BODY_PREVIEW_CHAR_LIMIT: usize = 2_000;
pub(crate) const INVOCATION_LIVE_PHASE_QUEUED: &str = "queued";
pub(crate) const INVOCATION_LIVE_PHASE_REQUESTING: &str = "requesting";
pub(crate) const INVOCATION_LIVE_PHASE_RESPONDING: &str = "responding";
const INVOCATION_ANCHOR_TTL: Duration = Duration::from_secs(30 * 60);
const INVOCATION_ANCHOR_CACHE_LIMIT: usize = 32;

#[derive(Clone)]
struct InvocationAnchorSnapshot {
    snapshot_id: i64,
    upstream_account_id: i64,
    runtime_records: Vec<ApiInvocation>,
    expires_at: Instant,
}

static INVOCATION_ANCHOR_SNAPSHOTS: once_cell::sync::Lazy<
    StdMutex<HashMap<String, InvocationAnchorSnapshot>>,
> = once_cell::sync::Lazy::new(|| StdMutex::new(HashMap::new()));

fn store_invocation_anchor_snapshot(
    snapshot_id: i64,
    upstream_account_id: i64,
    runtime_records: Vec<ApiInvocation>,
) -> String {
    let anchor_id = nanoid::nanoid!(16);
    let now = Instant::now();
    let mut snapshots = INVOCATION_ANCHOR_SNAPSHOTS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    snapshots.retain(|_, snapshot| snapshot.expires_at > now);
    if snapshots.len() >= INVOCATION_ANCHOR_CACHE_LIMIT
        && let Some(oldest_id) = snapshots
            .iter()
            .min_by_key(|(_, snapshot)| snapshot.expires_at)
            .map(|(id, _)| id.clone())
    {
        snapshots.remove(&oldest_id);
    }
    snapshots.insert(
        anchor_id.clone(),
        InvocationAnchorSnapshot {
            snapshot_id,
            upstream_account_id,
            runtime_records,
            expires_at: now + INVOCATION_ANCHOR_TTL,
        },
    );
    anchor_id
}

fn load_invocation_anchor_runtime_records(
    params: &ListQuery,
) -> Result<Option<Vec<ApiInvocation>>, ApiError> {
    let Some(anchor_id) = normalize_query_text(params.anchor_id.as_deref()) else {
        return Ok(None);
    };
    let now = Instant::now();
    let mut snapshots = INVOCATION_ANCHOR_SNAPSHOTS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    snapshots.retain(|_, snapshot| snapshot.expires_at > now);
    let snapshot = snapshots
        .get(&anchor_id)
        .ok_or_else(|| ApiError::bad_request(anyhow!("invocation anchor expired or not found")))?;
    if params.snapshot_id != Some(snapshot.snapshot_id)
        || params.upstream_account_id != Some(snapshot.upstream_account_id)
    {
        return Err(ApiError::bad_request(anyhow!(
            "invocation anchor does not match snapshot or account"
        )));
    }
    Ok(Some(snapshot.runtime_records.clone()))
}

// Legacy records can carry `failure_class=none` or NULL while still representing failures.
// Keep classification consistent with `resolve_failure_classification` without requiring a
// backfill pass to complete before the summary + filters become accurate.
pub(crate) const INVOCATION_RESOLVED_FAILURE_CLASS_SQL: &str = "CASE   WHEN LOWER(TRIM(COALESCE(failure_class, ''))) IN ('service_failure', 'client_failure', 'client_abort')     THEN LOWER(TRIM(COALESCE(failure_class, '')))   ELSE     CASE       WHEN LOWER(TRIM(COALESCE(status, ''))) IN ('success', 'completed')         AND LOWER(TRIM(COALESCE(error_message, ''))) = ''         AND LOWER(TRIM(COALESCE(CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.downstreamErrorMessage') AS TEXT) END, ''))) = ''         AND LOWER(TRIM(COALESCE(CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.failureKind') AS TEXT) END, failure_kind, ''))) = '' THEN 'none'       WHEN LOWER(TRIM(COALESCE(status, ''))) IN ('running', 'pending')         AND LOWER(TRIM(COALESCE(error_message, ''))) = '' THEN 'none'       WHEN LOWER(TRIM(COALESCE(status, ''))) = ''         AND LOWER(TRIM(COALESCE(error_message, ''))) = ''         AND LOWER(TRIM(COALESCE(CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.downstreamErrorMessage') AS TEXT) END, ''))) = ''         AND LOWER(TRIM(COALESCE(CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.failureKind') AS TEXT) END, failure_kind, ''))) = '' THEN 'none'       WHEN LOWER(TRIM(COALESCE(CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.failureKind') AS TEXT) END, failure_kind, ''))) = 'downstream_closed'         OR LOWER(TRIM(COALESCE(error_message, ''))) LIKE '[downstream_closed]%'         OR LOWER(TRIM(COALESCE(error_message, ''))) LIKE '%downstream closed while streaming upstream response%'         OR LOWER(TRIM(COALESCE(CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.downstreamErrorMessage') AS TEXT) END, ''))) LIKE '%downstream closed while streaming upstream response%'         THEN 'client_abort'       WHEN LOWER(TRIM(COALESCE(status, ''))) = 'http_429'         OR LOWER(TRIM(COALESCE(CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.failureKind') AS TEXT) END, failure_kind, ''))) = 'upstream_http_429'         THEN 'service_failure'       WHEN LOWER(TRIM(COALESCE(CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.failureKind') AS TEXT) END, failure_kind, ''))) IN ('request_body_stream_error_client_closed', 'invalid_api_key', 'api_key_not_found', 'api_key_missing')         OR LOWER(TRIM(COALESCE(error_message, ''))) LIKE '[request_body_stream_error_client_closed]%'         OR LOWER(TRIM(COALESCE(error_message, ''))) LIKE '%failed to read request body stream%'         OR LOWER(TRIM(COALESCE(error_message, ''))) LIKE '%invalid api key format%'         OR LOWER(TRIM(COALESCE(error_message, ''))) LIKE '%api key format is invalid%'         OR LOWER(TRIM(COALESCE(error_message, ''))) LIKE '%incorrect api key provided%'         OR LOWER(TRIM(COALESCE(error_message, ''))) LIKE '%api key not found%'         OR LOWER(TRIM(COALESCE(error_message, ''))) LIKE '%please provide an api key%'         OR (LOWER(TRIM(COALESCE(status, ''))) LIKE 'http_4%' AND LOWER(TRIM(COALESCE(status, ''))) != 'http_429')         OR LOWER(TRIM(COALESCE(status, ''))) IN ('http_401', 'http_403')         THEN 'client_failure'       WHEN LOWER(TRIM(COALESCE(CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.failureKind') AS TEXT) END, failure_kind, ''))) IN ('failed_contact_upstream', 'upstream_response_failed', 'upstream_stream_error', 'request_body_read_timeout', 'upstream_handshake_timeout')         OR LOWER(TRIM(COALESCE(error_message, ''))) LIKE '[failed_contact_upstream]%'         OR LOWER(TRIM(COALESCE(error_message, ''))) LIKE '[upstream_response_failed]%'         OR LOWER(TRIM(COALESCE(error_message, ''))) LIKE '[upstream_stream_error]%'         OR LOWER(TRIM(COALESCE(error_message, ''))) LIKE '[request_body_read_timeout]%'         OR LOWER(TRIM(COALESCE(error_message, ''))) LIKE '[upstream_handshake_timeout]%'         OR LOWER(TRIM(COALESCE(error_message, ''))) LIKE '%failed to contact upstream%'         OR LOWER(TRIM(COALESCE(error_message, ''))) LIKE '%upstream response stream reported failure%'         OR LOWER(TRIM(COALESCE(error_message, ''))) LIKE '%upstream stream error%'         OR LOWER(TRIM(COALESCE(error_message, ''))) LIKE '%request body read timed out%'         OR LOWER(TRIM(COALESCE(error_message, ''))) LIKE '%upstream handshake timed out%'         OR LOWER(TRIM(COALESCE(status, ''))) LIKE 'http_5%'         THEN 'service_failure'       WHEN LOWER(TRIM(COALESCE(status, ''))) IN ('success', 'completed') THEN 'none'       WHEN LOWER(TRIM(COALESCE(status, ''))) = 'http_200'         AND LOWER(TRIM(COALESCE(error_message, ''))) = ''         AND LOWER(TRIM(COALESCE(CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.downstreamErrorMessage') AS TEXT) END, ''))) = ''         AND LOWER(TRIM(COALESCE(CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.failureKind') AS TEXT) END, failure_kind, ''))) = '' THEN 'none'       ELSE 'service_failure'     END END";

pub(crate) fn latest_pool_attempt_phase_sql(invocation_ref: &str) -> String {
    format!(
        "(SELECT LOWER(TRIM(COALESCE(attempt.phase, ''))) \
            FROM pool_upstream_request_attempts attempt \
           WHERE attempt.invoke_id = {invocation_ref}.invoke_id \
             AND attempt.occurred_at = {invocation_ref}.occurred_at \
           ORDER BY attempt.attempt_index DESC, attempt.id DESC \
           LIMIT 1)"
    )
}

pub(crate) fn invocation_live_phase_sql(invocation_ref: &str) -> String {
    let attempt_phase_sql = latest_pool_attempt_phase_sql(invocation_ref);
    let upstream_account_id_sql = format!(
        "CASE WHEN json_valid({invocation_ref}.payload) THEN CAST(json_extract({invocation_ref}.payload, '$.upstreamAccountId') AS INTEGER) END"
    );
    format!(
        "CASE \
           WHEN LOWER(TRIM(COALESCE({invocation_ref}.status, ''))) NOT IN ('running', 'pending') THEN NULL \
           WHEN LOWER(TRIM(COALESCE({invocation_ref}.status, ''))) = 'pending' THEN '{queued}' \
           WHEN {attempt_phase} = 'streaming_response' \
             OR ({invocation_ref}.t_upstream_ttfb_ms IS NOT NULL AND {invocation_ref}.t_upstream_ttfb_ms > 0) \
             OR ({invocation_ref}.t_upstream_stream_ms IS NOT NULL AND {invocation_ref}.t_upstream_stream_ms > 0) THEN '{responding}' \
           WHEN {attempt_phase} IN ('connecting', 'sending_request', 'waiting_first_byte') \
             OR {upstream_account_id} IS NOT NULL \
             OR ({invocation_ref}.t_upstream_connect_ms IS NOT NULL AND {invocation_ref}.t_upstream_connect_ms > 0) \
             OR ({invocation_ref}.t_req_read_ms IS NOT NULL AND {invocation_ref}.t_req_read_ms > 0) \
             OR ({invocation_ref}.t_req_parse_ms IS NOT NULL AND {invocation_ref}.t_req_parse_ms > 0) THEN '{requesting}' \
           ELSE '{queued}' \
         END",
        attempt_phase = attempt_phase_sql,
        upstream_account_id = upstream_account_id_sql,
        queued = INVOCATION_LIVE_PHASE_QUEUED,
        requesting = INVOCATION_LIVE_PHASE_REQUESTING,
        responding = INVOCATION_LIVE_PHASE_RESPONDING,
    )
}

pub(crate) fn runtime_invocation_live_phase(record: &ApiInvocation) -> Option<&'static str> {
    fn has_positive_timing(values: &[Option<f64>]) -> bool {
        values
            .iter()
            .flatten()
            .any(|value| value.is_finite() && *value > 0.0)
    }

    match normalized_runtime_text(record.status.as_deref()).as_str() {
        "pending" => Some(INVOCATION_LIVE_PHASE_QUEUED),
        "running" => {
            if has_positive_timing(&[record.t_upstream_ttfb_ms, record.t_upstream_stream_ms]) {
                Some(INVOCATION_LIVE_PHASE_RESPONDING)
            } else if record.upstream_account_id.is_some()
                || has_positive_timing(&[
                    record.t_upstream_connect_ms,
                    record.t_req_read_ms,
                    record.t_req_parse_ms,
                ])
            {
                Some(INVOCATION_LIVE_PHASE_REQUESTING)
            } else {
                Some(INVOCATION_LIVE_PHASE_QUEUED)
            }
        }
        _ => None,
    }
}

pub(crate) fn build_invocation_select_query() -> QueryBuilder<'static, Sqlite> {
    let mut query = QueryBuilder::new(
        "SELECT id, invoke_id, occurred_at, source, \
         CASE WHEN json_valid(payload) THEN json_extract(payload, '$.proxyDisplayName') END AS proxy_display_name, \
         model, \
         ",
    );
    query
        .push(INVOCATION_REQUEST_MODEL_SQL)
        .push(
            " AS request_model, \
         ",
        )
        .push(INVOCATION_RESPONSE_MODEL_SQL)
        .push(
            " AS response_model, \
         input_tokens, output_tokens, \
         cache_input_tokens, reasoning_tokens, \
         ",
        )
        .push(INVOCATION_REASONING_EFFORT_SQL)
        .push(
            " AS reasoning_effort, \
         total_tokens, cost, status, \
         ",
        )
        .push(invocation_live_phase_sql("codex_invocations"))
        .push(
            " AS live_phase, error_message, \
         ",
        )
        .push(INVOCATION_DOWNSTREAM_STATUS_CODE_SQL)
        .push(
            " AS downstream_status_code, \
         CASE WHEN json_valid(payload) THEN json_extract(payload, '$.endpoint') END AS endpoint, \
         ",
        )
        .push(INVOCATION_COMPACTION_REQUEST_KIND_SQL)
        .push(
            " AS compaction_request_kind, \
         ",
        )
        .push(INVOCATION_COMPACTION_RESPONSE_KIND_SQL)
        .push(
            " AS compaction_response_kind, \
         ",
        )
        .push(INVOCATION_IMAGE_INTENT_SQL)
        .push(
            " AS image_intent, \
         ",
        )
        .push(INVOCATION_FAILURE_KIND_SQL)
        .push(
            " AS failure_kind, \
         CASE WHEN json_valid(payload) THEN json_extract(payload, '$.streamTerminalEvent') END AS stream_terminal_event, \
         CASE WHEN json_valid(payload) THEN json_extract(payload, '$.upstreamErrorCode') END AS upstream_error_code, \
         CASE WHEN json_valid(payload) THEN json_extract(payload, '$.upstreamErrorMessage') END AS upstream_error_message, \
         ",
        )
        .push(INVOCATION_DOWNSTREAM_ERROR_MESSAGE_SQL)
        .push(
            " AS downstream_error_message, \
         CASE WHEN json_valid(payload) THEN json_extract(payload, '$.upstreamRequestId') END AS upstream_request_id, ",
        )
        .push(INVOCATION_RESOLVED_FAILURE_CLASS_SQL)
        .push(" AS failure_class, CASE WHEN ")
        .push(INVOCATION_RESOLVED_FAILURE_CLASS_SQL)
        .push(
            " = 'service_failure' THEN 1 ELSE 0 END AS is_actionable, \
         CASE WHEN json_valid(payload) THEN json_extract(payload, '$.requesterIp') END AS requester_ip, \
         ",
        )
        .push(INVOCATION_PROMPT_CACHE_KEY_SQL)
        .push(
            " AS prompt_cache_key, \
         ",
        )
        .push(INVOCATION_STICKY_KEY_SQL)
        .push(
            " AS sticky_key, \
         ",
        )
        .push(INVOCATION_ROUTE_MODE_SQL)
        .push(
            " AS route_mode, \
         ",
        )
        .push(INVOCATION_UPSTREAM_ACCOUNT_ID_SQL)
        .push(
            " AS upstream_account_id, \
         ",
        )
        .push(INVOCATION_UPSTREAM_ACCOUNT_NAME_SQL)
        .push(
            " AS upstream_account_name, \
         ",
        )
        .push(INVOCATION_RESPONSE_CONTENT_ENCODING_SQL)
        .push(
            " AS response_content_encoding, \
         ",
        )
        .push(INVOCATION_TRANSPORT_SQL)
        .push(
            " AS transport, \
         ",
        )
        .push(INVOCATION_POOL_ATTEMPT_COUNT_SQL)
        .push(
            " AS pool_attempt_count, \
         ",
        )
        .push(INVOCATION_POOL_DISTINCT_ACCOUNT_COUNT_SQL)
        .push(
            " AS pool_distinct_account_count, \
         ",
        )
        .push(INVOCATION_POOL_ATTEMPT_TERMINAL_REASON_SQL)
        .push(
            " AS pool_attempt_terminal_reason, \
         CASE \
           WHEN json_valid(payload) AND json_type(payload, '$.requestedServiceTier') = 'text' \
             THEN json_extract(payload, '$.requestedServiceTier') \
           WHEN json_valid(payload) AND json_type(payload, '$.requested_service_tier') = 'text' \
             THEN json_extract(payload, '$.requested_service_tier') END AS requested_service_tier, \
         CASE \
           WHEN json_valid(payload) AND json_type(payload, '$.serviceTier') = 'text' \
             THEN json_extract(payload, '$.serviceTier') \
           WHEN json_valid(payload) AND json_type(payload, '$.service_tier') = 'text' \
             THEN json_extract(payload, '$.service_tier') END AS service_tier, \
         ",
        )
        .push(INVOCATION_BILLING_SERVICE_TIER_SQL)
        .push(
            " AS billing_service_tier, \
         CASE WHEN json_valid(payload) \
           AND json_type(payload, '$.proxyWeightDelta') IN ('integer', 'real') \
           THEN json_extract(payload, '$.proxyWeightDelta') END AS proxy_weight_delta, \
         cost_estimated, price_version, \
         request_raw_path, request_raw_size, request_raw_truncated, request_raw_truncated_reason, \
         response_raw_path, response_raw_size, response_raw_truncated, response_raw_truncated_reason, \
         detail_level, detail_pruned_at, detail_prune_reason, \
         t_total_ms, t_req_read_ms, t_req_parse_ms, t_upstream_connect_ms, t_upstream_ttfb_ms, \
         t_upstream_stream_ms, t_resp_parse_ms, t_persist_ms, \
         created_at \
         FROM codex_invocations WHERE 1 = 1",
        );
    query
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum InvocationSortBy {
    OccurredAt,
    TotalTokens,
    Cost,
    TotalMs,
    TtfbMs,
    Status,
}

impl InvocationSortBy {
    fn parse(raw: Option<&str>) -> Self {
        match raw.map(str::trim).filter(|value| !value.is_empty()) {
            Some("totalTokens") => Self::TotalTokens,
            Some("cost") => Self::Cost,
            Some("tTotalMs") => Self::TotalMs,
            Some("tUpstreamTtfbMs") => Self::TtfbMs,
            Some("status") => Self::Status,
            _ => Self::OccurredAt,
        }
    }

    fn sql_expr(self) -> &'static str {
        match self {
            Self::OccurredAt => "occurred_at",
            Self::TotalTokens => "total_tokens",
            Self::Cost => "cost",
            Self::TotalMs => "t_total_ms",
            Self::TtfbMs => "t_upstream_ttfb_ms",
            Self::Status => INVOCATION_STATUS_NORMALIZED_SQL,
        }
    }
}

pub(crate) fn invocation_display_status_sql() -> String {
    format!(
        "CASE WHEN {status_norm} = 'interrupted' THEN 'interrupted' WHEN {resolved_failure} IN ('service_failure', 'client_failure', 'client_abort') THEN 'failed' WHEN {status_norm} = '' THEN 'unknown' ELSE {status_norm} END",
        resolved_failure = INVOCATION_RESOLVED_FAILURE_CLASS_SQL,
        status_norm = INVOCATION_STATUS_NORMALIZED_SQL,
    )
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum InvocationSortOrder {
    Asc,
    Desc,
}

impl InvocationSortOrder {
    fn parse(raw: Option<&str>) -> Self {
        match raw.map(str::trim).filter(|value| !value.is_empty()) {
            Some("asc") => Self::Asc,
            _ => Self::Desc,
        }
    }

    fn sql_keyword(self) -> &'static str {
        match self {
            Self::Asc => "ASC",
            Self::Desc => "DESC",
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum SnapshotConstraint {
    UpTo(i64),
    After(i64),
}

#[derive(Debug, Clone, Default)]
pub(crate) struct InvocationRecordsFilters {
    pub(crate) occurred_from: Option<String>,
    pub(crate) occurred_to: Option<String>,
    pub(crate) status: Option<String>,
    pub(crate) model: Option<String>,
    pub(crate) endpoint: Option<String>,
    pub(crate) request_id: Option<String>,
    pub(crate) failure_class: Option<String>,
    pub(crate) failure_kind: Option<String>,
    pub(crate) prompt_cache_key: Option<String>,
    pub(crate) sticky_key: Option<String>,
    pub(crate) upstream_scope: Option<String>,
    pub(crate) upstream_account_id: Option<i64>,
    pub(crate) requester_ip: Option<String>,
    pub(crate) keyword: Option<String>,
    pub(crate) min_total_tokens: Option<i64>,
    pub(crate) max_total_tokens: Option<i64>,
    pub(crate) min_total_ms: Option<f64>,
    pub(crate) max_total_ms: Option<f64>,
}

#[derive(Debug, Clone)]
pub(crate) struct InvocationListRequest {
    filters: InvocationRecordsFilters,
    page: i64,
    page_size: i64,
    sort_by: InvocationSortBy,
    sort_order: InvocationSortOrder,
    snapshot_id: Option<i64>,
}

#[derive(Debug, FromRow)]
pub(crate) struct InvocationSummaryAggRow {
    total_count: i64,
    success_count: i64,
    failure_count: i64,
    total_tokens: i64,
    total_cost: f64,
    cache_input_tokens: i64,
}

#[derive(Debug, FromRow)]
pub(crate) struct InvocationNetworkAggRow {
    avg_ttfb_ms: Option<f64>,
    ttfb_count: i64,
    avg_total_ms: Option<f64>,
    total_count: i64,
}

#[derive(Debug, FromRow)]
pub(crate) struct InvocationExceptionAggRow {
    failure_count: i64,
    service_failure_count: i64,
    client_failure_count: i64,
    client_abort_count: i64,
    actionable_failure_count: i64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct InvocationTokenSummary {
    pub(crate) request_count: i64,
    pub(crate) total_tokens: i64,
    pub(crate) avg_tokens_per_request: f64,
    pub(crate) cache_input_tokens: i64,
    pub(crate) total_cost: f64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct InvocationNetworkSummary {
    pub(crate) avg_ttfb_ms: Option<f64>,
    pub(crate) p95_ttfb_ms: Option<f64>,
    pub(crate) avg_total_ms: Option<f64>,
    pub(crate) p95_total_ms: Option<f64>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct InvocationExceptionSummary {
    pub(crate) failure_count: i64,
    pub(crate) service_failure_count: i64,
    pub(crate) client_failure_count: i64,
    pub(crate) client_abort_count: i64,
    pub(crate) actionable_failure_count: i64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct InvocationSummaryResponse {
    pub(crate) snapshot_id: i64,
    pub(crate) new_records_count: i64,
    pub(crate) total_count: i64,
    pub(crate) success_count: i64,
    pub(crate) failure_count: i64,
    pub(crate) total_tokens: i64,
    pub(crate) total_cost: f64,
    pub(crate) token: InvocationTokenSummary,
    pub(crate) network: InvocationNetworkSummary,
    pub(crate) exception: InvocationExceptionSummary,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct InvocationNewRecordsCountResponse {
    pub(crate) snapshot_id: i64,
    pub(crate) new_records_count: i64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct InvocationSuggestionItem {
    pub(crate) value: String,
    pub(crate) count: i64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct InvocationSuggestionBucket {
    pub(crate) items: Vec<InvocationSuggestionItem>,
    pub(crate) has_more: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct InvocationSuggestionsResponse {
    pub(crate) model: InvocationSuggestionBucket,
    pub(crate) endpoint: InvocationSuggestionBucket,
    pub(crate) failure_kind: InvocationSuggestionBucket,
    pub(crate) prompt_cache_key: InvocationSuggestionBucket,
    pub(crate) requester_ip: InvocationSuggestionBucket,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct InvocationAbnormalResponseBodyPreview {
    pub(crate) available: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) preview_text: Option<String>,
    pub(crate) has_more: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) unavailable_reason: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct InvocationRecordDetailResponse {
    pub(crate) id: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) abnormal_response_body: Option<InvocationAbnormalResponseBodyPreview>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct InvocationResponseBodyResponse {
    pub(crate) available: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) body_text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) unavailable_reason: Option<String>,
}

pub(crate) fn normalize_query_text(raw: Option<&str>) -> Option<String> {
    raw.map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

pub(crate) fn escape_sql_like(raw: &str) -> String {
    let mut escaped = String::with_capacity(raw.len());
    for ch in raw.chars() {
        match ch {
            '%' | '_' | '\\' => {
                escaped.push('\\');
                escaped.push(ch);
            }
            _ => escaped.push(ch),
        }
    }
    escaped
}

pub(crate) fn parse_invocation_bound(
    raw: Option<&str>,
    field_name: &str,
) -> Result<Option<String>, ApiError> {
    let Some(raw_value) = normalize_query_text(raw) else {
        return Ok(None);
    };
    let parsed = DateTime::parse_from_rfc3339(&raw_value)
        .with_context(|| format!("invalid {field_name}: {raw_value}"))
        .map_err(ApiError::bad_request)?
        .with_timezone(&Utc);
    Ok(Some(db_occurred_at_lower_bound(parsed)))
}

pub(crate) fn build_invocation_filters(
    params: &ListQuery,
) -> Result<InvocationRecordsFilters, ApiError> {
    let mut occurred_from = parse_invocation_bound(params.from.as_deref(), "from")?;
    let mut occurred_to = parse_invocation_bound(params.to.as_deref(), "to")?;

    // Keep compatibility with clients that only send rangePreset. When explicit from/to are
    // provided, they always take precedence over rangePreset.
    if occurred_from.is_none()
        && occurred_to.is_none()
        && let Some(preset) = normalize_query_text(params.range_preset.as_deref())
    {
        let now = Utc::now();
        let bounds = named_range_bounds(&preset, now, Shanghai).or_else(|| {
            parse_duration_spec(&preset)
                .ok()
                .map(|duration| (now - duration, now))
        });

        if let Some((start, end)) = bounds {
            occurred_from = Some(db_occurred_at_lower_bound(start));
            occurred_to = Some(db_occurred_at_lower_bound(end));
        }
    }

    if let (Some(min_tokens), Some(max_tokens)) = (params.min_total_tokens, params.max_total_tokens)
        && min_tokens > max_tokens
    {
        return Err(ApiError::bad_request(anyhow!(
            "minTotalTokens must be <= maxTotalTokens"
        )));
    }

    if let (Some(min_ms), Some(max_ms)) = (params.min_total_ms, params.max_total_ms)
        && min_ms > max_ms
    {
        return Err(ApiError::bad_request(anyhow!(
            "minTotalMs must be <= maxTotalMs"
        )));
    }

    Ok(InvocationRecordsFilters {
        occurred_from,
        occurred_to,
        status: normalize_query_text(params.status.as_deref()),
        model: normalize_query_text(params.model.as_deref()),
        endpoint: normalize_query_text(params.endpoint.as_deref()),
        request_id: normalize_query_text(params.request_id.as_deref()),
        failure_class: normalize_query_text(params.failure_class.as_deref()),
        failure_kind: normalize_query_text(params.failure_kind.as_deref()),
        prompt_cache_key: normalize_query_text(params.prompt_cache_key.as_deref()),
        sticky_key: normalize_query_text(params.sticky_key.as_deref()),
        upstream_scope: match normalize_query_text(params.upstream_scope.as_deref()) {
            Some(value) if value.eq_ignore_ascii_case("all") => None,
            other => other,
        },
        upstream_account_id: params.upstream_account_id,
        requester_ip: normalize_query_text(params.requester_ip.as_deref()),
        keyword: normalize_query_text(params.keyword.as_deref()),
        min_total_tokens: params.min_total_tokens,
        max_total_tokens: params.max_total_tokens,
        min_total_ms: params.min_total_ms,
        max_total_ms: params.max_total_ms,
    })
}

pub(crate) fn build_invocation_list_request(
    params: &ListQuery,
    list_limit_max: i64,
) -> Result<InvocationListRequest, ApiError> {
    let filters = build_invocation_filters(params)?;
    let page_size = params
        .page_size
        .or(params.limit)
        .unwrap_or(50)
        .clamp(1, list_limit_max);
    let page = params.page.unwrap_or(1).max(1);
    let snapshot_id = params.snapshot_id.filter(|value| *value >= 0);
    Ok(InvocationListRequest {
        filters,
        page,
        page_size,
        sort_by: InvocationSortBy::parse(params.sort_by.as_deref()),
        sort_order: InvocationSortOrder::parse(params.sort_order.as_deref()),
        snapshot_id,
    })
}

pub(crate) fn push_exact_text_filter(
    query: &mut QueryBuilder<Sqlite>,
    sql_expr: &str,
    value: &str,
) {
    query.push(" AND LOWER(TRIM(COALESCE(");
    query.push(sql_expr);
    query.push(", ''))) = ");
    query.push_bind(value.to_lowercase());
}

pub(crate) fn push_keyword_filter(query: &mut QueryBuilder<Sqlite>, keyword: &str) {
    let like_pattern = format!("%{}%", escape_sql_like(&keyword.to_lowercase()));
    query.push(" AND (");
    query
        .push("LOWER(invoke_id) LIKE ")
        .push_bind(like_pattern.clone())
        .push(" ESCAPE '\\'");
    query
        .push(" OR LOWER(COALESCE(model, '')) LIKE ")
        .push_bind(like_pattern.clone())
        .push(" ESCAPE '\\'");
    query.push(" OR LOWER(TRIM(COALESCE(");
    query.push(INVOCATION_PROXY_DISPLAY_SQL);
    query.push(", ''))) LIKE ");
    query.push_bind(like_pattern.clone()).push(" ESCAPE '\\'");
    query.push(" OR LOWER(TRIM(COALESCE(");
    query.push(INVOCATION_ENDPOINT_SQL);
    query.push(", ''))) LIKE ");
    query.push_bind(like_pattern.clone()).push(" ESCAPE '\\'");
    query.push(" OR LOWER(TRIM(COALESCE(");
    query.push(INVOCATION_FAILURE_KIND_SQL);
    query.push(", ''))) LIKE ");
    query.push_bind(like_pattern.clone()).push(" ESCAPE '\\'");
    query
        .push(" OR LOWER(COALESCE(error_message, '')) LIKE ")
        .push_bind(like_pattern.clone())
        .push(" ESCAPE '\\'");
    query.push(" OR LOWER(TRIM(COALESCE(");
    query.push(INVOCATION_DOWNSTREAM_ERROR_MESSAGE_SQL);
    query.push(", ''))) LIKE ");
    query.push_bind(like_pattern.clone()).push(" ESCAPE '\\'");
    query.push(" OR LOWER(TRIM(COALESCE(");
    query.push(INVOCATION_PROMPT_CACHE_KEY_SQL);
    query.push(", ''))) LIKE ");
    query.push_bind(like_pattern.clone()).push(" ESCAPE '\\'");
    query.push(" OR LOWER(TRIM(COALESCE(");
    query.push(INVOCATION_REQUESTER_IP_SQL);
    query.push(", ''))) LIKE ");
    query.push_bind(like_pattern).push(" ESCAPE '\\'");
    query.push(")");
}

pub(crate) fn apply_invocation_records_filters(
    query: &mut QueryBuilder<Sqlite>,
    filters: &InvocationRecordsFilters,
    source_scope: InvocationSourceScope,
    snapshot: Option<SnapshotConstraint>,
) {
    if source_scope == InvocationSourceScope::ProxyOnly {
        query.push(" AND source = ").push_bind(SOURCE_PROXY);
    }

    if let Some(snapshot_constraint) = snapshot {
        match snapshot_constraint {
            SnapshotConstraint::UpTo(snapshot_id) => {
                query.push(" AND id <= ").push_bind(snapshot_id);
            }
            SnapshotConstraint::After(snapshot_id) => {
                query.push(" AND id > ").push_bind(snapshot_id);
            }
        }
    }

    if let Some(from_bound) = filters.occurred_from.as_ref() {
        query
            .push(" AND occurred_at >= ")
            .push_bind(from_bound.clone());
    }

    if let Some(to_bound) = filters.occurred_to.as_ref() {
        query
            .push(" AND occurred_at < ")
            .push_bind(to_bound.clone());
    }

    if let Some(model) = filters.model.as_deref() {
        push_exact_text_filter(query, "model", model);
    }

    if let Some(status) = filters.status.as_deref() {
        let normalized_status = status.trim();
        if normalized_status.eq_ignore_ascii_case("failed") {
            // Legacy rows can still represent failures while `status` is NULL/`none`, so align the
            // UI-level failed filter with the same resolved failure-class semantics used by summary.
            query.push(" AND ");
            query.push(INVOCATION_RESOLVED_FAILURE_CLASS_SQL);
            query.push(" IN ('service_failure', 'client_failure', 'client_abort')");
            query.push(" AND ");
            query.push(INVOCATION_STATUS_NORMALIZED_SQL);
            query.push(" != 'interrupted'");
        } else if normalized_status.eq_ignore_ascii_case("success") {
            // Keep the success filter symmetric with the resolved failure-class logic so legacy rows
            // that still carry `status='success'` but classify as failures do not leak into success.
            query.push(" AND ");
            query.push(INVOCATION_RESOLVED_FAILURE_CLASS_SQL);
            query.push(" = 'none'");
            push_exact_text_filter(query, INVOCATION_STATUS_NORMALIZED_SQL, normalized_status);
        } else {
            push_exact_text_filter(query, "status", status);
        }
    }

    if let Some(endpoint) = filters.endpoint.as_deref() {
        push_exact_text_filter(query, INVOCATION_ENDPOINT_SQL, endpoint);
    }

    if let Some(request_id) = filters.request_id.as_deref() {
        push_exact_text_filter(query, "invoke_id", request_id);
    }

    if let Some(failure_class) = filters.failure_class.as_deref() {
        push_exact_text_filter(query, INVOCATION_RESOLVED_FAILURE_CLASS_SQL, failure_class);
    }

    if let Some(failure_kind) = filters.failure_kind.as_deref() {
        push_exact_text_filter(query, INVOCATION_FAILURE_KIND_SQL, failure_kind);
    }

    if let Some(prompt_cache_key) = filters.prompt_cache_key.as_deref() {
        push_exact_text_filter(query, INVOCATION_PROMPT_CACHE_KEY_SQL, prompt_cache_key);
    }

    if let Some(sticky_key) = filters.sticky_key.as_deref() {
        push_exact_text_filter(query, INVOCATION_STICKY_KEY_SQL, sticky_key);
    }

    if let Some(upstream_scope) = filters.upstream_scope.as_deref() {
        push_exact_text_filter(query, INVOCATION_UPSTREAM_SCOPE_SQL, upstream_scope);
    }

    if let Some(upstream_account_id) = filters.upstream_account_id {
        query.push(" AND ").push(INVOCATION_UPSTREAM_ACCOUNT_ID_SQL);
        query.push(" = ").push_bind(upstream_account_id);
    }

    if let Some(requester_ip) = filters.requester_ip.as_deref() {
        push_exact_text_filter(query, INVOCATION_REQUESTER_IP_SQL, requester_ip);
    }

    if let Some(keyword) = filters.keyword.as_deref() {
        push_keyword_filter(query, keyword);
    }

    if let Some(min_total_tokens) = filters.min_total_tokens {
        query
            .push(" AND total_tokens IS NOT NULL AND total_tokens >= ")
            .push_bind(min_total_tokens);
    }

    if let Some(max_total_tokens) = filters.max_total_tokens {
        query
            .push(" AND total_tokens IS NOT NULL AND total_tokens <= ")
            .push_bind(max_total_tokens);
    }

    if let Some(min_total_ms) = filters.min_total_ms {
        query
            .push(" AND t_total_ms IS NOT NULL AND t_total_ms >= ")
            .push_bind(min_total_ms);
    }

    if let Some(max_total_ms) = filters.max_total_ms {
        query
            .push(" AND t_total_ms IS NOT NULL AND t_total_ms <= ")
            .push_bind(max_total_ms);
    }
}

pub(crate) async fn resolve_invocation_snapshot_id(
    pool: &Pool<Sqlite>,
    source_scope: InvocationSourceScope,
) -> Result<i64> {
    #[derive(Debug, FromRow)]
    struct SnapshotRow {
        snapshot_id: Option<i64>,
    }

    let mut query =
        QueryBuilder::new("SELECT MAX(id) AS snapshot_id FROM codex_invocations WHERE 1 = 1");
    if source_scope == InvocationSourceScope::ProxyOnly {
        query.push(" AND source = ").push_bind(SOURCE_PROXY);
    }

    let row = query
        .build_query_as::<SnapshotRow>()
        .fetch_one(pool)
        .await?;
    Ok(row.snapshot_id.unwrap_or(0))
}

pub(crate) fn append_invocation_order_clause(
    query: &mut QueryBuilder<Sqlite>,
    sort_by: InvocationSortBy,
    sort_order: InvocationSortOrder,
) {
    let direction = sort_order.sql_keyword();
    query.push(" ORDER BY ");
    if matches!(sort_by, InvocationSortBy::Status) {
        let status_expr = invocation_display_status_sql();
        query.push("(");
        query.push(&status_expr);
        query.push(") IS NULL ASC, ");
        query.push(status_expr);
    } else {
        query.push(sort_by.sql_expr());
        query.push(" IS NULL ASC, ");
        query.push(sort_by.sql_expr());
    }
    query.push(" ");
    query.push(direction);
    match sort_by {
        InvocationSortBy::OccurredAt => {
            query.push(", id ");
            query.push(direction);
        }
        _ => {
            query.push(", occurred_at DESC, id DESC");
        }
    }
}

pub(crate) fn normalized_runtime_text(value: Option<&str>) -> String {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or_default()
        .to_lowercase()
}

pub(crate) fn runtime_text_equals(value: Option<&str>, expected: &str) -> bool {
    normalized_runtime_text(value) == expected.trim().to_lowercase()
}

pub(crate) fn runtime_keyword_matches(record: &ApiInvocation, keyword: &str) -> bool {
    let keyword = keyword.trim().to_lowercase();
    if keyword.is_empty() {
        return true;
    }
    [
        Some(record.invoke_id.as_str()),
        record.model.as_deref(),
        record.proxy_display_name.as_deref(),
        record.endpoint.as_deref(),
        record.failure_kind.as_deref(),
        record.error_message.as_deref(),
        record.downstream_error_message.as_deref(),
        record.prompt_cache_key.as_deref(),
        record.requester_ip.as_deref(),
    ]
    .into_iter()
    .flatten()
    .any(|value| value.to_lowercase().contains(&keyword))
}

pub(crate) fn runtime_sticky_key(record: &ApiInvocation) -> Option<&str> {
    record
        .sticky_key
        .as_deref()
        .or(record.prompt_cache_key.as_deref())
}

pub(crate) fn runtime_upstream_scope(record: &ApiInvocation) -> &'static str {
    if runtime_text_equals(record.route_mode.as_deref(), "pool") {
        "internal"
    } else {
        "external"
    }
}

pub(crate) fn runtime_record_is_retry(record: &ApiInvocation) -> bool {
    record.pool_attempt_count.unwrap_or(1) > 1
}

pub(crate) fn runtime_record_is_in_flight(record: &ApiInvocation) -> bool {
    matches!(
        normalized_runtime_text(record.status.as_deref()).as_str(),
        "running" | "pending"
    )
}

pub(crate) fn runtime_record_matches_filters(
    record: &ApiInvocation,
    filters: &InvocationRecordsFilters,
    source_scope: InvocationSourceScope,
) -> bool {
    if source_scope == InvocationSourceScope::ProxyOnly && record.source != SOURCE_PROXY {
        return false;
    }
    if let Some(from_bound) = filters.occurred_from.as_deref()
        && record.occurred_at.as_str() < from_bound
    {
        return false;
    }
    if let Some(to_bound) = filters.occurred_to.as_deref()
        && record.occurred_at.as_str() >= to_bound
    {
        return false;
    }
    if let Some(model) = filters.model.as_deref()
        && !runtime_text_equals(record.model.as_deref(), model)
    {
        return false;
    }
    if let Some(status) = filters.status.as_deref() {
        let normalized_status = status.trim();
        if !runtime_text_equals(record.status.as_deref(), normalized_status) {
            return false;
        }
    }
    if let Some(endpoint) = filters.endpoint.as_deref()
        && !runtime_text_equals(record.endpoint.as_deref(), endpoint)
    {
        return false;
    }
    if let Some(request_id) = filters.request_id.as_deref()
        && !runtime_text_equals(Some(record.invoke_id.as_str()), request_id)
    {
        return false;
    }
    if let Some(failure_class) = filters.failure_class.as_deref()
        && !runtime_text_equals(record.failure_class.as_deref(), failure_class)
    {
        return false;
    }
    if let Some(failure_kind) = filters.failure_kind.as_deref()
        && !runtime_text_equals(record.failure_kind.as_deref(), failure_kind)
    {
        return false;
    }
    if let Some(prompt_cache_key) = filters.prompt_cache_key.as_deref()
        && !runtime_text_equals(record.prompt_cache_key.as_deref(), prompt_cache_key)
    {
        return false;
    }
    if let Some(sticky_key) = filters.sticky_key.as_deref()
        && !runtime_text_equals(runtime_sticky_key(record), sticky_key)
    {
        return false;
    }
    if let Some(upstream_scope) = filters.upstream_scope.as_deref()
        && !runtime_text_equals(Some(runtime_upstream_scope(record)), upstream_scope)
    {
        return false;
    }
    if let Some(upstream_account_id) = filters.upstream_account_id
        && record.upstream_account_id != Some(upstream_account_id)
    {
        return false;
    }
    if let Some(requester_ip) = filters.requester_ip.as_deref()
        && !runtime_text_equals(record.requester_ip.as_deref(), requester_ip)
    {
        return false;
    }
    if let Some(keyword) = filters.keyword.as_deref()
        && !runtime_keyword_matches(record, keyword)
    {
        return false;
    }
    if let Some(min_total_tokens) = filters.min_total_tokens {
        let Some(total_tokens) = record.total_tokens else {
            return false;
        };
        if total_tokens < min_total_tokens {
            return false;
        }
    }
    if let Some(max_total_tokens) = filters.max_total_tokens {
        let Some(total_tokens) = record.total_tokens else {
            return false;
        };
        if total_tokens > max_total_tokens {
            return false;
        }
    }
    if let Some(min_total_ms) = filters.min_total_ms {
        let Some(total_ms) = record.t_total_ms else {
            return false;
        };
        if total_ms < min_total_ms {
            return false;
        }
    }
    if let Some(max_total_ms) = filters.max_total_ms {
        let Some(total_ms) = record.t_total_ms else {
            return false;
        };
        if total_ms > max_total_ms {
            return false;
        }
    }
    true
}

pub(crate) fn runtime_in_flight_record_matches_filters(
    record: &ApiInvocation,
    filters: &InvocationRecordsFilters,
    source_scope: InvocationSourceScope,
) -> bool {
    runtime_record_is_in_flight(record)
        && runtime_record_matches_filters(record, filters, source_scope)
}

pub(crate) fn option_presence_order(
    left_some: bool,
    right_some: bool,
) -> Option<std::cmp::Ordering> {
    match (left_some, right_some) {
        (true, false) => Some(std::cmp::Ordering::Less),
        (false, true) => Some(std::cmp::Ordering::Greater),
        (false, false) => Some(std::cmp::Ordering::Equal),
        (true, true) => None,
    }
}

pub(crate) fn apply_runtime_sort_order(
    ordering: std::cmp::Ordering,
    sort_order: InvocationSortOrder,
) -> std::cmp::Ordering {
    match sort_order {
        InvocationSortOrder::Asc => ordering,
        InvocationSortOrder::Desc => ordering.reverse(),
    }
}

pub(crate) fn compare_runtime_option_i64(
    left: Option<i64>,
    right: Option<i64>,
    sort_order: InvocationSortOrder,
) -> std::cmp::Ordering {
    option_presence_order(left.is_some(), right.is_some()).unwrap_or_else(|| {
        apply_runtime_sort_order(
            left.unwrap_or_default().cmp(&right.unwrap_or_default()),
            sort_order,
        )
    })
}

pub(crate) fn compare_runtime_option_f64(
    left: Option<f64>,
    right: Option<f64>,
    sort_order: InvocationSortOrder,
) -> std::cmp::Ordering {
    option_presence_order(left.is_some(), right.is_some()).unwrap_or_else(|| {
        apply_runtime_sort_order(
            left.unwrap_or_default()
                .partial_cmp(&right.unwrap_or_default())
                .unwrap_or(std::cmp::Ordering::Equal),
            sort_order,
        )
    })
}

pub(crate) fn compare_runtime_option_str(
    left: Option<&str>,
    right: Option<&str>,
    sort_order: InvocationSortOrder,
) -> std::cmp::Ordering {
    option_presence_order(left.is_some(), right.is_some()).unwrap_or_else(|| {
        apply_runtime_sort_order(
            left.unwrap_or_default().cmp(right.unwrap_or_default()),
            sort_order,
        )
    })
}

pub(crate) fn invocation_display_status_value(record: &ApiInvocation) -> Option<&str> {
    let status = record.status.as_deref().map(str::trim).unwrap_or_default();
    let failure_class = record
        .failure_class
        .as_deref()
        .map(str::trim)
        .unwrap_or_default();
    if status.eq_ignore_ascii_case("interrupted") {
        Some("interrupted")
    } else if matches!(
        failure_class,
        "service_failure" | "client_failure" | "client_abort"
    ) {
        Some("failed")
    } else if status.is_empty() {
        Some("unknown")
    } else {
        record.status.as_deref()
    }
}

pub(crate) fn compare_runtime_invocation_records(
    left: &ApiInvocation,
    right: &ApiInvocation,
    sort_by: InvocationSortBy,
    sort_order: InvocationSortOrder,
) -> std::cmp::Ordering {
    let primary = match sort_by {
        InvocationSortBy::OccurredAt => compare_runtime_option_str(
            Some(left.occurred_at.as_str()),
            Some(right.occurred_at.as_str()),
            sort_order,
        )
        .then_with(|| apply_runtime_sort_order(left.id.cmp(&right.id), sort_order)),
        InvocationSortBy::TotalTokens => {
            compare_runtime_option_i64(left.total_tokens, right.total_tokens, sort_order)
        }
        InvocationSortBy::Cost => compare_runtime_option_f64(left.cost, right.cost, sort_order),
        InvocationSortBy::TotalMs => {
            compare_runtime_option_f64(left.t_total_ms, right.t_total_ms, sort_order)
        }
        InvocationSortBy::TtfbMs => compare_runtime_option_f64(
            left.t_upstream_ttfb_ms,
            right.t_upstream_ttfb_ms,
            sort_order,
        ),
        InvocationSortBy::Status => compare_runtime_option_str(
            invocation_display_status_value(left),
            invocation_display_status_value(right),
            sort_order,
        ),
    };
    primary
        .then_with(|| right.occurred_at.cmp(&left.occurred_at))
        .then_with(|| right.id.cmp(&left.id))
}

pub(crate) fn should_overlay_runtime_records(request: &InvocationListRequest) -> bool {
    request.snapshot_id.is_none()
}

pub(crate) fn runtime_overlay_snapshot(state: &AppState) -> Vec<ApiInvocation> {
    state.proxy_runtime_invocations.snapshot()
}

pub(crate) async fn query_current_runtime_db_keys(
    pool: &Pool<Sqlite>,
    filters: &InvocationRecordsFilters,
    source_scope: InvocationSourceScope,
    snapshot: Option<SnapshotConstraint>,
) -> Result<HashSet<(String, String)>, ApiError> {
    #[derive(Debug, FromRow)]
    struct RuntimeKeyRow {
        invoke_id: String,
        occurred_at: String,
    }

    let mut query = QueryBuilder::<Sqlite>::new(
        "SELECT invoke_id, occurred_at FROM codex_invocations WHERE 1 = 1",
    );
    apply_invocation_records_filters(&mut query, filters, source_scope, snapshot);
    query.push(" AND LOWER(TRIM(COALESCE(status, ''))) IN ('running', 'pending')");

    Ok(query
        .build_query_as::<RuntimeKeyRow>()
        .fetch_all(pool)
        .await?
        .into_iter()
        .map(|row| (row.invoke_id, row.occurred_at))
        .collect())
}

async fn count_stale_runtime_db_rows_before_target(
    pool: &Pool<Sqlite>,
    stale_keys: &HashSet<(String, String)>,
    target_occurred_at: &str,
    target_id: i64,
    snapshot_id: i64,
) -> Result<i64, ApiError> {
    #[derive(Debug, FromRow)]
    struct CountRow {
        total: i64,
    }

    let mut count = stale_keys
        .iter()
        .filter(|(_, occurred_at)| occurred_at.as_str() > target_occurred_at)
        .count() as i64;
    let equal_time_keys = stale_keys
        .iter()
        .filter(|(_, occurred_at)| occurred_at.as_str() == target_occurred_at)
        .collect::<Vec<_>>();
    for chunk in equal_time_keys.chunks(100) {
        let mut query = QueryBuilder::<Sqlite>::new(
            "SELECT COUNT(*) AS total FROM codex_invocations WHERE id <= ",
        );
        query
            .push_bind(snapshot_id)
            .push(" AND id > ")
            .push_bind(target_id)
            .push(" AND occurred_at = ")
            .push_bind(target_occurred_at.to_string())
            .push(" AND (");
        for (index, (invoke_id, occurred_at)) in chunk.iter().enumerate() {
            if index > 0 {
                query.push(" OR ");
            }
            query
                .push("(invoke_id = ")
                .push_bind(invoke_id)
                .push(" AND occurred_at = ")
                .push_bind(occurred_at)
                .push(")");
        }
        query.push(")");
        count += query
            .build_query_as::<CountRow>()
            .fetch_one(pool)
            .await?
            .total;
    }
    Ok(count)
}

pub(crate) async fn query_terminal_db_keys_for_runtime_records(
    pool: &Pool<Sqlite>,
    runtime_records: &[ApiInvocation],
    snapshot: Option<SnapshotConstraint>,
) -> Result<HashSet<(String, String)>, ApiError> {
    #[derive(Debug, FromRow)]
    struct RuntimeKeyRow {
        invoke_id: String,
        occurred_at: String,
    }

    let keys = runtime_records
        .iter()
        .map(|record| (record.invoke_id.clone(), record.occurred_at.clone()))
        .collect::<HashSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    if keys.is_empty() {
        return Ok(HashSet::new());
    }

    let mut terminal_keys = HashSet::new();
    for chunk in keys.chunks(100) {
        let mut query = QueryBuilder::<Sqlite>::new(
            "SELECT invoke_id, occurred_at FROM codex_invocations \
             WHERE LOWER(TRIM(COALESCE(status, ''))) NOT IN ('running', 'pending') \
             AND (",
        );
        for (index, (invoke_id, occurred_at)) in chunk.iter().enumerate() {
            if index > 0 {
                query.push(" OR ");
            }
            query
                .push("(invoke_id = ")
                .push_bind(invoke_id)
                .push(" AND occurred_at = ")
                .push_bind(occurred_at)
                .push(")");
        }
        query.push(")");
        if let Some(snapshot_constraint) = snapshot {
            match snapshot_constraint {
                SnapshotConstraint::UpTo(snapshot_id) => {
                    query.push(" AND id <= ").push_bind(snapshot_id);
                }
                SnapshotConstraint::After(snapshot_id) => {
                    query.push(" AND id > ").push_bind(snapshot_id);
                }
            }
        }
        terminal_keys.extend(
            query
                .build_query_as::<RuntimeKeyRow>()
                .fetch_all(pool)
                .await?
                .into_iter()
                .map(|row| (row.invoke_id, row.occurred_at)),
        );
    }
    Ok(terminal_keys)
}

pub(crate) fn overlay_runtime_records_for_current_page(
    request: &InvocationListRequest,
    source_scope: InvocationSourceScope,
    runtime_records: Vec<ApiInvocation>,
    db_runtime_keys: &HashSet<(String, String)>,
    db_terminal_keys: &HashSet<(String, String)>,
    db_records_are_prefix: bool,
    mut records: Vec<ApiInvocation>,
    total: i64,
    endpoint: &'static str,
) -> (Vec<ApiInvocation>, i64) {
    if runtime_records.is_empty() {
        return (records, total);
    }
    let runtime_by_key = runtime_records
        .into_iter()
        .map(|record| {
            (
                (record.invoke_id.clone(), record.occurred_at.clone()),
                record,
            )
        })
        .collect::<HashMap<_, _>>();
    let mut runtime_overlay_row_count = 0_usize;
    let mut stale_db_runtime_row_count = 0_usize;
    records = records
        .into_iter()
        .filter_map(|mut record| {
            let key = (record.invoke_id.clone(), record.occurred_at.clone());
            let Some(runtime_record) = runtime_by_key.get(&key) else {
                return Some(record);
            };
            match normalized_runtime_text(record.status.as_deref()).as_str() {
                "running" | "pending" => {
                    if runtime_record_matches_filters(
                        runtime_record,
                        &request.filters,
                        source_scope,
                    ) {
                        record = runtime_record.clone();
                        runtime_overlay_row_count += 1;
                        Some(record)
                    } else {
                        stale_db_runtime_row_count += 1;
                        None
                    }
                }
                _ => Some(record),
            }
        })
        .collect();
    let stale_db_runtime_total_count = db_runtime_keys
        .iter()
        .filter(|key| {
            runtime_by_key.get(*key).is_some_and(|record| {
                !runtime_record_matches_filters(record, &request.filters, source_scope)
            })
        })
        .count();
    let runtime_new_records = runtime_by_key
        .iter()
        .filter(|(key, record)| {
            !db_runtime_keys.contains(*key)
                && !db_terminal_keys.contains(*key)
                && runtime_record_matches_filters(record, &request.filters, source_scope)
        })
        .map(|(_, record)| record.clone())
        .collect::<Vec<_>>();
    let runtime_new_row_count = runtime_new_records.len();
    let effective_stale_db_runtime_total_count = if db_runtime_keys.is_empty() {
        stale_db_runtime_row_count
    } else {
        stale_db_runtime_total_count
    };
    runtime_overlay_row_count += runtime_new_row_count;
    records.extend(runtime_new_records);
    records.sort_by(|left, right| {
        compare_runtime_invocation_records(left, right, request.sort_by, request.sort_order)
    });
    if db_records_are_prefix {
        let offset = (request.page - 1).saturating_mul(request.page_size) as usize;
        records = records
            .into_iter()
            .skip(offset)
            .take(request.page_size as usize)
            .collect();
    } else {
        records.truncate(request.page_size as usize);
    }
    if runtime_overlay_row_count > 0 {
        debug!(
            endpoint,
            runtime_overlay_row_count,
            stale_db_runtime_row_count,
            stale_db_runtime_total_count = effective_stale_db_runtime_total_count,
            "overlayed memory runtime invocation records into current response"
        );
    }
    (
        records,
        total.saturating_sub(effective_stale_db_runtime_total_count as i64)
            + runtime_new_row_count as i64,
    )
}

pub(crate) fn runtime_overlay_total_delta(
    request: &InvocationListRequest,
    source_scope: InvocationSourceScope,
    runtime_records: &[ApiInvocation],
    db_runtime_keys: &HashSet<(String, String)>,
    db_terminal_keys: &HashSet<(String, String)>,
) -> (RuntimeSummaryOverlayDelta, usize, usize) {
    if runtime_records.is_empty() {
        return (RuntimeSummaryOverlayDelta::default(), 0, 0);
    }
    let runtime_by_key = runtime_records
        .iter()
        .map(|record| {
            (
                (record.invoke_id.clone(), record.occurred_at.clone()),
                record,
            )
        })
        .collect::<HashMap<_, _>>();
    let stale_db_runtime_count = db_runtime_keys
        .iter()
        .filter(|key| {
            runtime_by_key.get(*key).is_some_and(|record| {
                !runtime_record_matches_filters(record, &request.filters, source_scope)
            })
        })
        .count();
    let mut delta = RuntimeSummaryOverlayDelta {
        total_count: -(stale_db_runtime_count as i64),
        ..RuntimeSummaryOverlayDelta::default()
    };
    let mut runtime_new_count = 0_usize;
    for (key, record) in &runtime_by_key {
        if db_terminal_keys.contains(key)
            || !runtime_record_matches_filters(record, &request.filters, source_scope)
        {
            continue;
        }
        let has_db_runtime_row = db_runtime_keys.contains(key);
        if !has_db_runtime_row {
            delta.total_count += 1;
            runtime_new_count += 1;
        }
        if runtime_record_is_in_flight(record) {
            continue;
        }
        delta.add_terminal_record(record);
    }
    (delta, runtime_new_count, stale_db_runtime_count)
}

#[derive(Debug, Default)]
pub(crate) struct RuntimeSummaryOverlayDelta {
    total_count: i64,
    success_count: i64,
    failure_count: i64,
    total_tokens: i64,
    total_cost: f64,
    cache_input_tokens: i64,
    service_failure_count: i64,
    client_failure_count: i64,
    client_abort_count: i64,
}

impl RuntimeSummaryOverlayDelta {
    fn add_terminal_record(&mut self, record: &ApiInvocation) {
        self.total_tokens += record.total_tokens.unwrap_or_default();
        self.total_cost += record.cost.unwrap_or_default();
        self.cache_input_tokens += record.cache_input_tokens.unwrap_or_default();
        let failure_class = normalized_runtime_text(record.failure_class.as_deref());
        if failure_class == "none" && runtime_record_is_success_for_summary(record) {
            self.success_count += 1;
        }
        match failure_class.as_str() {
            "service_failure" => {
                self.failure_count += 1;
                self.service_failure_count += 1;
            }
            "client_failure" => {
                self.failure_count += 1;
                self.client_failure_count += 1;
            }
            "client_abort" => {
                self.failure_count += 1;
                self.client_abort_count += 1;
            }
            _ => {}
        }
    }
}

pub(crate) fn runtime_record_is_success_for_summary(record: &ApiInvocation) -> bool {
    let status = normalized_runtime_text(record.status.as_deref());
    status == "success"
        || status == "completed"
        || (status == "http_200"
            && normalized_runtime_text(record.error_message.as_deref()).is_empty())
}

pub(crate) async fn query_invocation_network_summary(
    pool: &Pool<Sqlite>,
    filters: &InvocationRecordsFilters,
    source_scope: InvocationSourceScope,
    snapshot_id: i64,
) -> Result<InvocationNetworkSummary> {
    #[derive(Debug, FromRow)]
    struct ValueRow {
        value: Option<f64>,
    }

    let mut agg_query = QueryBuilder::new(
        "SELECT \
         AVG(CASE WHEN t_upstream_ttfb_ms IS NOT NULL AND t_upstream_ttfb_ms >= 0 THEN t_upstream_ttfb_ms END) AS avg_ttfb_ms, \
         COALESCE(SUM(CASE WHEN t_upstream_ttfb_ms IS NOT NULL AND t_upstream_ttfb_ms >= 0 THEN 1 ELSE 0 END), 0) AS ttfb_count, \
         AVG(CASE WHEN t_total_ms IS NOT NULL AND t_total_ms >= 0 THEN t_total_ms END) AS avg_total_ms, \
         COALESCE(SUM(CASE WHEN t_total_ms IS NOT NULL AND t_total_ms >= 0 THEN 1 ELSE 0 END), 0) AS total_count \
         FROM codex_invocations WHERE 1 = 1",
    );
    apply_invocation_records_filters(
        &mut agg_query,
        filters,
        source_scope,
        Some(SnapshotConstraint::UpTo(snapshot_id)),
    );
    let agg = agg_query
        .build_query_as::<InvocationNetworkAggRow>()
        .fetch_one(pool)
        .await?;

    async fn query_sorted_value(
        pool: &Pool<Sqlite>,
        filters: &InvocationRecordsFilters,
        source_scope: InvocationSourceScope,
        snapshot_id: i64,
        column: &'static str,
        offset: i64,
    ) -> Result<Option<f64>> {
        let mut query = QueryBuilder::new("SELECT ");
        query
            .push(column)
            .push(" AS value FROM codex_invocations WHERE 1 = 1");
        apply_invocation_records_filters(
            &mut query,
            filters,
            source_scope,
            Some(SnapshotConstraint::UpTo(snapshot_id)),
        );
        query
            .push(" AND ")
            .push(column)
            .push(" IS NOT NULL AND ")
            .push(column)
            .push(" >= 0");
        query.push(" ORDER BY ").push(column).push(" ASC");
        query.push(" LIMIT 1 OFFSET ").push_bind(offset.max(0));

        Ok(query
            .build_query_as::<ValueRow>()
            .fetch_optional(pool)
            .await?
            .and_then(|row| row.value))
    }

    fn resolve_p95_offsets(count: i64) -> Option<(i64, i64, f64)> {
        if count <= 0 {
            return None;
        }
        if count == 1 {
            return Some((0, 0, 0.0));
        }

        let rank = 0.95_f64 * (count.saturating_sub(1) as f64);
        let lower = rank.floor() as i64;
        let upper = rank.ceil() as i64;
        let weight = rank - lower as f64;
        Some((lower, upper, weight))
    }

    async fn query_p95(
        pool: &Pool<Sqlite>,
        filters: &InvocationRecordsFilters,
        source_scope: InvocationSourceScope,
        snapshot_id: i64,
        column: &'static str,
        count: i64,
    ) -> Result<Option<f64>> {
        let Some((lower, upper, weight)) = resolve_p95_offsets(count) else {
            return Ok(None);
        };

        let Some(lower_value) =
            query_sorted_value(pool, filters, source_scope, snapshot_id, column, lower).await?
        else {
            return Ok(None);
        };

        if lower == upper {
            return Ok(Some(lower_value));
        }

        let Some(upper_value) =
            query_sorted_value(pool, filters, source_scope, snapshot_id, column, upper).await?
        else {
            return Ok(None);
        };

        Ok(Some(lower_value + (upper_value - lower_value) * weight))
    }

    Ok(InvocationNetworkSummary {
        avg_ttfb_ms: agg.avg_ttfb_ms,
        p95_ttfb_ms: query_p95(
            pool,
            filters,
            source_scope,
            snapshot_id,
            "t_upstream_ttfb_ms",
            agg.ttfb_count,
        )
        .await?,
        avg_total_ms: agg.avg_total_ms,
        p95_total_ms: query_p95(
            pool,
            filters,
            source_scope,
            snapshot_id,
            "t_total_ms",
            agg.total_count,
        )
        .await?,
    })
}

pub(crate) async fn query_invocation_new_records_count(
    pool: &Pool<Sqlite>,
    filters: &InvocationRecordsFilters,
    source_scope: InvocationSourceScope,
    snapshot_id: i64,
) -> Result<i64> {
    #[derive(Debug, FromRow)]
    struct NewCountRow {
        total: i64,
    }

    let mut new_count_query =
        QueryBuilder::new("SELECT COUNT(*) AS total FROM codex_invocations WHERE 1 = 1");
    apply_invocation_records_filters(
        &mut new_count_query,
        filters,
        source_scope,
        Some(SnapshotConstraint::After(snapshot_id)),
    );

    Ok(new_count_query
        .build_query_as::<NewCountRow>()
        .fetch_one(pool)
        .await?
        .total)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum InvocationSuggestionField {
    Model,
    Endpoint,
    FailureKind,
    PromptCacheKey,
    RequesterIp,
}

impl InvocationSuggestionField {
    fn parse(raw: Option<&str>) -> Result<Option<Self>, ApiError> {
        let Some(value) = normalize_query_text(raw) else {
            return Ok(None);
        };
        let normalized = value.to_ascii_lowercase();

        match normalized.as_str() {
            "model" => Ok(Some(Self::Model)),
            "endpoint" => Ok(Some(Self::Endpoint)),
            "failurekind" => Ok(Some(Self::FailureKind)),
            "promptcachekey" => Ok(Some(Self::PromptCacheKey)),
            "requesterip" => Ok(Some(Self::RequesterIp)),
            _ => Err(ApiError::bad_request(anyhow!(
                "unsupported suggestField: {value}"
            ))),
        }
    }

    fn sql_expr(self) -> &'static str {
        match self {
            Self::Model => "model",
            Self::Endpoint => INVOCATION_ENDPOINT_SQL,
            Self::FailureKind => INVOCATION_FAILURE_KIND_SQL,
            Self::PromptCacheKey => INVOCATION_PROMPT_CACHE_KEY_SQL,
            Self::RequesterIp => INVOCATION_REQUESTER_IP_SQL,
        }
    }

    fn clear_field_filter(self, filters: &InvocationRecordsFilters) -> InvocationRecordsFilters {
        let mut next = filters.clone();
        match self {
            Self::Model => next.model = None,
            Self::Endpoint => next.endpoint = None,
            Self::FailureKind => next.failure_kind = None,
            Self::PromptCacheKey => next.prompt_cache_key = None,
            Self::RequesterIp => next.requester_ip = None,
        }
        next
    }
}

pub(crate) fn empty_invocation_suggestion_bucket() -> InvocationSuggestionBucket {
    InvocationSuggestionBucket {
        items: Vec::new(),
        has_more: false,
    }
}

pub(crate) fn suggestion_response_for_field(
    field: InvocationSuggestionField,
    bucket: InvocationSuggestionBucket,
) -> InvocationSuggestionsResponse {
    let empty = || empty_invocation_suggestion_bucket();
    match field {
        InvocationSuggestionField::Model => InvocationSuggestionsResponse {
            model: bucket,
            endpoint: empty(),
            failure_kind: empty(),
            prompt_cache_key: empty(),
            requester_ip: empty(),
        },
        InvocationSuggestionField::Endpoint => InvocationSuggestionsResponse {
            model: empty(),
            endpoint: bucket,
            failure_kind: empty(),
            prompt_cache_key: empty(),
            requester_ip: empty(),
        },
        InvocationSuggestionField::FailureKind => InvocationSuggestionsResponse {
            model: empty(),
            endpoint: empty(),
            failure_kind: bucket,
            prompt_cache_key: empty(),
            requester_ip: empty(),
        },
        InvocationSuggestionField::PromptCacheKey => InvocationSuggestionsResponse {
            model: empty(),
            endpoint: empty(),
            failure_kind: empty(),
            prompt_cache_key: bucket,
            requester_ip: empty(),
        },
        InvocationSuggestionField::RequesterIp => InvocationSuggestionsResponse {
            model: empty(),
            endpoint: empty(),
            failure_kind: empty(),
            prompt_cache_key: empty(),
            requester_ip: bucket,
        },
    }
}

pub(crate) async fn query_invocation_suggestion_bucket(
    pool: &Pool<Sqlite>,
    filters: &InvocationRecordsFilters,
    source_scope: InvocationSourceScope,
    snapshot: Option<SnapshotConstraint>,
    sql_expr: &str,
    match_query: Option<&str>,
    limit: i64,
) -> Result<InvocationSuggestionBucket> {
    #[derive(Debug, FromRow)]
    struct SuggestionRow {
        value: Option<String>,
        count: i64,
    }

    let mut query = QueryBuilder::new("SELECT MIN(TRIM(COALESCE(");
    query.push(sql_expr);
    query.push(", ''))) AS value, COUNT(*) AS count FROM codex_invocations WHERE 1 = 1");
    apply_invocation_records_filters(&mut query, filters, source_scope, snapshot);
    query.push(" AND TRIM(COALESCE(");
    query.push(sql_expr);
    query.push(", '')) != ''");
    if let Some(match_query) = match_query {
        let like_pattern = format!("%{}%", escape_sql_like(&match_query.to_lowercase()));
        query.push(" AND LOWER(TRIM(COALESCE(");
        query.push(sql_expr);
        query.push(", ''))) LIKE ");
        query.push_bind(like_pattern).push(" ESCAPE '\\'");
    }
    query.push(" GROUP BY LOWER(TRIM(COALESCE(");
    query.push(sql_expr);
    query.push(", '')))");
    query.push(" ORDER BY count DESC, value ASC");
    query.push(" LIMIT ").push_bind(limit.saturating_add(1));

    let rows = query
        .build_query_as::<SuggestionRow>()
        .fetch_all(pool)
        .await?;

    let has_more = rows.len() as i64 > limit;
    let items = rows
        .into_iter()
        .take(limit.max(0) as usize)
        .filter_map(|row| {
            let value = row.value?.trim().to_string();
            if value.is_empty() {
                None
            } else {
                Some(InvocationSuggestionItem {
                    value,
                    count: row.count,
                })
            }
        })
        .collect::<Vec<_>>();

    Ok(InvocationSuggestionBucket { items, has_more })
}

pub(crate) fn is_legacy_invocation_stream_query(params: &ListQuery) -> bool {
    params.limit.is_some()
        && params.page.is_none()
        && params.page_size.is_none()
        && params.snapshot_id.is_none()
        && params.sort_by.is_none()
        && params.sort_order.is_none()
        && params.range_preset.is_none()
        && params.from.is_none()
        && params.to.is_none()
        && params.endpoint.is_none()
        && params.request_id.is_none()
        && params.failure_class.is_none()
        && params.failure_kind.is_none()
        && params.prompt_cache_key.is_none()
        && params.upstream_scope.is_none()
        && params.requester_ip.is_none()
        && params.keyword.is_none()
        && params.min_total_tokens.is_none()
        && params.max_total_tokens.is_none()
        && params.min_total_ms.is_none()
        && params.max_total_ms.is_none()
}

pub(crate) async fn query_invocation_exception_summary(
    pool: &Pool<Sqlite>,
    filters: &InvocationRecordsFilters,
    source_scope: InvocationSourceScope,
    snapshot_id: i64,
) -> Result<InvocationExceptionSummary> {
    let mut query = QueryBuilder::new("SELECT ");
    query
        .push("COALESCE(SUM(CASE WHEN ")
        .push(INVOCATION_RESOLVED_FAILURE_CLASS_SQL)
        .push(" NOT IN ('', 'none') THEN 1 ELSE 0 END), 0) AS failure_count, ")
        .push("COALESCE(SUM(CASE WHEN ")
        .push(INVOCATION_RESOLVED_FAILURE_CLASS_SQL)
        .push(" = 'service_failure' THEN 1 ELSE 0 END), 0) AS service_failure_count, ")
        .push("COALESCE(SUM(CASE WHEN ")
        .push(INVOCATION_RESOLVED_FAILURE_CLASS_SQL)
        .push(" = 'client_failure' THEN 1 ELSE 0 END), 0) AS client_failure_count, ")
        .push("COALESCE(SUM(CASE WHEN ")
        .push(INVOCATION_RESOLVED_FAILURE_CLASS_SQL)
        .push(" = 'client_abort' THEN 1 ELSE 0 END), 0) AS client_abort_count, ")
        .push("COALESCE(SUM(CASE WHEN ")
        .push(INVOCATION_RESOLVED_FAILURE_CLASS_SQL)
        .push(" = 'service_failure' THEN 1 ELSE 0 END), 0) AS actionable_failure_count ")
        .push("FROM codex_invocations WHERE 1 = 1");
    apply_invocation_records_filters(
        &mut query,
        filters,
        source_scope,
        Some(SnapshotConstraint::UpTo(snapshot_id)),
    );
    let agg = query
        .build_query_as::<InvocationExceptionAggRow>()
        .fetch_one(pool)
        .await?;
    Ok(InvocationExceptionSummary {
        failure_count: agg.failure_count,
        service_failure_count: agg.service_failure_count,
        client_failure_count: agg.client_failure_count,
        client_abort_count: agg.client_abort_count,
        actionable_failure_count: agg.actionable_failure_count,
    })
}

pub(crate) async fn list_invocations(
    State(state): State<Arc<AppState>>,
    Query(params): Query<ListQuery>,
) -> Result<Json<ListResponse>, ApiError> {
    let runtime_overlay = load_invocation_anchor_runtime_records(&params)?;
    list_invocations_with_runtime_overlay(state, params, runtime_overlay).await
}

async fn list_invocations_with_runtime_overlay(
    state: Arc<AppState>,
    params: ListQuery,
    runtime_overlay_override: Option<Vec<ApiInvocation>>,
) -> Result<Json<ListResponse>, ApiError> {
    let request = build_invocation_list_request(&params, state.config.list_limit_max as i64)?;
    let source_scope = resolve_default_source_scope(&state.pool).await?;
    let runtime_overlay_records = match runtime_overlay_override {
        Some(records) => records,
        None if should_overlay_runtime_records(&request) => {
            runtime_overlay_snapshot(state.as_ref())
        }
        None => Vec::new(),
    };
    if is_legacy_invocation_stream_query(&params) {
        let db_terminal_keys = if runtime_overlay_records.is_empty() {
            HashSet::new()
        } else {
            query_terminal_db_keys_for_runtime_records(&state.pool, &runtime_overlay_records, None)
                .await?
        };
        let mut query = build_invocation_select_query();
        apply_invocation_records_filters(&mut query, &request.filters, source_scope, None);
        append_invocation_order_clause(&mut query, request.sort_by, request.sort_order);
        query.push(" LIMIT ").push_bind(request.page_size);

        let records = query
            .build_query_as::<ApiInvocation>()
            .fetch_all(&state.pool)
            .await?;
        let total = records.len() as i64;
        let (records, total) = overlay_runtime_records_for_current_page(
            &request,
            source_scope,
            runtime_overlay_records,
            &HashSet::new(),
            &db_terminal_keys,
            false,
            records,
            total,
            "invocation_records_legacy",
        );

        return Ok(Json(ListResponse {
            snapshot_id: 0,
            total,
            page: 1,
            page_size: request.page_size,
            records,
        }));
    }

    let snapshot_id = request
        .snapshot_id
        .unwrap_or(resolve_invocation_snapshot_id(&state.pool, source_scope).await?);
    let db_terminal_keys = if runtime_overlay_records.is_empty() {
        HashSet::new()
    } else {
        query_terminal_db_keys_for_runtime_records(
            &state.pool,
            &runtime_overlay_records,
            Some(SnapshotConstraint::UpTo(snapshot_id)),
        )
        .await?
    };

    #[derive(Debug, FromRow)]
    struct CountRow {
        total: i64,
    }

    let mut count_query =
        QueryBuilder::new("SELECT COUNT(*) AS total FROM codex_invocations WHERE 1 = 1");
    apply_invocation_records_filters(
        &mut count_query,
        &request.filters,
        source_scope,
        Some(SnapshotConstraint::UpTo(snapshot_id)),
    );
    let mut tx = state.pool.begin().await?;
    let total = count_query
        .build_query_as::<CountRow>()
        .fetch_one(&mut *tx)
        .await?
        .total;
    let db_runtime_keys = if runtime_overlay_records.is_empty() {
        HashSet::new()
    } else {
        query_current_runtime_db_keys(
            &state.pool,
            &request.filters,
            source_scope,
            Some(SnapshotConstraint::UpTo(snapshot_id)),
        )
        .await?
    };

    let offset = (request.page - 1).saturating_mul(request.page_size);
    #[derive(Debug, FromRow)]
    struct PageIdRow {
        id: i64,
    }

    let mut page_id_query = QueryBuilder::new("SELECT id FROM codex_invocations WHERE 1 = 1");
    apply_invocation_records_filters(
        &mut page_id_query,
        &request.filters,
        source_scope,
        Some(SnapshotConstraint::UpTo(snapshot_id)),
    );
    append_invocation_order_clause(&mut page_id_query, request.sort_by, request.sort_order);
    let db_page_size = if runtime_overlay_records.is_empty() {
        request.page_size
    } else {
        offset.saturating_add(request.page_size)
    };
    let db_offset = if runtime_overlay_records.is_empty() {
        offset
    } else {
        0
    };
    page_id_query
        .push(" LIMIT ")
        .push_bind(db_page_size)
        .push(" OFFSET ")
        .push_bind(db_offset);
    let page_ids = page_id_query
        .build_query_as::<PageIdRow>()
        .fetch_all(&mut *tx)
        .await?
        .into_iter()
        .map(|row| row.id)
        .collect::<Vec<_>>();

    if page_ids.is_empty() {
        let (records, total) = overlay_runtime_records_for_current_page(
            &request,
            source_scope,
            runtime_overlay_records,
            &db_runtime_keys,
            &db_terminal_keys,
            true,
            Vec::new(),
            total,
            "invocation_records",
        );
        return Ok(Json(ListResponse {
            snapshot_id,
            total,
            page: request.page,
            page_size: request.page_size,
            records,
        }));
    }

    let mut query = build_invocation_select_query();
    apply_invocation_records_filters(
        &mut query,
        &request.filters,
        source_scope,
        Some(SnapshotConstraint::UpTo(snapshot_id)),
    );
    query.push(" AND id IN (");
    {
        let mut separated = query.separated(", ");
        for &id in &page_ids {
            separated.push_bind(id);
        }
    }
    query.push(")");

    let mut records = query
        .build_query_as::<ApiInvocation>()
        .fetch_all(&mut *tx)
        .await?;
    let page_positions = page_ids
        .into_iter()
        .enumerate()
        .map(|(index, id)| (id, index))
        .collect::<HashMap<_, _>>();
    records.sort_by_key(|record| {
        page_positions
            .get(&record.id)
            .copied()
            .unwrap_or(usize::MAX)
    });
    let (records, total) = overlay_runtime_records_for_current_page(
        &request,
        source_scope,
        runtime_overlay_records,
        &db_runtime_keys,
        &db_terminal_keys,
        true,
        records,
        total,
        "invocation_records",
    );

    Ok(Json(ListResponse {
        snapshot_id,
        total,
        page: request.page,
        page_size: request.page_size,
        records,
    }))
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LocateInvocationResponse {
    pub(crate) anchor_id: String,
    pub(crate) snapshot_id: i64,
    pub(crate) total: i64,
    pub(crate) page: i64,
    pub(crate) page_size: i64,
    pub(crate) records: Vec<ApiInvocation>,
    pub(crate) target_index: usize,
    pub(crate) target_absolute_index: i64,
}

#[derive(Debug, FromRow)]
struct InvocationLocateRow {
    id: i64,
    occurred_at: String,
}

pub(crate) async fn locate_invocation_page(
    state: Arc<AppState>,
    params: &LocateInvocationQuery,
) -> Result<Option<LocateInvocationResponse>, ApiError> {
    let request_id = params.request_id.trim();
    if request_id.is_empty() {
        return Err(ApiError::bad_request(anyhow!("requestId is required")));
    }
    if params.upstream_account_id <= 0 {
        return Err(ApiError::bad_request(anyhow!(
            "upstreamAccountId must be positive"
        )));
    }

    let page_size = params
        .page_size
        .unwrap_or(50)
        .clamp(1, state.config.list_limit_max as i64);
    let source_scope = resolve_default_source_scope(&state.pool).await?;
    let snapshot_id = resolve_invocation_snapshot_id(&state.pool, source_scope).await?;
    let base_filters = InvocationRecordsFilters {
        upstream_account_id: Some(params.upstream_account_id),
        ..Default::default()
    };
    let runtime_records = runtime_overlay_snapshot(state.as_ref());
    let runtime_target = runtime_records.iter().find(|record| {
        runtime_text_equals(Some(record.invoke_id.as_str()), request_id)
            && runtime_record_matches_filters(record, &base_filters, source_scope)
    });

    let mut target_query =
        QueryBuilder::<Sqlite>::new("SELECT id, occurred_at FROM codex_invocations WHERE 1 = 1");
    apply_invocation_records_filters(
        &mut target_query,
        &base_filters,
        source_scope,
        Some(SnapshotConstraint::UpTo(snapshot_id)),
    );
    target_query
        .push(" AND invoke_id = ")
        .push_bind(request_id.to_string())
        .push(" ORDER BY occurred_at DESC, id DESC LIMIT 1");
    let db_target = target_query
        .build_query_as::<InvocationLocateRow>()
        .fetch_optional(&state.pool)
        .await?;

    let (target_occurred_at, target_id) = if let Some(row) = db_target.as_ref() {
        (row.occurred_at.as_str(), row.id)
    } else if let Some(record) = runtime_target {
        (record.occurred_at.as_str(), record.id)
    } else {
        return Ok(None);
    };

    #[derive(Debug, FromRow)]
    struct CountRow {
        total: i64,
    }

    let mut rank_query =
        QueryBuilder::<Sqlite>::new("SELECT COUNT(*) AS total FROM codex_invocations WHERE 1 = 1");
    apply_invocation_records_filters(
        &mut rank_query,
        &base_filters,
        source_scope,
        Some(SnapshotConstraint::UpTo(snapshot_id)),
    );
    rank_query
        .push(" AND (occurred_at > ")
        .push_bind(target_occurred_at.to_string())
        .push(" OR (occurred_at = ")
        .push_bind(target_occurred_at.to_string())
        .push(" AND id > ")
        .push_bind(target_id)
        .push("))");
    let db_rank = rank_query
        .build_query_as::<CountRow>()
        .fetch_one(&state.pool)
        .await?
        .total;

    let db_terminal_keys = if runtime_records.is_empty() {
        HashSet::new()
    } else {
        query_terminal_db_keys_for_runtime_records(
            &state.pool,
            &runtime_records,
            Some(SnapshotConstraint::UpTo(snapshot_id)),
        )
        .await?
    };
    let db_runtime_keys = if runtime_records.is_empty() {
        HashSet::new()
    } else {
        query_current_runtime_db_keys(
            &state.pool,
            &base_filters,
            source_scope,
            Some(SnapshotConstraint::UpTo(snapshot_id)),
        )
        .await?
    };
    let anchor_runtime_records = runtime_records
        .iter()
        .filter(|record| {
            let key = (record.invoke_id.clone(), record.occurred_at.clone());
            db_runtime_keys.contains(&key)
                || runtime_record_matches_filters(record, &base_filters, source_scope)
        })
        .cloned()
        .collect::<Vec<_>>();
    let runtime_by_key = runtime_records
        .iter()
        .map(|record| {
            (
                (record.invoke_id.clone(), record.occurred_at.clone()),
                record,
            )
        })
        .collect::<HashMap<_, _>>();
    let stale_db_runtime_keys = db_runtime_keys
        .iter()
        .filter(|key| {
            runtime_by_key.get(*key).is_some_and(|record| {
                !runtime_record_matches_filters(record, &base_filters, source_scope)
            })
        })
        .cloned()
        .collect::<HashSet<_>>();
    let stale_db_runtime_before_target = count_stale_runtime_db_rows_before_target(
        &state.pool,
        &stale_db_runtime_keys,
        target_occurred_at,
        target_id,
        snapshot_id,
    )
    .await?;
    let runtime_new_before_target = runtime_records
        .iter()
        .filter(|record| {
            let key = (record.invoke_id.clone(), record.occurred_at.clone());
            !db_runtime_keys.contains(&key)
                && !db_terminal_keys.contains(&key)
                && runtime_record_matches_filters(record, &base_filters, source_scope)
                && (record.occurred_at.as_str() > target_occurred_at
                    || (record.occurred_at.as_str() == target_occurred_at && record.id > target_id))
        })
        .count() as i64;
    let target_absolute_index = db_rank
        .saturating_sub(stale_db_runtime_before_target)
        .saturating_add(runtime_new_before_target);
    let target_page = target_absolute_index / page_size + 1;

    // A concurrent runtime transition can shift the page boundary by one. Probe only adjacent
    // windows when necessary; the response still contains exactly one relevant page.
    let candidate_pages = [
        target_page,
        target_page.saturating_sub(1).max(1),
        target_page.saturating_add(1),
    ];
    for page in candidate_pages {
        let Json(response) = list_invocations_with_runtime_overlay(
            state.clone(),
            ListQuery {
                page: Some(page),
                page_size: Some(page_size),
                snapshot_id: Some(snapshot_id),
                sort_by: Some("occurredAt".to_string()),
                sort_order: Some("desc".to_string()),
                upstream_account_id: Some(params.upstream_account_id),
                ..Default::default()
            },
            Some(anchor_runtime_records.clone()),
        )
        .await?;
        if let Some(target_index) = response
            .records
            .iter()
            .position(|record| runtime_text_equals(Some(record.invoke_id.as_str()), request_id))
        {
            let anchor_id = store_invocation_anchor_snapshot(
                snapshot_id,
                params.upstream_account_id,
                anchor_runtime_records.clone(),
            );
            return Ok(Some(LocateInvocationResponse {
                anchor_id,
                snapshot_id: response.snapshot_id,
                total: response.total,
                page: response.page,
                page_size: response.page_size,
                records: response.records,
                target_index,
                target_absolute_index: (response.page - 1) * response.page_size
                    + target_index as i64,
            }));
        }
    }

    Ok(None)
}

pub(crate) async fn locate_invocation(
    State(state): State<Arc<AppState>>,
    Query(params): Query<LocateInvocationQuery>,
) -> Result<axum::response::Response, ApiError> {
    match locate_invocation_page(state, &params).await? {
        Some(response) => Ok(Json(response).into_response()),
        None => Ok((
            StatusCode::NOT_FOUND,
            Json(json!({
                "code": "invocation_not_found",
                "message": "invocation record not found",
                "requestId": params.request_id.trim(),
            })),
        )
            .into_response()),
    }
}

pub(crate) async fn fetch_invocation_pool_attempts(
    State(state): State<Arc<AppState>>,
    axum::extract::Path(invoke_id): axum::extract::Path<String>,
) -> Result<Json<Vec<ApiPoolUpstreamRequestAttempt>>, ApiError> {
    Ok(Json(
        query_pool_attempt_records_from_live(&state.pool, &invoke_id).await?,
    ))
}

#[derive(Debug, FromRow)]
pub(crate) struct InvocationResponseBodyRow {
    pub(crate) id: i64,
    pub(crate) raw_response: String,
    pub(crate) response_raw_path: Option<String>,
    pub(crate) response_raw_size: Option<i64>,
    pub(crate) response_raw_truncated: Option<i64>,
    pub(crate) response_raw_truncated_reason: Option<String>,
    pub(crate) detail_level: String,
    pub(crate) detail_prune_reason: Option<String>,
    pub(crate) response_content_encoding: Option<String>,
    pub(crate) failure_class: Option<String>,
}

pub(crate) fn is_abnormal_invocation_failure(failure_class: Option<&str>) -> bool {
    matches!(
        failure_class
            .map(str::trim)
            .filter(|value| !value.is_empty()),
        Some("service_failure" | "client_failure" | "client_abort")
    )
}

pub(crate) fn truncate_response_preview_text(value: &str) -> (String, bool) {
    let mut end = value.len();
    let mut count = 0usize;
    for (index, _) in value.char_indices() {
        if count == INVOCATION_RESPONSE_BODY_PREVIEW_CHAR_LIMIT {
            end = index;
            break;
        }
        count += 1;
    }
    if count < INVOCATION_RESPONSE_BODY_PREVIEW_CHAR_LIMIT {
        return (value.to_string(), false);
    }
    (value[..end].to_string(), true)
}

pub(crate) fn raw_response_fallback_reason(row: &InvocationResponseBodyRow) -> String {
    if row.detail_level == DETAIL_LEVEL_STRUCTURED_ONLY {
        "detail_pruned".to_string()
    } else if row.response_raw_truncated.unwrap_or_default() != 0 {
        row.response_raw_truncated_reason
            .as_deref()
            .filter(|value| !value.trim().is_empty())
            .map(|reason| format!("preview_only:{reason}"))
            .unwrap_or_else(|| "preview_only".to_string())
    } else {
        "missing_body".to_string()
    }
}

pub(crate) fn resolve_response_body_text_from_row(
    row: &InvocationResponseBodyRow,
    raw_path_fallback_root: Option<&Path>,
) -> Result<(String, bool), String> {
    if let Some(path) = row.response_raw_path.as_deref() {
        match read_proxy_raw_bytes(path, raw_path_fallback_root) {
            Ok(bytes) => {
                let (decoded, _) = decode_response_payload_for_usage(
                    &bytes,
                    row.response_content_encoding.as_deref(),
                );
                return Ok((String::from_utf8_lossy(decoded.as_ref()).to_string(), true));
            }
            Err(err) if err.kind() == io::ErrorKind::NotFound => {
                let raw_preview = row.raw_response.trim();
                let preview_len = row.raw_response.len() as i64;
                if !raw_preview.is_empty()
                    && row.response_raw_size.unwrap_or(preview_len) <= preview_len
                {
                    return Ok((row.raw_response.clone(), false));
                }
                return Err("raw_file_missing".to_string());
            }
            Err(err) => {
                let raw_preview = row.raw_response.trim();
                let preview_len = row.raw_response.len() as i64;
                if !raw_preview.is_empty()
                    && row.response_raw_size.unwrap_or(preview_len) <= preview_len
                {
                    return Ok((row.raw_response.clone(), false));
                }
                return Err(format!("raw_file_unreadable:{err}"));
            }
        }
    }

    let raw_preview = row.raw_response.trim();
    if raw_preview.is_empty() {
        return Err(raw_response_fallback_reason(row));
    }

    let preview_len = row.raw_response.len() as i64;
    let fits_in_preview = row.response_raw_size.unwrap_or(preview_len) <= preview_len;
    if fits_in_preview && row.response_raw_truncated.unwrap_or_default() == 0 {
        return Ok((row.raw_response.clone(), false));
    }

    Err(raw_response_fallback_reason(row))
}

pub(crate) async fn fetch_invocation_response_body_row_by_id(
    pool: &Pool<Sqlite>,
    id: i64,
) -> Result<Option<InvocationResponseBodyRow>, ApiError> {
    let sql = format!(
        "SELECT \
         id, \
         raw_response, \
         response_raw_path, \
         response_raw_size, \
         response_raw_truncated, \
         response_raw_truncated_reason, \
         detail_level, \
         detail_prune_reason, \
         {response_content_encoding} AS response_content_encoding, \
         {resolved_failure} AS failure_class \
         FROM codex_invocations \
         WHERE id = ?1 \
         LIMIT 1",
        response_content_encoding = INVOCATION_RESPONSE_CONTENT_ENCODING_SQL,
        resolved_failure = INVOCATION_RESOLVED_FAILURE_CLASS_SQL,
    );

    sqlx::query_as::<_, InvocationResponseBodyRow>(&sql)
        .bind(id)
        .fetch_optional(pool)
        .await
        .map_err(ApiError::from)
}

pub(crate) async fn fetch_invocation_record_detail(
    State(state): State<Arc<AppState>>,
    axum::extract::Path(id): axum::extract::Path<i64>,
) -> Result<Json<InvocationRecordDetailResponse>, ApiError> {
    let row = fetch_invocation_response_body_row_by_id(&state.pool, id)
        .await?
        .ok_or_else(|| ApiError::bad_request(anyhow!("record not found")))?;

    let abnormal_response_body = if is_abnormal_invocation_failure(row.failure_class.as_deref()) {
        match resolve_response_body_text_from_row(&row, state.config.database_path.parent()) {
            Ok((text, from_full_body)) => {
                let (preview_text, truncated) = truncate_response_preview_text(&text);
                Some(InvocationAbnormalResponseBodyPreview {
                    available: true,
                    preview_text: Some(preview_text),
                    has_more: truncated || from_full_body,
                    unavailable_reason: None,
                })
            }
            Err(reason) => Some(InvocationAbnormalResponseBodyPreview {
                available: false,
                preview_text: None,
                has_more: false,
                unavailable_reason: Some(reason),
            }),
        }
    } else {
        None
    };

    Ok(Json(InvocationRecordDetailResponse {
        id: row.id,
        abnormal_response_body,
    }))
}

pub(crate) async fn fetch_invocation_response_body(
    State(state): State<Arc<AppState>>,
    axum::extract::Path(id): axum::extract::Path<i64>,
) -> Result<Json<InvocationResponseBodyResponse>, ApiError> {
    let row = fetch_invocation_response_body_row_by_id(&state.pool, id)
        .await?
        .ok_or_else(|| ApiError::bad_request(anyhow!("record not found")))?;

    if !is_abnormal_invocation_failure(row.failure_class.as_deref()) {
        return Ok(Json(InvocationResponseBodyResponse {
            available: false,
            body_text: None,
            unavailable_reason: Some("not_abnormal".to_string()),
        }));
    }

    match resolve_response_body_text_from_row(&row, state.config.database_path.parent()) {
        Ok((body_text, _)) => Ok(Json(InvocationResponseBodyResponse {
            available: true,
            body_text: Some(body_text),
            unavailable_reason: None,
        })),
        Err(reason) => Ok(Json(InvocationResponseBodyResponse {
            available: false,
            body_text: None,
            unavailable_reason: Some(reason),
        })),
    }
}

pub(crate) async fn fetch_invocation_summary(
    State(state): State<Arc<AppState>>,
    Query(params): Query<ListQuery>,
) -> Result<Json<InvocationSummaryResponse>, ApiError> {
    let request = build_invocation_list_request(&params, state.config.list_limit_max as i64)?;
    let source_scope = resolve_default_source_scope(&state.pool).await?;
    let snapshot_id = request
        .snapshot_id
        .unwrap_or(resolve_invocation_snapshot_id(&state.pool, source_scope).await?);

    let totals_sql = format!(
        "SELECT \
         COUNT(*) AS total_count, \
         COALESCE(SUM(CASE WHEN {resolved_failure} = 'none' AND ({status_norm} IN ('success', 'completed') OR ({status_norm} = 'http_200' AND LOWER(TRIM(COALESCE(error_message, ''))) = '')) THEN 1 ELSE 0 END), 0) AS success_count, \
         COALESCE(SUM(CASE WHEN {resolved_failure} IN ('service_failure', 'client_failure', 'client_abort') THEN 1 ELSE 0 END), 0) AS failure_count, \
         COALESCE(SUM(total_tokens), 0) AS total_tokens, \
         COALESCE(SUM(cost), 0.0) AS total_cost, \
         COALESCE(SUM(cache_input_tokens), 0) AS cache_input_tokens \
         FROM codex_invocations WHERE 1 = 1",
        status_norm = INVOCATION_STATUS_NORMALIZED_SQL,
        resolved_failure = INVOCATION_RESOLVED_FAILURE_CLASS_SQL,
    );
    let mut totals_query = QueryBuilder::new(totals_sql);
    apply_invocation_records_filters(
        &mut totals_query,
        &request.filters,
        source_scope,
        Some(SnapshotConstraint::UpTo(snapshot_id)),
    );
    let totals = totals_query
        .build_query_as::<InvocationSummaryAggRow>()
        .fetch_one(&state.pool)
        .await?;

    let network =
        query_invocation_network_summary(&state.pool, &request.filters, source_scope, snapshot_id)
            .await?;

    let exception = query_invocation_exception_summary(
        &state.pool,
        &request.filters,
        source_scope,
        snapshot_id,
    )
    .await?;

    let new_records_count = query_invocation_new_records_count(
        &state.pool,
        &request.filters,
        source_scope,
        snapshot_id,
    )
    .await?;

    let runtime_overlay_delta = if request.snapshot_id.is_none() {
        let db_runtime_keys = query_current_runtime_db_keys(
            &state.pool,
            &request.filters,
            source_scope,
            Some(SnapshotConstraint::UpTo(snapshot_id)),
        )
        .await?;
        let runtime_records = runtime_overlay_snapshot(state.as_ref());
        let db_terminal_keys = query_terminal_db_keys_for_runtime_records(
            &state.pool,
            &runtime_records,
            Some(SnapshotConstraint::UpTo(snapshot_id)),
        )
        .await?;
        let (delta, runtime_new_count, stale_db_runtime_count) = runtime_overlay_total_delta(
            &request,
            source_scope,
            &runtime_records,
            &db_runtime_keys,
            &db_terminal_keys,
        );
        if runtime_new_count > 0 || stale_db_runtime_count > 0 {
            debug!(
                endpoint = "invocation_summary",
                runtime_overlay_row_count = runtime_new_count,
                stale_db_runtime_total_count = stale_db_runtime_count,
                "adjusted current summary count with memory runtime overlay"
            );
        }
        delta
    } else {
        RuntimeSummaryOverlayDelta::default()
    };
    let total_count = (totals.total_count + runtime_overlay_delta.total_count).max(0);
    let success_count = (totals.success_count + runtime_overlay_delta.success_count).max(0);
    let failure_count = (totals.failure_count + runtime_overlay_delta.failure_count).max(0);
    let total_tokens = (totals.total_tokens + runtime_overlay_delta.total_tokens).max(0);
    let total_cost = totals.total_cost + runtime_overlay_delta.total_cost;
    let cache_input_tokens =
        (totals.cache_input_tokens + runtime_overlay_delta.cache_input_tokens).max(0);
    let avg_tokens_per_request = if total_count <= 0 {
        0.0
    } else {
        total_tokens as f64 / total_count as f64
    };
    let exception_summary = InvocationExceptionSummary {
        failure_count: (exception.failure_count + runtime_overlay_delta.failure_count).max(0),
        service_failure_count: (exception.service_failure_count
            + runtime_overlay_delta.service_failure_count)
            .max(0),
        client_failure_count: (exception.client_failure_count
            + runtime_overlay_delta.client_failure_count)
            .max(0),
        client_abort_count: (exception.client_abort_count
            + runtime_overlay_delta.client_abort_count)
            .max(0),
        actionable_failure_count: (exception.actionable_failure_count
            + runtime_overlay_delta.service_failure_count)
            .max(0),
    };

    Ok(Json(InvocationSummaryResponse {
        snapshot_id,
        new_records_count,
        total_count,
        success_count,
        failure_count,
        total_tokens,
        total_cost,
        token: InvocationTokenSummary {
            request_count: total_count,
            total_tokens,
            avg_tokens_per_request,
            cache_input_tokens,
            total_cost,
        },
        network,
        exception: exception_summary,
    }))
}

pub(crate) async fn fetch_invocation_new_records_count(
    State(state): State<Arc<AppState>>,
    Query(params): Query<ListQuery>,
) -> Result<Json<InvocationNewRecordsCountResponse>, ApiError> {
    let request = build_invocation_list_request(&params, state.config.list_limit_max as i64)?;
    let source_scope = resolve_default_source_scope(&state.pool).await?;
    let snapshot_id = request
        .snapshot_id
        .ok_or_else(|| ApiError::bad_request(anyhow!("snapshotId is required")))?;
    let new_records_count = query_invocation_new_records_count(
        &state.pool,
        &request.filters,
        source_scope,
        snapshot_id,
    )
    .await?;

    Ok(Json(InvocationNewRecordsCountResponse {
        snapshot_id,
        new_records_count,
    }))
}

pub(crate) async fn fetch_invocation_suggestions(
    State(state): State<Arc<AppState>>,
    Query(params): Query<ListQuery>,
) -> Result<Json<InvocationSuggestionsResponse>, ApiError> {
    const SUGGESTION_LIMIT: i64 = 30;
    let request = build_invocation_list_request(&params, state.config.list_limit_max as i64)?;
    let filters = request.filters;
    let source_scope = resolve_default_source_scope(&state.pool).await?;
    let snapshot = request.snapshot_id.map(SnapshotConstraint::UpTo);
    let suggest_field = InvocationSuggestionField::parse(params.suggest_field.as_deref())?;
    let suggest_query = normalize_query_text(params.suggest_query.as_deref());

    if let Some(field) = suggest_field {
        let bucket = query_invocation_suggestion_bucket(
            &state.pool,
            &field.clear_field_filter(&filters),
            source_scope,
            snapshot,
            field.sql_expr(),
            suggest_query.as_deref(),
            SUGGESTION_LIMIT,
        )
        .await?;
        return Ok(Json(suggestion_response_for_field(field, bucket)));
    }

    let model = query_invocation_suggestion_bucket(
        &state.pool,
        &InvocationRecordsFilters {
            model: None,
            ..filters.clone()
        },
        source_scope,
        snapshot,
        "model",
        None,
        SUGGESTION_LIMIT,
    )
    .await?;
    let endpoint = query_invocation_suggestion_bucket(
        &state.pool,
        &InvocationRecordsFilters {
            endpoint: None,
            ..filters.clone()
        },
        source_scope,
        snapshot,
        INVOCATION_ENDPOINT_SQL,
        None,
        SUGGESTION_LIMIT,
    )
    .await?;
    let failure_kind = query_invocation_suggestion_bucket(
        &state.pool,
        &InvocationRecordsFilters {
            failure_kind: None,
            ..filters.clone()
        },
        source_scope,
        snapshot,
        INVOCATION_FAILURE_KIND_SQL,
        None,
        SUGGESTION_LIMIT,
    )
    .await?;
    let prompt_cache_key = query_invocation_suggestion_bucket(
        &state.pool,
        &InvocationRecordsFilters {
            prompt_cache_key: None,
            ..filters.clone()
        },
        source_scope,
        snapshot,
        INVOCATION_PROMPT_CACHE_KEY_SQL,
        None,
        SUGGESTION_LIMIT,
    )
    .await?;
    let requester_ip = query_invocation_suggestion_bucket(
        &state.pool,
        &InvocationRecordsFilters {
            requester_ip: None,
            ..filters.clone()
        },
        source_scope,
        snapshot,
        INVOCATION_REQUESTER_IP_SQL,
        None,
        SUGGESTION_LIMIT,
    )
    .await?;

    Ok(Json(InvocationSuggestionsResponse {
        model,
        endpoint,
        failure_kind,
        prompt_cache_key,
        requester_ip,
    }))
}

pub(crate) async fn fetch_stats(
    State(state): State<Arc<AppState>>,
) -> Result<Json<StatsResponse>, ApiError> {
    let source_scope = resolve_default_source_scope(&state.pool).await?;
    let totals = query_combined_totals(&state.pool, StatsFilter::All, source_scope).await?;
    let mut response = totals.into_response();
    response.non_success_cost = Some(totals.non_success_cost);
    let augmentation = load_summary_live_augmentation(
        state.as_ref(),
        source_scope,
        None,
        None,
        SummaryLiveAugmentationPolicy {
            include_in_progress: true,
            include_non_success_tokens: false,
        },
    )
    .await?;
    apply_summary_live_augmentation(&mut response, augmentation);
    response.maintenance = Some(load_stats_maintenance_response(state.as_ref()).await?);
    Ok(Json(response))
}

pub(crate) async fn load_in_progress_conversation_count(
    state: &AppState,
    source_scope: InvocationSourceScope,
    upstream_account_id: Option<i64>,
) -> Result<i64, ApiError> {
    Ok(
        load_in_progress_summary_snapshot(state, source_scope, upstream_account_id)
            .await?
            .in_progress_count,
    )
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct SummaryLiveAugmentation {
    in_progress_conversation_count: Option<i64>,
    in_progress_retry_conversation_count: Option<i64>,
    in_progress_avg_wait_ms: Option<f64>,
    in_progress_phase_counts: Option<InvocationPhaseCountsResponse>,
    non_success_tokens: Option<i64>,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct SummaryLiveAugmentationPolicy {
    include_in_progress: bool,
    include_non_success_tokens: bool,
}

pub(crate) fn summary_window_range(
    window: &SummaryWindow,
    reporting_tz: Tz,
    now: DateTime<Utc>,
) -> Result<Option<(DateTime<Utc>, DateTime<Utc>)>, ApiError> {
    match window {
        SummaryWindow::All | SummaryWindow::Current(_) => Ok(None),
        SummaryWindow::Duration(duration) => Ok(Some((now - *duration, now))),
        SummaryWindow::Calendar(spec) => {
            let range =
                resolve_range_window(spec.as_str(), reporting_tz).map_err(ApiError::from)?;
            Ok(Some((range.start, range.end)))
        }
        SummaryWindow::PreviousFullDays(day_count) => {
            let (start, end) = previous_full_days_range_bounds(*day_count, now, reporting_tz)
                .ok_or_else(|| {
                    ApiError::bad_request(anyhow!("invalid previous full days window"))
                })?;
            Ok(Some((start, end)))
        }
    }
}

pub(crate) async fn load_summary_live_augmentation(
    state: &AppState,
    source_scope: InvocationSourceScope,
    upstream_account_id: Option<i64>,
    range: Option<(DateTime<Utc>, DateTime<Utc>)>,
    policy: SummaryLiveAugmentationPolicy,
) -> Result<SummaryLiveAugmentation, ApiError> {
    let in_progress = if policy.include_in_progress {
        let snapshot =
            load_in_progress_summary_snapshot(state, source_scope, upstream_account_id).await?;
        (
            Some(snapshot.in_progress_count),
            Some(snapshot.retry_count),
            snapshot.avg_wait_ms,
            Some(snapshot.phase_counts),
        )
    } else {
        (None, None, None, None)
    };
    let non_success_tokens = if policy.include_non_success_tokens {
        if let Some((start, end)) = range {
            load_non_success_tokens_snapshot(state, source_scope, upstream_account_id, start, end)
                .await?
        } else {
            None
        }
    } else {
        None
    };

    Ok(SummaryLiveAugmentation {
        in_progress_conversation_count: in_progress.0,
        in_progress_retry_conversation_count: in_progress.1,
        in_progress_avg_wait_ms: in_progress.2,
        in_progress_phase_counts: in_progress.3,
        non_success_tokens,
    })
}

pub(crate) fn apply_summary_live_augmentation(
    response: &mut StatsResponse,
    augmentation: SummaryLiveAugmentation,
) {
    response.in_progress_conversation_count = augmentation.in_progress_conversation_count;
    response.in_progress_retry_conversation_count =
        augmentation.in_progress_retry_conversation_count;
    response.in_progress_avg_wait_ms = augmentation.in_progress_avg_wait_ms;
    response.in_progress_phase_counts = augmentation.in_progress_phase_counts;
    response.non_success_tokens = augmentation.non_success_tokens;
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct InProgressSummarySnapshot {
    in_progress_count: i64,
    retry_count: i64,
    avg_wait_ms: Option<f64>,
    phase_counts: InvocationPhaseCountsResponse,
}

pub(crate) async fn load_in_progress_summary_snapshot(
    state: &AppState,
    source_scope: InvocationSourceScope,
    upstream_account_id: Option<i64>,
) -> Result<InProgressSummarySnapshot, ApiError> {
    #[derive(Debug, FromRow)]
    struct RuntimeKeyRow {
        invoke_id: String,
        occurred_at: String,
        upstream_account_id: Option<i64>,
        retry_count: i64,
        upstream_ttfb_ms: Option<f64>,
        live_phase: Option<String>,
    }

    let resolved_upstream_account_id_sql =
        invocation_upstream_account_id_with_attempt_fallback_sql("inv");
    let retry_sql = if upstream_account_id.is_some() {
        invocation_account_retry_after_failure_with_attempt_fallback_sql(
            resolved_upstream_account_id_sql.as_str(),
            source_scope,
        )
    } else {
        match source_scope {
            InvocationSourceScope::All => "live.is_retry_after_failure_all".to_string(),
            InvocationSourceScope::ProxyOnly => {
                "live.is_retry_after_failure_proxy_only".to_string()
            }
        }
    };
    let mut query = QueryBuilder::<Sqlite>::new(
        "SELECT \
            COALESCE(COUNT(*), 0) AS in_progress_count, \
            COALESCE(SUM(",
    );
    query.push(retry_sql.as_str()).push(
        "), 0) AS retry_count, \
         AVG(CASE WHEN live.upstream_ttfb_ms IS NOT NULL AND live.upstream_ttfb_ms >= 0 THEN live.upstream_ttfb_ms END) AS avg_wait_ms, \
         COUNT(CASE WHEN live.upstream_ttfb_ms IS NOT NULL AND live.upstream_ttfb_ms >= 0 THEN 1 END) AS avg_wait_sample_count \
         FROM invocation_in_progress_live live \
         JOIN codex_invocations inv ON inv.id = live.invocation_id \
         WHERE 1 = 1",
    );

    if source_scope == InvocationSourceScope::ProxyOnly {
        query.push(" AND live.source = ").push_bind(SOURCE_PROXY);
    }
    if let Some(upstream_account_id) = upstream_account_id {
        query
            .push(" AND ")
            .push(resolved_upstream_account_id_sql.as_str())
            .push(" = ")
            .push_bind(upstream_account_id);
    }

    let (mut db_in_progress_count, mut retry_count, avg_wait_ms, mut avg_wait_sample_count) = query
        .build_query_as::<(i64, i64, Option<f64>, i64)>()
        .fetch_one(&state.pool)
        .await?;
    let mut db_key_query = QueryBuilder::<Sqlite>::new(
        "SELECT inv.invoke_id AS invoke_id, inv.occurred_at AS occurred_at, ",
    );
    db_key_query
        .push(resolved_upstream_account_id_sql.as_str())
        .push(" AS upstream_account_id, ")
        .push(retry_sql.as_str())
        .push(" AS retry_count, live.upstream_ttfb_ms AS upstream_ttfb_ms, ");
    db_key_query.push(invocation_live_phase_sql("inv")).push(
        " AS live_phase \
         FROM invocation_in_progress_live live \
         JOIN codex_invocations inv ON inv.id = live.invocation_id \
         WHERE 1 = 1",
    );
    if source_scope == InvocationSourceScope::ProxyOnly {
        db_key_query
            .push(" AND live.source = ")
            .push_bind(SOURCE_PROXY);
    }
    if let Some(upstream_account_id) = upstream_account_id {
        db_key_query
            .push(" AND ")
            .push(resolved_upstream_account_id_sql.as_str())
            .push(" = ")
            .push_bind(upstream_account_id);
    }
    let db_runtime_rows = db_key_query
        .build_query_as::<RuntimeKeyRow>()
        .fetch_all(&state.pool)
        .await?;
    let mut phase_counts = InvocationPhaseCountsResponse::default();
    for row in &db_runtime_rows {
        phase_counts.increment_phase_name(row.live_phase.as_deref());
    }
    let runtime_snapshot = state.proxy_runtime_invocations.snapshot();
    let db_terminal_keys =
        query_terminal_db_keys_for_runtime_records(&state.pool, &runtime_snapshot, None).await?;
    let runtime_by_key = runtime_snapshot
        .iter()
        .map(|record| {
            (
                (record.invoke_id.clone(), record.occurred_at.clone()),
                record,
            )
        })
        .collect::<HashMap<_, _>>();
    let filter = InvocationRecordsFilters {
        upstream_account_id,
        ..InvocationRecordsFilters::default()
    };
    let filter_without_upstream_account = InvocationRecordsFilters {
        upstream_account_id: None,
        ..filter.clone()
    };
    let mut db_ttfb_sum = avg_wait_ms.unwrap_or_default() * avg_wait_sample_count as f64;
    for row in &db_runtime_rows {
        let key = (row.invoke_id.clone(), row.occurred_at.clone());
        let Some(runtime_record) = runtime_by_key.get(&key) else {
            continue;
        };
        let account_matches = upstream_account_id
            .map(|account_id| row.upstream_account_id == Some(account_id))
            .unwrap_or(true);
        if account_matches
            && runtime_in_flight_record_matches_filters(
                runtime_record,
                &filter_without_upstream_account,
                source_scope,
            )
        {
            let runtime_phase = runtime_record
                .live_phase
                .as_deref()
                .or_else(|| runtime_invocation_live_phase(runtime_record));
            if runtime_phase != row.live_phase.as_deref() {
                phase_counts.decrement_phase_name(row.live_phase.as_deref());
                phase_counts.increment_phase_name(runtime_phase);
            }
            continue;
        }
        db_in_progress_count = db_in_progress_count.saturating_sub(1);
        phase_counts.decrement_phase_name(row.live_phase.as_deref());
        if row.retry_count > 0 {
            retry_count = retry_count.saturating_sub(1);
        }
        if let Some(ttfb_ms) = row
            .upstream_ttfb_ms
            .filter(|value| value.is_finite() && *value >= 0.0)
        {
            db_ttfb_sum -= ttfb_ms;
            avg_wait_sample_count = avg_wait_sample_count.saturating_sub(1);
        }
    }
    let db_runtime_keys = db_runtime_rows
        .into_iter()
        .map(|row| ((row.invoke_id, row.occurred_at), row.retry_count > 0))
        .collect::<HashMap<_, _>>();
    let runtime_records = runtime_snapshot
        .into_iter()
        .filter(|record| {
            !db_terminal_keys.contains(&(record.invoke_id.clone(), record.occurred_at.clone()))
        })
        .filter(|record| runtime_in_flight_record_matches_filters(record, &filter, source_scope))
        .collect::<Vec<_>>();
    let runtime_in_progress_count = runtime_records
        .iter()
        .filter(|record| {
            !db_runtime_keys.contains_key(&(record.invoke_id.clone(), record.occurred_at.clone()))
        })
        .count() as i64;
    let runtime_retry_count = runtime_records
        .iter()
        .filter(|record| {
            let key = (record.invoke_id.clone(), record.occurred_at.clone());
            runtime_record_is_retry(record) && !db_runtime_keys.get(&key).copied().unwrap_or(false)
        })
        .count() as i64;
    let runtime_phase_counts = runtime_records
        .iter()
        .filter(|record| {
            !db_runtime_keys.contains_key(&(record.invoke_id.clone(), record.occurred_at.clone()))
        })
        .fold(
            InvocationPhaseCountsResponse::default(),
            |mut counts, record| {
                counts.increment_phase_name(
                    record
                        .live_phase
                        .as_deref()
                        .or_else(|| runtime_invocation_live_phase(record)),
                );
                counts
            },
        );
    let (ttfb_sum, ttfb_count) = runtime_records
        .iter()
        .filter(|record| {
            !db_runtime_keys.contains_key(&(record.invoke_id.clone(), record.occurred_at.clone()))
        })
        .filter_map(|record| {
            record
                .t_upstream_ttfb_ms
                .filter(|value| value.is_finite() && *value >= 0.0)
        })
        .fold((0.0, 0_i64), |(sum, count), value| (sum + value, count + 1));
    let combined_avg_wait_ms = match (avg_wait_sample_count, ttfb_count) {
        (db_sample_count, runtime_count) if db_sample_count > 0 || runtime_count > 0 => {
            Some((db_ttfb_sum + ttfb_sum) / (db_sample_count + runtime_count) as f64)
        }
        _ => None,
    };
    if runtime_in_progress_count > 0 {
        debug!(
            endpoint = "/api/stats/summary",
            runtime_overlay_row_count = runtime_in_progress_count,
            upstream_account_id,
            "overlayed memory runtime in-progress records into summary live augmentation"
        );
    }
    phase_counts.queued += runtime_phase_counts.queued;
    phase_counts.requesting += runtime_phase_counts.requesting;
    phase_counts.responding += runtime_phase_counts.responding;
    Ok(InProgressSummarySnapshot {
        in_progress_count: db_in_progress_count + runtime_in_progress_count,
        retry_count: retry_count + runtime_retry_count,
        avg_wait_ms: combined_avg_wait_ms,
        phase_counts,
    })
}

pub(crate) async fn load_live_invocation_ids_in_range(
    pool: &Pool<Sqlite>,
    source_scope: InvocationSourceScope,
    upstream_account_id: Option<i64>,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
) -> Result<std::collections::HashSet<i64>, ApiError> {
    #[derive(Debug, FromRow)]
    struct IdRow {
        id: i64,
    }

    let mut query =
        QueryBuilder::<Sqlite>::new("SELECT id FROM codex_invocations WHERE occurred_at >= ");
    query
        .push_bind(db_occurred_at_lower_bound(start))
        .push(" AND occurred_at < ")
        .push_bind(db_occurred_at_upper_bound(end));

    if source_scope == InvocationSourceScope::ProxyOnly {
        query.push(" AND source = ").push_bind(SOURCE_PROXY);
    }
    if let Some(upstream_account_id) = upstream_account_id {
        query
            .push(" AND ")
            .push(crate::api::INVOCATION_UPSTREAM_ACCOUNT_ID_SQL)
            .push(" = ")
            .push_bind(upstream_account_id);
    }

    Ok(query
        .build_query_as::<IdRow>()
        .fetch_all(pool)
        .await?
        .into_iter()
        .map(|row| row.id)
        .collect())
}

pub(crate) fn summary_live_augmentation_policy(
    window: &SummaryWindow,
    range: Option<(DateTime<Utc>, DateTime<Utc>)>,
    now: DateTime<Utc>,
) -> SummaryLiveAugmentationPolicy {
    let closed_named_window = matches!(
        window,
        SummaryWindow::Calendar(_) | SummaryWindow::PreviousFullDays(_)
    ) && range.is_some_and(|(_, end)| end < now);

    SummaryLiveAugmentationPolicy {
        include_in_progress: !closed_named_window,
        include_non_success_tokens: !closed_named_window && range.is_some(),
    }
}

pub(crate) async fn load_non_success_tokens_snapshot(
    state: &AppState,
    source_scope: InvocationSourceScope,
    upstream_account_id: Option<i64>,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
) -> Result<Option<i64>, ApiError> {
    let started_at = Instant::now();
    let mut query = QueryBuilder::<Sqlite>::new(
        "SELECT \
            COALESCE(SUM(COALESCE(total_tokens, 0)), 0) AS non_success_tokens \
         FROM codex_invocations \
         WHERE occurred_at >= ",
    );
    query
        .push_bind(db_occurred_at_lower_bound(start))
        .push(" AND occurred_at < ")
        .push_bind(db_occurred_at_upper_bound(end))
        .push(" AND LOWER(TRIM(")
        .push(invocation_display_status_sql())
        .push(")) IN ('failed', 'interrupted')");

    if source_scope == InvocationSourceScope::ProxyOnly {
        query.push(" AND source = ").push_bind(SOURCE_PROXY);
    }
    if let Some(upstream_account_id) = upstream_account_id {
        query
            .push(" AND ")
            .push(crate::api::INVOCATION_UPSTREAM_ACCOUNT_ID_SQL)
            .push(" = ")
            .push_bind(upstream_account_id);
    }

    let live_tokens = match query
        .build_query_scalar::<i64>()
        .fetch_one(&state.pool)
        .await
    {
        Ok(tokens) => tokens,
        Err(err) => {
            let err = anyhow!(err);
            if crate::is_sqlite_lock_error(&err) {
                tracing::warn!(
                    endpoint = "/api/stats/summary",
                    window = "range",
                    ?source_scope,
                    upstream_account_id,
                    start = %start,
                    end = %end,
                    "summary live non-success token snapshot skipped because sqlite is locked"
                );
                return Ok(None);
            }
            return Err(ApiError::from(err));
        }
    };

    let retention_cutoff = shanghai_retention_cutoff(state.config.invocation_max_days);
    if start >= retention_cutoff {
        let elapsed_ms = started_at.elapsed().as_millis() as u64;
        if elapsed_ms >= 250 {
            tracing::warn!(
                endpoint = "/api/stats/summary",
                window = "range",
                ?source_scope,
                upstream_account_id,
                cache_hit_or_miss = "live_only",
                elapsed_ms,
                row_count = 0_i64,
                start = %start,
                end = %end,
                "summary non-success token snapshot exceeded slow-path threshold"
            );
        } else {
            tracing::debug!(
                endpoint = "/api/stats/summary",
                window = "range",
                ?source_scope,
                upstream_account_id,
                cache_hit_or_miss = "live_only",
                elapsed_ms,
                row_count = 0_i64,
                start = %start,
                end = %end,
                "summary non-success token snapshot completed"
            );
        }
        return Ok(Some(live_tokens));
    }

    let live_invocation_ids = match load_live_invocation_ids_in_range(
        &state.pool,
        source_scope,
        upstream_account_id,
        start,
        end,
    )
    .await
    {
        Ok(ids) => ids,
        Err(ApiError::Internal(err)) if crate::is_sqlite_lock_error(&err) => {
            tracing::warn!(
                endpoint = "/api/stats/summary",
                window = "range",
                ?source_scope,
                upstream_account_id,
                start = %start,
                end = %end,
                "summary archive overlap scan skipped because sqlite is locked"
            );
            return Ok(None);
        }
        Err(err) => return Err(err),
    };
    let archived_tokens = if let Some(upstream_account_id) = upstream_account_id {
        match crate::stats::query_completed_upstream_account_archive_non_success_usage(
            &state.pool,
            source_scope,
            Some((start, end)),
            Some(&live_invocation_ids),
            upstream_account_id,
        )
        .await
        {
            Ok((_, tokens)) => tokens,
            Err(err) if crate::is_sqlite_lock_error(&err) => {
                tracing::warn!(
                    endpoint = "/api/stats/summary",
                    window = "range",
                    ?source_scope,
                    upstream_account_id,
                    start = %start,
                    end = %end,
                    "summary archived non-success token lookup skipped because sqlite is locked"
                );
                return Ok(None);
            }
            Err(err) => return Err(ApiError::from(err)),
        }
    } else {
        match crate::stats::query_completed_invocation_archive_non_success_usage(
            &state.pool,
            source_scope,
            Some((start, end)),
            Some(&live_invocation_ids),
        )
        .await
        {
            Ok((_, tokens)) => tokens,
            Err(err) if crate::is_sqlite_lock_error(&err) => {
                tracing::warn!(
                    endpoint = "/api/stats/summary",
                    window = "range",
                    ?source_scope,
                    start = %start,
                    end = %end,
                    "summary archived non-success token lookup skipped because sqlite is locked"
                );
                return Ok(None);
            }
            Err(err) => return Err(ApiError::from(err)),
        }
    };
    let elapsed_ms = started_at.elapsed().as_millis() as u64;
    let row_count = live_invocation_ids.len() as i64;
    if elapsed_ms >= 250 {
        tracing::warn!(
            endpoint = "/api/stats/summary",
            window = "range",
            ?source_scope,
            upstream_account_id,
            cache_hit_or_miss = "live_plus_archive",
            elapsed_ms,
            row_count,
            start = %start,
            end = %end,
            "summary non-success token snapshot exceeded slow-path threshold"
        );
    } else {
        tracing::debug!(
            endpoint = "/api/stats/summary",
            window = "range",
            ?source_scope,
            upstream_account_id,
            cache_hit_or_miss = "live_plus_archive",
            elapsed_ms,
            row_count,
            start = %start,
            end = %end,
            "summary non-success token snapshot completed"
        );
    }
    Ok(Some(live_tokens + archived_tokens))
}

pub(crate) async fn build_empty_summary_response(
    state: &AppState,
    source_scope: InvocationSourceScope,
    upstream_account_id: Option<i64>,
) -> Result<StatsResponse, ApiError> {
    let mut response = StatsResponse {
        total_count: 0,
        success_count: 0,
        failure_count: 0,
        total_cost: 0.0,
        total_tokens: 0,
        usage_breakdown: None,
        in_progress_conversation_count: None,
        in_progress_retry_conversation_count: None,
        in_progress_avg_wait_ms: None,
        in_progress_phase_counts: None,
        non_success_cost: Some(0.0),
        non_success_tokens: None,
        maintenance: Some(load_stats_maintenance_response(state).await?),
    };
    let augmentation = load_summary_live_augmentation(
        state,
        source_scope,
        upstream_account_id,
        None,
        SummaryLiveAugmentationPolicy {
            include_in_progress: true,
            include_non_success_tokens: false,
        },
    )
    .await?;
    apply_summary_live_augmentation(&mut response, augmentation);
    Ok(response)
}

#[derive(Debug, Clone, FromRow)]
pub(crate) struct UpstreamAccountActivityMetaRow {
    id: i64,
    kind: String,
    display_name: Option<String>,
    group_name: Option<String>,
    plan_type: Option<String>,
    status: String,
    enabled: i64,
    last_error: Option<String>,
    last_error_at: Option<String>,
    last_route_failure_at: Option<String>,
    last_route_failure_kind: Option<String>,
    last_action_reason_code: Option<String>,
    last_action_reason_message: Option<String>,
    cooldown_until: Option<String>,
    temporary_route_failure_streak_started_at: Option<String>,
    last_selected_at: Option<String>,
}

#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct UpstreamAccountInProgressSummary {
    in_progress_count: i64,
    retry_count: i64,
    phase_counts: InvocationPhaseCountsResponse,
}

impl UpstreamAccountInProgressSummary {
    fn add(&mut self, retry: bool, phase: Option<&str>) {
        self.in_progress_count += 1;
        if retry {
            self.retry_count += 1;
        }
        self.phase_counts.increment_phase_name(phase);
    }

    fn subtract(&mut self, retry: bool, phase: Option<&str>) {
        self.in_progress_count = self.in_progress_count.saturating_sub(1);
        if retry {
            self.retry_count = self.retry_count.saturating_sub(1);
        }
        self.phase_counts.decrement_phase_name(phase);
    }
}

#[derive(Debug, Default)]
pub(crate) struct UpstreamAccountActivityAccumulator {
    display_name_hint: Option<String>,
    plan_type_hint: Option<String>,
    request_count: i64,
    success_count: i64,
    failure_count: i64,
    non_success_count: i64,
    total_tokens: i64,
    success_tokens: i64,
    non_success_tokens: i64,
    failure_tokens: i64,
    failure_cost: f64,
    non_success_cost: f64,
    cache_input_tokens: i64,
    total_cost: f64,
    first_response_byte_total_sample_count: i64,
    first_response_byte_total_sum_ms: f64,
    total_latency_sample_count: i64,
    total_latency_sum_ms: f64,
    in_progress_wait_sample_count: i64,
    in_progress_wait_sum_ms: f64,
    last_occurred_at_epoch_ms: i64,
    latest_conversation_created_at: Option<String>,
    last_invocation_at: Option<String>,
    rate_usage_events: Vec<UpstreamAccountRateUsageEvent>,
    recent_invocations: Vec<PromptCacheConversationInvocationPreviewResponse>,
    usage_breakdown: UsageBreakdownAccumulator,
    model_performance: ModelPerformanceAccumulator,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct UsageBreakdownGroupKey {
    model: String,
    reasoning_effort: Option<String>,
}

#[derive(Debug, Default, Clone)]
pub(crate) struct UsageBreakdownAccumulator {
    cache_write_tokens: i64,
    cache_read_tokens: i64,
    output_tokens: i64,
    costs: UsageCostBreakdownResponse,
    has_cost: bool,
    models: HashMap<UsageBreakdownGroupKey, UsageBreakdownAccumulator>,
}

impl UsageBreakdownAccumulator {
    fn add_cost_row(&mut self, total_cost: Option<f64>, costs: [Option<f64>; 5]) {
        let Some(total_cost) = total_cost else {
            return;
        };
        self.has_cost = true;
        if costs.iter().all(Option::is_some) {
            self.costs.input += costs[0].unwrap_or_default();
            self.costs.cache_write += costs[1].unwrap_or_default();
            self.costs.cache_read += costs[2].unwrap_or_default();
            self.costs.output += costs[3].unwrap_or_default();
            self.costs.reasoning += costs[4].unwrap_or_default();
        } else {
            self.costs.unknown += total_cost;
        }
    }

    fn merge_costs(&mut self, costs: &UsageCostBreakdownResponse) {
        self.has_cost = true;
        self.costs.input += costs.input;
        self.costs.cache_write += costs.cache_write;
        self.costs.cache_read += costs.cache_read;
        self.costs.output += costs.output;
        self.costs.reasoning += costs.reasoning;
        self.costs.unknown += costs.unknown;
    }

    fn add_row(&mut self, row: &UpstreamAccountInvocationPreviewRow) {
        let cache_read_tokens = row.cache_input_tokens.unwrap_or_default().max(0);
        let cache_write_tokens = row
            .input_tokens
            .unwrap_or_default()
            .max(0)
            .saturating_sub(cache_read_tokens);
        self.cache_write_tokens += cache_write_tokens;
        self.cache_read_tokens += cache_read_tokens;
        self.output_tokens += row.output_tokens.unwrap_or_default().max(0);

        let costs = [
            row.cost_input,
            row.cost_cache_write,
            row.cost_cache_read,
            row.cost_output,
            row.cost_reasoning,
        ];
        self.add_cost_row(row.cost, costs);

        let model = row
            .response_model
            .as_deref()
            .or(row.model.as_deref())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("unknown")
            .to_string();
        let reasoning_effort = row
            .reasoning_effort
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        let model_entry = self
            .models
            .entry(UsageBreakdownGroupKey {
                model,
                reasoning_effort,
            })
            .or_default();
        model_entry.cache_write_tokens += cache_write_tokens;
        model_entry.cache_read_tokens += cache_read_tokens;
        model_entry.output_tokens += row.output_tokens.unwrap_or_default().max(0);
        model_entry.add_cost_row(row.cost, costs);
    }

    fn add_aggregate_row(&mut self, row: &UpstreamAccountUsageBreakdownAggregateRow) {
        self.cache_write_tokens += row.cache_write_tokens;
        self.cache_read_tokens += row.cache_read_tokens;
        self.output_tokens += row.output_tokens;
        if row.has_cost > 0 {
            self.has_cost = true;
            self.costs.input += row.cost_input;
            self.costs.cache_write += row.cost_cache_write;
            self.costs.cache_read += row.cost_cache_read;
            self.costs.output += row.cost_output;
            self.costs.reasoning += row.cost_reasoning;
            self.costs.unknown += row.cost_unknown;
        }

        let model_entry = self
            .models
            .entry(UsageBreakdownGroupKey {
                model: row.model.clone(),
                reasoning_effort: row.reasoning_effort.clone(),
            })
            .or_default();
        model_entry.cache_write_tokens += row.cache_write_tokens;
        model_entry.cache_read_tokens += row.cache_read_tokens;
        model_entry.output_tokens += row.output_tokens;
        if row.has_cost > 0 {
            model_entry.has_cost = true;
            model_entry.costs.input += row.cost_input;
            model_entry.costs.cache_write += row.cost_cache_write;
            model_entry.costs.cache_read += row.cost_cache_read;
            model_entry.costs.output += row.cost_output;
            model_entry.costs.reasoning += row.cost_reasoning;
            model_entry.costs.unknown += row.cost_unknown;
        }
    }

    fn merge_response(&mut self, response: &UsageBreakdownResponse) {
        self.cache_write_tokens += response.cache_write_tokens;
        self.cache_read_tokens += response.cache_read_tokens;
        self.output_tokens += response.output_tokens;
        if let Some(costs) = &response.costs {
            self.merge_costs(costs);
        }
        for model in &response.models {
            let entry = self
                .models
                .entry(UsageBreakdownGroupKey {
                    model: model.model.clone(),
                    reasoning_effort: model.reasoning_effort.clone(),
                })
                .or_default();
            entry.cache_write_tokens += model.cache_write_tokens;
            entry.cache_read_tokens += model.cache_read_tokens;
            entry.output_tokens += model.output_tokens;
            if let Some(costs) = &model.costs {
                entry.merge_costs(costs);
            }
        }
    }

    fn into_response(self) -> UsageBreakdownResponse {
        let mut models = self
            .models
            .into_iter()
            .filter_map(|(group, entry)| {
                let has_usage = entry.cache_write_tokens > 0
                    || entry.cache_read_tokens > 0
                    || entry.output_tokens > 0
                    || entry.costs.input != 0.0
                    || entry.costs.cache_write != 0.0
                    || entry.costs.cache_read != 0.0
                    || entry.costs.output != 0.0
                    || entry.costs.reasoning != 0.0
                    || entry.costs.unknown != 0.0
                    || entry.has_cost;
                has_usage.then_some(UsageBreakdownModelResponse {
                    model: group.model,
                    reasoning_effort: group.reasoning_effort,
                    cache_write_tokens: entry.cache_write_tokens,
                    cache_read_tokens: entry.cache_read_tokens,
                    output_tokens: entry.output_tokens,
                    costs: entry.has_cost.then_some(entry.costs),
                })
            })
            .collect::<Vec<_>>();
        models.sort_by(|left, right| {
            left.model
                .cmp(&right.model)
                .then_with(|| left.reasoning_effort.cmp(&right.reasoning_effort))
        });
        UsageBreakdownResponse {
            cache_write_tokens: self.cache_write_tokens,
            cache_read_tokens: self.cache_read_tokens,
            output_tokens: self.output_tokens,
            costs: self.has_cost.then_some(self.costs),
            models,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct UpstreamAccountRateUsageEvent {
    occurred_at_epoch_ms: i64,
    total_tokens: i64,
    total_cost: f64,
}

pub(crate) fn normalize_trimmed_optional_string_local(raw: Option<String>) -> Option<String> {
    raw.and_then(|value| {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    })
}

pub(crate) fn resolve_upstream_account_activity_display_name(
    account_id: i64,
    meta: Option<&UpstreamAccountActivityMetaRow>,
    hint: Option<&str>,
) -> String {
    if let Some(display_name) = meta
        .and_then(|row| row.display_name.as_deref())
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return display_name.to_string();
    }
    if let Some(display_name) = hint.map(str::trim).filter(|value| !value.is_empty()) {
        return display_name.to_string();
    }
    format!("账号 #{account_id}")
}

#[derive(Debug, Clone)]
pub(crate) struct UpstreamAccountActivityStatusFields {
    enabled: bool,
    display_status: String,
    enable_status: String,
    work_status: String,
    health_status: String,
    sync_state: String,
    last_error: Option<String>,
    last_action_reason_message: Option<String>,
}

pub(crate) fn build_upstream_account_activity_status_fields(
    meta: &UpstreamAccountActivityMetaRow,
    now: DateTime<Utc>,
) -> UpstreamAccountActivityStatusFields {
    let enabled = meta.enabled != 0;
    let enable_status = crate::upstream_accounts::derive_upstream_account_enable_status(enabled);
    let health_status = crate::upstream_accounts::derive_upstream_account_health_status(
        &meta.kind,
        enabled,
        &meta.status,
        meta.last_error.as_deref(),
        meta.last_error_at.as_deref(),
        meta.last_route_failure_at.as_deref(),
        meta.last_route_failure_kind.as_deref(),
        meta.last_action_reason_code.as_deref(),
    );
    let sync_state =
        crate::upstream_accounts::derive_upstream_account_sync_state(enabled, &meta.status);
    let work_status = crate::upstream_accounts::derive_upstream_account_work_status(
        enabled,
        &meta.status,
        health_status,
        sync_state,
        false,
        meta.cooldown_until.as_deref(),
        meta.last_error_at.as_deref(),
        meta.last_route_failure_at.as_deref(),
        meta.last_route_failure_kind.as_deref(),
        meta.last_action_reason_code.as_deref(),
        meta.temporary_route_failure_streak_started_at.as_deref(),
        meta.last_selected_at.as_deref(),
        now,
    );
    let display_status = crate::upstream_accounts::classify_upstream_account_display_status(
        &meta.kind,
        enabled,
        &meta.status,
        meta.last_error.as_deref(),
        meta.last_error_at.as_deref(),
        meta.last_route_failure_at.as_deref(),
        meta.last_route_failure_kind.as_deref(),
        meta.last_action_reason_code.as_deref(),
    );

    UpstreamAccountActivityStatusFields {
        enabled,
        display_status: display_status.to_string(),
        enable_status: enable_status.to_string(),
        work_status: work_status.to_string(),
        health_status: health_status.to_string(),
        sync_state: sync_state.to_string(),
        last_error: normalize_trimmed_optional_string_local(meta.last_error.clone()),
        last_action_reason_message: normalize_trimmed_optional_string_local(
            meta.last_action_reason_message.clone(),
        ),
    }
}

fn merge_latest_optional_timestamp(current: &mut Option<String>, candidate: Option<String>) {
    let Some(candidate) = candidate else {
        return;
    };
    *current = Some(match current.take() {
        Some(existing) => existing.max(candidate),
        None => candidate,
    });
}

pub(crate) fn invocation_upstream_account_id_with_attempt_fallback_sql(
    invocation_ref: &str,
) -> String {
    format!(
        "COALESCE(\
           CASE WHEN json_valid({invocation_ref}.payload) \
             THEN CAST(json_extract({invocation_ref}.payload, '$.upstreamAccountId') AS INTEGER) \
           END, \
           (SELECT attempt.upstream_account_id \
              FROM pool_upstream_request_attempts attempt \
             WHERE attempt.invoke_id = {invocation_ref}.invoke_id \
               AND attempt.upstream_account_id IS NOT NULL \
             ORDER BY attempt.attempt_index DESC, attempt.id DESC \
             LIMIT 1)\
         )"
    )
}

pub(crate) fn invocation_prompt_cache_key_sql(invocation_ref: &str) -> String {
    format!(
        "CASE WHEN json_valid({invocation_ref}.payload) \
           THEN TRIM(CAST(json_extract({invocation_ref}.payload, '$.promptCacheKey') AS TEXT)) \
         END"
    )
}

pub(crate) fn invocation_history_conversation_created_at_sql(
    prompt_cache_key_sql: &str,
    source_scope: InvocationSourceScope,
) -> String {
    let history_prompt_cache_key_sql = invocation_prompt_cache_key_sql("conversation_history");
    let source_filter = match source_scope {
        InvocationSourceScope::All => String::new(),
        InvocationSourceScope::ProxyOnly => {
            format!(" AND conversation_history.source = '{SOURCE_PROXY}'")
        }
    };
    format!(
        "(SELECT MIN(conversation_history.occurred_at) \
            FROM codex_invocations conversation_history \
           WHERE ({history_prompt_cache_key_sql}) = ({prompt_cache_key_sql}){source_filter})"
    )
}

pub(crate) fn prompt_cache_conversation_created_at_sql(
    prompt_cache_key_sql: &str,
    source_scope: InvocationSourceScope,
) -> String {
    let rollup_source_filter = match source_scope {
        InvocationSourceScope::All => String::new(),
        InvocationSourceScope::ProxyOnly => format!(" AND source = '{SOURCE_PROXY}'"),
    };
    let working_set_created_at_sql = match source_scope {
        InvocationSourceScope::All => "created_at",
        InvocationSourceScope::ProxyOnly => "COALESCE(proxy_created_at, created_at)",
    };
    let invocation_history_created_at_sql =
        invocation_history_conversation_created_at_sql(prompt_cache_key_sql, source_scope);
    format!(
        "COALESCE(\
            (SELECT MIN(first_seen_at) \
               FROM prompt_cache_rollup_hourly \
              WHERE prompt_cache_key = {prompt_cache_key_sql}{rollup_source_filter}), \
            (SELECT {working_set_created_at_sql} \
               FROM prompt_cache_working_set_live \
              WHERE prompt_cache_key = {prompt_cache_key_sql}), \
            {invocation_history_created_at_sql}\
         )"
    )
}

pub(crate) fn invocation_account_retry_after_failure_with_attempt_fallback_sql(
    current_upstream_account_id_sql: &str,
    source_scope: InvocationSourceScope,
) -> String {
    let previous_upstream_account_id_sql =
        invocation_upstream_account_id_with_attempt_fallback_sql("codex_invocations");
    let account_match_sql = format!(
        "({previous_upstream_account_id_sql} = {current_upstream_account_id_sql} \
         OR ({previous_upstream_account_id_sql} IS NULL AND {current_upstream_account_id_sql} IS NULL))"
    );
    let display_status_sql = invocation_display_status_sql();
    let source_filter = match source_scope {
        InvocationSourceScope::All => "",
        InvocationSourceScope::ProxyOnly => "AND source = 'proxy'",
    };
    format!(
        "COALESCE((
            SELECT CASE WHEN previous_terminal.display_status = 'failed' THEN 1 ELSE 0 END
            FROM (
                SELECT LOWER(TRIM({display_status_sql})) AS display_status
                FROM codex_invocations
                WHERE {prompt_cache_key_sql} = live.prompt_cache_key
                  AND {account_match_sql}
                  AND id < live.invocation_id
                  {source_filter}
                  AND LOWER(TRIM({display_status_sql})) NOT IN ('running', 'pending')
                ORDER BY id DESC
                LIMIT 1
            ) AS previous_terminal
        ), 0)",
        prompt_cache_key_sql = INVOCATION_PROMPT_CACHE_KEY_SQL,
        account_match_sql = account_match_sql,
    )
}

pub(crate) fn compute_upstream_account_activity_rates(
    rate_usage_events: &[UpstreamAccountRateUsageEvent],
    range_start: DateTime<Utc>,
    range_end: DateTime<Utc>,
) -> (Option<f64>, Option<f64>) {
    (
        compute_upstream_account_activity_tail_rate(
            rate_usage_events,
            range_start,
            range_end,
            |usage| usage.total_tokens.max(0) as f64,
        ),
        compute_upstream_account_activity_tail_rate(
            rate_usage_events,
            range_start,
            range_end,
            |usage| usage.total_cost.max(0.0),
        ),
    )
}

pub(crate) fn compute_upstream_account_activity_tail_rate(
    rate_usage_events: &[UpstreamAccountRateUsageEvent],
    range_start: DateTime<Utc>,
    range_end: DateTime<Utc>,
    value_of: impl Fn(&UpstreamAccountRateUsageEvent) -> f64,
) -> Option<f64> {
    const RATE_WINDOW_MILLIS: i64 = 5 * 60 * 1_000;
    const MINUTE_MILLIS: i64 = 60 * 1_000;

    let anchor_epoch_ms = range_end.timestamp_millis();
    let range_start_epoch_ms = range_start.timestamp_millis();
    if anchor_epoch_ms <= range_start_epoch_ms {
        return Some(0.0);
    }
    let window_start_epoch_ms = range_start_epoch_ms.max(anchor_epoch_ms - RATE_WINDOW_MILLIS);

    let first_active_epoch_ms = rate_usage_events
        .iter()
        .filter_map(|usage| {
            if usage.occurred_at_epoch_ms < window_start_epoch_ms
                || usage.occurred_at_epoch_ms >= anchor_epoch_ms
            {
                return None;
            }
            let value = value_of(usage);
            (value.is_finite() && value > 0.0).then_some(usage.occurred_at_epoch_ms)
        })
        .min();

    let Some(first_active_epoch_ms) = first_active_epoch_ms else {
        return Some(0.0);
    };
    let active_bucket_start_epoch_ms =
        first_active_epoch_ms.div_euclid(MINUTE_MILLIS) * MINUTE_MILLIS;
    let active_start_epoch_ms = window_start_epoch_ms.max(active_bucket_start_epoch_ms);
    let active_millis = (anchor_epoch_ms - active_start_epoch_ms).max(0);
    if active_millis == 0 {
        return Some(0.0);
    }
    let total_value = rate_usage_events
        .iter()
        .filter_map(|usage| {
            if usage.occurred_at_epoch_ms < active_start_epoch_ms
                || usage.occurred_at_epoch_ms >= anchor_epoch_ms
            {
                return None;
            }
            let value = value_of(usage);
            (value.is_finite() && value > 0.0).then_some(value)
        })
        .sum::<f64>();

    Some(total_value / (active_millis as f64 / MINUTE_MILLIS as f64))
}

#[derive(Debug, FromRow)]
struct UpstreamAccountActivityAggregateRow {
    upstream_account_id: Option<i64>,
    latest_conversation_created_at: Option<String>,
    last_invocation_at: Option<String>,
    request_count: i64,
    success_count: i64,
    failure_count: i64,
    non_success_count: i64,
    total_tokens: i64,
    success_tokens: i64,
    non_success_tokens: i64,
    failure_tokens: i64,
    failure_cost: f64,
    non_success_cost: f64,
    cache_input_tokens: i64,
    total_cost: f64,
    first_response_byte_total_sample_count: i64,
    first_response_byte_total_sum_ms: f64,
    total_latency_sample_count: i64,
    total_latency_sum_ms: f64,
}

#[derive(Debug, FromRow)]
struct UpstreamAccountPromptCacheCreatedAtRow {
    upstream_account_id: Option<i64>,
    prompt_cache_key: String,
    first_occurred_at: String,
}

#[derive(Debug, FromRow)]
struct UpstreamAccountUsageBreakdownAggregateRow {
    upstream_account_id: Option<i64>,
    model: String,
    reasoning_effort: Option<String>,
    cache_write_tokens: i64,
    cache_read_tokens: i64,
    output_tokens: i64,
    cost_input: f64,
    cost_cache_write: f64,
    cost_cache_read: f64,
    cost_output: f64,
    cost_reasoning: f64,
    cost_unknown: f64,
    has_cost: i64,
    performance_total_tokens: i64,
    performance_stream_output_tokens: i64,
    performance_stream_duration_ms: f64,
    performance_response_sample_count: i64,
    performance_response_sum_ms: f64,
    performance_first_byte_sample_count: i64,
    performance_first_byte_sum_ms: f64,
    performance_usage_duration_sample_count: i64,
    performance_usage_duration_sum_ms: f64,
}

#[derive(Debug, Default, Clone)]
struct ModelPerformanceAccumulator {
    total_tokens: i64,
    stream_output_tokens: i64,
    stream_duration_ms: f64,
    response_sample_count: i64,
    response_sum_ms: f64,
    first_byte_sample_count: i64,
    first_byte_sum_ms: f64,
    usage_duration_sample_count: i64,
    usage_duration_sum_ms: f64,
    models: HashMap<UsageBreakdownGroupKey, ModelPerformanceAccumulator>,
}

impl ModelPerformanceAccumulator {
    fn add_aggregate_row(&mut self, row: &UpstreamAccountUsageBreakdownAggregateRow) {
        self.total_tokens += row.performance_total_tokens.max(0);
        self.stream_output_tokens += row.performance_stream_output_tokens.max(0);
        self.stream_duration_ms += row.performance_stream_duration_ms.max(0.0);
        self.response_sample_count += row.performance_response_sample_count.max(0);
        self.response_sum_ms += row.performance_response_sum_ms.max(0.0);
        self.first_byte_sample_count += row.performance_first_byte_sample_count.max(0);
        self.first_byte_sum_ms += row.performance_first_byte_sum_ms.max(0.0);
        self.usage_duration_sample_count += row.performance_usage_duration_sample_count.max(0);
        self.usage_duration_sum_ms += row.performance_usage_duration_sum_ms.max(0.0);

        let entry = self
            .models
            .entry(UsageBreakdownGroupKey {
                model: row.model.clone(),
                reasoning_effort: row.reasoning_effort.clone(),
            })
            .or_default();
        entry.total_tokens += row.performance_total_tokens.max(0);
        entry.stream_output_tokens += row.performance_stream_output_tokens.max(0);
        entry.stream_duration_ms += row.performance_stream_duration_ms.max(0.0);
        entry.response_sample_count += row.performance_response_sample_count.max(0);
        entry.response_sum_ms += row.performance_response_sum_ms.max(0.0);
        entry.first_byte_sample_count += row.performance_first_byte_sample_count.max(0);
        entry.first_byte_sum_ms += row.performance_first_byte_sum_ms.max(0.0);
        entry.usage_duration_sample_count += row.performance_usage_duration_sample_count.max(0);
        entry.usage_duration_sum_ms += row.performance_usage_duration_sum_ms.max(0.0);
    }

    fn metrics(&self, range: ExactUtcRange) -> ModelPerformanceMetricsResponse {
        let range_minutes = (range.end - range.start).num_milliseconds() as f64 / 60_000.0;
        ModelPerformanceMetricsResponse {
            tokens_per_minute: if range_minutes > 0.0 {
                self.total_tokens as f64 / range_minutes
            } else {
                0.0
            },
            streaming_response_rate: (self.stream_duration_ms > 0.0)
                .then_some(self.stream_output_tokens as f64 / (self.stream_duration_ms / 1_000.0)),
            avg_response_ms: (self.response_sample_count > 0)
                .then_some(self.response_sum_ms / self.response_sample_count as f64),
            avg_first_response_byte_total_ms: (self.first_byte_sample_count > 0)
                .then_some(self.first_byte_sum_ms / self.first_byte_sample_count as f64),
            usage_duration_ms: (self.usage_duration_sample_count > 0)
                .then_some(self.usage_duration_sum_ms),
        }
    }

    fn into_response(self, range: ExactUtcRange, available: bool) -> ModelPerformanceResponse {
        let total = self.metrics(range);
        let mut models = self
            .models
            .into_iter()
            .filter_map(|(group, entry)| {
                (entry.total_tokens > 0
                    || entry.stream_duration_ms > 0.0
                    || entry.response_sample_count > 0
                    || entry.first_byte_sample_count > 0
                    || entry.usage_duration_sample_count > 0)
                    .then_some(ModelPerformanceModelResponse {
                        model: group.model,
                        reasoning_effort: group.reasoning_effort,
                        metrics: entry.metrics(range),
                    })
            })
            .collect::<Vec<_>>();
        models.sort_by(|left, right| {
            right
                .metrics
                .usage_duration_ms
                .unwrap_or_default()
                .total_cmp(&left.metrics.usage_duration_ms.unwrap_or_default())
                .then_with(|| left.model.cmp(&right.model))
                .then_with(|| left.reasoning_effort.cmp(&right.reasoning_effort))
        });
        ModelPerformanceResponse {
            available,
            total,
            models,
        }
    }

    fn unavailable(range: ExactUtcRange) -> ModelPerformanceResponse {
        Self::default().into_response(range, false)
    }
}

#[derive(Debug, FromRow)]
struct RuntimeRecentAccountFallbackRow {
    invoke_id: String,
    occurred_at: String,
    upstream_account_id: Option<i64>,
    upstream_account_name: Option<String>,
    upstream_account_plan_type: Option<String>,
}

async fn query_live_upstream_account_activity_aggregate_rows(
    pool: &Pool<Sqlite>,
    source_scope: InvocationSourceScope,
    range: ExactUtcRange,
    use_attempt_fallback: bool,
) -> Result<Vec<UpstreamAccountActivityAggregateRow>, ApiError> {
    let started_at = Instant::now();
    let upstream_account_id_sql = if use_attempt_fallback {
        invocation_upstream_account_id_with_attempt_fallback_sql("codex_invocations")
    } else {
        INVOCATION_UPSTREAM_ACCOUNT_ID_SQL.to_string()
    };
    let failure_class_sql = INVOCATION_RESOLVED_FAILURE_CLASS_SQL;
    let success_sql = format!(
        "LOWER(TRIM(COALESCE(status, ''))) IN ('success', 'completed') AND ({failure_class_sql}) = 'none'"
    );
    let failure_sql = format!(
        "LOWER(TRIM(COALESCE(status, ''))) NOT IN ('', 'running', 'pending') AND ({failure_class_sql}) <> 'none'"
    );
    let non_success_sql = format!(
        "(LOWER(TRIM(COALESCE(status, ''))) = 'interrupted' OR \
         (LOWER(TRIM(COALESCE(status, ''))) NOT IN ('running', 'pending') \
          AND ({failure_class_sql}) <> 'none'))"
    );
    let first_response_byte_total_sql = "COALESCE(t_req_read_ms, 0) + COALESCE(t_req_parse_ms, 0) + COALESCE(t_upstream_connect_ms, 0) + COALESCE(t_upstream_ttfb_ms, 0)";
    let prompt_cache_key_sql = INVOCATION_PROMPT_CACHE_KEY_SQL;
    let filtered_prompt_cache_key_sql = "filtered_invocations.prompt_cache_key";
    let preaggregated_conversation_created_at_sql = if use_attempt_fallback {
        prompt_cache_conversation_created_at_sql(filtered_prompt_cache_key_sql, source_scope)
    } else {
        invocation_history_conversation_created_at_sql(filtered_prompt_cache_key_sql, source_scope)
    };
    let mut query = QueryBuilder::<Sqlite>::new(format!(
        r#"
        WITH filtered_invocations AS (
            SELECT
                occurred_at,
                status,
                total_tokens,
                cost,
                cache_input_tokens,
                payload,
                error_message,
                failure_kind,
                failure_class,
                is_actionable,
                t_req_read_ms,
                t_req_parse_ms,
                t_upstream_connect_ms,
                t_upstream_ttfb_ms,
                t_total_ms,
                {upstream_account_id_sql} AS upstream_account_id,
                {prompt_cache_key_sql} AS prompt_cache_key
            FROM codex_invocations
            WHERE occurred_at >=
        "#,
    ));
    query
        .push_bind(db_occurred_at_lower_bound(range.start))
        .push(" AND occurred_at < ")
        .push_bind(db_occurred_at_upper_bound(range.end));
    if source_scope == InvocationSourceScope::ProxyOnly {
        query.push(" AND source = ").push_bind(SOURCE_PROXY);
    }
    query.push(" AND LOWER(TRIM(COALESCE(status, ''))) NOT IN ('running', 'pending')");
    query.push(format!(
        r#"
        ),
        conversation_created_at_by_key AS (
            SELECT
                filtered_invocations.prompt_cache_key AS prompt_cache_key,
                COALESCE(
                    {preaggregated_conversation_created_at_sql},
                    MIN(filtered_invocations.occurred_at)
                ) AS conversation_created_at
            FROM filtered_invocations
            WHERE filtered_invocations.prompt_cache_key IS NOT NULL
              AND filtered_invocations.prompt_cache_key <> ''
            GROUP BY filtered_invocations.prompt_cache_key
        )
        SELECT
            filtered_invocations.upstream_account_id AS upstream_account_id,
            MAX(COALESCE(conversation_created_at_by_key.conversation_created_at, filtered_invocations.occurred_at)) AS latest_conversation_created_at,
            MAX(filtered_invocations.occurred_at) AS last_invocation_at,
            COUNT(*) AS request_count,
            SUM(CASE WHEN {success_sql} THEN 1 ELSE 0 END) AS success_count,
            SUM(CASE WHEN {failure_sql} THEN 1 ELSE 0 END) AS failure_count,
            SUM(CASE WHEN {non_success_sql} THEN 1 ELSE 0 END) AS non_success_count,
            COALESCE(SUM(COALESCE(total_tokens, 0)), 0) AS total_tokens,
            COALESCE(SUM(CASE WHEN {success_sql} THEN COALESCE(total_tokens, 0) ELSE 0 END), 0) AS success_tokens,
            COALESCE(SUM(CASE WHEN {non_success_sql} THEN COALESCE(total_tokens, 0) ELSE 0 END), 0) AS non_success_tokens,
            COALESCE(SUM(CASE WHEN {failure_sql} THEN COALESCE(total_tokens, 0) ELSE 0 END), 0) AS failure_tokens,
            CAST(COALESCE(SUM(CASE WHEN {failure_sql} THEN COALESCE(cost, 0) ELSE 0 END), 0) AS REAL) AS failure_cost,
            CAST(COALESCE(SUM(CASE WHEN {non_success_sql} THEN COALESCE(cost, 0) ELSE 0 END), 0) AS REAL) AS non_success_cost,
            COALESCE(SUM(COALESCE(cache_input_tokens, 0)), 0) AS cache_input_tokens,
            CAST(COALESCE(SUM(COALESCE(cost, 0)), 0) AS REAL) AS total_cost,
            SUM(CASE WHEN {success_sql} AND t_upstream_ttfb_ms > 0 THEN 1 ELSE 0 END) AS first_response_byte_total_sample_count,
            CAST(COALESCE(SUM(CASE WHEN {success_sql} AND t_upstream_ttfb_ms > 0 THEN {first_response_byte_total_sql} ELSE 0 END), 0) AS REAL) AS first_response_byte_total_sum_ms,
            SUM(CASE WHEN {success_sql} AND t_total_ms >= 0 THEN 1 ELSE 0 END) AS total_latency_sample_count,
            CAST(COALESCE(SUM(CASE WHEN {success_sql} AND t_total_ms >= 0 THEN t_total_ms ELSE 0 END), 0) AS REAL) AS total_latency_sum_ms
        FROM filtered_invocations
        LEFT JOIN conversation_created_at_by_key
          ON conversation_created_at_by_key.prompt_cache_key = filtered_invocations.prompt_cache_key
        "#,
    ));
    query.push(" GROUP BY filtered_invocations.upstream_account_id");

    let rows = query
        .build_query_as::<UpstreamAccountActivityAggregateRow>()
        .fetch_all(pool)
        .await?;
    let elapsed_ms = started_at.elapsed().as_millis() as u64;
    if elapsed_ms >= 1_000 {
        tracing::warn!(
            endpoint = "/api/stats/upstream-account-activity",
            operation = "live_account_aggregate",
            ?source_scope,
            start = %range.start,
            end = %range.end,
            row_count = rows.len(),
            elapsed_ms,
            "slow upstream-account activity aggregate"
        );
    }
    Ok(rows)
}

async fn query_live_upstream_account_prompt_cache_created_at_rows(
    pool: &Pool<Sqlite>,
    source_scope: InvocationSourceScope,
    range: ExactUtcRange,
    use_attempt_fallback: bool,
) -> Result<Vec<UpstreamAccountPromptCacheCreatedAtRow>, ApiError> {
    let upstream_account_id_sql = if use_attempt_fallback {
        invocation_upstream_account_id_with_attempt_fallback_sql("codex_invocations")
    } else {
        INVOCATION_UPSTREAM_ACCOUNT_ID_SQL.to_string()
    };
    let prompt_cache_key_sql = INVOCATION_PROMPT_CACHE_KEY_SQL;
    let normalized_status_sql = INVOCATION_STATUS_NORMALIZED_SQL;
    let mut query = QueryBuilder::<Sqlite>::new("SELECT ");
    query
        .push(upstream_account_id_sql.as_str())
        .push(" AS upstream_account_id, ")
        .push(prompt_cache_key_sql)
        .push(" AS prompt_cache_key, MIN(occurred_at) AS first_occurred_at FROM codex_invocations WHERE occurred_at >= ")
        .push_bind(db_occurred_at_lower_bound(range.start))
        .push(" AND occurred_at < ")
        .push_bind(db_occurred_at_upper_bound(range.end))
        .push(" AND ")
        .push(normalized_status_sql)
        .push(" NOT IN ('running', 'pending') AND ")
        .push(prompt_cache_key_sql)
        .push(" IS NOT NULL AND ")
        .push(prompt_cache_key_sql)
        .push(" <> ''");
    if source_scope == InvocationSourceScope::ProxyOnly {
        query.push(" AND source = ").push_bind(SOURCE_PROXY);
    }
    query.push(" GROUP BY upstream_account_id, prompt_cache_key");
    query
        .build_query_as::<UpstreamAccountPromptCacheCreatedAtRow>()
        .fetch_all(pool)
        .await
        .map_err(Into::into)
}

async fn query_live_upstream_account_usage_breakdown_rows(
    pool: &Pool<Sqlite>,
    source_scope: InvocationSourceScope,
    range: ExactUtcRange,
    has_cost_breakdown_columns: bool,
    use_attempt_fallback: bool,
) -> Result<Vec<UpstreamAccountUsageBreakdownAggregateRow>, ApiError> {
    let started_at = Instant::now();
    let upstream_account_id_sql = if use_attempt_fallback {
        invocation_upstream_account_id_with_attempt_fallback_sql("codex_invocations")
    } else {
        INVOCATION_UPSTREAM_ACCOUNT_ID_SQL.to_string()
    };
    let model_sql = format!(
        "COALESCE(NULLIF(TRIM({}), ''), NULLIF(TRIM(model), ''), 'unknown')",
        INVOCATION_RESPONSE_MODEL_SQL
    );
    let reasoning_effort_sql = INVOCATION_REASONING_EFFORT_SQL;
    let cost_complete_sql = if has_cost_breakdown_columns {
        "cost_input IS NOT NULL AND cost_cache_write IS NOT NULL AND cost_cache_read IS NOT NULL AND cost_output IS NOT NULL AND cost_reasoning IS NOT NULL"
    } else {
        "0"
    };
    let cost_input_sql = if has_cost_breakdown_columns {
        "cost_input"
    } else {
        "0"
    };
    let cost_cache_write_sql = if has_cost_breakdown_columns {
        "cost_cache_write"
    } else {
        "0"
    };
    let cost_cache_read_sql = if has_cost_breakdown_columns {
        "cost_cache_read"
    } else {
        "0"
    };
    let cost_output_sql = if has_cost_breakdown_columns {
        "cost_output"
    } else {
        "0"
    };
    let cost_reasoning_sql = if has_cost_breakdown_columns {
        "cost_reasoning"
    } else {
        "0"
    };
    let failure_class_sql = INVOCATION_RESOLVED_FAILURE_CLASS_SQL;
    let success_billed_sql = format!(
        "LOWER(TRIM(COALESCE(status, ''))) IN ('success', 'completed') AND ({failure_class_sql}) = 'none' AND cost IS NOT NULL"
    );
    let first_response_byte_total_sql = "COALESCE(t_req_read_ms, 0) + COALESCE(t_req_parse_ms, 0) + COALESCE(t_upstream_connect_ms, 0) + COALESCE(t_upstream_ttfb_ms, 0)";
    let mut query = QueryBuilder::<Sqlite>::new(format!(
        r#"
        SELECT
            {upstream_account_id_sql} AS upstream_account_id,
            {model_sql} AS model,
            {reasoning_effort_sql} AS reasoning_effort,
            COALESCE(SUM(MAX(COALESCE(input_tokens, 0) - COALESCE(cache_input_tokens, 0), 0)), 0) AS cache_write_tokens,
            COALESCE(SUM(COALESCE(cache_input_tokens, 0)), 0) AS cache_read_tokens,
            COALESCE(SUM(COALESCE(output_tokens, 0)), 0) AS output_tokens,
            CAST(COALESCE(SUM(CASE WHEN cost IS NOT NULL AND {cost_complete_sql} THEN {cost_input_sql} ELSE 0 END), 0) AS REAL) AS cost_input,
            CAST(COALESCE(SUM(CASE WHEN cost IS NOT NULL AND {cost_complete_sql} THEN {cost_cache_write_sql} ELSE 0 END), 0) AS REAL) AS cost_cache_write,
            CAST(COALESCE(SUM(CASE WHEN cost IS NOT NULL AND {cost_complete_sql} THEN {cost_cache_read_sql} ELSE 0 END), 0) AS REAL) AS cost_cache_read,
            CAST(COALESCE(SUM(CASE WHEN cost IS NOT NULL AND {cost_complete_sql} THEN {cost_output_sql} ELSE 0 END), 0) AS REAL) AS cost_output,
            CAST(COALESCE(SUM(CASE WHEN cost IS NOT NULL AND {cost_complete_sql} THEN {cost_reasoning_sql} ELSE 0 END), 0) AS REAL) AS cost_reasoning,
            CAST(COALESCE(SUM(CASE WHEN cost IS NOT NULL AND NOT ({cost_complete_sql}) THEN cost ELSE 0 END), 0) AS REAL) AS cost_unknown,
            SUM(CASE WHEN cost IS NOT NULL THEN 1 ELSE 0 END) AS has_cost,
            COALESCE(SUM(CASE WHEN {success_billed_sql} THEN COALESCE(total_tokens, 0) ELSE 0 END), 0) AS performance_total_tokens,
            COALESCE(SUM(CASE WHEN {success_billed_sql} AND t_upstream_stream_ms >= 0 THEN COALESCE(output_tokens, 0) ELSE 0 END), 0) AS performance_stream_output_tokens,
            CAST(COALESCE(SUM(CASE WHEN {success_billed_sql} AND t_upstream_stream_ms >= 0 THEN t_upstream_stream_ms ELSE 0 END), 0) AS REAL) AS performance_stream_duration_ms,
            SUM(CASE WHEN {success_billed_sql} AND t_upstream_stream_ms >= 0 THEN 1 ELSE 0 END) AS performance_response_sample_count,
            CAST(COALESCE(SUM(CASE WHEN {success_billed_sql} AND t_upstream_stream_ms >= 0 THEN t_upstream_stream_ms ELSE 0 END), 0) AS REAL) AS performance_response_sum_ms,
            SUM(CASE WHEN {success_billed_sql} AND t_upstream_ttfb_ms > 0 THEN 1 ELSE 0 END) AS performance_first_byte_sample_count,
            CAST(COALESCE(SUM(CASE WHEN {success_billed_sql} AND t_upstream_ttfb_ms > 0 THEN {first_response_byte_total_sql} ELSE 0 END), 0) AS REAL) AS performance_first_byte_sum_ms,
            SUM(CASE WHEN {success_billed_sql} AND t_total_ms >= 0 THEN 1 ELSE 0 END) AS performance_usage_duration_sample_count,
            CAST(COALESCE(SUM(CASE WHEN {success_billed_sql} AND t_total_ms >= 0 THEN t_total_ms ELSE 0 END), 0) AS REAL) AS performance_usage_duration_sum_ms
        FROM codex_invocations
        WHERE occurred_at >=
        "#,
    ));
    query
        .push_bind(db_occurred_at_lower_bound(range.start))
        .push(" AND occurred_at < ")
        .push_bind(db_occurred_at_upper_bound(range.end));
    if source_scope == InvocationSourceScope::ProxyOnly {
        query.push(" AND source = ").push_bind(SOURCE_PROXY);
    }
    query.push(" AND LOWER(TRIM(COALESCE(status, ''))) NOT IN ('running', 'pending')");
    query.push(" GROUP BY upstream_account_id, model, reasoning_effort");
    let rows = query
        .build_query_as::<UpstreamAccountUsageBreakdownAggregateRow>()
        .fetch_all(pool)
        .await?;
    let elapsed_ms = started_at.elapsed().as_millis() as u64;
    if elapsed_ms >= 1_000 {
        tracing::warn!(
            endpoint = "/api/stats/upstream-account-activity",
            operation = "live_usage_breakdown",
            ?source_scope,
            start = %range.start,
            end = %range.end,
            row_count = rows.len(),
            elapsed_ms,
            "slow upstream-account usage breakdown"
        );
    }
    Ok(rows)
}

async fn query_completed_invocation_archive_activity_aggregate_rows(
    pool: &Pool<Sqlite>,
    source_scope: InvocationSourceScope,
    range: ExactUtcRange,
) -> Result<
    (
        Vec<UpstreamAccountActivityAggregateRow>,
        Vec<UpstreamAccountUsageBreakdownAggregateRow>,
    ),
    ApiError,
> {
    let archive_rows = crate::stats::load_completed_invocation_archive_paths_in_range(
        pool,
        Some((range.start, range.end)),
    )
    .await?;
    let mut aggregates = Vec::new();
    let mut usage_breakdowns = Vec::new();
    let mut earliest_created_at_by_prompt_cache_key = HashMap::<String, String>::new();
    let mut prompt_cache_keys_by_account = HashMap::<Option<i64>, HashSet<String>>::new();
    for archive_row in archive_rows {
        let Some((archive_pool, temp_cleanup)) = crate::stats::open_invocation_archive_batch_pool(
            &archive_row,
            "dashboard-activity-summary",
        )
        .await?
        else {
            continue;
        };
        let has_cost_breakdown_columns =
            crate::stats::sqlite_table_has_column(&archive_pool, "codex_invocations", "cost_input")
                .await?;
        aggregates.extend(
            query_live_upstream_account_activity_aggregate_rows(
                &archive_pool,
                source_scope,
                range,
                false,
            )
            .await?,
        );
        for row in query_live_upstream_account_prompt_cache_created_at_rows(
            &archive_pool,
            source_scope,
            range,
            false,
        )
        .await?
        {
            let UpstreamAccountPromptCacheCreatedAtRow {
                upstream_account_id,
                prompt_cache_key,
                first_occurred_at,
            } = row;
            earliest_created_at_by_prompt_cache_key
                .entry(prompt_cache_key.clone())
                .and_modify(|current| {
                    if first_occurred_at < *current {
                        *current = first_occurred_at.clone();
                    }
                })
                .or_insert(first_occurred_at);
            prompt_cache_keys_by_account
                .entry(upstream_account_id)
                .or_default()
                .insert(prompt_cache_key);
        }
        usage_breakdowns.extend(
            query_live_upstream_account_usage_breakdown_rows(
                &archive_pool,
                source_scope,
                range,
                has_cost_breakdown_columns,
                false,
            )
            .await?,
        );
        archive_pool.close().await;
        drop(temp_cleanup);
    }
    let mut latest_created_at_by_account = HashMap::<Option<i64>, String>::new();
    for (upstream_account_id, prompt_cache_keys) in prompt_cache_keys_by_account {
        let mut latest_created_at = None;
        for prompt_cache_key in prompt_cache_keys {
            merge_latest_optional_timestamp(
                &mut latest_created_at,
                earliest_created_at_by_prompt_cache_key
                    .get(&prompt_cache_key)
                    .cloned(),
            );
        }
        if let Some(latest_created_at) = latest_created_at {
            latest_created_at_by_account.insert(upstream_account_id, latest_created_at);
        }
    }
    for aggregate in &mut aggregates {
        if let Some(latest_created_at) =
            latest_created_at_by_account.get(&aggregate.upstream_account_id)
        {
            aggregate.latest_conversation_created_at = Some(latest_created_at.clone());
        }
    }
    Ok((aggregates, usage_breakdowns))
}

#[derive(Debug, FromRow)]
struct UpstreamAccountActivityRateRow {
    upstream_account_id: Option<i64>,
    occurred_at: String,
    total_tokens: i64,
    total_cost: f64,
}

async fn query_live_upstream_account_activity_rate_rows(
    pool: &Pool<Sqlite>,
    source_scope: InvocationSourceScope,
    range: ExactUtcRange,
) -> Result<Vec<UpstreamAccountActivityRateRow>, ApiError> {
    let rate_start = range.start.max(range.end - chrono::Duration::minutes(5));
    let resolved_upstream_account_id_sql =
        invocation_upstream_account_id_with_attempt_fallback_sql("codex_invocations");
    let mut query = QueryBuilder::<Sqlite>::new("SELECT ");
    query
        .push(resolved_upstream_account_id_sql.as_str())
        .push(" AS upstream_account_id, occurred_at, COALESCE(total_tokens, 0) AS total_tokens, COALESCE(cost, 0.0) AS total_cost FROM codex_invocations WHERE occurred_at >= ")
        .push_bind(db_occurred_at_lower_bound(rate_start))
        .push(" AND occurred_at < ")
        .push_bind(db_occurred_at_upper_bound(range.end));
    if source_scope == InvocationSourceScope::ProxyOnly {
        query.push(" AND source = ").push_bind(SOURCE_PROXY);
    }
    query.push(" AND LOWER(TRIM(COALESCE(status, ''))) NOT IN ('running', 'pending')");
    query
        .build_query_as::<UpstreamAccountActivityRateRow>()
        .fetch_all(pool)
        .await
        .map_err(Into::into)
}

fn add_upstream_account_activity_preview_row(
    account_activity: &mut HashMap<Option<i64>, UpstreamAccountActivityAccumulator>,
    row: UpstreamAccountInvocationPreviewRow,
    recent_limit: usize,
    include_accounts: bool,
) {
    let Some(occurred_at) = parse_to_utc_datetime(&row.occurred_at) else {
        return;
    };
    let classification = resolve_failure_classification(
        Some(row.status.as_str()),
        row.error_message.as_deref(),
        row.failure_kind.as_deref(),
        row.failure_class.as_deref(),
        row.is_actionable,
    );
    let is_success = prompt_cache_and_timeseries_shared::prompt_invocation_status_is_success_like(
        Some(row.status.as_str()),
        row.error_message.as_deref(),
    ) && classification.failure_class == FailureClass::None;
    let counts_toward_failure =
        prompt_cache_and_timeseries_shared::prompt_invocation_status_counts_toward_terminal_totals(
            Some(row.status.as_str()),
        ) && classification.failure_class != FailureClass::None;
    let counts_toward_non_success = invocation_counts_toward_non_success_usage(
        Some(row.status.as_str()),
        row.error_message.as_deref(),
        row.failure_kind.as_deref(),
        row.failure_class.as_deref(),
        row.is_actionable,
    );

    let entry = account_activity.entry(row.upstream_account_id).or_default();
    entry.last_occurred_at_epoch_ms = entry
        .last_occurred_at_epoch_ms
        .max(occurred_at.timestamp_millis());
    merge_latest_optional_timestamp(&mut entry.last_invocation_at, Some(row.occurred_at.clone()));
    merge_latest_optional_timestamp(
        &mut entry.latest_conversation_created_at,
        row.conversation_created_at.clone(),
    );
    if entry.display_name_hint.is_none() {
        entry.display_name_hint =
            normalize_trimmed_optional_string_local(row.upstream_account_name.clone());
    }
    if entry.plan_type_hint.is_none() {
        entry.plan_type_hint =
            normalize_trimmed_optional_string_local(row.upstream_account_plan_type.clone());
    }
    entry.request_count += 1;
    entry.total_tokens += row.total_tokens.max(0);
    entry.cache_input_tokens += row.cache_input_tokens.unwrap_or_default().max(0);
    entry.total_cost += row.cost.unwrap_or_default();
    entry.usage_breakdown.add_row(&row);
    if is_success {
        entry.success_count += 1;
        entry.success_tokens += row.total_tokens.max(0);
        if let Some(first_response_byte_total_ms) =
            crate::stats::resolve_first_response_byte_total_ms(
                row.t_req_read_ms,
                row.t_req_parse_ms,
                row.t_upstream_connect_ms,
                row.t_upstream_ttfb_ms,
            )
        {
            entry.first_response_byte_total_sample_count += 1;
            entry.first_response_byte_total_sum_ms += first_response_byte_total_ms;
        }
        if let Some(total_ms) = row
            .t_total_ms
            .filter(|value| value.is_finite() && *value >= 0.0)
        {
            entry.total_latency_sample_count += 1;
            entry.total_latency_sum_ms += total_ms;
        }
    } else if counts_toward_failure {
        entry.failure_count += 1;
        entry.failure_tokens += row.total_tokens.max(0);
        entry.failure_cost += row.cost.unwrap_or_default();
    }
    if counts_toward_non_success {
        entry.non_success_count += 1;
        entry.non_success_tokens += row.total_tokens.max(0);
        entry.non_success_cost += row.cost.unwrap_or_default();
    }
    entry.rate_usage_events.push(UpstreamAccountRateUsageEvent {
        occurred_at_epoch_ms: occurred_at.timestamp_millis(),
        total_tokens: row.total_tokens.max(0),
        total_cost: row.cost.unwrap_or_default().max(0.0),
    });
    if matches!(
        normalized_runtime_text(Some(row.status.as_str())).as_str(),
        "running" | "pending"
    ) && let Some(ttfb_ms) = row
        .t_upstream_ttfb_ms
        .filter(|value| value.is_finite() && *value >= 0.0)
    {
        entry.in_progress_wait_sum_ms += ttfb_ms;
        entry.in_progress_wait_sample_count += 1;
    }
    if include_accounts && entry.recent_invocations.len() < recent_limit {
        entry
            .recent_invocations
            .push(upstream_account_invocation_preview_from_row(row));
    }
}

pub(crate) async fn query_live_upstream_account_activity_preview_rows(
    pool: &Pool<Sqlite>,
    source_scope: InvocationSourceScope,
    range: ExactUtcRange,
) -> Result<Vec<UpstreamAccountInvocationPreviewRow>, ApiError> {
    query_live_upstream_account_activity_preview_rows_with_limit(
        pool,
        source_scope,
        range,
        None,
        None,
        false,
    )
    .await
}

async fn query_live_upstream_account_activity_preview_rows_with_limit(
    pool: &Pool<Sqlite>,
    source_scope: InvocationSourceScope,
    range: ExactUtcRange,
    upstream_account_id: Option<Option<i64>>,
    limit: Option<usize>,
    in_progress_only: bool,
) -> Result<Vec<UpstreamAccountInvocationPreviewRow>, ApiError> {
    let started_at = Instant::now();
    let resolved_upstream_account_id_sql =
        invocation_upstream_account_id_with_attempt_fallback_sql("codex_invocations");
    let conversation_created_at_sql = format!(
        "COALESCE({}, occurred_at)",
        prompt_cache_conversation_created_at_sql(INVOCATION_PROMPT_CACHE_KEY_SQL, source_scope)
    );
    let mut query = QueryBuilder::<Sqlite>::new("SELECT id, invoke_id, ");
    query
        .push(INVOCATION_PROMPT_CACHE_KEY_SQL)
        .push(" AS prompt_cache_key, occurred_at, ")
        .push(conversation_created_at_sql.as_str())
        .push(" AS conversation_created_at, ")
        .push(invocation_display_status_sql())
        .push(" AS status, ")
        .push(invocation_live_phase_sql("codex_invocations"))
        .push(" AS live_phase, ")
        .push(INVOCATION_RESOLVED_FAILURE_CLASS_SQL)
        .push(" AS failure_class, ")
        .push(INVOCATION_ROUTE_MODE_SQL)
        .push(" AS route_mode, model, ")
        .push(INVOCATION_REQUEST_MODEL_SQL)
        .push(" AS request_model, ")
        .push(INVOCATION_RESPONSE_MODEL_SQL)
        .push(" AS response_model, COALESCE(total_tokens, 0) AS total_tokens, cost, cost_input, cost_cache_write, cost_cache_read, cost_output, cost_reasoning, source, input_tokens, output_tokens, cache_input_tokens, reasoning_tokens, ")
        .push(INVOCATION_REASONING_EFFORT_SQL)
        .push(" AS reasoning_effort, error_message, ")
        .push(INVOCATION_FAILURE_KIND_SQL)
        .push(" AS failure_kind, CASE WHEN ")
        .push(INVOCATION_RESOLVED_FAILURE_CLASS_SQL)
        .push(" = 'service_failure' THEN 1 ELSE 0 END AS is_actionable, ")
        .push(INVOCATION_PROXY_DISPLAY_SQL)
        .push(" AS proxy_display_name, ")
        .push(resolved_upstream_account_id_sql.as_str())
        .push(" AS upstream_account_id, ")
        .push(INVOCATION_UPSTREAM_ACCOUNT_NAME_SQL)
        .push(" AS upstream_account_name, ")
        .push(INVOCATION_UPSTREAM_ACCOUNT_PLAN_TYPE_SQL)
        .push(" AS upstream_account_plan_type, ")
        .push(INVOCATION_RESPONSE_CONTENT_ENCODING_SQL)
        .push(" AS response_content_encoding, ")
        .push(INVOCATION_TRANSPORT_SQL)
        .push(" AS transport, ")
        .push(INVOCATION_COMPACTION_REQUEST_KIND_SQL)
        .push(" AS compaction_request_kind, ")
        .push(INVOCATION_COMPACTION_RESPONSE_KIND_SQL)
        .push(" AS compaction_response_kind, ")
        .push(INVOCATION_IMAGE_INTENT_SQL)
        .push(
            " AS image_intent, \
             CASE \
               WHEN json_valid(payload) AND json_type(payload, '$.requestedServiceTier') = 'text' \
                 THEN json_extract(payload, '$.requestedServiceTier') \
               WHEN json_valid(payload) AND json_type(payload, '$.requested_service_tier') = 'text' \
                 THEN json_extract(payload, '$.requested_service_tier') END AS requested_service_tier, \
             CASE \
               WHEN json_valid(payload) AND json_type(payload, '$.serviceTier') = 'text' \
                 THEN json_extract(payload, '$.serviceTier') \
               WHEN json_valid(payload) AND json_type(payload, '$.service_tier') = 'text' \
                 THEN json_extract(payload, '$.service_tier') END AS service_tier, \
             ",
        )
        .push(INVOCATION_BILLING_SERVICE_TIER_SQL)
        .push(" AS billing_service_tier, t_req_read_ms, t_req_parse_ms, t_upstream_connect_ms, t_upstream_ttfb_ms, t_upstream_stream_ms, t_resp_parse_ms, t_persist_ms, t_total_ms, ")
        .push(INVOCATION_DOWNSTREAM_STATUS_CODE_SQL)
        .push(" AS downstream_status_code, ")
        .push(INVOCATION_DOWNSTREAM_ERROR_MESSAGE_SQL)
        .push(" AS downstream_error_message, ")
        .push(INVOCATION_ENDPOINT_SQL)
        .push(
            " AS endpoint \
             FROM codex_invocations \
             WHERE occurred_at >= ",
        )
        .push_bind(db_occurred_at_lower_bound(range.start))
        .push(" AND occurred_at < ")
        .push_bind(db_occurred_at_upper_bound(range.end));
    if source_scope == InvocationSourceScope::ProxyOnly {
        query.push(" AND source = ").push_bind(SOURCE_PROXY);
    }
    if in_progress_only {
        query.push(" AND LOWER(TRIM(COALESCE(status, ''))) IN ('running', 'pending')");
    }
    if let Some(upstream_account_id) = upstream_account_id {
        query
            .push(" AND ")
            .push(resolved_upstream_account_id_sql.as_str());
        match upstream_account_id {
            Some(upstream_account_id) => {
                query.push(" = ").push_bind(upstream_account_id);
            }
            None => {
                query.push(" IS NULL");
            }
        }
    }
    query.push(" ORDER BY occurred_at DESC, id DESC");
    if let Some(limit) = limit {
        query.push(" LIMIT ").push_bind(limit as i64);
    }

    let rows = query
        .build_query_as::<UpstreamAccountInvocationPreviewRow>()
        .fetch_all(pool)
        .await?;
    let elapsed_ms = started_at.elapsed().as_millis() as u64;
    if elapsed_ms >= 1_000 {
        tracing::warn!(
            endpoint = "/api/stats/upstream-account-activity",
            operation = if limit.is_some() {
                "live_preview_rows_limited"
            } else {
                "live_preview_rows"
            },
            ?source_scope,
            start = %range.start,
            end = %range.end,
            upstream_account_id = ?upstream_account_id.flatten(),
            row_count = rows.len(),
            elapsed_ms,
            "slow upstream-account activity read"
        );
    }
    Ok(rows)
}

async fn query_runtime_recent_account_fallback_rows(
    pool: &Pool<Sqlite>,
    source_scope: InvocationSourceScope,
    keys: &HashSet<(String, String)>,
) -> Result<Vec<RuntimeRecentAccountFallbackRow>, ApiError> {
    if keys.is_empty() {
        return Ok(Vec::new());
    }

    let keys_json = serde_json::Value::Array(
        keys.iter()
            .map(|(invoke_id, occurred_at)| json!([invoke_id, occurred_at]))
            .collect(),
    )
    .to_string();
    let resolved_upstream_account_id_sql =
        invocation_upstream_account_id_with_attempt_fallback_sql("codex_invocations");
    let mut query = QueryBuilder::<Sqlite>::new(
        "WITH runtime_keys AS (\
           SELECT CAST(json_extract(value, '$[0]') AS TEXT) AS invoke_id, \
                  CAST(json_extract(value, '$[1]') AS TEXT) AS occurred_at \
             FROM json_each(",
    );
    query
        .push_bind(keys_json)
        .push(
            ")\
         ) \
         SELECT codex_invocations.invoke_id, codex_invocations.occurred_at, ",
        )
        .push(resolved_upstream_account_id_sql.as_str())
        .push(" AS upstream_account_id, ")
        .push(INVOCATION_UPSTREAM_ACCOUNT_NAME_SQL)
        .push(" AS upstream_account_name, ")
        .push(INVOCATION_UPSTREAM_ACCOUNT_PLAN_TYPE_SQL)
        .push(
            " AS upstream_account_plan_type \
             FROM codex_invocations \
             JOIN runtime_keys \
               ON runtime_keys.invoke_id = codex_invocations.invoke_id \
              AND runtime_keys.occurred_at = codex_invocations.occurred_at \
             WHERE 1 = 1",
        );
    if source_scope == InvocationSourceScope::ProxyOnly {
        query
            .push(" AND codex_invocations.source = ")
            .push_bind(SOURCE_PROXY);
    }

    query
        .build_query_as::<RuntimeRecentAccountFallbackRow>()
        .fetch_all(pool)
        .await
        .map_err(Into::into)
}

pub(crate) fn runtime_upstream_account_activity_preview_row(
    record: ApiInvocation,
    source_scope: InvocationSourceScope,
) -> Option<UpstreamAccountInvocationPreviewRow> {
    runtime_upstream_account_activity_preview_row_with_terminal(record, source_scope, false)
}

fn runtime_upstream_account_activity_preview_row_with_terminal(
    record: ApiInvocation,
    source_scope: InvocationSourceScope,
    include_terminal: bool,
) -> Option<UpstreamAccountInvocationPreviewRow> {
    if source_scope == InvocationSourceScope::ProxyOnly && record.source != SOURCE_PROXY {
        return None;
    }
    if !include_terminal
        && !matches!(
            normalized_runtime_text(record.status.as_deref()).as_str(),
            "running" | "pending"
        )
    {
        return None;
    }
    let live_phase = record
        .live_phase
        .clone()
        .or_else(|| runtime_invocation_live_phase(&record).map(str::to_string));
    Some(UpstreamAccountInvocationPreviewRow {
        upstream_account_id: record.upstream_account_id,
        id: record.id,
        invoke_id: record.invoke_id,
        prompt_cache_key: record.prompt_cache_key,
        occurred_at: record.occurred_at,
        conversation_created_at: None,
        status: record.status.unwrap_or_else(|| "running".to_string()),
        live_phase,
        failure_class: record.failure_class,
        route_mode: record.route_mode,
        model: record.model,
        request_model: record.request_model,
        response_model: record.response_model,
        total_tokens: record.total_tokens.unwrap_or_default(),
        cost: record.cost,
        cost_input: record.cost_input,
        cost_cache_write: record.cost_cache_write,
        cost_cache_read: record.cost_cache_read,
        cost_output: record.cost_output,
        cost_reasoning: record.cost_reasoning,
        source: Some(record.source),
        input_tokens: record.input_tokens,
        output_tokens: record.output_tokens,
        cache_input_tokens: record.cache_input_tokens,
        reasoning_tokens: record.reasoning_tokens,
        reasoning_effort: record.reasoning_effort,
        error_message: record.error_message,
        downstream_status_code: record.downstream_status_code,
        downstream_error_message: record.downstream_error_message,
        failure_kind: record.failure_kind,
        is_actionable: record.is_actionable.map(|value| if value { 1 } else { 0 }),
        proxy_display_name: record.proxy_display_name,
        upstream_account_name: record.upstream_account_name,
        upstream_account_plan_type: None,
        response_content_encoding: record.response_content_encoding,
        transport: record.transport,
        requested_service_tier: record.requested_service_tier,
        service_tier: record.service_tier,
        billing_service_tier: record.billing_service_tier,
        t_req_read_ms: record.t_req_read_ms,
        t_req_parse_ms: record.t_req_parse_ms,
        t_upstream_connect_ms: record.t_upstream_connect_ms,
        t_upstream_ttfb_ms: record.t_upstream_ttfb_ms,
        t_upstream_stream_ms: record.t_upstream_stream_ms,
        t_resp_parse_ms: record.t_resp_parse_ms,
        t_persist_ms: record.t_persist_ms,
        t_total_ms: record.t_total_ms,
        endpoint: record.endpoint,
        compaction_request_kind: record.compaction_request_kind,
        compaction_response_kind: record.compaction_response_kind,
        image_intent: record.image_intent,
    })
}

pub(crate) fn overlay_runtime_upstream_account_activity_preview_rows(
    state: &AppState,
    rows: &mut Vec<UpstreamAccountInvocationPreviewRow>,
    source_scope: InvocationSourceScope,
    range: ExactUtcRange,
) {
    let mut runtime_overlay_row_count = 0_i64;
    for record in state.proxy_runtime_invocations.snapshot() {
        let Some(mut row) = runtime_upstream_account_activity_preview_row(record, source_scope)
        else {
            continue;
        };
        let Some(occurred_at) = parse_to_utc_datetime(&row.occurred_at) else {
            continue;
        };
        if occurred_at < range.start || occurred_at >= range.end {
            continue;
        }
        let key = (row.invoke_id.clone(), row.occurred_at.clone());
        if let Some(existing) = rows
            .iter_mut()
            .find(|existing| (existing.invoke_id.clone(), existing.occurred_at.clone()) == key)
        {
            if matches!(
                normalized_runtime_text(Some(existing.status.as_str())).as_str(),
                "running" | "pending"
            ) {
                if row.upstream_account_id.is_none() && existing.upstream_account_id.is_some() {
                    row.upstream_account_id = existing.upstream_account_id;
                    row.upstream_account_name = existing.upstream_account_name.clone();
                    row.upstream_account_plan_type = existing.upstream_account_plan_type.clone();
                }
                *existing = row;
                runtime_overlay_row_count += 1;
            }
        } else {
            rows.push(row);
            runtime_overlay_row_count += 1;
        }
    }
    if runtime_overlay_row_count > 0 {
        debug!(
            endpoint = "/api/upstream-account-activity",
            runtime_overlay_row_count,
            "overlayed memory runtime invocation rows into upstream account activity"
        );
    }
}

async fn overlay_runtime_terminal_upstream_account_activity_preview_rows(
    state: &AppState,
    rows: &mut Vec<UpstreamAccountInvocationPreviewRow>,
    source_scope: InvocationSourceScope,
    range: ExactUtcRange,
) -> Result<(), ApiError> {
    let mut terminal_rows = Vec::new();
    for record in state.proxy_runtime_invocations.snapshot() {
        if matches!(
            normalized_runtime_text(record.status.as_deref()).as_str(),
            "running" | "pending"
        ) {
            continue;
        }
        let Some(row) =
            runtime_upstream_account_activity_preview_row_with_terminal(record, source_scope, true)
        else {
            continue;
        };
        let Some(occurred_at) = parse_to_utc_datetime(&row.occurred_at) else {
            continue;
        };
        if occurred_at >= range.start && occurred_at < range.end {
            terminal_rows.push(row);
        }
    }
    if terminal_rows.is_empty() {
        return Ok(());
    }

    let fallback_keys = terminal_rows
        .iter()
        .filter(|row| row.upstream_account_id.is_none())
        .map(|row| (row.invoke_id.clone(), row.occurred_at.clone()))
        .collect::<HashSet<_>>();
    let fallback_rows =
        query_runtime_recent_account_fallback_rows(&state.pool, source_scope, &fallback_keys)
            .await?
            .into_iter()
            .map(|row| ((row.invoke_id.clone(), row.occurred_at.clone()), row))
            .collect::<HashMap<_, _>>();

    let mut rows_by_key = rows
        .drain(..)
        .map(|row| ((row.invoke_id.clone(), row.occurred_at.clone()), row))
        .collect::<HashMap<_, _>>();
    for mut row in terminal_rows {
        let key = (row.invoke_id.clone(), row.occurred_at.clone());
        if let Some(existing) = rows_by_key.get(&key) {
            if row.upstream_account_id.is_none() {
                row.upstream_account_id = existing.upstream_account_id;
            }
            if row.upstream_account_name.is_none() {
                row.upstream_account_name = existing.upstream_account_name.clone();
            }
            if row.upstream_account_plan_type.is_none() {
                row.upstream_account_plan_type = existing.upstream_account_plan_type.clone();
            }
        }
        if let Some(fallback) = fallback_rows.get(&key) {
            if row.upstream_account_id.is_none() {
                row.upstream_account_id = fallback.upstream_account_id;
            }
            if row.upstream_account_name.is_none() {
                row.upstream_account_name = fallback.upstream_account_name.clone();
            }
            if row.upstream_account_plan_type.is_none() {
                row.upstream_account_plan_type = fallback.upstream_account_plan_type.clone();
            }
        }
        rows_by_key.insert(key, row);
    }
    rows.extend(rows_by_key.into_values());
    Ok(())
}

pub(crate) async fn load_usage_breakdown_for_range(
    state: &AppState,
    source_scope: InvocationSourceScope,
    upstream_account_id: Option<i64>,
    range: ExactUtcRange,
) -> Result<UsageBreakdownResponse, ApiError> {
    let account_metadata_table_count = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM sqlite_master \
         WHERE type = 'table' \
           AND name IN ('pool_upstream_accounts', 'pool_upstream_account_limit_samples')",
    )
    .fetch_one(&state.pool)
    .await?;
    let mut live_rows = if account_metadata_table_count == 2 {
        query_live_upstream_account_activity_preview_rows(&state.pool, source_scope, range).await?
    } else {
        Vec::new()
    };
    overlay_runtime_upstream_account_activity_preview_rows(
        state,
        &mut live_rows,
        source_scope,
        range,
    );
    let live_ids = live_rows.iter().map(|row| row.id).collect::<HashSet<_>>();
    let mut rows = live_rows;
    if range.start < shanghai_retention_cutoff(state.config.invocation_max_days) {
        rows.extend(
            crate::stats::query_completed_invocation_archive_preview_rows(
                &state.pool,
                source_scope,
                range,
                Some(&live_ids),
            )
            .await?,
        );
    }

    let mut usage_breakdown = UsageBreakdownAccumulator::default();
    for row in rows {
        if upstream_account_id.is_none_or(|account_id| row.upstream_account_id == Some(account_id))
        {
            usage_breakdown.add_row(&row);
        }
    }
    Ok(usage_breakdown.into_response())
}

pub(crate) async fn query_upstream_account_activity_meta(
    pool: &Pool<Sqlite>,
    account_ids: &[i64],
) -> Result<HashMap<i64, UpstreamAccountActivityMetaRow>, ApiError> {
    if account_ids.is_empty() {
        return Ok(HashMap::new());
    }
    let mut query = QueryBuilder::<Sqlite>::new(
        "SELECT \
            id, kind, display_name, group_name, plan_type, status, enabled, \
            last_error, last_error_at, last_route_failure_at, last_route_failure_kind, \
            last_action_reason_code, last_action_reason_message, cooldown_until, \
            temporary_route_failure_streak_started_at, last_selected_at \
         FROM pool_upstream_accounts WHERE id IN (",
    );
    {
        let mut separated = query.separated(", ");
        for account_id in account_ids {
            separated.push_bind(*account_id);
        }
    }
    query.push(")");
    Ok(query
        .build_query_as::<UpstreamAccountActivityMetaRow>()
        .fetch_all(pool)
        .await?
        .into_iter()
        .map(|row| (row.id, row))
        .collect())
}

pub(crate) async fn query_upstream_account_in_progress_counts(
    state: &AppState,
    source_scope: InvocationSourceScope,
) -> Result<HashMap<Option<i64>, UpstreamAccountInProgressSummary>, ApiError> {
    query_upstream_account_in_progress_counts_from_runtime(
        &state.pool,
        state.proxy_runtime_invocations.as_ref(),
        source_scope,
    )
    .await
}

pub(crate) async fn query_upstream_account_in_progress_counts_from_runtime(
    pool: &Pool<Sqlite>,
    proxy_runtime_invocations: &ProxyRuntimeInvocationStore,
    source_scope: InvocationSourceScope,
) -> Result<HashMap<Option<i64>, UpstreamAccountInProgressSummary>, ApiError> {
    #[derive(Debug, FromRow)]
    struct RuntimeKeyRow {
        invoke_id: String,
        occurred_at: String,
        upstream_account_id: Option<i64>,
        retry_count: i64,
        live_phase: Option<String>,
    }

    let resolved_upstream_account_id_sql =
        invocation_upstream_account_id_with_attempt_fallback_sql("inv");
    let retry_sql = invocation_account_retry_after_failure_with_attempt_fallback_sql(
        resolved_upstream_account_id_sql.as_str(),
        source_scope,
    );
    let mut db_key_query = QueryBuilder::<Sqlite>::new(
        "SELECT inv.invoke_id AS invoke_id, inv.occurred_at AS occurred_at, ",
    );
    db_key_query
        .push(resolved_upstream_account_id_sql.as_str())
        .push(" AS upstream_account_id, ")
        .push(retry_sql.as_str())
        .push(" AS retry_count, ");
    db_key_query.push(invocation_live_phase_sql("inv")).push(
        " AS live_phase \
         FROM invocation_in_progress_live live \
         JOIN codex_invocations inv ON inv.id = live.invocation_id \
         WHERE 1 = 1",
    );
    if source_scope == InvocationSourceScope::ProxyOnly {
        db_key_query
            .push(" AND live.source = ")
            .push_bind(SOURCE_PROXY);
    }
    let db_rows = db_key_query
        .build_query_as::<RuntimeKeyRow>()
        .fetch_all(pool)
        .await?;
    let mut counts = HashMap::<Option<i64>, UpstreamAccountInProgressSummary>::new();
    for row in &db_rows {
        counts
            .entry(row.upstream_account_id)
            .or_default()
            .add(row.retry_count > 0, row.live_phase.as_deref());
    }
    let db_runtime_keys = db_rows
        .into_iter()
        .map(|row| {
            (
                (row.invoke_id, row.occurred_at),
                (row.upstream_account_id, row.retry_count > 0, row.live_phase),
            )
        })
        .collect::<HashMap<_, _>>();
    let runtime_snapshot = proxy_runtime_invocations.snapshot();
    let db_terminal_keys =
        query_terminal_db_keys_for_runtime_records(pool, &runtime_snapshot, None).await?;
    let mut runtime_overlay_row_count = 0_i64;
    for record in runtime_snapshot {
        if db_terminal_keys.contains(&(record.invoke_id.clone(), record.occurred_at.clone())) {
            continue;
        }
        if source_scope == InvocationSourceScope::ProxyOnly && record.source != SOURCE_PROXY {
            continue;
        }
        if !matches!(
            normalized_runtime_text(record.status.as_deref()).as_str(),
            "running" | "pending"
        ) {
            continue;
        }
        let key = (record.invoke_id.clone(), record.occurred_at.clone());
        let runtime_phase = record
            .live_phase
            .as_deref()
            .or_else(|| runtime_invocation_live_phase(&record));
        if let Some((db_upstream_account_id, db_is_retry, db_phase)) = db_runtime_keys.get(&key) {
            let runtime_is_retry = runtime_record_is_retry(&record);
            let upstream_account_id = record.upstream_account_id.or(*db_upstream_account_id);
            if *db_upstream_account_id != upstream_account_id {
                if let Some(entry) = counts.get_mut(db_upstream_account_id) {
                    entry.subtract(*db_is_retry, db_phase.as_deref());
                }
                counts
                    .entry(upstream_account_id)
                    .or_default()
                    .add(runtime_is_retry, runtime_phase);
                runtime_overlay_row_count += 1;
            } else {
                let entry = counts.entry(upstream_account_id).or_default();
                if runtime_is_retry && !*db_is_retry {
                    entry.retry_count += 1;
                }
                if runtime_phase != db_phase.as_deref() {
                    entry.phase_counts.decrement_phase_name(db_phase.as_deref());
                    entry.phase_counts.increment_phase_name(runtime_phase);
                    runtime_overlay_row_count += 1;
                }
            }
            continue;
        }
        counts
            .entry(record.upstream_account_id)
            .or_default()
            .add(runtime_record_is_retry(&record), runtime_phase);
        runtime_overlay_row_count += 1;
    }
    if runtime_overlay_row_count > 0 {
        debug!(
            endpoint = "/api/upstream-account-activity",
            runtime_overlay_row_count, "overlayed memory runtime account in-progress counts"
        );
    }
    Ok(counts)
}

pub(crate) async fn query_dashboard_activity_live_snapshot(
    state: &AppState,
    revision: u64,
) -> Result<DashboardActivityLiveSnapshot, ApiError> {
    query_dashboard_activity_live_snapshot_from_runtime(
        &state.pool,
        state.proxy_runtime_invocations.as_ref(),
        revision,
    )
    .await
}

pub(crate) async fn query_dashboard_activity_live_snapshot_from_runtime(
    pool: &Pool<Sqlite>,
    proxy_runtime_invocations: &ProxyRuntimeInvocationStore,
    revision: u64,
) -> Result<DashboardActivityLiveSnapshot, ApiError> {
    let counts = query_upstream_account_in_progress_counts_from_runtime(
        pool,
        proxy_runtime_invocations,
        InvocationSourceScope::All,
    )
    .await?;
    let mut accounts = counts
        .into_iter()
        .map(
            |(upstream_account_id, summary)| DashboardActivityLiveAccount {
                account_key: upstream_account_id
                    .map(|id| format!("upstream:{id}"))
                    .unwrap_or_else(|| "unassigned".to_string()),
                upstream_account_id,
                in_progress_invocation_count: summary.in_progress_count,
                in_progress_phase_counts: summary.phase_counts,
                retry_invocation_count: summary.retry_count,
            },
        )
        .collect::<Vec<_>>();
    accounts.sort_by(|left, right| left.account_key.cmp(&right.account_key));

    let mut in_progress_phase_counts = InvocationPhaseCountsResponse::default();
    let mut in_progress_invocation_count = 0;
    let mut retry_invocation_count = 0;
    for account in &accounts {
        in_progress_invocation_count += account.in_progress_invocation_count;
        retry_invocation_count += account.retry_invocation_count;
        in_progress_phase_counts.queued += account.in_progress_phase_counts.queued;
        in_progress_phase_counts.requesting += account.in_progress_phase_counts.requesting;
        in_progress_phase_counts.responding += account.in_progress_phase_counts.responding;
    }

    Ok(DashboardActivityLiveSnapshot {
        revision,
        generated_at: format_utc_iso(Utc::now()),
        in_progress_invocation_count,
        in_progress_phase_counts,
        retry_invocation_count,
        accounts,
    })
}

fn dashboard_live_snapshot_in_progress_counts(
    snapshot: &DashboardActivityLiveSnapshot,
) -> HashMap<Option<i64>, UpstreamAccountInProgressSummary> {
    snapshot
        .accounts
        .iter()
        .map(|account| {
            (
                account.upstream_account_id,
                UpstreamAccountInProgressSummary {
                    in_progress_count: account.in_progress_invocation_count,
                    retry_count: account.retry_invocation_count,
                    phase_counts: account.in_progress_phase_counts,
                },
            )
        })
        .collect()
}

pub(crate) const DASHBOARD_ACTIVITY_RATE_WINDOW_MINUTES: i64 = 5;

#[derive(Debug)]
pub(crate) struct DashboardActivitySnapshot {
    range: String,
    range_start: DateTime<Utc>,
    range_end: DateTime<Utc>,
    accounts: Vec<DashboardActivityAccountResponse>,
    summary: DashboardActivitySummaryResponse,
}

#[cfg(test)]
impl DashboardActivitySnapshot {
    pub(crate) fn exact_range(&self) -> ExactUtcRange {
        ExactUtcRange {
            start: self.range_start,
            end: self.range_end,
        }
    }

    pub(crate) fn accounts(&self) -> &[DashboardActivityAccountResponse] {
        &self.accounts
    }

    pub(crate) fn summary(&self) -> &DashboardActivitySummaryResponse {
        &self.summary
    }
}

#[derive(Debug, FromRow)]
pub(crate) struct DashboardActivityRateUsageRow {
    invoke_id: String,
    occurred_at: String,
    upstream_account_id: Option<i64>,
    total_tokens: i64,
    total_cost: f64,
}

pub(crate) fn validate_dashboard_activity_params(
    endpoint: &str,
    range: &str,
    recent_limit: Option<i64>,
) -> Result<usize, ApiError> {
    if !matches!(range, "today" | "yesterday" | "1d" | "7d") {
        return Err(ApiError::bad_request(anyhow!(
            "unsupported {endpoint} range: {range}",
        )));
    }

    match recent_limit {
        Some(value) if !(1..=16).contains(&value) => Err(ApiError::bad_request(anyhow!(
            "recentLimit must be between 1 and 16"
        ))),
        Some(value) => Ok(value as usize),
        None => Ok(4),
    }
}

pub(crate) fn sum_optional_rates(
    accounts: &[DashboardActivityAccountResponse],
    value_of: impl Fn(&DashboardActivityAccountResponse) -> Option<f64>,
) -> Option<f64> {
    let mut saw_value = false;
    let mut total = 0.0;
    for account in accounts {
        if let Some(value) = value_of(account).filter(|value| value.is_finite()) {
            saw_value = true;
            total += value;
        }
    }
    saw_value.then_some(total)
}

fn compute_dashboard_range_rate(value: f64, range: ExactUtcRange) -> Option<f64> {
    let range_minutes = (range.end - range.start).num_milliseconds() as f64 / 60_000.0;
    (range_minutes > 0.0).then_some(value.max(0.0) / range_minutes)
}

pub(crate) fn build_dashboard_activity_summary(
    accounts: &[DashboardActivityAccountResponse],
    include_live_counts: bool,
    model_performance: ModelPerformanceResponse,
) -> DashboardActivitySummaryResponse {
    let ttfb_sum = accounts
        .iter()
        .map(|account| account.in_progress_wait_sum_ms)
        .sum::<f64>();
    let ttfb_count = accounts
        .iter()
        .map(|account| account.in_progress_wait_sample_count)
        .sum::<i64>();

    let mut usage_breakdown = UsageBreakdownAccumulator::default();
    for account in accounts {
        usage_breakdown.merge_response(&account.usage_breakdown);
    }
    let stats = StatsResponse {
        total_count: accounts.iter().map(|account| account.request_count).sum(),
        success_count: accounts.iter().map(|account| account.success_count).sum(),
        failure_count: accounts.iter().map(|account| account.failure_count).sum(),
        total_cost: accounts.iter().map(|account| account.total_cost).sum(),
        total_tokens: accounts.iter().map(|account| account.total_tokens).sum(),
        usage_breakdown: Some(usage_breakdown.into_response()),
        in_progress_conversation_count: include_live_counts.then(|| {
            accounts
                .iter()
                .map(|account| account.in_progress_invocation_count.unwrap_or(0))
                .sum()
        }),
        in_progress_retry_conversation_count: include_live_counts.then(|| {
            accounts
                .iter()
                .map(|account| account.retry_invocation_count.unwrap_or(0))
                .sum()
        }),
        in_progress_avg_wait_ms: (ttfb_count > 0).then_some(ttfb_sum / ttfb_count as f64),
        in_progress_phase_counts: include_live_counts.then(|| {
            accounts.iter().fold(
                InvocationPhaseCountsResponse::default(),
                |mut total, account| {
                    if let Some(counts) = account.in_progress_phase_counts {
                        total.queued += counts.queued;
                        total.requesting += counts.requesting;
                        total.responding += counts.responding;
                    }
                    total
                },
            )
        }),
        non_success_cost: Some(
            accounts
                .iter()
                .map(|account| account.non_success_cost)
                .sum(),
        ),
        non_success_tokens: Some(
            accounts
                .iter()
                .map(|account| account.non_success_tokens)
                .sum(),
        ),
        maintenance: None,
    };

    DashboardActivitySummaryResponse {
        stats,
        tokens_per_minute: model_performance
            .available
            .then_some(model_performance.total.tokens_per_minute),
        spend_rate: sum_optional_rates(accounts, |account| account.spend_rate),
        model_performance,
    }
}

pub(crate) async fn query_dashboard_activity_rate_usage_rows(
    pool: &Pool<Sqlite>,
    source_scope: InvocationSourceScope,
    range: ExactUtcRange,
) -> Result<Vec<DashboardActivityRateUsageRow>, ApiError> {
    let rate_window_start = range
        .start
        .max(range.end - ChronoDuration::minutes(DASHBOARD_ACTIVITY_RATE_WINDOW_MINUTES));
    let upstream_account_id_sql =
        invocation_upstream_account_id_with_attempt_fallback_sql("codex_invocations");
    let mut query = QueryBuilder::<Sqlite>::new("SELECT invoke_id, occurred_at, ");
    query
        .push(upstream_account_id_sql.as_str())
        .push(
            " AS upstream_account_id, \
             COALESCE(total_tokens, 0) AS total_tokens, \
             COALESCE(cost, 0.0) AS total_cost \
             FROM codex_invocations \
             WHERE occurred_at >= ",
        )
        .push_bind(db_occurred_at_lower_bound(rate_window_start))
        .push(" AND occurred_at < ")
        .push_bind(db_occurred_at_upper_bound(range.end));
    if source_scope == InvocationSourceScope::ProxyOnly {
        query.push(" AND source = ").push_bind(SOURCE_PROXY);
    }

    Ok(query
        .build_query_as::<DashboardActivityRateUsageRow>()
        .fetch_all(pool)
        .await?)
}

pub(crate) async fn load_dashboard_activity_rate_events_by_account(
    state: &AppState,
    source_scope: InvocationSourceScope,
    range: ExactUtcRange,
) -> Result<HashMap<Option<i64>, Vec<UpstreamAccountRateUsageEvent>>, ApiError> {
    let rows = query_dashboard_activity_rate_usage_rows(&state.pool, source_scope, range).await?;
    let db_account_by_key = rows
        .iter()
        .map(|row| {
            (
                (row.invoke_id.clone(), row.occurred_at.clone()),
                row.upstream_account_id,
            )
        })
        .collect::<HashMap<_, _>>();
    let runtime_snapshot = state.proxy_runtime_invocations.snapshot();
    let terminal_keys =
        query_terminal_db_keys_for_runtime_records(&state.pool, &runtime_snapshot, None).await?;
    let rate_window_start = range
        .start
        .max(range.end - ChronoDuration::minutes(DASHBOARD_ACTIVITY_RATE_WINDOW_MINUTES));
    let mut runtime_events_by_key =
        HashMap::<(String, String), (Option<i64>, UpstreamAccountRateUsageEvent)>::new();
    for record in runtime_snapshot {
        if source_scope == InvocationSourceScope::ProxyOnly && record.source != SOURCE_PROXY {
            continue;
        }
        if !matches!(
            normalized_runtime_text(record.status.as_deref()).as_str(),
            "running" | "pending"
        ) {
            continue;
        }
        let key = (record.invoke_id.clone(), record.occurred_at.clone());
        if terminal_keys.contains(&key) {
            continue;
        }
        let Some(occurred_at) = parse_to_utc_datetime(&record.occurred_at) else {
            continue;
        };
        if occurred_at < rate_window_start || occurred_at >= range.end {
            continue;
        }
        let upstream_account_id = record
            .upstream_account_id
            .or_else(|| db_account_by_key.get(&key).copied().flatten());
        runtime_events_by_key.insert(
            key,
            (
                upstream_account_id,
                UpstreamAccountRateUsageEvent {
                    occurred_at_epoch_ms: occurred_at.timestamp_millis(),
                    total_tokens: record.total_tokens.unwrap_or_default().max(0),
                    total_cost: record.cost.unwrap_or_default().max(0.0),
                },
            ),
        );
    }

    let mut events_by_account = HashMap::<Option<i64>, Vec<UpstreamAccountRateUsageEvent>>::new();
    for row in rows {
        let Some(occurred_at) = parse_to_utc_datetime(&row.occurred_at) else {
            continue;
        };
        if runtime_events_by_key.contains_key(&(row.invoke_id.clone(), row.occurred_at.clone())) {
            continue;
        }
        events_by_account
            .entry(row.upstream_account_id)
            .or_default()
            .push(UpstreamAccountRateUsageEvent {
                occurred_at_epoch_ms: occurred_at.timestamp_millis(),
                total_tokens: row.total_tokens.max(0),
                total_cost: row.total_cost.max(0.0),
            });
    }

    for (_key, (upstream_account_id, event)) in runtime_events_by_key {
        events_by_account
            .entry(upstream_account_id)
            .or_default()
            .push(event);
    }

    Ok(events_by_account)
}

pub(crate) fn sum_dashboard_activity_rate_events(
    events_by_account: &HashMap<Option<i64>, Vec<UpstreamAccountRateUsageEvent>>,
    range: ExactUtcRange,
) -> (Option<f64>, Option<f64>) {
    let mut saw_tokens_per_minute = false;
    let mut saw_spend_rate = false;
    let mut tokens_per_minute_total = 0.0;
    let mut spend_rate_total = 0.0;

    for events in events_by_account.values() {
        let (tokens_per_minute, spend_rate) =
            compute_upstream_account_activity_rates(events, range.start, range.end);
        if let Some(value) = tokens_per_minute.filter(|value| value.is_finite()) {
            saw_tokens_per_minute = true;
            tokens_per_minute_total += value;
        }
        if let Some(value) = spend_rate.filter(|value| value.is_finite()) {
            saw_spend_rate = true;
            spend_rate_total += value;
        }
    }

    (
        saw_tokens_per_minute.then_some(tokens_per_minute_total),
        saw_spend_rate.then_some(spend_rate_total),
    )
}

pub(crate) async fn load_dashboard_activity_summary_only_snapshot(
    state: &AppState,
    range_name: &str,
    source_scope: InvocationSourceScope,
    range: ExactUtcRange,
) -> Result<DashboardActivitySnapshot, ApiError> {
    let totals =
        query_hourly_backed_summary_range(state, range.start, range.end, source_scope).await?;
    let mut stats = totals.into_response();
    stats.non_success_cost = Some(totals.non_success_cost);
    let augmentation = load_summary_live_augmentation(
        state,
        source_scope,
        None,
        Some((range.start, range.end)),
        SummaryLiveAugmentationPolicy {
            include_in_progress: range_name != "yesterday",
            include_non_success_tokens: true,
        },
    )
    .await?;
    apply_summary_live_augmentation(&mut stats, augmentation);
    let spend_rate = compute_dashboard_range_rate(stats.total_cost, range);

    Ok(DashboardActivitySnapshot {
        range: range_name.to_string(),
        range_start: range.start,
        range_end: range.end,
        accounts: Vec::new(),
        summary: DashboardActivitySummaryResponse {
            stats,
            // Archived rollups cannot distinguish every successful, billed invocation
            // required by the dashboard TPM contract.
            tokens_per_minute: None,
            spend_rate,
            model_performance: ModelPerformanceAccumulator::unavailable(range),
        },
    })
}

pub(crate) async fn load_dashboard_activity_snapshot(
    state: &AppState,
    range_name: &str,
    reporting_tz: Tz,
    recent_limit: usize,
    include_accounts: bool,
    include_recent: bool,
    in_progress_counts_override: Option<HashMap<Option<i64>, UpstreamAccountInProgressSummary>>,
) -> Result<DashboardActivitySnapshot, ApiError> {
    let range_window = resolve_range_window(range_name, reporting_tz).map_err(ApiError::from)?;
    let source_scope = resolve_default_source_scope(&state.pool).await?;
    let range = ExactUtcRange {
        start: range_window.start,
        end: range_window.end,
    };
    let retention_cutoff = shanghai_retention_cutoff(state.config.invocation_max_days);
    let model_performance_available = range.start >= retention_cutoff;
    if !include_accounts && range.start < retention_cutoff {
        return load_dashboard_activity_summary_only_snapshot(
            state,
            range_name,
            source_scope,
            range,
        )
        .await;
    }
    let retention_cutoff = shanghai_retention_cutoff(state.config.invocation_max_days);
    let mut account_activity = HashMap::<Option<i64>, UpstreamAccountActivityAccumulator>::new();
    let mut model_performance = ModelPerformanceAccumulator::default();
    for row in
        query_live_upstream_account_activity_aggregate_rows(&state.pool, source_scope, range, true)
            .await?
    {
        let entry = account_activity.entry(row.upstream_account_id).or_default();
        merge_latest_optional_timestamp(
            &mut entry.latest_conversation_created_at,
            row.latest_conversation_created_at,
        );
        merge_latest_optional_timestamp(&mut entry.last_invocation_at, row.last_invocation_at);
        entry.request_count += row.request_count;
        entry.success_count += row.success_count;
        entry.failure_count += row.failure_count;
        entry.non_success_count += row.non_success_count;
        entry.total_tokens += row.total_tokens;
        entry.success_tokens += row.success_tokens;
        entry.non_success_tokens += row.non_success_tokens;
        entry.failure_tokens += row.failure_tokens;
        entry.failure_cost += row.failure_cost;
        entry.non_success_cost += row.non_success_cost;
        entry.cache_input_tokens += row.cache_input_tokens;
        entry.total_cost += row.total_cost;
        entry.first_response_byte_total_sample_count += row.first_response_byte_total_sample_count;
        entry.first_response_byte_total_sum_ms += row.first_response_byte_total_sum_ms;
        entry.total_latency_sample_count += row.total_latency_sample_count;
        entry.total_latency_sum_ms += row.total_latency_sum_ms;
    }
    for row in query_live_upstream_account_usage_breakdown_rows(
        &state.pool,
        source_scope,
        range,
        true,
        true,
    )
    .await?
    {
        let entry = account_activity.entry(row.upstream_account_id).or_default();
        entry.usage_breakdown.add_aggregate_row(&row);
        entry.model_performance.add_aggregate_row(&row);
        model_performance.add_aggregate_row(&row);
    }
    if range.start < retention_cutoff {
        let (archived_aggregates, archived_usage_breakdowns) =
            query_completed_invocation_archive_activity_aggregate_rows(
                &state.pool,
                source_scope,
                range,
            )
            .await?;
        for row in archived_aggregates {
            let entry = account_activity.entry(row.upstream_account_id).or_default();
            merge_latest_optional_timestamp(
                &mut entry.latest_conversation_created_at,
                row.latest_conversation_created_at,
            );
            merge_latest_optional_timestamp(&mut entry.last_invocation_at, row.last_invocation_at);
            entry.request_count += row.request_count;
            entry.success_count += row.success_count;
            entry.failure_count += row.failure_count;
            entry.non_success_count += row.non_success_count;
            entry.total_tokens += row.total_tokens;
            entry.success_tokens += row.success_tokens;
            entry.non_success_tokens += row.non_success_tokens;
            entry.failure_tokens += row.failure_tokens;
            entry.failure_cost += row.failure_cost;
            entry.non_success_cost += row.non_success_cost;
            entry.cache_input_tokens += row.cache_input_tokens;
            entry.total_cost += row.total_cost;
            entry.first_response_byte_total_sample_count +=
                row.first_response_byte_total_sample_count;
            entry.first_response_byte_total_sum_ms += row.first_response_byte_total_sum_ms;
            entry.total_latency_sample_count += row.total_latency_sample_count;
            entry.total_latency_sum_ms += row.total_latency_sum_ms;
        }
        for row in archived_usage_breakdowns {
            account_activity
                .entry(row.upstream_account_id)
                .or_default()
                .usage_breakdown
                .add_aggregate_row(&row);
        }
    }
    for row in
        query_live_upstream_account_activity_rate_rows(&state.pool, source_scope, range).await?
    {
        if let Some(occurred_at) = parse_to_utc_datetime(&row.occurred_at) {
            account_activity
                .entry(row.upstream_account_id)
                .or_default()
                .rate_usage_events
                .push(UpstreamAccountRateUsageEvent {
                    occurred_at_epoch_ms: occurred_at.timestamp_millis(),
                    total_tokens: row.total_tokens.max(0),
                    total_cost: row.total_cost.max(0.0),
                });
        }
    }

    let mut runtime_rows = query_live_upstream_account_activity_preview_rows_with_limit(
        &state.pool,
        source_scope,
        range,
        None,
        None,
        true,
    )
    .await?;
    overlay_runtime_upstream_account_activity_preview_rows(
        state,
        &mut runtime_rows,
        source_scope,
        range,
    );
    let mut runtime_recent_rows =
        HashMap::<Option<i64>, Vec<UpstreamAccountInvocationPreviewRow>>::new();
    if include_accounts {
        let mut runtime_recent_by_key =
            HashMap::<(String, String), UpstreamAccountInvocationPreviewRow>::new();
        for row in &runtime_rows {
            runtime_recent_by_key.insert(
                (row.invoke_id.clone(), row.occurred_at.clone()),
                row.clone(),
            );
        }
        let mut terminal_rows = Vec::new();
        for record in state.proxy_runtime_invocations.snapshot() {
            if matches!(
                normalized_runtime_text(record.status.as_deref()).as_str(),
                "running" | "pending"
            ) {
                continue;
            }
            let Some(row) = runtime_upstream_account_activity_preview_row_with_terminal(
                record,
                source_scope,
                true,
            ) else {
                continue;
            };
            let Some(occurred_at) = parse_to_utc_datetime(&row.occurred_at) else {
                continue;
            };
            if occurred_at < range.start || occurred_at >= range.end {
                continue;
            }
            terminal_rows.push(((row.invoke_id.clone(), row.occurred_at.clone()), row));
        }
        let fallback_keys = terminal_rows
            .iter()
            .filter(|(_, row)| row.upstream_account_id.is_none())
            .map(|(key, _)| key.clone())
            .collect::<HashSet<_>>();
        let fallback_rows =
            query_runtime_recent_account_fallback_rows(&state.pool, source_scope, &fallback_keys)
                .await?
                .into_iter()
                .map(|row| ((row.invoke_id.clone(), row.occurred_at.clone()), row))
                .collect::<HashMap<_, _>>();
        for (key, mut row) in terminal_rows {
            if let Some(existing) = runtime_recent_by_key.get(&key) {
                if row.upstream_account_id.is_none() {
                    row.upstream_account_id = existing.upstream_account_id;
                }
                if row.upstream_account_name.is_none() {
                    row.upstream_account_name = existing.upstream_account_name.clone();
                }
                if row.upstream_account_plan_type.is_none() {
                    row.upstream_account_plan_type = existing.upstream_account_plan_type.clone();
                }
            }
            if let Some(fallback) = fallback_rows.get(&key) {
                if row.upstream_account_id.is_none() {
                    row.upstream_account_id = fallback.upstream_account_id;
                }
                if row.upstream_account_name.is_none() {
                    row.upstream_account_name = fallback.upstream_account_name.clone();
                }
                if row.upstream_account_plan_type.is_none() {
                    row.upstream_account_plan_type = fallback.upstream_account_plan_type.clone();
                }
            }
            runtime_recent_by_key.insert(key, row);
        }
        for row in runtime_recent_by_key.into_values() {
            runtime_recent_rows
                .entry(row.upstream_account_id)
                .or_default()
                .push(row);
        }
        for rows in runtime_recent_rows.values_mut() {
            rows.sort_by(|left, right| {
                right
                    .occurred_at
                    .cmp(&left.occurred_at)
                    .then_with(|| right.id.cmp(&left.id))
            });
        }
        for (account_id, rows) in &runtime_recent_rows {
            let entry = account_activity.entry(*account_id).or_default();
            if let Some(row) = rows.first() {
                entry.last_occurred_at_epoch_ms = parse_to_utc_datetime(&row.occurred_at).map_or(
                    entry.last_occurred_at_epoch_ms,
                    |occurred_at| {
                        entry
                            .last_occurred_at_epoch_ms
                            .max(occurred_at.timestamp_millis())
                    },
                );
                if entry.display_name_hint.is_none() {
                    entry.display_name_hint =
                        normalize_trimmed_optional_string_local(row.upstream_account_name.clone());
                }
                if entry.plan_type_hint.is_none() {
                    entry.plan_type_hint = normalize_trimmed_optional_string_local(
                        row.upstream_account_plan_type.clone(),
                    );
                }
            }
        }
    }
    let live_ids = runtime_rows
        .iter()
        .map(|row| row.id)
        .collect::<HashSet<_>>();
    for row in runtime_rows {
        add_upstream_account_activity_preview_row(
            &mut account_activity,
            row,
            recent_limit,
            include_accounts && include_recent,
        );
    }

    if include_recent && range.start < retention_cutoff {
        let mut archived_rows = crate::stats::query_completed_invocation_archive_preview_rows(
            &state.pool,
            source_scope,
            range,
            Some(&live_ids),
        )
        .await?;
        archived_rows.sort_by(|left, right| {
            right
                .occurred_at
                .cmp(&left.occurred_at)
                .then_with(|| right.id.cmp(&left.id))
        });
        for row in archived_rows {
            let Some(occurred_at) = parse_to_utc_datetime(&row.occurred_at) else {
                continue;
            };
            let entry = account_activity.entry(row.upstream_account_id).or_default();
            entry.last_occurred_at_epoch_ms = entry
                .last_occurred_at_epoch_ms
                .max(occurred_at.timestamp_millis());
            if entry.display_name_hint.is_none() {
                entry.display_name_hint =
                    normalize_trimmed_optional_string_local(row.upstream_account_name.clone());
            }
            if entry.plan_type_hint.is_none() {
                entry.plan_type_hint =
                    normalize_trimmed_optional_string_local(row.upstream_account_plan_type.clone());
            }
            if entry.recent_invocations.len() < recent_limit {
                entry
                    .recent_invocations
                    .push(upstream_account_invocation_preview_from_row(row));
            }
        }
    }

    if include_accounts && include_recent {
        let rows = query_live_upstream_account_activity_preview_rows_with_limit(
            &state.pool,
            source_scope,
            range,
            None,
            None,
            false,
        )
        .await?;
        let mut recent_rows_by_account =
            HashMap::<Option<i64>, Vec<PromptCacheConversationInvocationPreviewResponse>>::new();
        for (upstream_account_id, runtime_rows) in runtime_recent_rows {
            recent_rows_by_account
                .entry(upstream_account_id)
                .or_default()
                .extend(
                    runtime_rows
                        .into_iter()
                        .map(upstream_account_invocation_preview_from_row),
                );
        }
        for row in rows {
            recent_rows_by_account
                .entry(row.upstream_account_id)
                .or_default()
                .push(upstream_account_invocation_preview_from_row(row));
        }
        for (upstream_account_id, mut recent_rows) in recent_rows_by_account {
            let entry = account_activity.entry(upstream_account_id).or_default();
            recent_rows.append(&mut entry.recent_invocations);
            let mut seen_keys = HashSet::with_capacity(recent_rows.len());
            recent_rows.retain(|invocation| {
                seen_keys.insert((invocation.invoke_id.clone(), invocation.occurred_at.clone()))
            });
            recent_rows.sort_by(|left, right| {
                right
                    .occurred_at
                    .cmp(&left.occurred_at)
                    .then_with(|| right.id.cmp(&left.id))
            });
            recent_rows.truncate(recent_limit);
            entry.recent_invocations = recent_rows;
        }
    }

    let in_progress_counts = if range_name == "yesterday" {
        HashMap::new()
    } else {
        match in_progress_counts_override {
            Some(counts) => counts,
            None => query_upstream_account_in_progress_counts(state, source_scope).await?,
        }
    };
    for upstream_account_id in in_progress_counts.keys() {
        account_activity.entry(*upstream_account_id).or_default();
    }

    let account_ids = account_activity
        .keys()
        .filter_map(|id| *id)
        .collect::<Vec<_>>();
    let account_meta = if include_accounts {
        query_upstream_account_activity_meta(&state.pool, &account_ids).await?
    } else {
        HashMap::new()
    };
    let effective_routing_rules = if include_accounts {
        crate::upstream_accounts::load_effective_routing_rules_for_accounts(
            &state.pool,
            &account_ids,
        )
        .await?
    } else {
        HashMap::new()
    };
    let mut accounts = account_activity
        .into_iter()
        .map(|(upstream_account_id, aggregate)| {
            let meta = upstream_account_id.and_then(|id| account_meta.get(&id));
            let status_fields =
                meta.map(|row| build_upstream_account_activity_status_fields(row, Utc::now()));
            let model_performance = aggregate
                .model_performance
                .into_response(range, model_performance_available);
            let tokens_per_minute = model_performance
                .available
                .then_some(model_performance.total.tokens_per_minute);
            let spend_rate = compute_dashboard_range_rate(aggregate.total_cost, range);
            let (in_progress_invocation_count, in_progress_phase_counts, retry_invocation_count) =
                if range_name == "yesterday" {
                    (None, None, None)
                } else {
                    let summary = in_progress_counts
                        .get(&upstream_account_id)
                        .copied()
                        .unwrap_or_default();
                    (
                        Some(summary.in_progress_count),
                        Some(summary.phase_counts),
                        Some(summary.retry_count),
                    )
                };
            let account_key = upstream_account_id
                .map(|id| format!("upstream:{id}"))
                .unwrap_or_else(|| "unassigned".to_string());
            let is_unassigned = upstream_account_id.is_none();

            DashboardActivityAccountResponse {
                account_key,
                upstream_account_id,
                display_name: upstream_account_id
                    .map(|id| {
                        resolve_upstream_account_activity_display_name(
                            id,
                            meta,
                            aggregate.display_name_hint.as_deref(),
                        )
                    })
                    .unwrap_or_else(|| "未分配上游账号".to_string()),
                is_unassigned,
                latest_conversation_created_at: aggregate.latest_conversation_created_at,
                last_invocation_at: aggregate.last_invocation_at,
                group_name: normalize_trimmed_optional_string_local(
                    meta.and_then(|row| row.group_name.clone()),
                ),
                plan_type: normalize_trimmed_optional_string_local(
                    meta.and_then(|row| row.plan_type.clone())
                        .or(aggregate.plan_type_hint),
                ),
                enabled: status_fields.as_ref().map(|fields| fields.enabled),
                display_status: status_fields
                    .as_ref()
                    .map(|fields| fields.display_status.clone()),
                enable_status: status_fields
                    .as_ref()
                    .map(|fields| fields.enable_status.clone()),
                work_status: status_fields
                    .as_ref()
                    .map(|fields| fields.work_status.clone()),
                health_status: status_fields
                    .as_ref()
                    .map(|fields| fields.health_status.clone()),
                sync_state: status_fields
                    .as_ref()
                    .map(|fields| fields.sync_state.clone()),
                last_error: status_fields
                    .as_ref()
                    .and_then(|fields| fields.last_error.clone()),
                last_action_reason_message: status_fields
                    .as_ref()
                    .and_then(|fields| fields.last_action_reason_message.clone()),
                request_count: aggregate.request_count,
                success_count: aggregate.success_count,
                failure_count: aggregate.failure_count,
                non_success_count: aggregate.non_success_count,
                total_tokens: aggregate.total_tokens,
                success_tokens: aggregate.success_tokens,
                non_success_tokens: aggregate.non_success_tokens,
                failure_tokens: aggregate.failure_tokens,
                failure_cost: aggregate.failure_cost,
                non_success_cost: aggregate.non_success_cost,
                total_cost: aggregate.total_cost,
                usage_breakdown: aggregate.usage_breakdown.clone().into_response(),
                model_performance,
                cache_hit_rate: (aggregate.total_tokens > 0)
                    .then_some(aggregate.cache_input_tokens as f64 / aggregate.total_tokens as f64),
                tokens_per_minute,
                spend_rate,
                first_byte_avg_ms: (aggregate.first_response_byte_total_sample_count > 0)
                    .then_some(
                        aggregate.first_response_byte_total_sum_ms
                            / aggregate.first_response_byte_total_sample_count as f64,
                    ),
                first_response_byte_total_avg_ms: (aggregate
                    .first_response_byte_total_sample_count
                    > 0)
                .then_some(
                    aggregate.first_response_byte_total_sum_ms
                        / aggregate.first_response_byte_total_sample_count as f64,
                ),
                avg_total_ms: (aggregate.total_latency_sample_count > 0).then_some(
                    aggregate.total_latency_sum_ms / aggregate.total_latency_sample_count as f64,
                ),
                in_progress_invocation_count,
                in_progress_phase_counts,
                retry_invocation_count,
                in_progress_wait_sum_ms: aggregate.in_progress_wait_sum_ms,
                in_progress_wait_sample_count: aggregate.in_progress_wait_sample_count,
                effective_routing_rule: upstream_account_id.map(|id| {
                    effective_routing_rules
                        .get(&id)
                        .cloned()
                        .unwrap_or_else(crate::upstream_accounts::default_effective_routing_rule)
                }),
                recent_invocations: aggregate.recent_invocations,
            }
        })
        .collect::<Vec<_>>();

    accounts.sort_by(|left, right| {
        right
            .total_tokens
            .cmp(&left.total_tokens)
            .then_with(|| {
                right
                    .recent_invocations
                    .first()
                    .map(|row| row.occurred_at.as_str())
                    .cmp(
                        &left
                            .recent_invocations
                            .first()
                            .map(|row| row.occurred_at.as_str()),
                    )
            })
            .then_with(|| {
                right
                    .upstream_account_id
                    .unwrap_or(i64::MIN)
                    .cmp(&left.upstream_account_id.unwrap_or(i64::MIN))
            })
    });

    let summary = build_dashboard_activity_summary(
        &accounts,
        range_name != "yesterday",
        model_performance.into_response(range, model_performance_available),
    );
    Ok(DashboardActivitySnapshot {
        range: range_name.to_string(),
        range_start: range.start,
        range_end: range.end,
        accounts,
        summary,
    })
}

pub(crate) fn dashboard_account_to_upstream_account(
    account: DashboardActivityAccountResponse,
) -> Option<UpstreamAccountActivityAccountResponse> {
    let upstream_account_id = account.upstream_account_id?;
    Some(UpstreamAccountActivityAccountResponse {
        upstream_account_id,
        display_name: account.display_name,
        latest_conversation_created_at: account.latest_conversation_created_at,
        last_invocation_at: account.last_invocation_at,
        group_name: account.group_name,
        plan_type: account.plan_type,
        enabled: account.enabled.unwrap_or(true),
        display_status: account
            .display_status
            .unwrap_or_else(|| "active".to_string()),
        enable_status: account
            .enable_status
            .unwrap_or_else(|| "enabled".to_string()),
        work_status: account.work_status.unwrap_or_else(|| "idle".to_string()),
        health_status: account
            .health_status
            .unwrap_or_else(|| "normal".to_string()),
        sync_state: account.sync_state.unwrap_or_else(|| "idle".to_string()),
        last_error: account.last_error,
        last_action_reason_message: account.last_action_reason_message,
        request_count: account.request_count,
        success_count: account.success_count,
        failure_count: account.failure_count,
        non_success_count: account.non_success_count,
        total_tokens: account.total_tokens,
        success_tokens: account.success_tokens,
        non_success_tokens: account.non_success_tokens,
        failure_tokens: account.failure_tokens,
        failure_cost: account.failure_cost,
        total_cost: account.total_cost,
        usage_breakdown: account.usage_breakdown,
        cache_hit_rate: account.cache_hit_rate,
        tokens_per_minute: account.tokens_per_minute,
        spend_rate: account.spend_rate,
        first_byte_avg_ms: account.first_byte_avg_ms,
        first_response_byte_total_avg_ms: account.first_response_byte_total_avg_ms,
        avg_total_ms: account.avg_total_ms,
        in_progress_invocation_count: account.in_progress_invocation_count,
        in_progress_phase_counts: account.in_progress_phase_counts,
        retry_invocation_count: account.retry_invocation_count,
        effective_routing_rule: account
            .effective_routing_rule
            .unwrap_or_else(crate::upstream_accounts::default_effective_routing_rule),
        recent_invocations: account.recent_invocations,
    })
}

pub(crate) async fn fetch_dashboard_activity(
    State(state): State<Arc<AppState>>,
    Query(params): Query<DashboardActivityQuery>,
) -> Result<Json<DashboardActivityResponse>, ApiError> {
    let recent_limit = validate_dashboard_activity_params(
        "dashboard-activity",
        params.range.as_str(),
        params.recent_limit,
    )?;
    let reporting_tz = parse_reporting_tz(params.time_zone.as_deref())?;
    let live = if params.range != "yesterday" {
        Some(capture_dashboard_activity_live_snapshot(state.as_ref()).await?)
    } else {
        None
    };
    let mut snapshot = load_dashboard_activity_snapshot(
        state.as_ref(),
        params.range.as_str(),
        reporting_tz,
        recent_limit,
        params.include_accounts,
        params.include_recent.unwrap_or(true),
        live.as_ref()
            .map(dashboard_live_snapshot_in_progress_counts),
    )
    .await?;
    let live_revision = live.as_ref().map_or(0, |snapshot| snapshot.revision);
    if let Some(live) = live {
        snapshot.summary.stats.in_progress_conversation_count =
            Some(live.in_progress_invocation_count);
        snapshot.summary.stats.in_progress_retry_conversation_count =
            Some(live.retry_invocation_count);
        snapshot.summary.stats.in_progress_phase_counts = Some(live.in_progress_phase_counts);
        let live_accounts = live
            .accounts
            .into_iter()
            .map(|account| (account.account_key.clone(), account))
            .collect::<HashMap<_, _>>();
        for account in &mut snapshot.accounts {
            let live_account = live_accounts.get(&account.account_key);
            account.in_progress_invocation_count =
                Some(live_account.map_or(0, |row| row.in_progress_invocation_count));
            account.in_progress_phase_counts = Some(
                live_account
                    .map(|row| row.in_progress_phase_counts)
                    .unwrap_or_default(),
            );
            account.retry_invocation_count =
                Some(live_account.map_or(0, |row| row.retry_invocation_count));
        }
    }
    let range_start = format_utc_iso_precise(snapshot.range_start);
    let range_end = format_utc_iso_precise(snapshot.range_end);
    let accounts = params.include_accounts.then_some(snapshot.accounts);

    Ok(Json(DashboardActivityResponse {
        range: snapshot.range,
        range_start: range_start.clone(),
        range_end: range_end.clone(),
        snapshot_id: snapshot.range_end.timestamp_millis(),
        live_revision,
        rate_window: DashboardActivityRateWindowResponse {
            start: range_start.clone(),
            end: range_end,
            window_minutes: ((snapshot.range_end - snapshot.range_start).num_seconds() / 60).max(0),
            mode: "range_average".to_string(),
        },
        summary: snapshot.summary,
        accounts,
    }))
}

pub(crate) async fn fetch_dashboard_activity_recent(
    State(state): State<Arc<AppState>>,
    Query(params): Query<DashboardActivityRecentQuery>,
) -> Result<Json<DashboardActivityRecentResponse>, ApiError> {
    let recent_limit = validate_dashboard_activity_params(
        "dashboard-activity/recent",
        "today",
        params.recent_limit,
    )?;
    let range_start = parse_to_utc_datetime(&params.range_start)
        .ok_or_else(|| ApiError::bad_request(anyhow!("invalid rangeStart")))?;
    let range_end = parse_to_utc_datetime(&params.range_end)
        .ok_or_else(|| ApiError::bad_request(anyhow!("invalid rangeEnd")))?;
    if range_start >= range_end
        || range_end - range_start > ChronoDuration::days(7)
        || params.snapshot_id != range_end.timestamp_millis()
    {
        return Err(ApiError::bad_request(anyhow!(
            "snapshotId must match rangeEnd and range must be between 0 and 7 days"
        )));
    }
    let range = ExactUtcRange {
        start: range_start,
        end: range_end,
    };
    let source_scope = resolve_default_source_scope(&state.pool).await?;
    let mut rows = query_live_upstream_account_activity_preview_rows_with_limit(
        &state.pool,
        source_scope,
        range,
        None,
        None,
        false,
    )
    .await?;
    overlay_runtime_upstream_account_activity_preview_rows(
        state.as_ref(),
        &mut rows,
        source_scope,
        range,
    );
    overlay_runtime_terminal_upstream_account_activity_preview_rows(
        state.as_ref(),
        &mut rows,
        source_scope,
        range,
    )
    .await?;
    let live_ids = rows.iter().map(|row| row.id).collect::<HashSet<_>>();
    if range.start < shanghai_retention_cutoff(state.config.invocation_max_days) {
        rows.extend(
            crate::stats::query_completed_invocation_archive_preview_rows(
                &state.pool,
                source_scope,
                range,
                Some(&live_ids),
            )
            .await?,
        );
    }
    rows.sort_by(|left, right| {
        right
            .occurred_at
            .cmp(&left.occurred_at)
            .then_with(|| right.id.cmp(&left.id))
    });
    let mut grouped =
        HashMap::<Option<i64>, Vec<PromptCacheConversationInvocationPreviewResponse>>::new();
    for row in rows {
        let invocations = grouped.entry(row.upstream_account_id).or_default();
        if invocations.len() < recent_limit {
            invocations.push(upstream_account_invocation_preview_from_row(row));
        }
    }
    let mut accounts = grouped
        .into_iter()
        .map(
            |(account_id, recent_invocations)| DashboardActivityRecentAccountResponse {
                account_key: account_id
                    .map(|id| format!("upstream:{id}"))
                    .unwrap_or_else(|| "unassigned".to_string()),
                recent_invocations,
            },
        )
        .collect::<Vec<_>>();
    accounts.sort_by(|left, right| left.account_key.cmp(&right.account_key));

    Ok(Json(DashboardActivityRecentResponse {
        range_start: format_utc_iso_precise(range.start),
        range_end: format_utc_iso_precise(range.end),
        snapshot_id: params.snapshot_id,
        accounts,
    }))
}

pub(crate) async fn fetch_upstream_account_activity(
    State(state): State<Arc<AppState>>,
    Query(params): Query<UpstreamAccountActivityQuery>,
) -> Result<Json<UpstreamAccountActivityResponse>, ApiError> {
    let recent_limit = validate_dashboard_activity_params(
        "upstream-account-activity",
        params.range.as_str(),
        params.recent_limit,
    )?;
    let reporting_tz = parse_reporting_tz(params.time_zone.as_deref())?;
    let snapshot = load_dashboard_activity_snapshot(
        state.as_ref(),
        params.range.as_str(),
        reporting_tz,
        recent_limit,
        true,
        true,
        None,
    )
    .await?;
    let accounts = snapshot
        .accounts
        .into_iter()
        .filter_map(dashboard_account_to_upstream_account)
        .collect();

    Ok(Json(UpstreamAccountActivityResponse {
        range: snapshot.range,
        range_start: format_utc_iso(snapshot.range_start),
        range_end: format_utc_iso(snapshot.range_end),
        accounts,
    }))
}

#[cfg(test)]
mod upstream_account_activity_rate_tests {
    use super::*;

    fn utc_at(epoch: i64) -> DateTime<Utc> {
        Utc.timestamp_opt(epoch, 0)
            .single()
            .expect("valid test epoch")
    }

    #[test]
    fn rates_use_recent_five_minute_active_tail() {
        let range_start = utc_at(600);
        let range_end = utc_at(1_000);
        let usage = vec![
            UpstreamAccountRateUsageEvent {
                occurred_at_epoch_ms: 600_000,
                total_tokens: 10_000,
                total_cost: 10.0,
            },
            UpstreamAccountRateUsageEvent {
                occurred_at_epoch_ms: 720_000,
                total_tokens: 0,
                total_cost: 0.0,
            },
            UpstreamAccountRateUsageEvent {
                occurred_at_epoch_ms: 780_000,
                total_tokens: 300,
                total_cost: 0.30,
            },
            UpstreamAccountRateUsageEvent {
                occurred_at_epoch_ms: 840_000,
                total_tokens: 100,
                total_cost: 0.10,
            },
        ];

        let (tokens_per_minute, spend_rate) =
            compute_upstream_account_activity_rates(&usage, range_start, range_end);

        assert!((tokens_per_minute.expect("token rate") - (400.0 / (220.0 / 60.0))).abs() < 1e-9);
        assert!((spend_rate.expect("spend rate") - (0.40 / (220.0 / 60.0))).abs() < 1e-9);
    }

    #[test]
    fn rates_return_zero_when_recent_window_has_no_usage() {
        let range_start = utc_at(600);
        let range_end = utc_at(1_000);
        let usage = vec![UpstreamAccountRateUsageEvent {
            occurred_at_epoch_ms: 600_000,
            total_tokens: 10_000,
            total_cost: 10.0,
        }];

        let (tokens_per_minute, spend_rate) =
            compute_upstream_account_activity_rates(&usage, range_start, range_end);

        assert_eq!(tokens_per_minute, Some(0.0));
        assert_eq!(spend_rate, Some(0.0));
    }

    #[test]
    fn rates_exclude_events_before_mid_minute_tail_window() {
        let range_start = utc_at(0);
        let range_end = Utc
            .with_ymd_and_hms(2026, 7, 1, 12, 5, 30)
            .single()
            .expect("valid range end");
        let usage = vec![
            UpstreamAccountRateUsageEvent {
                occurred_at_epoch_ms: Utc
                    .with_ymd_and_hms(2026, 7, 1, 12, 0, 1)
                    .single()
                    .expect("valid excluded event time")
                    .timestamp_millis(),
                total_tokens: 10_000,
                total_cost: 10.0,
            },
            UpstreamAccountRateUsageEvent {
                occurred_at_epoch_ms: Utc
                    .with_ymd_and_hms(2026, 7, 1, 12, 0, 31)
                    .single()
                    .expect("valid included event time")
                    .timestamp_millis(),
                total_tokens: 100,
                total_cost: 0.10,
            },
        ];

        let (tokens_per_minute, spend_rate) =
            compute_upstream_account_activity_rates(&usage, range_start, range_end);

        assert!((tokens_per_minute.expect("token rate") - (100.0 / 5.0)).abs() < 1e-9);
        assert!((spend_rate.expect("spend rate") - (0.10 / 5.0)).abs() < 1e-9);
    }

    #[test]
    fn rates_floor_first_active_event_to_minute_boundary() {
        let range_start = utc_at(0);
        let range_end = Utc
            .with_ymd_and_hms(2026, 7, 1, 12, 5, 0)
            .single()
            .expect("valid range end");
        let usage = vec![UpstreamAccountRateUsageEvent {
            occurred_at_epoch_ms: Utc
                .with_ymd_and_hms(2026, 7, 1, 12, 4, 59)
                .single()
                .expect("valid late-minute event time")
                .timestamp_millis(),
            total_tokens: 600,
            total_cost: 0.60,
        }];

        let (tokens_per_minute, spend_rate) =
            compute_upstream_account_activity_rates(&usage, range_start, range_end);

        assert!((tokens_per_minute.expect("token rate") - 600.0).abs() < 1e-9);
        assert!((spend_rate.expect("spend rate") - 0.60).abs() < 1e-9);
    }

    #[test]
    fn rates_use_earliest_active_event_when_events_are_newest_first() {
        let range_start = utc_at(600);
        let range_end = utc_at(1_000);
        let usage = vec![
            UpstreamAccountRateUsageEvent {
                occurred_at_epoch_ms: 900_000,
                total_tokens: 100,
                total_cost: 0.10,
            },
            UpstreamAccountRateUsageEvent {
                occurred_at_epoch_ms: 780_000,
                total_tokens: 300,
                total_cost: 0.30,
            },
        ];

        let (tokens_per_minute, spend_rate) =
            compute_upstream_account_activity_rates(&usage, range_start, range_end);

        assert!((tokens_per_minute.expect("token rate") - (400.0 / (220.0 / 60.0))).abs() < 1e-9);
        assert!((spend_rate.expect("spend rate") - (0.40 / (220.0 / 60.0))).abs() < 1e-9);
    }
}

pub(crate) async fn fetch_summary(
    State(state): State<Arc<AppState>>,
    Query(params): Query<SummaryQuery>,
) -> Result<Json<StatsResponse>, ApiError> {
    let default_limit = state.config.list_limit_max as i64;
    let window = parse_summary_window(&params, default_limit)?;
    let reporting_tz = parse_reporting_tz(params.time_zone.as_deref())?;
    let source_scope = resolve_default_source_scope(&state.pool).await?;
    let upstream_account_id = params.upstream_account_id;
    let now = Utc::now();

    let totals = match window {
        SummaryWindow::All => {
            if let Some(upstream_account_id) = upstream_account_id {
                let start = Utc.timestamp_opt(0, 0).single().ok_or_else(|| {
                    ApiError::from(anyhow!("invalid account all-time summary start"))
                })?;
                query_hourly_backed_summary_range_for_account(
                    state.as_ref(),
                    start,
                    Utc::now(),
                    source_scope,
                    upstream_account_id,
                )
                .await?
            } else {
                query_combined_totals(&state.pool, StatsFilter::All, source_scope).await?
            }
        }
        SummaryWindow::Current(limit) => {
            if let Some(upstream_account_id) = upstream_account_id {
                StatsTotals::from(
                    crate::stats::query_upstream_account_stats_row(
                        &state.pool,
                        StatsFilter::RecentLimit(limit),
                        source_scope,
                        upstream_account_id,
                    )
                    .await?,
                )
            } else {
                query_combined_totals(&state.pool, StatsFilter::RecentLimit(limit), source_scope)
                    .await?
            }
        }
        SummaryWindow::Duration(duration) => {
            let start = now - duration;
            if let Some(upstream_account_id) = upstream_account_id {
                query_hourly_backed_summary_range_for_account(
                    state.as_ref(),
                    start,
                    now,
                    source_scope,
                    upstream_account_id,
                )
                .await?
            } else {
                query_hourly_backed_summary_since(state.as_ref(), start, source_scope).await?
            }
        }
        SummaryWindow::Calendar(ref spec) => {
            let range_window =
                resolve_range_window(spec.as_str(), reporting_tz).map_err(ApiError::from)?;
            if range_window.start >= range_window.end {
                return Ok(Json(
                    build_empty_summary_response(state.as_ref(), source_scope, upstream_account_id)
                        .await?,
                ));
            }
            if let Some(upstream_account_id) = upstream_account_id {
                query_hourly_backed_summary_range_for_account(
                    state.as_ref(),
                    range_window.start,
                    range_window.end,
                    source_scope,
                    upstream_account_id,
                )
                .await?
            } else {
                query_hourly_backed_summary_range(
                    state.as_ref(),
                    range_window.start,
                    range_window.end,
                    source_scope,
                )
                .await?
            }
        }
        SummaryWindow::PreviousFullDays(day_count) => {
            let (start, end) = previous_full_days_range_bounds(day_count, now, reporting_tz)
                .ok_or_else(|| {
                    ApiError::bad_request(anyhow!("invalid previous full days window"))
                })?;
            if let Some(upstream_account_id) = upstream_account_id {
                query_hourly_backed_summary_range_for_account(
                    state.as_ref(),
                    start,
                    end,
                    source_scope,
                    upstream_account_id,
                )
                .await?
            } else {
                query_hourly_backed_summary_range(state.as_ref(), start, end, source_scope).await?
            }
        }
    };

    let mut response = totals.into_response();
    response.non_success_cost = Some(totals.non_success_cost);
    let range = summary_window_range(&window, reporting_tz, now)?;
    if let Some((start, end)) = range {
        response.usage_breakdown = Some(
            load_usage_breakdown_for_range(
                state.as_ref(),
                source_scope,
                upstream_account_id,
                ExactUtcRange { start, end },
            )
            .await?,
        );
    }
    let policy = summary_live_augmentation_policy(&window, range, now);
    let augmentation = load_summary_live_augmentation(
        state.as_ref(),
        source_scope,
        upstream_account_id,
        range,
        policy,
    )
    .await?;
    apply_summary_live_augmentation(&mut response, augmentation);
    response.maintenance = Some(load_stats_maintenance_response(state.as_ref()).await?);
    Ok(Json(response))
}

pub(crate) async fn load_stats_maintenance_response(
    state: &AppState,
) -> Result<StatsMaintenanceResponse, ApiError> {
    {
        let cache = state.maintenance_stats_cache.lock().await;
        if let Some(response) = cache.fresh_response() {
            return Ok(response);
        }
    }

    let raw_backlog = load_raw_compression_backlog_snapshot(&state.pool, &state.config).await?;
    let startup_progress = load_startup_backfill_progress(
        &state.pool,
        StartupBackfillTask::UpstreamActivityArchives.name(),
    )
    .await?;
    let pending_accounts = count_upstream_accounts_missing_last_activity(&state.pool).await?;
    let historical_rollup_backfill =
        load_historical_rollup_backfill_snapshot(&state.pool, &state.config).await?;
    let response = StatsMaintenanceResponse {
        raw_compression_backlog: Some(RawCompressionBacklogResponse {
            oldest_uncompressed_age_secs: raw_backlog.oldest_uncompressed_age_secs,
            uncompressed_count: raw_backlog.uncompressed_count,
            uncompressed_bytes: raw_backlog.uncompressed_bytes,
            alert_level: raw_backlog.alert_level,
        }),
        startup_backfill: Some(StartupBackfillMaintenanceResponse {
            upstream_activity_archive_pending_accounts: pending_accounts,
            zero_update_streak: startup_progress.zero_update_streak,
            next_run_after: startup_progress.next_run_after,
        }),
        historical_rollup_backfill: Some(HistoricalRollupBackfillMaintenanceResponse {
            pending_buckets: historical_rollup_backfill.pending_buckets,
            legacy_archive_pending: historical_rollup_backfill.legacy_archive_pending,
            last_materialized_hour: historical_rollup_backfill.last_materialized_hour,
            alert_level: historical_rollup_backfill.alert_level,
        }),
    };
    let mut cache = state.maintenance_stats_cache.lock().await;
    if let Some(cached) = cache.fresh_response() {
        return Ok(cached);
    }
    cache.store(response.clone());
    Ok(response)
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct ExactUtcRange {
    pub(crate) start: DateTime<Utc>,
    pub(crate) end: DateTime<Utc>,
}

#[derive(Debug, Default)]
pub(crate) struct HourlyRollupExactRangePlan {
    pub(crate) full_hour_range: Option<(i64, i64)>,
    pub(crate) live_exact_ranges: Vec<ExactUtcRange>,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub(crate) struct InvocationAggregateRecord {
    pub(crate) id: i64,
    pub(crate) invoke_id: String,
    pub(crate) occurred_at: String,
    pub(crate) status: Option<String>,
    pub(crate) total_tokens: Option<i64>,
    pub(crate) cache_input_tokens: Option<i64>,
    pub(crate) cost: Option<f64>,
    pub(crate) error_message: Option<String>,
    pub(crate) failure_kind: Option<String>,
    pub(crate) failure_class: Option<String>,
    pub(crate) is_actionable: Option<i64>,
    #[sqlx(default)]
    pub(crate) live_phase: Option<String>,
    pub(crate) t_total_ms: Option<f64>,
    pub(crate) t_req_read_ms: Option<f64>,
    pub(crate) t_req_parse_ms: Option<f64>,
    pub(crate) t_upstream_connect_ms: Option<f64>,
    pub(crate) t_upstream_ttfb_ms: Option<f64>,
    pub(crate) t_upstream_stream_ms: Option<f64>,
    pub(crate) t_resp_parse_ms: Option<f64>,
    pub(crate) t_persist_ms: Option<f64>,
}

pub(crate) fn ceil_hour_epoch(epoch: i64) -> i64 {
    let floor = align_bucket_epoch(epoch, 3_600, 0);
    if floor < epoch { floor + 3_600 } else { floor }
}

pub(crate) fn exact_utc_range(
    start: DateTime<Utc>,
    end: DateTime<Utc>,
) -> Result<Option<ExactUtcRange>, ApiError> {
    if start >= end {
        return Ok(None);
    }
    Ok(Some(ExactUtcRange { start, end }))
}

pub(crate) fn push_exact_range(
    ranges: &mut Vec<ExactUtcRange>,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
) -> Result<(), ApiError> {
    let Some(range) = exact_utc_range(start, end)? else {
        return Ok(());
    };
    if ranges
        .iter()
        .any(|existing| existing.start == range.start && existing.end == range.end)
    {
        return Ok(());
    }
    ranges.push(range);
    Ok(())
}
