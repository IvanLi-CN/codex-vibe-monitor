use super::*;
use crate::{
    EXPLICIT_BILLING_PRICE_VERSION_SUFFIX, ProxyPricingMode, REQUESTED_TIER_PRICE_VERSION_SUFFIX,
    RESPONSE_TIER_PRICE_VERSION_SUFFIX, estimate_proxy_cost_breakdown, has_billable_usage,
    proxy_price_version,
};
use anyhow::anyhow;
use chrono::Timelike;
use futures_util::TryStreamExt;
use serde::Serialize;
use serde_json::{Value, json};
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
pub(crate) const INVOCATION_BLOCKED_BINDING_JSON_SQL: &str = "CASE WHEN json_valid(payload) AND json_type(payload, '$.blockedBinding') = 'object' THEN json_extract(payload, '$.blockedBinding') END";
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
    upstream_account_id: Option<i64>,
    runtime_records: Vec<ApiInvocation>,
    expires_at: Instant,
}

static INVOCATION_ANCHOR_SNAPSHOTS: once_cell::sync::Lazy<
    StdMutex<HashMap<String, InvocationAnchorSnapshot>>,
> = once_cell::sync::Lazy::new(|| StdMutex::new(HashMap::new()));

fn store_invocation_anchor_snapshot(
    snapshot_id: i64,
    upstream_account_id: Option<i64>,
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
        || params.upstream_account_id != snapshot.upstream_account_id
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
pub(crate) const INVOCATION_RESOLVED_FAILURE_CLASS_SQL: &str = concat!(
    "CASE ",
    "  WHEN LOWER(TRIM(COALESCE(failure_class, ''))) IN ('service_failure', 'client_failure', 'client_abort') ",
    "    THEN LOWER(TRIM(COALESCE(failure_class, ''))) ",
    "  ELSE ",
    "    CASE ",
    "      WHEN LOWER(TRIM(COALESCE(status, ''))) IN ('success', 'completed') ",
    "        AND LOWER(TRIM(COALESCE(error_message, ''))) = '' ",
    "        AND LOWER(TRIM(COALESCE(CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.downstreamErrorMessage') AS TEXT) END, ''))) = '' ",
    "        AND LOWER(TRIM(COALESCE(CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.failureKind') AS TEXT) END, failure_kind, ''))) = '' ",
    "        THEN 'none' ",
    "      WHEN LOWER(TRIM(COALESCE(status, ''))) = 'warning_success' ",
    "        AND LOWER(TRIM(COALESCE(error_message, ''))) = '' ",
    "        AND LOWER(TRIM(COALESCE(CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.failureKind') AS TEXT) END, failure_kind, ''))) = 'downstream_closed' ",
    "        THEN 'none' ",
    "      WHEN LOWER(TRIM(COALESCE(status, ''))) IN ('running', 'pending') ",
    "        AND LOWER(TRIM(COALESCE(error_message, ''))) = '' ",
    "        THEN 'none' ",
    "      WHEN LOWER(TRIM(COALESCE(status, ''))) = '' ",
    "        AND LOWER(TRIM(COALESCE(error_message, ''))) = '' ",
    "        AND LOWER(TRIM(COALESCE(CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.downstreamErrorMessage') AS TEXT) END, ''))) = '' ",
    "        AND LOWER(TRIM(COALESCE(CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.failureKind') AS TEXT) END, failure_kind, ''))) = '' ",
    "        THEN 'none' ",
    "      WHEN LOWER(TRIM(COALESCE(CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.failureKind') AS TEXT) END, failure_kind, ''))) = 'downstream_closed' ",
    "        OR LOWER(TRIM(COALESCE(error_message, ''))) LIKE '[downstream_closed]%' ",
    "        OR LOWER(TRIM(COALESCE(error_message, ''))) LIKE '%downstream closed while streaming upstream response%' ",
    "        OR LOWER(TRIM(COALESCE(CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.downstreamErrorMessage') AS TEXT) END, ''))) LIKE '%downstream closed while streaming upstream response%' ",
    "        THEN 'client_abort' ",
    "      WHEN LOWER(TRIM(COALESCE(status, ''))) = 'http_429' ",
    "        OR LOWER(TRIM(COALESCE(CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.failureKind') AS TEXT) END, failure_kind, ''))) = 'upstream_http_429' ",
    "        THEN 'service_failure' ",
    "      WHEN LOWER(TRIM(COALESCE(CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.failureKind') AS TEXT) END, failure_kind, ''))) IN ('request_body_stream_error_client_closed', 'invalid_api_key', 'api_key_not_found', 'api_key_missing') ",
    "        OR LOWER(TRIM(COALESCE(error_message, ''))) LIKE '[request_body_stream_error_client_closed]%' ",
    "        OR LOWER(TRIM(COALESCE(error_message, ''))) LIKE '%failed to read request body stream%' ",
    "        OR LOWER(TRIM(COALESCE(error_message, ''))) LIKE '%invalid api key format%' ",
    "        OR LOWER(TRIM(COALESCE(error_message, ''))) LIKE '%api key format is invalid%' ",
    "        OR LOWER(TRIM(COALESCE(error_message, ''))) LIKE '%incorrect api key provided%' ",
    "        OR LOWER(TRIM(COALESCE(error_message, ''))) LIKE '%api key not found%' ",
    "        OR LOWER(TRIM(COALESCE(error_message, ''))) LIKE '%please provide an api key%' ",
    "        OR (LOWER(TRIM(COALESCE(status, ''))) LIKE 'http_4%' AND LOWER(TRIM(COALESCE(status, ''))) != 'http_429') ",
    "        OR LOWER(TRIM(COALESCE(status, ''))) IN ('http_401', 'http_403') ",
    "        THEN 'client_failure' ",
    "      WHEN LOWER(TRIM(COALESCE(CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.failureKind') AS TEXT) END, failure_kind, ''))) IN ('failed_contact_upstream', 'upstream_response_failed', 'upstream_stream_error', 'request_body_read_timeout', 'upstream_handshake_timeout') ",
    "        OR LOWER(TRIM(COALESCE(error_message, ''))) LIKE '[failed_contact_upstream]%' ",
    "        OR LOWER(TRIM(COALESCE(error_message, ''))) LIKE '[upstream_response_failed]%' ",
    "        OR LOWER(TRIM(COALESCE(error_message, ''))) LIKE '[upstream_stream_error]%' ",
    "        OR LOWER(TRIM(COALESCE(error_message, ''))) LIKE '[request_body_read_timeout]%' ",
    "        OR LOWER(TRIM(COALESCE(error_message, ''))) LIKE '[upstream_handshake_timeout]%' ",
    "        OR LOWER(TRIM(COALESCE(error_message, ''))) LIKE '%failed to contact upstream%' ",
    "        OR LOWER(TRIM(COALESCE(error_message, ''))) LIKE '%upstream response stream reported failure%' ",
    "        OR LOWER(TRIM(COALESCE(error_message, ''))) LIKE '%upstream stream error%' ",
    "        OR LOWER(TRIM(COALESCE(error_message, ''))) LIKE '%request body read timed out%' ",
    "        OR LOWER(TRIM(COALESCE(error_message, ''))) LIKE '%upstream handshake timed out%' ",
    "        OR LOWER(TRIM(COALESCE(status, ''))) LIKE 'http_5%' ",
    "        THEN 'service_failure' ",
    "      WHEN LOWER(TRIM(COALESCE(status, ''))) IN ('success', 'completed', 'warning_success') ",
    "        THEN 'none' ",
    "      WHEN LOWER(TRIM(COALESCE(status, ''))) = 'http_200' ",
    "        AND LOWER(TRIM(COALESCE(error_message, ''))) = '' ",
    "        AND LOWER(TRIM(COALESCE(CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.downstreamErrorMessage') AS TEXT) END, ''))) = '' ",
    "        AND LOWER(TRIM(COALESCE(CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.failureKind') AS TEXT) END, failure_kind, ''))) = '' ",
    "        THEN 'none' ",
    "      ELSE 'service_failure' ",
    "    END ",
    "END"
);

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
         ",
        )
        .push(INVOCATION_BLOCKED_BINDING_JSON_SQL)
        .push(
            " AS blocked_binding_json, \
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
    cache_write_tokens: i64,
    cache_input_tokens: i64,
    output_tokens: i64,
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
    pub(crate) cache_write_tokens: i64,
    pub(crate) cache_input_tokens: i64,
    pub(crate) output_tokens: i64,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) headers: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) routing: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) body_size: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) body_truncated: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) body_truncated_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) detail_level: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) detail_prune_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) capture_source: Option<String>,
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
        } else if normalized_status.eq_ignore_ascii_case(INVOCATION_STATUS_WARNING_SUCCESS) {
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
        if normalized_status.eq_ignore_ascii_case("failed") {
            let classification = resolve_failure_classification(
                record.status.as_deref(),
                record.error_message.as_deref(),
                record.failure_kind.as_deref(),
                record.failure_class.as_deref(),
                record.is_actionable.map(|value| if value { 1 } else { 0 }),
            );
            if !prompt_cache_and_timeseries_shared::prompt_invocation_status_counts_toward_terminal_totals(
                record.status.as_deref(),
            ) || classification.failure_class == FailureClass::None
                || runtime_text_equals(record.status.as_deref(), "interrupted")
            {
                return false;
            }
        } else if normalized_status.eq_ignore_ascii_case("success") {
            let classification = resolve_failure_classification(
                record.status.as_deref(),
                record.error_message.as_deref(),
                record.failure_kind.as_deref(),
                record.failure_class.as_deref(),
                record.is_actionable.map(|value| if value { 1 } else { 0 }),
            );
            if !runtime_text_equals(record.status.as_deref(), normalized_status)
                || classification.failure_class != FailureClass::None
            {
                return false;
            }
        } else if normalized_status.eq_ignore_ascii_case(INVOCATION_STATUS_WARNING_SUCCESS) {
            let classification = resolve_failure_classification(
                record.status.as_deref(),
                record.error_message.as_deref(),
                record.failure_kind.as_deref(),
                record.failure_class.as_deref(),
                record.is_actionable.map(|value| if value { 1 } else { 0 }),
            );
            if !runtime_text_equals(record.status.as_deref(), normalized_status)
                || classification.failure_class != FailureClass::None
            {
                return false;
            }
        } else if !runtime_text_equals(record.status.as_deref(), normalized_status) {
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
    cache_write_tokens: i64,
    cache_input_tokens: i64,
    output_tokens: i64,
    service_failure_count: i64,
    client_failure_count: i64,
    client_abort_count: i64,
}

impl RuntimeSummaryOverlayDelta {
    fn add_terminal_record(&mut self, record: &ApiInvocation) {
        self.total_tokens += record.total_tokens.unwrap_or_default();
        self.total_cost += record.cost.unwrap_or_default();
        self.cache_write_tokens +=
            resolve_invocation_cache_write_tokens(record).unwrap_or_default();
        self.cache_input_tokens += record.cache_input_tokens.unwrap_or_default();
        self.output_tokens += record.output_tokens.unwrap_or_default();
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
        || status == INVOCATION_STATUS_WARNING_SUCCESS
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

        let mut records = query
            .build_query_as::<ApiInvocation>()
            .fetch_all(&state.pool)
            .await?;
        for record in &mut records {
            hydrate_api_invocation_blocked_binding(record);
        }
        let total = records.len() as i64;
        let (mut records, total) = overlay_runtime_records_for_current_page(
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
        let pricing_catalog = state.pricing_catalog.read().await.clone();
        apply_invocation_cost_audits(&mut records, &pricing_catalog);

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
        let (mut records, total) = overlay_runtime_records_for_current_page(
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
        let pricing_catalog = state.pricing_catalog.read().await.clone();
        apply_invocation_cost_audits(&mut records, &pricing_catalog);
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
    for record in &mut records {
        hydrate_api_invocation_blocked_binding(record);
    }
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
    let (mut records, total) = overlay_runtime_records_for_current_page(
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
    let pricing_catalog = state.pricing_catalog.read().await.clone();
    apply_invocation_cost_audits(&mut records, &pricing_catalog);

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
    pub(crate) request_id: String,
    pub(crate) attempt_id: Option<String>,
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

#[derive(Debug, FromRow)]
struct InvocationLocateAttemptRow {
    invoke_id: String,
    upstream_account_id: Option<i64>,
}

pub(crate) async fn locate_invocation_page(
    state: Arc<AppState>,
    params: &LocateInvocationQuery,
) -> Result<Option<LocateInvocationResponse>, ApiError> {
    let request_id = normalize_query_text(params.request_id.as_deref());
    let attempt_id = normalize_query_text(params.attempt_id.as_deref());
    if request_id.is_none() && attempt_id.is_none() {
        return Err(ApiError::bad_request(anyhow!(
            "requestId or attemptId is required"
        )));
    }
    let upstream_account_id = match params.upstream_account_id {
        Some(value) if value > 0 => Some(value),
        Some(_) => {
            return Err(ApiError::bad_request(anyhow!(
                "upstreamAccountId must be positive"
            )));
        }
        None => None,
    };

    let page_size = params
        .page_size
        .unwrap_or(50)
        .clamp(1, state.config.list_limit_max as i64);
    let source_scope = resolve_default_source_scope(&state.pool).await?;
    let snapshot_id = resolve_invocation_snapshot_id(&state.pool, source_scope).await?;
    let (request_id, resolved_attempt_id, upstream_account_id) =
        if let Some(attempt_public_id) = attempt_id.as_deref() {
            let target = sqlx::query_as::<_, InvocationLocateAttemptRow>(
                r#"
                SELECT invoke_id, upstream_account_id
                FROM pool_upstream_request_attempts
                WHERE attempt_public_id = ?1
                LIMIT 1
                "#,
            )
            .bind(attempt_public_id)
            .fetch_optional(&state.pool)
            .await?;
            let Some(target) = target else {
                return Ok(None);
            };
            if upstream_account_id.is_some() && upstream_account_id != target.upstream_account_id {
                return Ok(None);
            }
            (
                target.invoke_id,
                Some(attempt_public_id.to_string()),
                upstream_account_id.or(target.upstream_account_id),
            )
        } else {
            (request_id.unwrap_or_default(), None, upstream_account_id)
        };
    let base_filters = InvocationRecordsFilters {
        upstream_account_id,
        ..Default::default()
    };
    let runtime_records = runtime_overlay_snapshot(state.as_ref());
    let runtime_target = runtime_records.iter().find(|record| {
        runtime_text_equals(Some(record.invoke_id.as_str()), request_id.as_str())
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
        .push_bind(request_id.clone())
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
                upstream_account_id,
                ..Default::default()
            },
            Some(anchor_runtime_records.clone()),
        )
        .await?;
        if let Some(target_index) = response.records.iter().position(|record| {
            runtime_text_equals(Some(record.invoke_id.as_str()), request_id.as_str())
        }) {
            let anchor_id = store_invocation_anchor_snapshot(
                snapshot_id,
                upstream_account_id,
                anchor_runtime_records.clone(),
            );
            return Ok(Some(LocateInvocationResponse {
                anchor_id,
                snapshot_id: response.snapshot_id,
                request_id: request_id.clone(),
                attempt_id: resolved_attempt_id.clone(),
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
                "requestId": normalize_query_text(params.request_id.as_deref()),
                "attemptId": normalize_query_text(params.attempt_id.as_deref()),
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
struct InvocationWorkflowIdentityRow {
    id: i64,
    invoke_id: String,
    occurred_at: String,
    timeline_json: Option<String>,
}

#[derive(Debug, FromRow)]
struct InvocationWorkflowAttemptRow {
    attempt_id: Option<String>,
    invoke_id: String,
    occurred_at: String,
    endpoint: String,
    sticky_key: Option<String>,
    upstream_account_id: Option<i64>,
    upstream_account_name: Option<String>,
    upstream_route_key: Option<String>,
    proxy_binding_key_snapshot: Option<String>,
    attempt_index: i64,
    distinct_account_index: i64,
    same_account_retry_index: i64,
    requester_ip: Option<String>,
    started_at: Option<String>,
    finished_at: Option<String>,
    status: String,
    phase: Option<String>,
    http_status: Option<i64>,
    downstream_http_status: Option<i64>,
    failure_kind: Option<String>,
    error_message: Option<String>,
    downstream_error_message: Option<String>,
    connect_latency_ms: Option<f64>,
    first_byte_latency_ms: Option<f64>,
    stream_latency_ms: Option<f64>,
    upstream_request_id: Option<String>,
    upstream_request_compression_algorithm: Option<String>,
    upstream_request_compression_mode: Option<String>,
    upstream_request_logical_body_bytes: Option<i64>,
    upstream_request_transmitted_body_bytes: Option<i64>,
    upstream_request_header_bytes_approx: Option<i64>,
    upstream_response_body_bytes: Option<i64>,
    upstream_response_header_bytes_approx: Option<i64>,
    compact_support_status: Option<String>,
    compact_support_reason: Option<String>,
    request_summary_json: Option<String>,
    response_summary_json: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct InvocationWorkflowDetailResponse {
    pub(crate) hero: InvocationWorkflowHero,
    pub(crate) timeline: Vec<InvocationWorkflowTimelineEntry>,
    pub(crate) reconstructed: bool,
    pub(crate) partial: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) partial_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct InvocationWorkflowHero {
    pub(crate) record_id: i64,
    pub(crate) invoke_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) prompt_cache_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) route_mode: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) endpoint: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) request_model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) response_model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) final_status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) failure_class: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) downstream_status_code: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) upstream_account_id: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) upstream_account_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) total_duration_ms: Option<f64>,
    pub(crate) timeline_attempt_count: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) pool_attempt_count: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) total_tokens: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) cost: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) occurred_at: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct InvocationWorkflowAttempt {
    pub(crate) synthetic: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) attempt_id: Option<String>,
    pub(crate) occurred_at: String,
    pub(crate) endpoint: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) sticky_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) upstream_account_id: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) upstream_account_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) request_model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) response_model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) upstream_route_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) proxy_binding_key_snapshot: Option<String>,
    pub(crate) attempt_index: i64,
    pub(crate) distinct_account_index: i64,
    pub(crate) same_account_retry_index: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) requester_ip: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) started_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) finished_at: Option<String>,
    pub(crate) status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) phase: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) http_status: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) downstream_http_status: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) failure_kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) error_message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) downstream_error_message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) connect_latency_ms: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) first_byte_latency_ms: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) stream_latency_ms: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) upstream_request_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) request_summary: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) response_summary: Option<Value>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct InvocationWorkflowResponseBody {
    pub(crate) available: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) body_text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) unavailable_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct InvocationWorkflowTimelineEntry {
    pub(crate) block_id: String,
    pub(crate) kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) occurred_at: Option<String>,
    pub(crate) title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) subtitle: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) attempt: Option<InvocationWorkflowAttempt>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) detail: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) response_body: Option<InvocationWorkflowResponseBody>,
}

fn normalize_optional_timestamp(value: Option<&str>) -> Option<String> {
    let normalized = value?.trim();
    if normalized.is_empty() {
        return None;
    }
    parse_to_utc_datetime(normalized)
        .map(format_utc_iso)
        .or_else(|| Some(normalized.to_string()))
}

fn parse_summary_json_or_fallback(
    raw: Option<&str>,
    fallback: impl FnOnce() -> Value,
) -> Option<Value> {
    match raw.map(str::trim).filter(|value| !value.is_empty()) {
        Some(raw) => serde_json::from_str::<Value>(raw)
            .ok()
            .or_else(|| Some(fallback())),
        None => Some(fallback()),
    }
}

fn parse_optional_json_value(raw: Option<&str>) -> Option<Value> {
    raw.and_then(|value| {
        let normalized = value.trim();
        if normalized.is_empty() {
            None
        } else {
            serde_json::from_str::<Value>(normalized).ok()
        }
    })
}

fn payload_value<'a>(payload: Option<&'a Value>, keys: &[&str]) -> Option<&'a Value> {
    let object = payload?.as_object()?;
    keys.iter().find_map(|key| object.get(*key))
}

fn payload_string(payload: Option<&Value>, keys: &[&str]) -> Option<String> {
    payload_value(payload, keys)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn payload_bool(payload: Option<&Value>, keys: &[&str]) -> Option<bool> {
    payload_value(payload, keys).and_then(Value::as_bool)
}

fn payload_i64(payload: Option<&Value>, keys: &[&str]) -> Option<i64> {
    payload_value(payload, keys).and_then(Value::as_i64)
}

fn payload_u64(payload: Option<&Value>, keys: &[&str]) -> Option<u64> {
    payload_value(payload, keys).and_then(Value::as_u64)
}

fn payload_f64(payload: Option<&Value>, keys: &[&str]) -> Option<f64> {
    payload_value(payload, keys).and_then(Value::as_f64)
}

fn payload_string_array(payload: Option<&Value>, keys: &[&str]) -> Option<Vec<String>> {
    payload_value(payload, keys)
        .and_then(Value::as_array)
        .map(|entries| {
            entries
                .iter()
                .filter_map(|entry| entry.as_str())
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned)
                .collect::<Vec<_>>()
        })
        .filter(|entries| !entries.is_empty())
}

fn payload_clone(payload: Option<&Value>, keys: &[&str]) -> Option<Value> {
    payload_value(payload, keys).cloned()
}

fn request_compression_value(
    algorithm: Option<String>,
    mode: Option<String>,
    derived: RequestCompressionDerivedFields,
) -> Value {
    json!({
        "algorithm": algorithm,
        "mode": mode,
        "logicalBodyBytes": derived.logical_body_bytes,
        "transmittedBodyBytes": derived.transmitted_body_bytes,
        "savedBytes": derived.saved_bytes,
        "ratioPct": derived.ratio_pct,
        "approxUploadBytes": derived.approx_upload_bytes,
        "approxDownloadBytes": derived.approx_download_bytes,
    })
}

fn derive_request_compression_from_payload(
    payload: Option<&Value>,
) -> RequestCompressionDerivedFields {
    let mut derived = derive_request_compression_fields(
        payload_i64(payload, &["requestCompressionLogicalBodyBytes"]),
        payload_i64(payload, &["requestCompressionTransmittedBodyBytes"]),
        None,
        None,
        None,
        payload_bool(payload, &["requestCompressionTransmissionComplete"]).unwrap_or(false),
    );
    derived.approx_upload_bytes =
        payload_i64(payload, &["upstreamApproxUploadBytes"]).filter(|value| *value >= 0);
    derived.approx_download_bytes =
        payload_i64(payload, &["upstreamApproxDownloadBytes"]).filter(|value| *value >= 0);
    derived
}

fn build_request_header_snapshot(payload: Option<&Value>) -> Value {
    json!({
        "userAgent": payload_string(payload, &["requestUserAgent"]),
        "xForwardedFor": payload_string(payload, &["requestXForwardedFor"]),
        "forwarded": payload_string(payload, &["requestForwarded"]),
        "xRealIp": payload_string(payload, &["requestXRealIp"]),
    })
}

fn build_request_routing_snapshot(
    record: &ApiInvocation,
    attempt: Option<&InvocationWorkflowAttemptRow>,
    payload: Option<&Value>,
) -> Value {
    json!({
        "routeMode": record.route_mode.clone(),
        "upstreamScope": payload_string(payload, &["upstreamScope"]),
        "stickyKey": attempt
            .and_then(|row| row.sticky_key.clone())
            .or_else(|| record.sticky_key.clone())
            .or_else(|| payload_string(payload, &["stickyKey"])),
        "promptCacheKey": record
            .prompt_cache_key
            .clone()
            .or_else(|| payload_string(payload, &["promptCacheKey"])),
        "proxyDisplayName": payload_string(payload, &["proxyDisplayName"]),
        "upstreamRouteKey": attempt
            .and_then(|row| row.upstream_route_key.clone())
            .or_else(|| payload_string(payload, &["upstreamRouteKey"])),
        "proxyBindingKey": attempt
            .and_then(|row| row.proxy_binding_key_snapshot.clone())
            .or_else(|| payload_string(payload, &["proxyBindingKey", "proxyBindingKeySnapshot"])),
        "clientFingerprint": payload_string(payload, &["clientFingerprint"]),
        "clientHeaderFingerprints": payload_clone(payload, &["clientHeaderFingerprints"]),
        "oauthForwardedHeaderNames": payload_string_array(payload, &["oauthForwardedHeaderNames"]),
        "oauthPromptCacheHeaderForwarded": payload_bool(payload, &["oauthPromptCacheHeaderForwarded"]),
    })
}

fn build_request_client_snapshot(payload: Option<&Value>) -> Value {
    json!({
        "requestContainsEncryptedContent": payload_bool(payload, &["requestContainsEncryptedContent"]),
        "requestParseError": payload_string(payload, &["requestParseError"]),
        "oauthAccountHeaderAttached": payload_bool(payload, &["oauthAccountHeaderAttached"]),
        "oauthAccountIdShape": payload_string(payload, &["oauthAccountIdShape"]),
        "oauthRequestBodyPrefixFingerprint": payload_string(payload, &["oauthRequestBodyPrefixFingerprint"]),
        "oauthRequestBodyPrefixBytes": payload_u64(payload, &["oauthRequestBodyPrefixBytes"]),
        "oauthRequestBodySnapshotKind": payload_string(payload, &["oauthRequestBodySnapshotKind"]),
        "oauthResponsesBodyMode": payload_string(payload, &["oauthResponsesBodyMode"]),
        "oauthResponsesRewrite": payload_clone(payload, &["oauthResponsesRewrite"]),
    })
}

fn build_response_header_snapshot(record: &ApiInvocation, payload: Option<&Value>) -> Value {
    json!({
        "contentEncoding": record
            .response_content_encoding
            .clone()
            .or_else(|| payload_string(payload, &["responseContentEncoding"])),
        "contentEncodingChain": payload_string(payload, &["contentEncodingChain"]),
        "upstreamRequestId": record
            .upstream_request_id
            .clone()
            .or_else(|| payload_string(payload, &["upstreamRequestId"])),
        "cvmInvokeId": Some(record.invoke_id.clone()),
    })
}

fn build_response_delivery_snapshot(payload: Option<&Value>) -> Value {
    json!({
        "forwardedChunkCount": payload_u64(payload, &["forwardedChunkCount"]),
        "forwardedBytes": payload_u64(payload, &["forwardedBytes"]),
        "usageObserved": payload_bool(payload, &["usageObserved"]),
        "downstreamClosePhase": payload_string(payload, &["downstreamClosePhase"]),
        "downstreamWriteErrorKind": payload_string(payload, &["downstreamWriteErrorKind"]),
        "lastUpstreamChunkGapMs": payload_u64(payload, &["lastUpstreamChunkGapMs"]),
        "streamFailureOrigin": payload_string(payload, &["streamFailureOrigin"]),
        "upstreamReadErrorKind": payload_string(payload, &["upstreamReadErrorKind"]),
        "responseContainsEncryptedContent": payload_bool(payload, &["responseContainsEncryptedContent"]),
    })
}

fn invocation_status_is_success_like(record: &ApiInvocation) -> bool {
    matches!(
        normalized_runtime_text(record.status.as_deref()).as_str(),
        "success" | "completed" | "warning_success"
    )
}

pub(crate) const INVOCATION_COST_AUDIT_MISMATCH_EPSILON_USD: f64 = 0.000001;
pub(crate) const INVOCATION_COST_AUDIT_REASON_RECORDED_COST_MISSING: &str = "recorded_cost_missing";
pub(crate) const INVOCATION_COST_AUDIT_REASON_RECORDED_PRICE_VERSION_MISSING: &str =
    "recorded_price_version_missing";
pub(crate) const INVOCATION_COST_AUDIT_REASON_PRICING_MODE_UNKNOWN: &str = "pricing_mode_unknown";
pub(crate) const INVOCATION_COST_AUDIT_REASON_USAGE_MISSING: &str = "usage_missing";
pub(crate) const INVOCATION_COST_AUDIT_REASON_MODEL_MISSING: &str = "model_missing";
pub(crate) const INVOCATION_COST_AUDIT_REASON_MODEL_PRICING_MISSING: &str = "model_pricing_missing";
pub(crate) const INVOCATION_COST_AUDIT_REASON_PRICE_VERSION_CHANGED: &str = "price_version_changed";
pub(crate) const INVOCATION_COST_AUDIT_REASON_TOTAL_MISMATCH: &str = "total_mismatch";

fn resolve_invocation_cache_write_tokens(record: &ApiInvocation) -> Option<i64> {
    record.cache_write_tokens.or_else(|| {
        record
            .input_tokens
            .map(|input| input.saturating_sub(record.cache_input_tokens.unwrap_or_default().max(0)))
    })
}

fn invocation_has_usage_evidence(record: &ApiInvocation) -> bool {
    [
        record.input_tokens,
        record.output_tokens,
        record.cache_input_tokens,
        record.reasoning_tokens,
        record.total_tokens,
        resolve_invocation_cache_write_tokens(record),
    ]
    .into_iter()
    .any(|value| value.is_some())
        || record.cost.is_some()
        || record.cost_input.is_some()
        || record.cost_cache_write.is_some()
        || record.cost_cache_read.is_some()
        || record.cost_output.is_some()
        || record.cost_reasoning.is_some()
}

fn invocation_usage_for_cost_audit(record: &ApiInvocation) -> ParsedUsage {
    ParsedUsage {
        input_tokens: record.input_tokens,
        output_tokens: record.output_tokens,
        cache_input_tokens: record.cache_input_tokens,
        reasoning_tokens: record.reasoning_tokens,
        total_tokens: record.total_tokens,
    }
}

fn invocation_billable_model(record: &ApiInvocation) -> Option<&str> {
    record
        .response_model
        .as_deref()
        .or(record.model.as_deref())
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn resolve_invocation_pricing_mode(
    price_version: Option<&str>,
) -> Result<ProxyPricingMode, &'static str> {
    let normalized = price_version
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or(INVOCATION_COST_AUDIT_REASON_RECORDED_PRICE_VERSION_MISSING)?;
    if normalized.ends_with(REQUESTED_TIER_PRICE_VERSION_SUFFIX) {
        return Ok(ProxyPricingMode::RequestedTier);
    }
    if normalized.ends_with(RESPONSE_TIER_PRICE_VERSION_SUFFIX) {
        return Ok(ProxyPricingMode::ResponseTier);
    }
    if normalized.ends_with(EXPLICIT_BILLING_PRICE_VERSION_SUFFIX) {
        return Ok(ProxyPricingMode::ExplicitBilling);
    }
    Err(INVOCATION_COST_AUDIT_REASON_PRICING_MODE_UNKNOWN)
}

fn build_invocation_cost_audit_breakdown(
    total: Option<f64>,
    input: Option<f64>,
    cache_write: Option<f64>,
    cache_read: Option<f64>,
    output: Option<f64>,
    reasoning: Option<f64>,
    include_breakdown: bool,
) -> Option<InvocationCostAuditBreakdown> {
    let has_components = [input, cache_write, cache_read, output, reasoning]
        .into_iter()
        .any(|value| value.is_some());
    if total.is_none() && !(include_breakdown && has_components) {
        return None;
    }
    Some(InvocationCostAuditBreakdown {
        input: include_breakdown.then_some(input).flatten(),
        cache_write: include_breakdown.then_some(cache_write).flatten(),
        cache_read: include_breakdown.then_some(cache_read).flatten(),
        output: include_breakdown.then_some(output).flatten(),
        reasoning: include_breakdown.then_some(reasoning).flatten(),
        total,
    })
}

fn build_recorded_invocation_cost_breakdown(
    record: &ApiInvocation,
    include_breakdown: bool,
) -> Option<InvocationCostAuditBreakdown> {
    build_invocation_cost_audit_breakdown(
        record.cost,
        record.cost_input,
        record.cost_cache_write,
        record.cost_cache_read,
        record.cost_output,
        record.cost_reasoning,
        include_breakdown,
    )
}

fn build_local_invocation_cost_breakdown(
    record: &ApiInvocation,
    catalog: &PricingCatalog,
    include_breakdown: bool,
) -> (
    Option<InvocationCostAuditBreakdown>,
    Option<String>,
    Option<&'static str>,
) {
    let pricing_mode = match resolve_invocation_pricing_mode(record.price_version.as_deref()) {
        Ok(mode) => mode,
        Err(reason) => return (None, None, Some(reason)),
    };
    let local_price_version = Some(proxy_price_version(&catalog.version, pricing_mode));
    let usage = invocation_usage_for_cost_audit(record);
    if !has_billable_usage(&usage) {
        return (
            None,
            local_price_version,
            Some(INVOCATION_COST_AUDIT_REASON_USAGE_MISSING),
        );
    }
    let Some(model) = invocation_billable_model(record) else {
        return (
            None,
            local_price_version,
            Some(INVOCATION_COST_AUDIT_REASON_MODEL_MISSING),
        );
    };
    let (breakdown, _, _) = estimate_proxy_cost_breakdown(
        catalog,
        Some(model),
        &usage,
        record.billing_service_tier.as_deref(),
        pricing_mode,
    );
    let Some(breakdown) = breakdown else {
        return (
            None,
            local_price_version,
            Some(INVOCATION_COST_AUDIT_REASON_MODEL_PRICING_MISSING),
        );
    };
    (
        build_invocation_cost_audit_breakdown(
            Some(breakdown.total()),
            Some(breakdown.input),
            Some(breakdown.cache_write),
            Some(breakdown.cache_read),
            Some(breakdown.output),
            Some(breakdown.reasoning),
            include_breakdown,
        ),
        local_price_version,
        None,
    )
}

fn build_invocation_cost_audit(
    record: &ApiInvocation,
    catalog: &PricingCatalog,
    include_breakdown: bool,
) -> Option<InvocationCostAudit> {
    let recorded = build_recorded_invocation_cost_breakdown(record, include_breakdown);
    let recorded_total = recorded.as_ref().and_then(|value| value.total);
    let (local, local_price_version, unavailable_reason) =
        build_local_invocation_cost_breakdown(record, catalog, include_breakdown);
    let local_total = local.as_ref().and_then(|value| value.total);
    let absolute_diff_usd = match (recorded_total, local_total) {
        (Some(recorded_total), Some(local_total)) => Some((recorded_total - local_total).abs()),
        _ => None,
    };
    let mismatch =
        absolute_diff_usd.is_some_and(|diff| diff > INVOCATION_COST_AUDIT_MISMATCH_EPSILON_USD);
    let recorded_price_version = normalize_query_text(record.price_version.as_deref());
    let reason = if mismatch {
        if recorded_price_version.as_deref() != local_price_version.as_deref() {
            Some(INVOCATION_COST_AUDIT_REASON_PRICE_VERSION_CHANGED.to_string())
        } else {
            Some(INVOCATION_COST_AUDIT_REASON_TOTAL_MISMATCH.to_string())
        }
    } else if recorded_total.is_none() && local_total.is_some() {
        Some(INVOCATION_COST_AUDIT_REASON_RECORDED_COST_MISSING.to_string())
    } else if recorded_total.is_some() && local_total.is_none() {
        unavailable_reason.map(str::to_string)
    } else {
        None
    };
    if recorded.is_none()
        && local.is_none()
        && recorded_price_version.is_none()
        && local_price_version.is_none()
        && reason.is_none()
    {
        return None;
    }
    Some(InvocationCostAudit {
        recorded,
        local,
        mismatch,
        reason,
        absolute_diff_usd,
        recorded_price_version,
        local_price_version,
    })
}

fn apply_invocation_cost_audits(records: &mut [ApiInvocation], catalog: &PricingCatalog) {
    for record in records {
        record.cost_audit = build_invocation_cost_audit(record, catalog, false);
    }
}

fn build_invocation_usage_summary(
    record: &ApiInvocation,
    cost_audit: &InvocationCostAudit,
) -> Value {
    let recorded = cost_audit
        .recorded
        .as_ref()
        .map(|breakdown| json!(breakdown));
    let local = cost_audit.local.as_ref().map(|breakdown| json!(breakdown));
    json!({
        "inputTokens": record.input_tokens,
        "cacheWriteTokens": resolve_invocation_cache_write_tokens(record),
        "cacheInputTokens": record.cache_input_tokens,
        "outputTokens": record.output_tokens,
        "reasoningTokens": record.reasoning_tokens,
        "totalTokens": record.total_tokens,
        "cost": record.cost,
        "tokens": {
            "input": record.input_tokens,
            "cacheWrite": resolve_invocation_cache_write_tokens(record),
            "cacheRead": record.cache_input_tokens,
            "output": record.output_tokens,
            "reasoning": record.reasoning_tokens,
            "total": record.total_tokens,
        },
        "costs": {
            "recorded": recorded,
            "local": local,
        },
        "audit": cost_audit,
    })
}

fn build_workflow_hero(
    record: &ApiInvocation,
    timeline_attempt_count: usize,
) -> InvocationWorkflowHero {
    InvocationWorkflowHero {
        record_id: record.id,
        invoke_id: record.invoke_id.clone(),
        prompt_cache_key: record.prompt_cache_key.clone(),
        route_mode: record.route_mode.clone(),
        endpoint: record.endpoint.clone(),
        request_model: record.request_model.clone(),
        response_model: record
            .response_model
            .clone()
            .or_else(|| record.model.clone()),
        final_status: record.status.clone(),
        failure_class: record.failure_class.clone(),
        downstream_status_code: record.downstream_status_code,
        upstream_account_id: record.upstream_account_id,
        upstream_account_name: record.upstream_account_name.clone(),
        total_duration_ms: record.t_total_ms,
        timeline_attempt_count,
        pool_attempt_count: record.pool_attempt_count,
        total_tokens: record.total_tokens,
        cost: record.cost,
        occurred_at: Some(record.occurred_at.clone()),
    }
}

fn build_attempt_request_summary(
    record: &ApiInvocation,
    attempt: &InvocationWorkflowAttemptRow,
    payload: Option<&Value>,
) -> Value {
    let compression = request_compression_value(
        attempt.upstream_request_compression_algorithm.clone(),
        attempt.upstream_request_compression_mode.clone(),
        derive_request_compression_fields(
            attempt.upstream_request_logical_body_bytes,
            attempt.upstream_request_transmitted_body_bytes,
            attempt.upstream_request_header_bytes_approx,
            attempt.upstream_response_body_bytes,
            attempt.upstream_response_header_bytes_approx,
            attempt.http_status.is_some()
                || attempt
                    .status
                    .eq_ignore_ascii_case(POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS),
        ),
    );
    json!({
        "endpoint": attempt.endpoint.clone(),
        "routeMode": record.route_mode.clone(),
        "transport": record.transport.clone(),
        "requestModel": record.request_model.clone(),
        "responseModel": record.response_model.as_ref().or(record.model.as_ref()),
        "requestedServiceTier": record.requested_service_tier.clone(),
        "reasoningEffort": record.reasoning_effort.clone(),
        "compactionRequestKind": record.compaction_request_kind.clone(),
        "imageIntent": record.image_intent.clone(),
        "promptCacheKey": record.prompt_cache_key.clone(),
        "stickyKey": attempt.sticky_key.as_ref().or(record.sticky_key.as_ref()),
        "requesterIp": attempt.requester_ip.as_ref().or(record.requester_ip.as_ref()),
        "account": {
            "id": attempt.upstream_account_id,
            "name": attempt.upstream_account_name.as_ref().or(record.upstream_account_name.as_ref()),
        },
        "routing": {
            "upstreamRouteKey": attempt.upstream_route_key.clone(),
            "proxyBindingKey": attempt.proxy_binding_key_snapshot.clone(),
            "proxyDisplayName": record.proxy_display_name.clone(),
            "upstreamScope": payload_string(payload, &["upstreamScope"]),
            "clientFingerprint": payload_string(payload, &["clientFingerprint"]),
            "oauthForwardedHeaderNames": payload_string_array(payload, &["oauthForwardedHeaderNames"]),
            "oauthPromptCacheHeaderForwarded": payload_bool(payload, &["oauthPromptCacheHeaderForwarded"]),
        },
        "headers": build_request_header_snapshot(payload),
        "client": build_request_client_snapshot(payload),
        "compression": compression,
        "bodyCapture": {
            "availableAtInvocationLevel": record.request_raw_path.is_some(),
            "size": record.request_raw_size,
            "truncated": record.request_raw_truncated.unwrap_or_default() != 0,
            "truncatedReason": record.request_raw_truncated_reason.clone(),
            "detailLevel": record.detail_level.clone(),
            "detailPruneReason": record.detail_prune_reason.clone(),
        },
    })
}

fn build_attempt_response_summary(
    record: &ApiInvocation,
    attempt: &InvocationWorkflowAttemptRow,
    payload: Option<&Value>,
    usage_cost_audit: Option<&InvocationCostAudit>,
) -> Value {
    json!({
        "status": attempt.status.clone(),
        "phase": attempt.phase.clone(),
        "httpStatus": attempt.http_status,
        "compactionResponseKind": record.compaction_response_kind.clone(),
        "failureKind": attempt.failure_kind.as_ref().or(record.failure_kind.as_ref()),
        "errorMessage": attempt.error_message.as_ref().or(record.error_message.as_ref()),
        "downstreamErrorMessage": attempt
            .downstream_error_message
            .as_ref()
            .or(record.downstream_error_message.as_ref()),
        "upstreamRequestId": attempt.upstream_request_id.as_ref().or(record.upstream_request_id.as_ref()),
        "upstreamErrorCode": record.upstream_error_code.clone(),
        "upstreamErrorMessage": record.upstream_error_message.clone(),
        "streamTerminalEvent": record.stream_terminal_event.clone(),
        "responseContentEncoding": record.response_content_encoding.clone(),
        "serviceTier": record.service_tier.clone(),
        "billingServiceTier": record.billing_service_tier.clone(),
        "headers": build_response_header_snapshot(record, payload),
        "delivery": build_response_delivery_snapshot(payload),
        "compactSupport": {
            "status": attempt.compact_support_status.clone(),
            "reason": attempt.compact_support_reason.clone(),
        },
        "latencyMs": {
            "connect": attempt.connect_latency_ms.or(record.t_upstream_connect_ms),
            "firstByte": attempt.first_byte_latency_ms.or(record.t_upstream_ttfb_ms),
            "stream": attempt.stream_latency_ms.or(record.t_upstream_stream_ms),
            "requestRead": record.t_req_read_ms,
            "requestParse": record.t_req_parse_ms,
            "responseParse": record.t_resp_parse_ms,
            "persist": record.t_persist_ms,
            "total": record.t_total_ms,
        },
        "responseBodyCapture": {
            "availableAtInvocationLevel": record.response_raw_path.is_some(),
            "size": record.response_raw_size,
            "truncated": record.response_raw_truncated.unwrap_or_default() != 0,
            "truncatedReason": record.response_raw_truncated_reason.clone(),
            "detailLevel": record.detail_level.clone(),
            "detailPruneReason": record.detail_prune_reason.clone(),
        },
        "usage": usage_cost_audit.map(|audit| build_invocation_usage_summary(record, audit)),
    })
}

fn build_workflow_attempt_from_row(
    record: &ApiInvocation,
    attempt: &InvocationWorkflowAttemptRow,
    payload: Option<&Value>,
    usage_cost_audit: Option<&InvocationCostAudit>,
) -> InvocationWorkflowAttempt {
    InvocationWorkflowAttempt {
        synthetic: false,
        attempt_id: attempt.attempt_id.clone(),
        occurred_at: attempt.occurred_at.clone(),
        endpoint: attempt.endpoint.clone(),
        sticky_key: attempt.sticky_key.clone(),
        upstream_account_id: attempt.upstream_account_id,
        upstream_account_name: attempt.upstream_account_name.clone(),
        request_model: record.request_model.clone(),
        response_model: record
            .response_model
            .clone()
            .or_else(|| record.model.clone()),
        upstream_route_key: attempt.upstream_route_key.clone(),
        proxy_binding_key_snapshot: attempt.proxy_binding_key_snapshot.clone(),
        attempt_index: attempt.attempt_index,
        distinct_account_index: attempt.distinct_account_index,
        same_account_retry_index: attempt.same_account_retry_index,
        requester_ip: attempt
            .requester_ip
            .clone()
            .or_else(|| record.requester_ip.clone()),
        started_at: normalize_optional_timestamp(attempt.started_at.as_deref()),
        finished_at: normalize_optional_timestamp(attempt.finished_at.as_deref()),
        status: attempt.status.clone(),
        phase: attempt.phase.clone(),
        http_status: attempt.http_status,
        downstream_http_status: attempt.downstream_http_status,
        failure_kind: attempt
            .failure_kind
            .clone()
            .or_else(|| record.failure_kind.clone()),
        error_message: attempt
            .error_message
            .clone()
            .or_else(|| record.error_message.clone()),
        downstream_error_message: attempt
            .downstream_error_message
            .clone()
            .or_else(|| record.downstream_error_message.clone()),
        connect_latency_ms: attempt.connect_latency_ms.or(record.t_upstream_connect_ms),
        first_byte_latency_ms: attempt.first_byte_latency_ms.or(record.t_upstream_ttfb_ms),
        stream_latency_ms: attempt.stream_latency_ms.or(record.t_upstream_stream_ms),
        upstream_request_id: attempt
            .upstream_request_id
            .clone()
            .or_else(|| record.upstream_request_id.clone()),
        request_summary: parse_summary_json_or_fallback(
            attempt.request_summary_json.as_deref(),
            || build_attempt_request_summary(record, attempt, payload),
        ),
        response_summary: parse_summary_json_or_fallback(
            attempt.response_summary_json.as_deref(),
            || build_attempt_response_summary(record, attempt, payload, usage_cost_audit),
        ),
    }
}

fn build_synthetic_workflow_attempt(
    record: &ApiInvocation,
    payload: Option<&Value>,
    usage_cost_audit: Option<&InvocationCostAudit>,
) -> InvocationWorkflowAttempt {
    let request_compression = request_compression_value(
        payload_string(payload, &["requestCompressionAlgorithm"]),
        payload_string(payload, &["requestCompressionMode"]),
        derive_request_compression_from_payload(payload),
    );
    let request_summary = json!({
        "endpoint": record.endpoint.clone(),
        "routeMode": record.route_mode.clone(),
        "transport": record.transport.clone(),
        "requestModel": record.request_model.clone(),
        "responseModel": record.response_model.as_ref().or(record.model.as_ref()),
        "requestedServiceTier": record.requested_service_tier.clone(),
        "reasoningEffort": record.reasoning_effort.clone(),
        "compactionRequestKind": record.compaction_request_kind.clone(),
        "imageIntent": record.image_intent.clone(),
        "promptCacheKey": record.prompt_cache_key.clone(),
        "stickyKey": record.sticky_key.clone(),
        "requesterIp": record.requester_ip.clone(),
        "account": {
            "id": record.upstream_account_id,
            "name": record.upstream_account_name.clone(),
        },
        "routing": {
            "proxyDisplayName": record.proxy_display_name.clone(),
            "upstreamScope": payload_string(payload, &["upstreamScope"]),
            "clientFingerprint": payload_string(payload, &["clientFingerprint"]),
            "oauthForwardedHeaderNames": payload_string_array(payload, &["oauthForwardedHeaderNames"]),
            "oauthPromptCacheHeaderForwarded": payload_bool(payload, &["oauthPromptCacheHeaderForwarded"]),
        },
        "headers": build_request_header_snapshot(payload),
        "client": build_request_client_snapshot(payload),
        "compression": request_compression,
        "bodyCapture": {
            "availableAtInvocationLevel": record.request_raw_path.is_some(),
            "size": record.request_raw_size,
            "truncated": record.request_raw_truncated.unwrap_or_default() != 0,
            "truncatedReason": record.request_raw_truncated_reason.clone(),
            "detailLevel": record.detail_level.clone(),
            "detailPruneReason": record.detail_prune_reason.clone(),
        },
    });
    let response_summary = json!({
        "status": record.status.clone(),
        "phase": record.live_phase.clone(),
        "downstreamHttpStatus": record.downstream_status_code,
        "failureKind": record.failure_kind.clone(),
        "errorMessage": record.error_message.clone(),
        "downstreamErrorMessage": record.downstream_error_message.clone(),
        "upstreamRequestId": record.upstream_request_id.clone(),
        "upstreamErrorCode": record.upstream_error_code.clone(),
        "upstreamErrorMessage": record.upstream_error_message.clone(),
        "streamTerminalEvent": record.stream_terminal_event.clone(),
        "responseContentEncoding": record.response_content_encoding.clone(),
        "serviceTier": record.service_tier.clone(),
        "billingServiceTier": record.billing_service_tier.clone(),
        "headers": build_response_header_snapshot(record, payload),
        "delivery": build_response_delivery_snapshot(payload),
        "latencyMs": {
            "connect": record.t_upstream_connect_ms,
            "firstByte": record.t_upstream_ttfb_ms,
            "stream": record.t_upstream_stream_ms,
            "requestRead": record.t_req_read_ms,
            "requestParse": record.t_req_parse_ms,
            "responseParse": record.t_resp_parse_ms,
            "persist": record.t_persist_ms,
            "total": record.t_total_ms,
        },
        "responseBodyCapture": {
            "availableAtInvocationLevel": record.response_raw_path.is_some(),
            "size": record.response_raw_size,
            "truncated": record.response_raw_truncated.unwrap_or_default() != 0,
            "truncatedReason": record.response_raw_truncated_reason.clone(),
            "detailLevel": record.detail_level.clone(),
            "detailPruneReason": record.detail_prune_reason.clone(),
        },
        "usage": usage_cost_audit.map(|audit| build_invocation_usage_summary(record, audit)),
    });
    InvocationWorkflowAttempt {
        synthetic: true,
        attempt_id: None,
        occurred_at: record.occurred_at.clone(),
        endpoint: record.endpoint.clone().unwrap_or_default(),
        sticky_key: record.sticky_key.clone(),
        upstream_account_id: record.upstream_account_id,
        upstream_account_name: record.upstream_account_name.clone(),
        request_model: record.request_model.clone(),
        response_model: record
            .response_model
            .clone()
            .or_else(|| record.model.clone()),
        upstream_route_key: None,
        proxy_binding_key_snapshot: None,
        attempt_index: 1,
        distinct_account_index: 1,
        same_account_retry_index: 1,
        requester_ip: record.requester_ip.clone(),
        started_at: Some(record.occurred_at.clone()),
        finished_at: record.t_total_ms.and_then(|total| {
            parse_to_utc_datetime(&record.occurred_at).and_then(|occurred_at| {
                chrono::Duration::from_std(Duration::from_secs_f64(total.max(0.0) / 1000.0))
                    .ok()
                    .map(|delta| format_utc_iso(occurred_at + delta))
            })
        }),
        status: record
            .status
            .clone()
            .unwrap_or_else(|| "unknown".to_string()),
        phase: record.live_phase.clone(),
        http_status: None,
        downstream_http_status: record.downstream_status_code,
        failure_kind: record.failure_kind.clone(),
        error_message: record.error_message.clone(),
        downstream_error_message: record.downstream_error_message.clone(),
        connect_latency_ms: record.t_upstream_connect_ms,
        first_byte_latency_ms: record.t_upstream_ttfb_ms,
        stream_latency_ms: record.t_upstream_stream_ms,
        upstream_request_id: record.upstream_request_id.clone(),
        request_summary: Some(request_summary),
        response_summary: Some(response_summary),
    }
}

fn workflow_attempt_account_label(attempt: &InvocationWorkflowAttempt) -> String {
    attempt
        .upstream_account_name
        .clone()
        .or_else(|| attempt.upstream_account_id.map(|id| format!("账号 #{id}")))
        .unwrap_or_else(|| "未定账号".to_string())
}

fn workflow_route_subtitle(attempt: &InvocationWorkflowAttempt) -> Option<String> {
    let mut parts = Vec::new();
    if let Some(model) = attempt
        .request_model
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        parts.push(model.to_string());
    }
    if let Some(proxy) = attempt
        .proxy_binding_key_snapshot
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        parts.push(proxy.to_string());
    }
    if let Some(route_key) = attempt
        .upstream_route_key
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        parts.push(route_key.to_string());
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join(" · "))
    }
}

fn workflow_route_title(attempt: &InvocationWorkflowAttempt) -> String {
    let account_label = workflow_attempt_account_label(attempt);
    if account_label == "未定账号" {
        "Route resolution".to_string()
    } else {
        format!("Route {account_label}")
    }
}

fn build_routing_detail(request_summary: Option<&Value>) -> Option<Value> {
    let request = request_summary?.clone();
    let request_headers = request
        .as_object()
        .and_then(|value| value.get("headers"))
        .cloned();
    let request_body = request
        .as_object()
        .and_then(|value| value.get("bodyCapture"))
        .cloned();
    Some(json!({
        "request": request,
        "requestHeaders": request_headers,
        "requestBody": request_body,
    }))
}

fn build_routing_timeline_entry(
    block_id: String,
    attempt: &InvocationWorkflowAttempt,
) -> InvocationWorkflowTimelineEntry {
    InvocationWorkflowTimelineEntry {
        block_id,
        kind: "routingDecision".to_string(),
        occurred_at: attempt
            .started_at
            .clone()
            .or_else(|| Some(attempt.occurred_at.clone())),
        title: workflow_route_title(attempt),
        subtitle: workflow_route_subtitle(attempt)
            .or_else(|| (!attempt.endpoint.trim().is_empty()).then(|| attempt.endpoint.clone())),
        status: None,
        attempt: None,
        detail: build_routing_detail(attempt.request_summary.as_ref()),
        response_body: None,
    }
}

fn invocation_workflow_attempt_row_is_pseudo_terminal(
    attempt: &InvocationWorkflowAttemptRow,
) -> bool {
    normalized_runtime_text(Some(attempt.status.as_str())) == "budget_exhausted_final"
        && attempt.same_account_retry_index == 0
        && normalize_optional_timestamp(attempt.started_at.as_deref())
            == normalize_optional_timestamp(attempt.finished_at.as_deref())
        && attempt
            .connect_latency_ms
            .is_none_or(|value| !value.is_finite() || value <= 0.0)
        && attempt
            .first_byte_latency_ms
            .is_none_or(|value| !value.is_finite() || value <= 0.0)
        && attempt
            .stream_latency_ms
            .is_none_or(|value| !value.is_finite() || value <= 0.0)
        && attempt
            .upstream_request_id
            .as_deref()
            .map(str::trim)
            .is_none_or(str::is_empty)
        && attempt
            .request_summary_json
            .as_deref()
            .map(str::trim)
            .is_none_or(str::is_empty)
        && attempt
            .response_summary_json
            .as_deref()
            .map(str::trim)
            .is_none_or(str::is_empty)
}

fn last_success_like_attempt_index(attempt_rows: &[&InvocationWorkflowAttemptRow]) -> Option<i64> {
    attempt_rows
        .iter()
        .rfind(|attempt| {
            matches!(
                normalized_runtime_text(Some(attempt.status.as_str())).as_str(),
                "success" | "completed" | "warning_success"
            )
        })
        .map(|attempt| attempt.attempt_index)
}

fn build_workflow_timeline_entries(
    record: &ApiInvocation,
    attempts: &[InvocationWorkflowAttempt],
    route_only_attempt: Option<&InvocationWorkflowAttempt>,
    failure_entry: Option<InvocationWorkflowTimelineEntry>,
) -> Vec<InvocationWorkflowTimelineEntry> {
    let mut entries = Vec::new();
    if let Some(route_only_attempt) = route_only_attempt {
        entries.push(build_routing_timeline_entry(
            route_only_attempt
                .attempt_id
                .clone()
                .map(|attempt_id| format!("route-{attempt_id}"))
                .unwrap_or_else(|| "route-terminal".to_string()),
            route_only_attempt,
        ));
    } else if attempts.len() == 1 && attempts[0].synthetic {
        let attempt = attempts[0].clone();
        entries.push(InvocationWorkflowTimelineEntry {
            block_id: "attempt-direct".to_string(),
            kind: "attempt".to_string(),
            occurred_at: Some(attempt.occurred_at.clone()),
            title: "Direct attempt".to_string(),
            subtitle: Some(attempt.endpoint.clone()),
            status: Some(attempt.status.clone()),
            attempt: Some(attempt),
            detail: None,
            response_body: None,
        });
    } else {
        let mut previous_finished_at: Option<DateTime<Utc>> = None;
        let mut previous_attempt_id: Option<String> = None;
        for attempt in attempts {
            if let Some(started_at) = attempt
                .started_at
                .as_deref()
                .and_then(parse_to_utc_datetime)
                && let Some(previous_finished) = previous_finished_at
            {
                let gap_ms = (started_at - previous_finished).num_milliseconds();
                if gap_ms > 0 {
                    entries.push(InvocationWorkflowTimelineEntry {
                        block_id: format!(
                            "wait-{}",
                            attempt
                                .attempt_id
                                .clone()
                                .unwrap_or_else(|| attempt.attempt_index.to_string())
                        ),
                        kind: "routingWait".to_string(),
                        occurred_at: Some(format_utc_iso(started_at)),
                        title: "Retry wait".to_string(),
                        subtitle: Some(format!("{} ms", gap_ms)),
                        status: None,
                        attempt: None,
                        detail: Some(json!({
                            "durationMs": gap_ms,
                            "fromAttemptId": previous_attempt_id.clone(),
                            "toAttemptId": attempt.attempt_id.clone(),
                        })),
                        response_body: None,
                    });
                }
            }

            entries.push(build_routing_timeline_entry(
                format!(
                    "route-{}",
                    attempt
                        .attempt_id
                        .clone()
                        .unwrap_or_else(|| attempt.attempt_index.to_string())
                ),
                attempt,
            ));

            entries.push(InvocationWorkflowTimelineEntry {
                block_id: format!(
                    "attempt-{}",
                    attempt
                        .attempt_id
                        .clone()
                        .unwrap_or_else(|| attempt.attempt_index.to_string())
                ),
                kind: "attempt".to_string(),
                occurred_at: Some(attempt.occurred_at.clone()),
                title: format!("Attempt #{}", attempt.attempt_index),
                subtitle: Some(workflow_attempt_account_label(attempt)),
                status: Some(attempt.status.clone()),
                attempt: Some(attempt.clone()),
                detail: None,
                response_body: None,
            });

            previous_finished_at = attempt
                .finished_at
                .as_deref()
                .and_then(parse_to_utc_datetime);
            previous_attempt_id = attempt.attempt_id.clone();
        }
    }

    if let Some(failure_entry) = failure_entry {
        entries.push(failure_entry);
    }
    if entries.is_empty() && !invocation_status_is_success_like(record) {
        entries.push(InvocationWorkflowTimelineEntry {
            block_id: "failure-only".to_string(),
            kind: "systemFinalFailure".to_string(),
            occurred_at: Some(record.occurred_at.clone()),
            title: "Final downstream response".to_string(),
            subtitle: record.failure_kind.clone(),
            status: record.status.clone(),
            attempt: None,
            detail: Some(json!({
                "downstreamStatusCode": record.downstream_status_code,
                "failureKind": record.failure_kind.clone(),
                "errorMessage": record.error_message.clone(),
                "downstreamErrorMessage": record.downstream_error_message.clone(),
            })),
            response_body: None,
        });
    }
    entries
}

async fn load_invocation_workflow_identity(
    pool: &Pool<Sqlite>,
    id: i64,
) -> Result<Option<InvocationWorkflowIdentityRow>, ApiError> {
    sqlx::query_as::<_, InvocationWorkflowIdentityRow>(
        r#"
        SELECT id, invoke_id, occurred_at, timeline_json
        FROM codex_invocations
        WHERE id = ?1
        LIMIT 1
        "#,
    )
    .bind(id)
    .fetch_optional(pool)
    .await
    .map_err(ApiError::from)
}

async fn query_invocation_workflow_attempt_rows(
    pool: &Pool<Sqlite>,
    invoke_id: &str,
    occurred_at: &str,
) -> Result<Vec<InvocationWorkflowAttemptRow>, ApiError> {
    sqlx::query_as::<_, InvocationWorkflowAttemptRow>(
        r#"
        SELECT
            attempts.attempt_public_id AS attempt_id,
            attempts.invoke_id,
            attempts.occurred_at,
            attempts.endpoint,
            attempts.sticky_key,
            attempts.upstream_account_id,
            accounts.display_name AS upstream_account_name,
            attempts.upstream_route_key,
            attempts.proxy_binding_key_snapshot,
            attempts.attempt_index,
            attempts.distinct_account_index,
            attempts.same_account_retry_index,
            attempts.requester_ip,
            attempts.started_at,
            attempts.finished_at,
            attempts.status,
            COALESCE(
                attempts.phase,
                CASE
                    WHEN attempts.status = 'pending' THEN 'sending_request'
                    WHEN attempts.status = 'success' THEN 'completed'
                    ELSE 'failed'
                END
            ) AS phase,
            attempts.http_status,
            attempts.downstream_http_status,
            attempts.failure_kind,
            attempts.error_message,
            attempts.downstream_error_message,
            attempts.connect_latency_ms,
            attempts.first_byte_latency_ms,
            attempts.stream_latency_ms,
            attempts.upstream_request_id,
            attempts.upstream_request_compression_algorithm,
            attempts.upstream_request_compression_mode,
            attempts.upstream_request_logical_body_bytes,
            attempts.upstream_request_transmitted_body_bytes,
            attempts.upstream_request_header_bytes_approx,
            attempts.upstream_response_body_bytes,
            attempts.upstream_response_header_bytes_approx,
            attempts.compact_support_status,
            attempts.compact_support_reason,
            attempts.request_summary_json,
            attempts.response_summary_json
        FROM pool_upstream_request_attempts AS attempts
        LEFT JOIN pool_upstream_accounts AS accounts
            ON accounts.id = attempts.upstream_account_id
        WHERE attempts.invoke_id = ?1
          AND attempts.occurred_at = ?2
        ORDER BY attempts.attempt_index ASC, attempts.id ASC
        "#,
    )
    .bind(invoke_id)
    .bind(occurred_at)
    .fetch_all(pool)
    .await
    .map_err(ApiError::from)
}

fn build_final_failure_timeline_entry(
    record: &ApiInvocation,
    body_row: Option<&InvocationResponseBodyRow>,
    raw_path_fallback_root: Option<&Path>,
) -> Option<InvocationWorkflowTimelineEntry> {
    if invocation_status_is_success_like(record) {
        return None;
    }

    let response_body = body_row.map(|row| {
        match resolve_response_body_text_from_row(row, raw_path_fallback_root) {
            Ok((text, _)) => InvocationWorkflowResponseBody {
                available: true,
                body_text: Some(text),
                unavailable_reason: None,
            },
            Err(reason) => InvocationWorkflowResponseBody {
                available: false,
                body_text: None,
                unavailable_reason: Some(reason),
            },
        }
    });

    let occurred_at = record
        .t_total_ms
        .and_then(|total| {
            parse_to_utc_datetime(&record.occurred_at).and_then(|started_at| {
                chrono::Duration::from_std(Duration::from_secs_f64(total.max(0.0) / 1000.0))
                    .ok()
                    .map(|delta| format_utc_iso(started_at + delta))
            })
        })
        .or_else(|| Some(record.occurred_at.clone()));

    Some(InvocationWorkflowTimelineEntry {
        block_id: "system-final-failure".to_string(),
        kind: "systemFinalFailure".to_string(),
        occurred_at,
        title: "Final downstream response".to_string(),
        subtitle: record
            .failure_kind
            .clone()
            .or_else(|| record.failure_class.clone()),
        status: record.status.clone(),
        attempt: None,
        detail: Some(json!({
            "invokeId": record.invoke_id.clone(),
            "downstreamStatusCode": record.downstream_status_code,
            "failureClass": record.failure_class.clone(),
            "failureKind": record.failure_kind.clone(),
            "errorMessage": record.error_message.clone(),
            "downstreamErrorMessage": record.downstream_error_message.clone(),
            "upstreamErrorCode": record.upstream_error_code.clone(),
            "upstreamErrorMessage": record.upstream_error_message.clone(),
            "upstreamRequestId": record.upstream_request_id.clone(),
            "streamTerminalEvent": record.stream_terminal_event.clone(),
            "responseContentEncoding": record.response_content_encoding.clone(),
        })),
        response_body,
    })
}

pub(crate) async fn fetch_invocation_workflow_detail(
    State(state): State<Arc<AppState>>,
    axum::extract::Path(id): axum::extract::Path<i64>,
) -> Result<Json<InvocationWorkflowDetailResponse>, ApiError> {
    let identity = load_invocation_workflow_identity(&state.pool, id)
        .await?
        .ok_or_else(|| ApiError::bad_request(anyhow!("record not found")))?;
    let record =
        load_persisted_api_invocation(&state.pool, &identity.invoke_id, &identity.occurred_at)
            .await
            .map_err(ApiError::from)?;
    let body_row = fetch_invocation_response_body_row_by_id(&state.pool, id).await?;
    let payload_value = body_row
        .as_ref()
        .and_then(|row| parse_optional_json_value(row.payload.as_deref()));
    let attempt_rows = query_invocation_workflow_attempt_rows(
        &state.pool,
        &identity.invoke_id,
        &identity.occurred_at,
    )
    .await?;
    let pseudo_attempt_rows = attempt_rows
        .iter()
        .filter(|attempt| invocation_workflow_attempt_row_is_pseudo_terminal(attempt))
        .collect::<Vec<_>>();
    let real_attempt_rows = attempt_rows
        .iter()
        .filter(|attempt| !invocation_workflow_attempt_row_is_pseudo_terminal(attempt))
        .collect::<Vec<_>>();
    let last_success_attempt_index = last_success_like_attempt_index(&real_attempt_rows);
    let pricing_catalog = state.pricing_catalog.read().await.clone();
    let usage_cost_audit = (invocation_status_is_success_like(&record)
        && invocation_has_usage_evidence(&record))
    .then(|| build_invocation_cost_audit(&record, &pricing_catalog, true))
    .flatten();
    let pool_route = normalized_runtime_text(record.route_mode.as_deref()) == "pool";
    let render_route_only = pool_route
        && !invocation_status_is_success_like(&record)
        && real_attempt_rows.is_empty()
        && (!pseudo_attempt_rows.is_empty() || record.pool_attempt_count.unwrap_or_default() == 0);
    let route_only_attempt = render_route_only.then(|| {
        pseudo_attempt_rows
            .last()
            .map(|attempt| {
                build_workflow_attempt_from_row(&record, attempt, payload_value.as_ref(), None)
            })
            .unwrap_or_else(|| {
                build_synthetic_workflow_attempt(&record, payload_value.as_ref(), None)
            })
    });
    let attempts = if real_attempt_rows.is_empty() {
        if route_only_attempt.is_some() {
            Vec::new()
        } else {
            vec![build_synthetic_workflow_attempt(
                &record,
                payload_value.as_ref(),
                usage_cost_audit.as_ref(),
            )]
        }
    } else {
        real_attempt_rows
            .iter()
            .map(|attempt| {
                build_workflow_attempt_from_row(
                    &record,
                    attempt,
                    payload_value.as_ref(),
                    (last_success_attempt_index == Some(attempt.attempt_index))
                        .then_some(usage_cost_audit.as_ref())
                        .flatten(),
                )
            })
            .collect::<Vec<_>>()
    };
    let failure_entry = build_final_failure_timeline_entry(
        &record,
        body_row.as_ref(),
        state.config.database_path.parent(),
    );
    let partial = pool_route
        && record.pool_attempt_count.unwrap_or_default() > 0
        && real_attempt_rows.is_empty()
        && pseudo_attempt_rows.is_empty();
    let timeline_attempt_count = attempts.len();
    let response = InvocationWorkflowDetailResponse {
        hero: build_workflow_hero(&record, timeline_attempt_count),
        timeline: build_workflow_timeline_entries(
            &record,
            &attempts,
            route_only_attempt.as_ref(),
            failure_entry,
        ),
        reconstructed: identity.timeline_json.is_none(),
        partial,
        partial_reason: partial.then(|| "attempt_rows_missing".to_string()),
    };
    Ok(Json(response))
}

#[derive(Debug, FromRow)]
pub(crate) struct InvocationResponseBodyRow {
    pub(crate) id: i64,
    pub(crate) invoke_id: String,
    pub(crate) payload: Option<String>,
    pub(crate) raw_response: String,
    pub(crate) request_raw_path: Option<String>,
    pub(crate) request_raw_size: Option<i64>,
    pub(crate) request_raw_truncated: Option<i64>,
    pub(crate) request_raw_truncated_reason: Option<String>,
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

pub(crate) fn raw_request_fallback_reason(row: &InvocationResponseBodyRow) -> String {
    if row.detail_level == DETAIL_LEVEL_STRUCTURED_ONLY {
        "detail_pruned".to_string()
    } else if row.request_raw_truncated.unwrap_or_default() != 0 {
        row.request_raw_truncated_reason
            .as_deref()
            .filter(|value| !value.trim().is_empty())
            .map(ToOwned::to_owned)
            .unwrap_or_else(|| "capture_unavailable".to_string())
    } else {
        "missing_body".to_string()
    }
}

pub(crate) fn resolve_request_body_text_from_row(
    row: &InvocationResponseBodyRow,
    raw_path_fallback_root: Option<&Path>,
) -> Result<(String, bool), String> {
    let Some(path) = row.request_raw_path.as_deref() else {
        return Err(raw_request_fallback_reason(row));
    };

    match read_proxy_raw_bytes(path, raw_path_fallback_root) {
        Ok(bytes) => Ok((String::from_utf8_lossy(&bytes).to_string(), true)),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Err("raw_file_missing".to_string()),
        Err(err) => Err(format!("raw_file_unreadable:{err}")),
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
         invoke_id, \
         payload, \
         raw_response, \
         request_raw_path, \
         request_raw_size, \
         request_raw_truncated, \
         request_raw_truncated_reason, \
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

fn build_request_body_capture_summary(
    row: &InvocationResponseBodyRow,
    capture_source: Option<&str>,
) -> Value {
    json!({
        "source": capture_source,
        "size": row.request_raw_size,
        "truncated": row.request_raw_truncated.unwrap_or_default() != 0,
        "truncatedReason": row.request_raw_truncated_reason.clone(),
        "detailLevel": Some(row.detail_level.clone()),
        "detailPruneReason": row.detail_prune_reason.clone(),
    })
}

fn build_response_body_capture_summary(
    row: &InvocationResponseBodyRow,
    capture_source: Option<&str>,
) -> Value {
    json!({
        "source": capture_source,
        "size": row.response_raw_size,
        "truncated": row.response_raw_truncated.unwrap_or_default() != 0,
        "truncatedReason": row.response_raw_truncated_reason.clone(),
        "detailLevel": Some(row.detail_level.clone()),
        "detailPruneReason": row.detail_prune_reason.clone(),
    })
}

fn build_request_body_routing_snapshot(
    row: &InvocationResponseBodyRow,
    payload: Option<&Value>,
) -> Value {
    json!({
        "routeMode": payload_string(payload, &["routeMode"]),
        "upstreamScope": payload_string(payload, &["upstreamScope"]),
        "stickyKey": payload_string(payload, &["stickyKey"]),
        "promptCacheKey": payload_string(payload, &["promptCacheKey"]),
        "proxyDisplayName": payload_string(payload, &["proxyDisplayName"]),
        "clientFingerprint": payload_string(payload, &["clientFingerprint"]),
        "clientHeaderFingerprints": payload_clone(payload, &["clientHeaderFingerprints"]),
        "oauthForwardedHeaderNames": payload_string_array(payload, &["oauthForwardedHeaderNames"]),
        "oauthPromptCacheHeaderForwarded": payload_bool(payload, &["oauthPromptCacheHeaderForwarded"]),
        "client": build_request_client_snapshot(payload),
        "invokeId": Some(row.invoke_id.clone()),
    })
}

fn build_response_body_header_snapshot(
    row: &InvocationResponseBodyRow,
    payload: Option<&Value>,
) -> Value {
    json!({
        "contentEncoding": row
            .response_content_encoding
            .clone()
            .or_else(|| payload_string(payload, &["responseContentEncoding"])),
        "contentEncodingChain": payload_string(payload, &["contentEncodingChain"]),
        "upstreamRequestId": payload_string(payload, &["upstreamRequestId"]),
        "cvmInvokeId": Some(row.invoke_id.clone()),
    })
}

fn build_request_body_response(
    row: &InvocationResponseBodyRow,
    raw_path_fallback_root: Option<&Path>,
) -> InvocationResponseBodyResponse {
    let payload = parse_optional_json_value(row.payload.as_deref());
    let headers = Some(build_request_header_snapshot(payload.as_ref()));
    let routing = Some(build_request_body_routing_snapshot(row, payload.as_ref()));
    match resolve_request_body_text_from_row(row, raw_path_fallback_root) {
        Ok((body_text, from_full_body)) => InvocationResponseBodyResponse {
            available: true,
            body_text: Some(body_text),
            unavailable_reason: None,
            headers,
            routing,
            body_size: row.request_raw_size,
            body_truncated: Some(row.request_raw_truncated.unwrap_or_default() != 0),
            body_truncated_reason: row.request_raw_truncated_reason.clone(),
            detail_level: Some(row.detail_level.clone()),
            detail_prune_reason: row.detail_prune_reason.clone(),
            capture_source: Some(
                if from_full_body {
                    "raw_file"
                } else {
                    "preview"
                }
                .to_string(),
            ),
        },
        Err(reason) => InvocationResponseBodyResponse {
            available: false,
            body_text: None,
            unavailable_reason: Some(reason),
            headers,
            routing,
            body_size: row.request_raw_size,
            body_truncated: Some(row.request_raw_truncated.unwrap_or_default() != 0),
            body_truncated_reason: row.request_raw_truncated_reason.clone(),
            detail_level: Some(row.detail_level.clone()),
            detail_prune_reason: row.detail_prune_reason.clone(),
            capture_source: None,
        },
    }
}

fn build_response_body_response(
    row: &InvocationResponseBodyRow,
    raw_path_fallback_root: Option<&Path>,
) -> InvocationResponseBodyResponse {
    let payload = parse_optional_json_value(row.payload.as_deref());
    let headers = Some(build_response_body_header_snapshot(row, payload.as_ref()));
    let routing = Some(build_response_delivery_snapshot(payload.as_ref()));
    match resolve_response_body_text_from_row(row, raw_path_fallback_root) {
        Ok((body_text, from_full_body)) => InvocationResponseBodyResponse {
            available: true,
            body_text: Some(body_text),
            unavailable_reason: None,
            headers,
            routing,
            body_size: row.response_raw_size,
            body_truncated: Some(row.response_raw_truncated.unwrap_or_default() != 0),
            body_truncated_reason: row.response_raw_truncated_reason.clone(),
            detail_level: Some(row.detail_level.clone()),
            detail_prune_reason: row.detail_prune_reason.clone(),
            capture_source: Some(
                if from_full_body {
                    "raw_file"
                } else {
                    "preview"
                }
                .to_string(),
            ),
        },
        Err(reason) => InvocationResponseBodyResponse {
            available: false,
            body_text: None,
            unavailable_reason: Some(reason),
            headers,
            routing,
            body_size: row.response_raw_size,
            body_truncated: Some(row.response_raw_truncated.unwrap_or_default() != 0),
            body_truncated_reason: row.response_raw_truncated_reason.clone(),
            detail_level: Some(row.detail_level.clone()),
            detail_prune_reason: row.detail_prune_reason.clone(),
            capture_source: None,
        },
    }
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
    Ok(Json(build_response_body_response(
        &row,
        state.config.database_path.parent(),
    )))
}

pub(crate) async fn fetch_invocation_request_body(
    State(state): State<Arc<AppState>>,
    axum::extract::Path(id): axum::extract::Path<i64>,
) -> Result<Json<InvocationResponseBodyResponse>, ApiError> {
    let row = fetch_invocation_response_body_row_by_id(&state.pool, id)
        .await?
        .ok_or_else(|| ApiError::bad_request(anyhow!("record not found")))?;
    Ok(Json(build_request_body_response(
        &row,
        state.config.database_path.parent(),
    )))
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
         COALESCE(SUM(CASE WHEN {resolved_failure} = 'none' AND ({status_norm} IN ('success', 'completed', '{warning_success}') OR ({status_norm} = 'http_200' AND LOWER(TRIM(COALESCE(error_message, ''))) = '')) THEN 1 ELSE 0 END), 0) AS success_count, \
         COALESCE(SUM(CASE WHEN {resolved_failure} IN ('service_failure', 'client_failure', 'client_abort') THEN 1 ELSE 0 END), 0) AS failure_count, \
         COALESCE(SUM(total_tokens), 0) AS total_tokens, \
         COALESCE(SUM(cost), 0.0) AS total_cost, \
         COALESCE(SUM(MAX(COALESCE(input_tokens, 0) - COALESCE(cache_input_tokens, 0), 0)), 0) AS cache_write_tokens, \
         COALESCE(SUM(cache_input_tokens), 0) AS cache_input_tokens, \
         COALESCE(SUM(output_tokens), 0) AS output_tokens \
         FROM codex_invocations WHERE 1 = 1",
        status_norm = INVOCATION_STATUS_NORMALIZED_SQL,
        resolved_failure = INVOCATION_RESOLVED_FAILURE_CLASS_SQL,
        warning_success = INVOCATION_STATUS_WARNING_SUCCESS,
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
    let cache_write_tokens =
        (totals.cache_write_tokens + runtime_overlay_delta.cache_write_tokens).max(0);
    let cache_input_tokens =
        (totals.cache_input_tokens + runtime_overlay_delta.cache_input_tokens).max(0);
    let output_tokens = (totals.output_tokens + runtime_overlay_delta.output_tokens).max(0);
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
            cache_write_tokens,
            cache_input_tokens,
            output_tokens,
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
    latest_first_response_byte_total_at: Option<String>,
    latest_first_response_byte_total_ms: Option<f64>,
    latest_avg_total_at: Option<String>,
    latest_avg_total_ms: Option<f64>,
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

#[derive(Debug, Clone, Default)]
struct LatestTimedMetricValue {
    at: Option<String>,
    value: Option<f64>,
}

impl LatestTimedMetricValue {
    fn update(&mut self, candidate_at: Option<String>, candidate_value: Option<f64>) {
        let Some(candidate_at) = candidate_at else {
            return;
        };
        let Some(candidate_value) = candidate_value.filter(|value| value.is_finite()) else {
            return;
        };
        let replace = match self.at.as_deref() {
            Some(current_at) => candidate_at.as_str() >= current_at,
            None => true,
        };
        if replace {
            self.at = Some(candidate_at);
            self.value = Some(candidate_value);
        }
    }
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

    fn add_coarse_rollup_totals(
        &mut self,
        cache_write_tokens: i64,
        cache_read_tokens: i64,
        output_tokens: i64,
        total_cost: f64,
    ) {
        self.cache_write_tokens += cache_write_tokens.max(0);
        self.cache_read_tokens += cache_read_tokens.max(0);
        self.output_tokens += output_tokens.max(0);
        if total_cost > 0.0 {
            self.add_cost_row(Some(total_cost), [None, None, None, None, None]);
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

#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct DashboardActivityCurrentMinuteAccumulator {
    qualified_tokens: i64,
    total_cost: f64,
    first_response_byte_total_sample_count: i64,
    first_response_byte_total_sum_ms: f64,
    total_latency_sample_count: i64,
    total_latency_sum_ms: f64,
}

impl DashboardActivityCurrentMinuteAccumulator {
    fn add_row(&mut self, row: &UpstreamAccountInvocationPreviewRow) {
        self.total_cost += row.cost.unwrap_or_default().max(0.0);

        let classification = resolve_failure_classification(
            Some(row.status.as_str()),
            row.error_message.as_deref(),
            row.failure_kind.as_deref(),
            row.failure_class.as_deref(),
            row.is_actionable,
        );
        let is_success =
            prompt_cache_and_timeseries_shared::prompt_invocation_status_is_success_like(
                Some(row.status.as_str()),
                row.error_message.as_deref(),
            ) && classification.failure_class == FailureClass::None;
        let is_qualified_tpm = is_success && row.cost.is_some();

        if is_qualified_tpm {
            self.qualified_tokens += row.total_tokens.max(0);
        }
        if !is_success {
            return;
        }

        if let Some(first_response_byte_total_ms) =
            crate::stats::resolve_first_response_byte_total_ms(
                row.t_req_read_ms,
                row.t_req_parse_ms,
                row.t_upstream_connect_ms,
                row.t_upstream_ttfb_ms,
            )
        {
            self.first_response_byte_total_sample_count += 1;
            self.first_response_byte_total_sum_ms += first_response_byte_total_ms;
        }
        if let Some(total_ms) = row
            .t_total_ms
            .filter(|value| value.is_finite() && *value >= 0.0)
        {
            self.total_latency_sample_count += 1;
            self.total_latency_sum_ms += total_ms;
        }
    }

    fn merge(&mut self, other: Self) {
        self.qualified_tokens += other.qualified_tokens;
        self.total_cost += other.total_cost;
        self.first_response_byte_total_sample_count += other.first_response_byte_total_sample_count;
        self.first_response_byte_total_sum_ms += other.first_response_byte_total_sum_ms;
        self.total_latency_sample_count += other.total_latency_sample_count;
        self.total_latency_sum_ms += other.total_latency_sum_ms;
    }

    fn first_response_byte_total_avg_ms(&self) -> Option<f64> {
        (self.first_response_byte_total_sample_count > 0).then_some(
            self.first_response_byte_total_sum_ms
                / self.first_response_byte_total_sample_count as f64,
        )
    }

    fn avg_total_ms(&self) -> Option<f64> {
        (self.total_latency_sample_count > 0)
            .then_some(self.total_latency_sum_ms / self.total_latency_sample_count as f64)
    }

    fn into_current_snapshot(self) -> DashboardActivityCurrentSnapshot {
        DashboardActivityCurrentSnapshot {
            qualified_tokens: self.qualified_tokens,
            total_cost: self.total_cost,
            first_response_byte_total_sample_count: self.first_response_byte_total_sample_count,
            first_response_byte_total_sum_ms: self.first_response_byte_total_sum_ms,
            total_latency_sample_count: self.total_latency_sample_count,
            total_latency_sum_ms: self.total_latency_sum_ms,
        }
    }
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

fn merge_latest_timed_metric(
    current_at: &mut Option<String>,
    current_value: &mut Option<f64>,
    candidate_at: Option<String>,
    candidate_value: Option<f64>,
) {
    let mut latest = LatestTimedMetricValue {
        at: current_at.take(),
        value: current_value.take(),
    };
    latest.update(candidate_at, candidate_value);
    *current_at = latest.at;
    *current_value = latest.value;
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
    latest_first_response_byte_total_at: Option<String>,
    latest_first_response_byte_total_ms: Option<f64>,
    latest_avg_total_at: Option<String>,
    latest_avg_total_ms: Option<f64>,
}

fn merge_upstream_account_activity_aggregate_row(
    entry: &mut UpstreamAccountActivityAccumulator,
    row: &UpstreamAccountActivityAggregateRow,
) {
    merge_latest_optional_timestamp(
        &mut entry.latest_conversation_created_at,
        row.latest_conversation_created_at.clone(),
    );
    merge_latest_optional_timestamp(
        &mut entry.last_invocation_at,
        row.last_invocation_at.clone(),
    );
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
    merge_latest_timed_metric(
        &mut entry.latest_first_response_byte_total_at,
        &mut entry.latest_first_response_byte_total_ms,
        row.latest_first_response_byte_total_at.clone(),
        row.latest_first_response_byte_total_ms,
    );
    merge_latest_timed_metric(
        &mut entry.latest_avg_total_at,
        &mut entry.latest_avg_total_ms,
        row.latest_avg_total_at.clone(),
        row.latest_avg_total_ms,
    );
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

#[derive(Debug, Clone, FromRow)]
struct SuccessfulBilledUsageDurationIntervalRow {
    upstream_account_id: Option<i64>,
    model: String,
    reasoning_effort: Option<String>,
    start_epoch_ms: f64,
    end_epoch_ms: f64,
}

#[derive(Debug, Default)]
struct ModelPerformanceDurationOverrides {
    total_wall_clock_ms: Option<f64>,
    by_account_wall_clock_ms: HashMap<Option<i64>, f64>,
    by_group_wall_clock_ms: HashMap<UsageBreakdownGroupKey, f64>,
    by_account_group_wall_clock_ms: HashMap<AccountModelGroupKey, f64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct AccountModelGroupKey {
    upstream_account_id: Option<i64>,
    group: UsageBreakdownGroupKey,
}

#[derive(Debug, Default, Clone, Copy)]
struct UsageDurationUnionAccumulator {
    saw_interval: bool,
    total_ms: f64,
    current_end_epoch_ms: Option<f64>,
}

impl UsageDurationUnionAccumulator {
    fn push_interval(&mut self, start_epoch_ms: f64, end_epoch_ms: f64) {
        if !start_epoch_ms.is_finite() || !end_epoch_ms.is_finite() || end_epoch_ms < start_epoch_ms
        {
            return;
        }
        self.saw_interval = true;
        match self.current_end_epoch_ms {
            None => {
                self.total_ms += end_epoch_ms - start_epoch_ms;
                self.current_end_epoch_ms = Some(end_epoch_ms);
            }
            Some(current_end_epoch_ms) if end_epoch_ms <= current_end_epoch_ms => {}
            Some(current_end_epoch_ms) => {
                self.total_ms += end_epoch_ms - start_epoch_ms.max(current_end_epoch_ms);
                self.current_end_epoch_ms = Some(end_epoch_ms);
            }
        }
    }

    fn total_ms(self) -> Option<f64> {
        self.saw_interval.then_some(self.total_ms)
    }
}

#[derive(Debug, Default)]
struct ModelPerformanceWallClockUnionState {
    total: UsageDurationUnionAccumulator,
    by_account: HashMap<Option<i64>, UsageDurationUnionAccumulator>,
    by_group: HashMap<UsageBreakdownGroupKey, UsageDurationUnionAccumulator>,
    by_account_group: HashMap<AccountModelGroupKey, UsageDurationUnionAccumulator>,
}

impl ModelPerformanceWallClockUnionState {
    fn push_row(&mut self, row: &SuccessfulBilledUsageDurationIntervalRow) {
        let group = UsageBreakdownGroupKey {
            model: row.model.clone(),
            reasoning_effort: row.reasoning_effort.clone(),
        };
        self.total
            .push_interval(row.start_epoch_ms, row.end_epoch_ms);
        self.by_account
            .entry(row.upstream_account_id)
            .or_default()
            .push_interval(row.start_epoch_ms, row.end_epoch_ms);
        self.by_group
            .entry(group.clone())
            .or_default()
            .push_interval(row.start_epoch_ms, row.end_epoch_ms);
        self.by_account_group
            .entry(AccountModelGroupKey {
                upstream_account_id: row.upstream_account_id,
                group,
            })
            .or_default()
            .push_interval(row.start_epoch_ms, row.end_epoch_ms);
    }

    fn into_overrides(self) -> ModelPerformanceDurationOverrides {
        ModelPerformanceDurationOverrides {
            total_wall_clock_ms: self.total.total_ms(),
            by_account_wall_clock_ms: self
                .by_account
                .into_iter()
                .filter_map(|(upstream_account_id, accumulator)| {
                    accumulator
                        .total_ms()
                        .map(|total_ms| (upstream_account_id, total_ms))
                })
                .collect(),
            by_group_wall_clock_ms: self
                .by_group
                .into_iter()
                .filter_map(|(group, accumulator)| {
                    accumulator.total_ms().map(|total_ms| (group, total_ms))
                })
                .collect(),
            by_account_group_wall_clock_ms: self
                .by_account_group
                .into_iter()
                .filter_map(|(key, accumulator)| {
                    accumulator.total_ms().map(|total_ms| (key, total_ms))
                })
                .collect(),
        }
    }
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
    cumulative_usage_duration_sample_count: i64,
    cumulative_usage_duration_sum_ms: f64,
    wall_clock_usage_duration_ms: Option<f64>,
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
        self.cumulative_usage_duration_sample_count +=
            row.performance_usage_duration_sample_count.max(0);
        self.cumulative_usage_duration_sum_ms += row.performance_usage_duration_sum_ms.max(0.0);

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
        entry.cumulative_usage_duration_sample_count +=
            row.performance_usage_duration_sample_count.max(0);
        entry.cumulative_usage_duration_sum_ms += row.performance_usage_duration_sum_ms.max(0.0);
    }

    fn metrics(&self, range: ExactUtcRange) -> ModelPerformanceMetricsResponse {
        let range_minutes = (range.end - range.start).num_milliseconds() as f64 / 60_000.0;
        let cumulative_usage_duration_ms = (self.cumulative_usage_duration_sample_count > 0)
            .then_some(self.cumulative_usage_duration_sum_ms);
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
            wall_clock_usage_duration_ms: self.wall_clock_usage_duration_ms,
            cumulative_usage_duration_ms,
            parallelism: match (
                self.wall_clock_usage_duration_ms,
                cumulative_usage_duration_ms,
            ) {
                (Some(wall_clock_ms), Some(cumulative_ms)) if wall_clock_ms > 0.0 => {
                    Some(cumulative_ms / wall_clock_ms)
                }
                _ => None,
            },
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
                    || entry.cumulative_usage_duration_sample_count > 0
                    || entry.wall_clock_usage_duration_ms.is_some())
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
                .cumulative_usage_duration_ms
                .unwrap_or_default()
                .total_cmp(
                    &left
                        .metrics
                        .cumulative_usage_duration_ms
                        .unwrap_or_default(),
                )
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

fn compute_model_performance_duration_overrides(
    rows: &[SuccessfulBilledUsageDurationIntervalRow],
) -> ModelPerformanceDurationOverrides {
    let mut union_state = ModelPerformanceWallClockUnionState::default();
    for row in rows {
        union_state.push_row(row);
    }
    union_state.into_overrides()
}

#[derive(Debug, FromRow)]
struct RuntimeRecentAccountFallbackRow {
    invoke_id: String,
    occurred_at: String,
    upstream_account_id: Option<i64>,
    upstream_account_name: Option<String>,
    upstream_account_plan_type: Option<String>,
}

const DASHBOARD_ACTIVITY_EXCLUDED_IDS_INLINE_LIMIT: usize = 500;
const DASHBOARD_ACTIVITY_EXCLUDED_IDS_TABLE: &str = "dashboard_activity_excluded_invocation_ids";
const DASHBOARD_ACTIVITY_PREVIEW_ID_HYDRATION_CHUNK_SIZE: usize = 400;

#[derive(Clone, Copy)]
enum DashboardActivityExcludedInvocationIdsFilter<'a> {
    None,
    Inline(&'a HashSet<i64>),
    Table,
}

async fn prepare_dashboard_activity_excluded_invocation_ids_filter<'a>(
    pool: &Pool<Sqlite>,
    exclude_invocation_ids: Option<&'a HashSet<i64>>,
) -> Result<DashboardActivityExcludedInvocationIdsFilter<'a>, ApiError> {
    let Some(exclude_invocation_ids) =
        exclude_invocation_ids.filter(|exclude_invocation_ids| !exclude_invocation_ids.is_empty())
    else {
        return Ok(DashboardActivityExcludedInvocationIdsFilter::None);
    };
    if exclude_invocation_ids.len() <= DASHBOARD_ACTIVITY_EXCLUDED_IDS_INLINE_LIMIT {
        return Ok(DashboardActivityExcludedInvocationIdsFilter::Inline(
            exclude_invocation_ids,
        ));
    }

    sqlx::query(
        "CREATE TABLE IF NOT EXISTS dashboard_activity_excluded_invocation_ids (\
             id INTEGER PRIMARY KEY\
         )",
    )
    .execute(pool)
    .await?;
    sqlx::query("DELETE FROM dashboard_activity_excluded_invocation_ids")
        .execute(pool)
        .await?;

    let ids = exclude_invocation_ids.iter().copied().collect::<Vec<_>>();
    for chunk in ids.chunks(DASHBOARD_ACTIVITY_EXCLUDED_IDS_INLINE_LIMIT) {
        let mut query = QueryBuilder::<Sqlite>::new(
            "INSERT OR IGNORE INTO dashboard_activity_excluded_invocation_ids (id) ",
        );
        query.push_values(chunk.iter().copied(), |mut row, id| {
            row.push_bind(id);
        });
        query.build().execute(pool).await?;
    }

    Ok(DashboardActivityExcludedInvocationIdsFilter::Table)
}

fn push_excluded_invocation_ids_filter(
    query: &mut QueryBuilder<Sqlite>,
    exclude_invocation_ids: DashboardActivityExcludedInvocationIdsFilter<'_>,
) {
    match exclude_invocation_ids {
        DashboardActivityExcludedInvocationIdsFilter::None => {}
        DashboardActivityExcludedInvocationIdsFilter::Inline(exclude_invocation_ids) => {
            query.push(" AND id NOT IN (");
            {
                let mut separated = query.separated(", ");
                for id in exclude_invocation_ids {
                    separated.push_bind(*id);
                }
            }
            query.push(")");
        }
        DashboardActivityExcludedInvocationIdsFilter::Table => {
            query.push(" AND id NOT IN (SELECT id FROM ");
            query.push(DASHBOARD_ACTIVITY_EXCLUDED_IDS_TABLE);
            query.push(")");
        }
    }
}

async fn query_live_upstream_account_activity_aggregate_rows(
    pool: &Pool<Sqlite>,
    source_scope: InvocationSourceScope,
    range: ExactUtcRange,
    use_attempt_fallback: bool,
    exclude_invocation_ids: DashboardActivityExcludedInvocationIdsFilter<'_>,
) -> Result<Vec<UpstreamAccountActivityAggregateRow>, ApiError> {
    let started_at = Instant::now();
    let upstream_account_id_sql = if use_attempt_fallback {
        invocation_upstream_account_id_with_attempt_fallback_sql("codex_invocations")
    } else {
        INVOCATION_UPSTREAM_ACCOUNT_ID_SQL.to_string()
    };
    let failure_class_sql = INVOCATION_RESOLVED_FAILURE_CLASS_SQL;
    let success_sql = format!(
        "LOWER(TRIM(COALESCE(status, ''))) IN ('success', 'completed', '{warning_success}') AND ({failure_class_sql}) = 'none'",
        warning_success = INVOCATION_STATUS_WARNING_SUCCESS,
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
                id,
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
    push_excluded_invocation_ids_filter(&mut query, exclude_invocation_ids);
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
        ),
        latest_first_response_byte_total_by_account AS (
            SELECT
                ranked.upstream_account_id AS upstream_account_id,
                ranked.occurred_at AS latest_first_response_byte_total_at,
                ranked.first_response_byte_total_ms AS latest_first_response_byte_total_ms
            FROM (
                SELECT
                    filtered_invocations.upstream_account_id AS upstream_account_id,
                    filtered_invocations.occurred_at AS occurred_at,
                    {first_response_byte_total_sql} AS first_response_byte_total_ms,
                    ROW_NUMBER() OVER (
                        PARTITION BY filtered_invocations.upstream_account_id
                        ORDER BY filtered_invocations.occurred_at DESC, filtered_invocations.id DESC
                    ) AS row_num
                FROM filtered_invocations
                WHERE {success_sql}
                  AND filtered_invocations.t_upstream_ttfb_ms > 0
            ) AS ranked
            WHERE ranked.row_num = 1
        ),
        latest_total_latency_by_account AS (
            SELECT
                ranked.upstream_account_id AS upstream_account_id,
                ranked.occurred_at AS latest_avg_total_at,
                ranked.t_total_ms AS latest_avg_total_ms
            FROM (
                SELECT
                    filtered_invocations.upstream_account_id AS upstream_account_id,
                    filtered_invocations.occurred_at AS occurred_at,
                    filtered_invocations.t_total_ms AS t_total_ms,
                    ROW_NUMBER() OVER (
                        PARTITION BY filtered_invocations.upstream_account_id
                        ORDER BY filtered_invocations.occurred_at DESC, filtered_invocations.id DESC
                    ) AS row_num
                FROM filtered_invocations
                WHERE {success_sql}
                  AND filtered_invocations.t_total_ms >= 0
            ) AS ranked
            WHERE ranked.row_num = 1
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
            CAST(COALESCE(SUM(CASE WHEN {success_sql} AND t_total_ms >= 0 THEN t_total_ms ELSE 0 END), 0) AS REAL) AS total_latency_sum_ms,
            latest_first_response_byte_total_by_account.latest_first_response_byte_total_at AS latest_first_response_byte_total_at,
            latest_first_response_byte_total_by_account.latest_first_response_byte_total_ms AS latest_first_response_byte_total_ms,
            latest_total_latency_by_account.latest_avg_total_at AS latest_avg_total_at,
            latest_total_latency_by_account.latest_avg_total_ms AS latest_avg_total_ms
        FROM filtered_invocations
        LEFT JOIN conversation_created_at_by_key
          ON conversation_created_at_by_key.prompt_cache_key = filtered_invocations.prompt_cache_key
        LEFT JOIN latest_first_response_byte_total_by_account
          ON latest_first_response_byte_total_by_account.upstream_account_id IS filtered_invocations.upstream_account_id
        LEFT JOIN latest_total_latency_by_account
          ON latest_total_latency_by_account.upstream_account_id IS filtered_invocations.upstream_account_id
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
    exclude_invocation_ids: DashboardActivityExcludedInvocationIdsFilter<'_>,
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
    push_excluded_invocation_ids_filter(&mut query, exclude_invocation_ids);
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
    exclude_invocation_ids: DashboardActivityExcludedInvocationIdsFilter<'_>,
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
        "LOWER(TRIM(COALESCE(status, ''))) IN ('success', 'completed', '{warning_success}') AND ({failure_class_sql}) = 'none' AND cost IS NOT NULL",
        warning_success = INVOCATION_STATUS_WARNING_SUCCESS,
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
    push_excluded_invocation_ids_filter(&mut query, exclude_invocation_ids);
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

async fn query_live_model_performance_duration_overrides(
    pool: &Pool<Sqlite>,
    source_scope: InvocationSourceScope,
    range: ExactUtcRange,
    use_attempt_fallback: bool,
) -> Result<ModelPerformanceDurationOverrides, ApiError> {
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
    let occurred_at_epoch_ms_sql = "(CAST(CASE WHEN instr(occurred_at, 'T') > 0 THEN strftime('%s', occurred_at) ELSE strftime('%s', occurred_at || '+08:00') END AS REAL) * 1000.0)";
    let failure_class_sql = INVOCATION_RESOLVED_FAILURE_CLASS_SQL;
    let success_billed_sql = format!(
        "LOWER(TRIM(COALESCE(status, ''))) IN ('success', 'completed', '{warning_success}') AND ({failure_class_sql}) = 'none' AND cost IS NOT NULL",
        warning_success = INVOCATION_STATUS_WARNING_SUCCESS,
    );
    let range_end_epoch_ms = range.end.timestamp_millis() as f64;
    let mut query = QueryBuilder::<Sqlite>::new("SELECT ");
    query
        .push(upstream_account_id_sql.as_str())
        .push(" AS upstream_account_id, ")
        .push(model_sql.as_str())
        .push(" AS model, ")
        .push(reasoning_effort_sql)
        .push(" AS reasoning_effort, ")
        .push(occurred_at_epoch_ms_sql)
        .push(" AS start_epoch_ms, MIN(")
        .push(occurred_at_epoch_ms_sql)
        .push(" + t_total_ms, ")
        .push_bind(range_end_epoch_ms)
        .push(") AS end_epoch_ms FROM codex_invocations WHERE occurred_at >= ")
        .push_bind(db_occurred_at_lower_bound(range.start))
        .push(" AND occurred_at < ")
        .push_bind(db_occurred_at_upper_bound(range.end));
    if source_scope == InvocationSourceScope::ProxyOnly {
        query.push(" AND source = ").push_bind(SOURCE_PROXY);
    }
    query
        .push(" AND ")
        .push(success_billed_sql.as_str())
        .push(" AND t_total_ms >= 0 ORDER BY occurred_at ASC, id ASC");
    let mut rows = query
        .build_query_as::<SuccessfulBilledUsageDurationIntervalRow>()
        .fetch(pool);
    let mut row_count = 0usize;
    let mut union_state = ModelPerformanceWallClockUnionState::default();
    while let Some(row) = rows.try_next().await? {
        row_count += 1;
        union_state.push_row(&row);
    }
    let elapsed_ms = started_at.elapsed().as_millis() as u64;
    if elapsed_ms >= 1_000 {
        tracing::warn!(
            endpoint = "/api/stats/upstream-account-activity",
            operation = "live_model_performance_usage_duration",
            ?source_scope,
            start = %range.start,
            end = %range.end,
            row_count,
            elapsed_ms,
            "slow model performance usage duration query"
        );
    }
    Ok(union_state.into_overrides())
}

#[derive(Debug, Default)]
struct QueryCompletedInvocationArchiveActivityAggregateRows {
    aggregates: Vec<UpstreamAccountActivityAggregateRow>,
    usage_breakdowns: Vec<UpstreamAccountUsageBreakdownAggregateRow>,
    skipped_materialized_ranges: Vec<ExactUtcRange>,
}

fn dashboard_activity_archive_row_overlap_range(
    archive_row: &crate::stats::ArchiveBatchPathRow,
    requested_range: ExactUtcRange,
) -> Option<ExactUtcRange> {
    let coverage_start =
        parse_to_utc_datetime(archive_row.coverage_start_at()?).unwrap_or(requested_range.start);
    let coverage_end =
        parse_to_utc_datetime(archive_row.coverage_end_at()?).unwrap_or(requested_range.end);
    let start = coverage_start.max(requested_range.start);
    let end = coverage_end.min(requested_range.end);
    (start < end).then_some(ExactUtcRange { start, end })
}

async fn query_completed_invocation_archive_activity_aggregate_rows(
    pool: &Pool<Sqlite>,
    source_scope: InvocationSourceScope,
    range: ExactUtcRange,
) -> Result<QueryCompletedInvocationArchiveActivityAggregateRows, ApiError> {
    let archive_rows = crate::stats::load_completed_invocation_archive_paths_in_range(
        pool,
        Some((range.start, range.end)),
    )
    .await?;
    let mut aggregates = Vec::new();
    let mut usage_breakdowns = Vec::new();
    let mut skipped_materialized_ranges = Vec::new();
    let mut earliest_created_at_by_prompt_cache_key = HashMap::<String, String>::new();
    let mut prompt_cache_keys_by_account = HashMap::<Option<i64>, HashSet<String>>::new();
    for archive_row in archive_rows {
        let Some((archive_pool, temp_cleanup)) = crate::stats::open_invocation_archive_batch_pool(
            &archive_row,
            "dashboard-activity-summary",
        )
        .await?
        else {
            if archive_row.has_materialized_historical_rollups()
                && let Some(skipped_range) =
                    dashboard_activity_archive_row_overlap_range(&archive_row, range)
            {
                skipped_materialized_ranges.push(skipped_range);
            }
            continue;
        };
        let has_cost_breakdown_columns =
            crate::stats::sqlite_table_has_column(&archive_pool, "codex_invocations", "cost_input")
                .await?;
        let overlapping_live_ids = query_archive_upstream_account_activity_overlapping_live_ids(
            pool,
            &archive_pool,
            source_scope,
            range,
        )
        .await?;
        let exclude_invocation_ids_filter =
            prepare_dashboard_activity_excluded_invocation_ids_filter(
                &archive_pool,
                Some(&overlapping_live_ids),
            )
            .await?;
        aggregates.extend(
            query_live_upstream_account_activity_aggregate_rows(
                &archive_pool,
                source_scope,
                range,
                false,
                exclude_invocation_ids_filter,
            )
            .await?,
        );
        for row in query_live_upstream_account_prompt_cache_created_at_rows(
            &archive_pool,
            source_scope,
            range,
            false,
            exclude_invocation_ids_filter,
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
                exclude_invocation_ids_filter,
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
    Ok(QueryCompletedInvocationArchiveActivityAggregateRows {
        aggregates,
        usage_breakdowns,
        skipped_materialized_ranges,
    })
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

fn merge_upstream_account_activity_recent_row_metadata(
    account_activity: &mut HashMap<Option<i64>, UpstreamAccountActivityAccumulator>,
    row: &UpstreamAccountInvocationPreviewRow,
) {
    let Some(occurred_at) = parse_to_utc_datetime(&row.occurred_at) else {
        return;
    };
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
}

#[derive(Debug, Clone, Copy)]
struct UpstreamAccountActivityPreviewReadTelemetry {
    route: &'static str,
    builder: &'static str,
    purpose: &'static str,
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
        UpstreamAccountActivityPreviewReadTelemetry {
            route: "shared",
            builder: "usage_breakdown",
            purpose: "full_range_preview_rows",
        },
    )
    .await
}

fn build_upstream_account_activity_preview_select(
    query: &mut QueryBuilder<Sqlite>,
    source_scope: InvocationSourceScope,
) {
    let resolved_upstream_account_id_sql =
        invocation_upstream_account_id_with_attempt_fallback_sql("codex_invocations");
    let conversation_created_at_sql = format!(
        "COALESCE({}, occurred_at)",
        prompt_cache_conversation_created_at_sql(INVOCATION_PROMPT_CACHE_KEY_SQL, source_scope)
    );
    query.push("SELECT id, invoke_id, ");
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
        .push(" AS endpoint FROM codex_invocations");
}

async fn query_live_upstream_account_activity_preview_rows_with_limit(
    pool: &Pool<Sqlite>,
    source_scope: InvocationSourceScope,
    range: ExactUtcRange,
    upstream_account_id: Option<Option<i64>>,
    limit: Option<usize>,
    in_progress_only: bool,
    telemetry: UpstreamAccountActivityPreviewReadTelemetry,
) -> Result<Vec<UpstreamAccountInvocationPreviewRow>, ApiError> {
    let started_at = Instant::now();
    let resolved_upstream_account_id_sql =
        invocation_upstream_account_id_with_attempt_fallback_sql("codex_invocations");
    let mut query = QueryBuilder::<Sqlite>::new("");
    build_upstream_account_activity_preview_select(&mut query, source_scope);
    query
        .push(" WHERE occurred_at >= ")
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
            route = telemetry.route,
            builder = telemetry.builder,
            operation = telemetry.purpose,
            purpose = telemetry.purpose,
            ?source_scope,
            start = %range.start,
            end = %range.end,
            range_seconds = (range.end - range.start).num_seconds(),
            upstream_account_id = ?upstream_account_id.flatten(),
            limit = ?limit,
            in_progress_only,
            row_count = rows.len(),
            elapsed_ms,
            "slow upstream-account activity read"
        );
    }
    Ok(rows)
}

async fn query_live_upstream_account_activity_preview_candidate_ids_per_account(
    pool: &Pool<Sqlite>,
    source_scope: InvocationSourceScope,
    range: ExactUtcRange,
    limit_per_account: usize,
) -> Result<Vec<i64>, ApiError> {
    let resolved_upstream_account_id_sql =
        invocation_upstream_account_id_with_attempt_fallback_sql("codex_invocations");
    let mut query = QueryBuilder::<Sqlite>::new(
        "WITH ranked AS (\
           SELECT id \
             FROM (\
               SELECT id, ROW_NUMBER() OVER (PARTITION BY ",
    );
    query
        .push(resolved_upstream_account_id_sql.as_str())
        .push(
            " ORDER BY occurred_at DESC, id DESC) AS account_rank \
               FROM codex_invocations \
               WHERE occurred_at >= ",
        )
        .push_bind(db_occurred_at_lower_bound(range.start))
        .push(" AND occurred_at < ")
        .push_bind(db_occurred_at_upper_bound(range.end));
    if source_scope == InvocationSourceScope::ProxyOnly {
        query.push(" AND source = ").push_bind(SOURCE_PROXY);
    }
    query
        .push(") WHERE account_rank <= ")
        .push_bind(limit_per_account as i64)
        .push(") SELECT id FROM ranked");
    Ok(query.build_query_scalar::<i64>().fetch_all(pool).await?)
}

async fn query_live_upstream_account_activity_preview_rows_by_ids(
    pool: &Pool<Sqlite>,
    source_scope: InvocationSourceScope,
    ids: &[i64],
    telemetry: UpstreamAccountActivityPreviewReadTelemetry,
) -> Result<Vec<UpstreamAccountInvocationPreviewRow>, ApiError> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }

    let started_at = Instant::now();
    let mut rows = Vec::with_capacity(ids.len());
    for chunk in ids.chunks(DASHBOARD_ACTIVITY_PREVIEW_ID_HYDRATION_CHUNK_SIZE) {
        let mut query = QueryBuilder::<Sqlite>::new("");
        build_upstream_account_activity_preview_select(&mut query, source_scope);
        query.push(" WHERE id IN (");
        {
            let mut separated = query.separated(", ");
            for id in chunk {
                separated.push_bind(*id);
            }
            separated.push_unseparated(")");
        }
        query.push(" ORDER BY occurred_at DESC, id DESC");

        rows.extend(
            query
                .build_query_as::<UpstreamAccountInvocationPreviewRow>()
                .fetch_all(pool)
                .await?,
        );
    }
    rows.sort_by(|left, right| {
        right
            .occurred_at
            .cmp(&left.occurred_at)
            .then_with(|| right.id.cmp(&left.id))
    });
    let elapsed_ms = started_at.elapsed().as_millis() as u64;
    if elapsed_ms >= 1_000 {
        tracing::warn!(
            endpoint = "/api/stats/upstream-account-activity",
            route = telemetry.route,
            builder = telemetry.builder,
            operation = telemetry.purpose,
            purpose = telemetry.purpose,
            candidate_preview_id_count = ids.len(),
            hydrated_preview_row_count = rows.len(),
            ?source_scope,
            selected_preview_row_count = ids.len(),
            row_count = rows.len(),
            elapsed_ms,
            "slow upstream-account activity preview hydration"
        );
    }
    Ok(rows)
}

struct HydratedUpstreamAccountPreviewRows {
    rows: Vec<UpstreamAccountInvocationPreviewRow>,
    candidate_preview_id_count: usize,
    hydrated_preview_row_count: usize,
}

async fn query_live_upstream_account_activity_preview_rows_per_account_limit_with_stats(
    pool: &Pool<Sqlite>,
    source_scope: InvocationSourceScope,
    range: ExactUtcRange,
    limit_per_account: usize,
    telemetry: UpstreamAccountActivityPreviewReadTelemetry,
) -> Result<HydratedUpstreamAccountPreviewRows, ApiError> {
    let ids = query_live_upstream_account_activity_preview_candidate_ids_per_account(
        pool,
        source_scope,
        range,
        limit_per_account,
    )
    .await?;
    let rows = query_live_upstream_account_activity_preview_rows_by_ids(
        pool,
        source_scope,
        &ids,
        telemetry,
    )
    .await?;
    Ok(HydratedUpstreamAccountPreviewRows {
        candidate_preview_id_count: ids.len(),
        hydrated_preview_row_count: rows.len(),
        rows,
    })
}

async fn query_live_upstream_account_activity_preview_rows_per_account_limit(
    pool: &Pool<Sqlite>,
    source_scope: InvocationSourceScope,
    range: ExactUtcRange,
    limit_per_account: usize,
    telemetry: UpstreamAccountActivityPreviewReadTelemetry,
) -> Result<Vec<UpstreamAccountInvocationPreviewRow>, ApiError> {
    Ok(
        query_live_upstream_account_activity_preview_rows_per_account_limit_with_stats(
            pool,
            source_scope,
            range,
            limit_per_account,
            telemetry,
        )
        .await?
        .rows,
    )
}

async fn query_live_upstream_account_activity_existing_invocation_ids(
    pool: &Pool<Sqlite>,
    source_scope: InvocationSourceScope,
    ids: &[i64],
) -> Result<HashSet<i64>, ApiError> {
    if ids.is_empty() {
        return Ok(HashSet::new());
    }

    let mut existing_ids = HashSet::new();
    for chunk in ids.chunks(DASHBOARD_ACTIVITY_PREVIEW_ID_HYDRATION_CHUNK_SIZE) {
        let mut query = QueryBuilder::<Sqlite>::new(
            "SELECT id FROM codex_invocations \
             WHERE id IN (",
        );
        {
            let mut separated = query.separated(", ");
            for id in chunk {
                separated.push_bind(*id);
            }
            separated.push_unseparated(")");
        }
        if source_scope == InvocationSourceScope::ProxyOnly {
            query.push(" AND source = ").push_bind(SOURCE_PROXY);
        }
        existing_ids.extend(query.build_query_scalar::<i64>().fetch_all(pool).await?);
    }
    Ok(existing_ids)
}

async fn query_archive_upstream_account_activity_overlapping_live_ids(
    live_pool: &Pool<Sqlite>,
    archive_pool: &Pool<Sqlite>,
    source_scope: InvocationSourceScope,
    range: ExactUtcRange,
) -> Result<HashSet<i64>, ApiError> {
    let mut overlapping_ids = HashSet::new();
    let mut cursor_id = i64::MIN;
    loop {
        let mut query = QueryBuilder::<Sqlite>::new(
            "SELECT id FROM codex_invocations \
             WHERE id > ",
        );
        query
            .push_bind(cursor_id)
            .push(" AND occurred_at >= ")
            .push_bind(db_occurred_at_lower_bound(range.start))
            .push(" AND occurred_at < ")
            .push_bind(db_occurred_at_upper_bound(range.end));
        if source_scope == InvocationSourceScope::ProxyOnly {
            query.push(" AND source = ").push_bind(SOURCE_PROXY);
        }
        query
            .push(" ORDER BY id LIMIT ")
            .push_bind(DASHBOARD_ACTIVITY_PREVIEW_ID_HYDRATION_CHUNK_SIZE as i64);
        let ids = query
            .build_query_scalar::<i64>()
            .fetch_all(archive_pool)
            .await?;
        let Some(last_id) = ids.last().copied() else {
            break;
        };
        cursor_id = last_id;
        overlapping_ids.extend(
            query_live_upstream_account_activity_existing_invocation_ids(
                live_pool,
                source_scope,
                &ids,
            )
            .await?,
        );
        if ids.len() < DASHBOARD_ACTIVITY_PREVIEW_ID_HYDRATION_CHUNK_SIZE {
            break;
        }
    }
    Ok(overlapping_ids)
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
    let terminal_rows =
        load_dashboard_activity_runtime_terminal_preview_rows(state, source_scope, range).await?;
    if terminal_rows.is_empty() {
        return Ok(());
    }

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
        rows_by_key.insert(key, row);
    }
    rows.extend(rows_by_key.into_values());
    Ok(())
}

async fn load_dashboard_activity_runtime_terminal_preview_rows(
    state: &AppState,
    source_scope: InvocationSourceScope,
    range: ExactUtcRange,
) -> Result<Vec<UpstreamAccountInvocationPreviewRow>, ApiError> {
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
        return Ok(Vec::new());
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

    for row in &mut terminal_rows {
        let key = (row.invoke_id.clone(), row.occurred_at.clone());
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
    }

    Ok(terminal_rows)
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
        state.dashboard_network_speed_cache.as_ref(),
        revision,
    )
    .await
}

pub(crate) async fn query_dashboard_activity_live_snapshot_from_runtime(
    pool: &Pool<Sqlite>,
    proxy_runtime_invocations: &ProxyRuntimeInvocationStore,
    dashboard_network_speed_cache: &DashboardNetworkSpeedCache,
    revision: u64,
) -> Result<DashboardActivityLiveSnapshot, ApiError> {
    let now = Utc::now();
    let source_scope = resolve_default_source_scope(pool).await?;
    flush_dashboard_network_socket_minute_rollups(pool, dashboard_network_speed_cache, now).await?;
    let counts = query_upstream_account_in_progress_counts_from_runtime(
        pool,
        proxy_runtime_invocations,
        source_scope,
    )
    .await?;
    let account_rates = dashboard_network_speed_cache.snapshot_account_rates(now);
    let global_realtime_rate = build_dashboard_network_realtime_rate_response(
        dashboard_network_speed_cache
            .snapshot_scope_realtime_bytes(DashboardNetworkScopeKey::Global, now),
    );
    let mut live_bucket_account_ids = counts.keys().copied().collect::<Vec<_>>();
    for upstream_account_id in account_rates.keys() {
        if !live_bucket_account_ids.contains(upstream_account_id) {
            live_bucket_account_ids.push(*upstream_account_id);
        }
    }
    let mut live_buckets = HashMap::new();
    for upstream_account_id in live_bucket_account_ids {
        live_buckets.insert(
            upstream_account_id,
            load_dashboard_network_live_bucket_point(
                pool,
                dashboard_network_speed_cache,
                source_scope,
                now,
                match upstream_account_id {
                    Some(id) => DashboardNetworkScopeKey::Account(id),
                    None => DashboardNetworkScopeKey::Unassigned,
                },
            )
            .await?,
        );
    }
    let mut accounts = counts
        .into_iter()
        .map(|(upstream_account_id, summary)| {
            let rate = account_rates
                .get(&upstream_account_id)
                .copied()
                .unwrap_or_default();
            DashboardActivityLiveAccount {
                account_key: upstream_account_id
                    .map(|id| format!("upstream:{id}"))
                    .unwrap_or_else(|| "unassigned".to_string()),
                upstream_account_id,
                in_progress_invocation_count: summary.in_progress_count,
                in_progress_phase_counts: summary.phase_counts,
                retry_invocation_count: summary.retry_count,
                upload_bytes_per_second: rate.upload_bytes_per_second,
                download_bytes_per_second: rate.download_bytes_per_second,
                network_live_bucket: live_buckets.get(&upstream_account_id).cloned(),
            }
        })
        .collect::<Vec<_>>();
    let existing_account_keys = accounts
        .iter()
        .map(|account| account.account_key.clone())
        .collect::<HashSet<_>>();
    for (upstream_account_id, rate) in account_rates {
        let account_key = upstream_account_id
            .map(|id| format!("upstream:{id}"))
            .unwrap_or_else(|| "unassigned".to_string());
        if existing_account_keys.contains(&account_key) {
            continue;
        }
        accounts.push(DashboardActivityLiveAccount {
            account_key,
            upstream_account_id,
            in_progress_invocation_count: 0,
            in_progress_phase_counts: InvocationPhaseCountsResponse::default(),
            retry_invocation_count: 0,
            upload_bytes_per_second: rate.upload_bytes_per_second,
            download_bytes_per_second: rate.download_bytes_per_second,
            network_live_bucket: live_buckets.get(&upstream_account_id).cloned(),
        });
    }
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
        generated_at: format_utc_iso(now),
        in_progress_invocation_count,
        in_progress_phase_counts,
        retry_invocation_count,
        network_live_bucket: Some(
            load_dashboard_network_live_bucket_point(
                pool,
                dashboard_network_speed_cache,
                source_scope,
                now,
                DashboardNetworkScopeKey::Global,
            )
            .await?,
        ),
        network_realtime_rate: Some(global_realtime_rate),
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
pub(crate) const DASHBOARD_ACTIVITY_SNAPSHOT_CACHE_TTL_SECS: u64 = 5;

#[derive(Debug, Clone)]
pub(crate) struct DashboardActivitySnapshot {
    range: String,
    range_start: DateTime<Utc>,
    range_end: DateTime<Utc>,
    accounts: Vec<DashboardActivityAccountResponse>,
    summary: DashboardActivitySummaryResponse,
    materialized_archive_fallback_totals: StatsTotals,
    materialized_archive_details_limited: bool,
    build_telemetry: DashboardActivityBuildTelemetry,
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

    #[cfg(test)]
    pub(crate) fn test_stub(range: &str) -> Self {
        let now = Utc::now();
        Self {
            range: range.to_string(),
            range_start: now,
            range_end: now + ChronoDuration::minutes(1),
            accounts: Vec::new(),
            summary: DashboardActivitySummaryResponse {
                stats: StatsResponse {
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
                    non_success_cost: None,
                    non_success_tokens: None,
                    maintenance: None,
                },
                tokens_per_minute: None,
                spend_rate: None,
                current_first_response_byte_total_avg_ms: None,
                current_avg_total_ms: None,
                model_performance: ModelPerformanceResponse {
                    available: false,
                    total: ModelPerformanceMetricsResponse {
                        tokens_per_minute: 0.0,
                        streaming_response_rate: None,
                        avg_response_ms: None,
                        avg_first_response_byte_total_ms: None,
                        wall_clock_usage_duration_ms: None,
                        cumulative_usage_duration_ms: None,
                        parallelism: None,
                    },
                    models: Vec::new(),
                },
            },
            materialized_archive_fallback_totals: StatsTotals::default(),
            materialized_archive_details_limited: false,
            build_telemetry: DashboardActivityBuildTelemetry::default(),
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct DashboardActivitySnapshotCacheOutcome {
    cache_hit_or_miss: &'static str,
    cache_bypass_reason: &'static str,
    coalesced_waiter_count: usize,
    db_build_elapsed_ms: u64,
    cache_ttl_ms: u64,
    cache_entry_age_ms: u64,
    cache_entry_count: usize,
    in_flight_count: usize,
}

#[derive(Debug, Clone, Copy)]
struct DashboardActivityBuildTelemetry {
    preview_read_mode: &'static str,
    candidate_preview_id_count: usize,
    hydrated_preview_row_count: usize,
}

impl Default for DashboardActivityBuildTelemetry {
    fn default() -> Self {
        Self {
            preview_read_mode: "none",
            candidate_preview_id_count: 0,
            hydrated_preview_row_count: 0,
        }
    }
}

fn dashboard_activity_build_scope(include_accounts: bool, _include_recent: bool) -> &'static str {
    if !include_accounts {
        "summary_only"
    } else {
        "full"
    }
}

fn dashboard_activity_stats_totals_from_aggregate_rows(
    rows: &[UpstreamAccountActivityAggregateRow],
) -> StatsTotals {
    let mut totals = StatsTotals::default();
    for row in rows {
        totals.total_count += row.request_count;
        totals.success_count += row.success_count;
        totals.failure_count += row.failure_count;
        totals.total_cost += row.total_cost;
        totals.total_tokens += row.total_tokens;
        totals.non_success_cost += row.non_success_cost;
    }
    totals
}

fn dashboard_activity_stats_totals_subtract(left: StatsTotals, right: StatsTotals) -> StatsTotals {
    StatsTotals {
        total_count: left.total_count.saturating_sub(right.total_count).max(0),
        success_count: left
            .success_count
            .saturating_sub(right.success_count)
            .max(0),
        failure_count: left
            .failure_count
            .saturating_sub(right.failure_count)
            .max(0),
        total_cost: (left.total_cost - right.total_cost).max(0.0),
        total_tokens: left.total_tokens.saturating_sub(right.total_tokens).max(0),
        non_success_cost: (left.non_success_cost - right.non_success_cost).max(0.0),
    }
}

fn dashboard_activity_stats_totals_has_values(totals: StatsTotals) -> bool {
    totals.total_count > 0
        || totals.success_count > 0
        || totals.failure_count > 0
        || totals.total_cost > 0.0
        || totals.total_tokens > 0
        || totals.non_success_cost > 0.0
}

async fn dashboard_activity_materialized_archive_fallback_totals(
    state: &AppState,
    source_scope: InvocationSourceScope,
    skipped_materialized_ranges: Vec<ExactUtcRange>,
) -> Result<StatsTotals, ApiError> {
    let mut fallback_totals = StatsTotals::default();
    for skipped_range in skipped_materialized_ranges {
        let rollup_totals = query_hourly_backed_summary_range(
            state,
            skipped_range.start,
            skipped_range.end,
            source_scope,
        )
        .await?;
        let live_totals = dashboard_activity_stats_totals_from_aggregate_rows(
            &query_live_upstream_account_activity_aggregate_rows(
                &state.pool,
                source_scope,
                skipped_range,
                true,
                DashboardActivityExcludedInvocationIdsFilter::None,
            )
            .await?,
        );
        fallback_totals = fallback_totals.add(dashboard_activity_stats_totals_subtract(
            rollup_totals,
            live_totals,
        ));
    }
    Ok(fallback_totals)
}

fn dashboard_activity_historical_live_gap_ranges(
    range: ExactUtcRange,
    retention_cutoff: DateTime<Utc>,
) -> Result<Vec<ExactUtcRange>, ApiError> {
    if range.start >= retention_cutoff {
        return Ok(Vec::new());
    }

    let full_hour_start_epoch = ceil_hour_epoch(range.start.timestamp());
    let full_hour_end_epoch = crate::stats::align_bucket_epoch(range.end.timestamp(), 3_600, 0);
    let full_hour_start = Utc
        .timestamp_opt(full_hour_start_epoch, 0)
        .single()
        .ok_or_else(|| ApiError::from(anyhow!("invalid dashboard activity gap start epoch")))?;
    let full_hour_end = Utc
        .timestamp_opt(full_hour_end_epoch, 0)
        .single()
        .ok_or_else(|| ApiError::from(anyhow!("invalid dashboard activity gap end epoch")))?;

    let mut gap_ranges = Vec::new();
    if let Some(gap) = exact_utc_range(
        range.start,
        range.end.min(full_hour_start).min(retention_cutoff),
    )? {
        gap_ranges.push(gap);
    }
    if let Some(gap) = exact_utc_range(
        range.start.max(full_hour_end),
        range.end.min(retention_cutoff),
    )? {
        gap_ranges.push(gap);
    }
    Ok(gap_ranges)
}

async fn dashboard_activity_historical_live_gap_totals(
    state: &AppState,
    source_scope: InvocationSourceScope,
    range: ExactUtcRange,
    retention_cutoff: DateTime<Utc>,
) -> Result<StatsTotals, ApiError> {
    let mut totals = StatsTotals::default();
    for gap_range in dashboard_activity_historical_live_gap_ranges(range, retention_cutoff)? {
        totals = totals.add(dashboard_activity_stats_totals_from_aggregate_rows(
            &query_live_upstream_account_activity_aggregate_rows(
                &state.pool,
                source_scope,
                gap_range,
                true,
                DashboardActivityExcludedInvocationIdsFilter::None,
            )
            .await?,
        ));
    }
    Ok(totals)
}

fn dashboard_activity_apply_materialized_archive_fallback_to_stats(
    stats: &mut StatsResponse,
    fallback_totals: StatsTotals,
) {
    if !dashboard_activity_stats_totals_has_values(fallback_totals) {
        return;
    }

    stats.total_count += fallback_totals.total_count;
    stats.success_count += fallback_totals.success_count;
    stats.failure_count += fallback_totals.failure_count;
    stats.total_cost += fallback_totals.total_cost;
    stats.total_tokens += fallback_totals.total_tokens;
    stats.non_success_cost =
        Some(stats.non_success_cost.unwrap_or_default() + fallback_totals.non_success_cost);
    // Materialized invocation rollups do not retain model/cost-breakdown or non-success-token
    // detail, so omit those partial fields when they would no longer align with top-level totals.
    stats.usage_breakdown = None;
    stats.non_success_tokens = None;
}

fn dashboard_activity_clear_materialized_archive_detail_fields(stats: &mut StatsResponse) {
    stats.usage_breakdown = None;
    stats.non_success_tokens = None;
}

#[derive(Debug, FromRow)]
struct DashboardActivityAccountStatsRollupAggregateRow {
    upstream_account_id: i64,
    request_count: i64,
    success_count: i64,
    failure_count: i64,
    total_tokens: i64,
    input_tokens: i64,
    output_tokens: i64,
    cache_input_tokens: i64,
    total_cost: f64,
    non_success_cost: f64,
    first_response_byte_total_sample_count: i64,
    first_response_byte_total_sum_ms: f64,
    total_latency_sample_count: i64,
    total_latency_sum_ms: f64,
}

#[derive(Debug, Default, Clone, Copy)]
struct DashboardActivityUsageFallbackTotals {
    cache_write_tokens: i64,
    cache_read_tokens: i64,
    output_tokens: i64,
}

#[derive(Debug, Default, Clone, Copy)]
struct DashboardActivityAccountFallbackTotals {
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
    cache_write_tokens: i64,
    cache_read_tokens: i64,
    output_tokens: i64,
    first_response_byte_total_sample_count: i64,
    first_response_byte_total_sum_ms: f64,
    total_latency_sample_count: i64,
    total_latency_sum_ms: f64,
}

impl DashboardActivityAccountFallbackTotals {
    fn from_stats_totals(totals: StatsTotals) -> Self {
        let non_success_count = totals
            .total_count
            .saturating_sub(totals.success_count)
            .max(0);
        Self {
            request_count: totals.total_count,
            success_count: totals.success_count,
            failure_count: totals.failure_count,
            non_success_count: non_success_count.max(totals.failure_count),
            total_tokens: totals.total_tokens,
            success_tokens: if non_success_count == 0 {
                totals.total_tokens
            } else {
                0
            },
            non_success_tokens: if totals.success_count == 0 {
                totals.total_tokens
            } else {
                0
            },
            failure_tokens: if totals.success_count == 0 && totals.failure_count > 0 {
                totals.total_tokens
            } else {
                0
            },
            failure_cost: if totals.success_count == 0 && totals.failure_count > 0 {
                totals.non_success_cost
            } else {
                0.0
            },
            non_success_cost: totals.non_success_cost,
            total_cost: totals.total_cost,
            ..Self::default()
        }
    }

    fn from_rollup_minus_live(
        row: &DashboardActivityAccountStatsRollupAggregateRow,
        live: Option<&UpstreamAccountActivityAggregateRow>,
        live_usage: DashboardActivityUsageFallbackTotals,
    ) -> Self {
        let live_request_count = live.map_or(0, |row| row.request_count);
        let live_success_count = live.map_or(0, |row| row.success_count);
        let live_failure_count = live.map_or(0, |row| row.failure_count);
        let live_total_tokens = live.map_or(0, |row| row.total_tokens);
        let live_cache_input_tokens = live.map_or(0, |row| row.cache_input_tokens);
        let live_total_cost = live.map_or(0.0, |row| row.total_cost);
        let live_non_success_cost = live.map_or(0.0, |row| row.non_success_cost);
        let live_first_response_count =
            live.map_or(0, |row| row.first_response_byte_total_sample_count);
        let live_first_response_sum = live.map_or(0.0, |row| row.first_response_byte_total_sum_ms);
        let live_total_latency_count = live.map_or(0, |row| row.total_latency_sample_count);
        let live_total_latency_sum = live.map_or(0.0, |row| row.total_latency_sum_ms);

        let request_count = row.request_count.saturating_sub(live_request_count).max(0);
        let success_count = row.success_count.saturating_sub(live_success_count).max(0);
        let failure_count = row.failure_count.saturating_sub(live_failure_count).max(0);
        let non_success_count = request_count
            .saturating_sub(success_count)
            .max(failure_count);
        let total_tokens = row.total_tokens.saturating_sub(live_total_tokens).max(0);
        let non_success_cost = (row.non_success_cost - live_non_success_cost).max(0.0);
        let total_cost = (row.total_cost - live_total_cost).max(0.0);
        let cache_input_tokens = row
            .cache_input_tokens
            .saturating_sub(live_cache_input_tokens)
            .max(0);
        let rollup_cache_write_tokens = row
            .input_tokens
            .max(0)
            .saturating_sub(row.cache_input_tokens.max(0))
            .max(0);
        let cache_write_tokens = rollup_cache_write_tokens
            .saturating_sub(live_usage.cache_write_tokens)
            .max(0);
        let cache_read_tokens = row
            .cache_input_tokens
            .max(0)
            .saturating_sub(live_usage.cache_read_tokens)
            .max(0);
        let output_tokens = row
            .output_tokens
            .max(0)
            .saturating_sub(live_usage.output_tokens)
            .max(0);

        Self {
            request_count,
            success_count,
            failure_count,
            non_success_count,
            total_tokens,
            success_tokens: if non_success_count == 0 {
                total_tokens
            } else {
                0
            },
            non_success_tokens: if success_count == 0 { total_tokens } else { 0 },
            failure_tokens: if success_count == 0 && failure_count > 0 {
                total_tokens
            } else {
                0
            },
            failure_cost: if success_count == 0 && failure_count > 0 {
                non_success_cost
            } else {
                0.0
            },
            non_success_cost,
            cache_input_tokens,
            total_cost,
            cache_write_tokens,
            cache_read_tokens,
            output_tokens,
            first_response_byte_total_sample_count: row
                .first_response_byte_total_sample_count
                .saturating_sub(live_first_response_count)
                .max(0),
            first_response_byte_total_sum_ms: (row.first_response_byte_total_sum_ms
                - live_first_response_sum)
                .max(0.0),
            total_latency_sample_count: row
                .total_latency_sample_count
                .saturating_sub(live_total_latency_count)
                .max(0),
            total_latency_sum_ms: (row.total_latency_sum_ms - live_total_latency_sum).max(0.0),
        }
    }

    fn add_assign(&mut self, other: Self) {
        self.request_count += other.request_count;
        self.success_count += other.success_count;
        self.failure_count += other.failure_count;
        self.non_success_count += other.non_success_count;
        self.total_tokens += other.total_tokens;
        self.success_tokens += other.success_tokens;
        self.non_success_tokens += other.non_success_tokens;
        self.failure_tokens += other.failure_tokens;
        self.failure_cost += other.failure_cost;
        self.non_success_cost += other.non_success_cost;
        self.cache_input_tokens += other.cache_input_tokens;
        self.total_cost += other.total_cost;
        self.cache_write_tokens += other.cache_write_tokens;
        self.cache_read_tokens += other.cache_read_tokens;
        self.output_tokens += other.output_tokens;
        self.first_response_byte_total_sample_count += other.first_response_byte_total_sample_count;
        self.first_response_byte_total_sum_ms += other.first_response_byte_total_sum_ms;
        self.total_latency_sample_count += other.total_latency_sample_count;
        self.total_latency_sum_ms += other.total_latency_sum_ms;
    }

    fn stats_totals(self) -> StatsTotals {
        StatsTotals {
            total_count: self.request_count,
            success_count: self.success_count,
            failure_count: self.failure_count,
            total_cost: self.total_cost,
            total_tokens: self.total_tokens,
            non_success_cost: self.non_success_cost,
        }
    }
}

async fn query_dashboard_activity_account_stats_rollup_aggregate_rows(
    pool: &Pool<Sqlite>,
    source_scope: InvocationSourceScope,
    range: ExactUtcRange,
) -> Result<Vec<DashboardActivityAccountStatsRollupAggregateRow>, ApiError> {
    let range_start_epoch = ceil_hour_epoch(range.start.timestamp());
    let range_end_epoch = crate::stats::align_bucket_epoch(range.end.timestamp(), 3_600, 0);
    if range_start_epoch >= range_end_epoch {
        return Ok(Vec::new());
    }

    let mut query = QueryBuilder::<Sqlite>::new(
        r#"
        SELECT
            upstream_account_id,
            COALESCE(SUM(total_count), 0) AS request_count,
            COALESCE(SUM(success_count), 0) AS success_count,
            COALESCE(SUM(failure_count), 0) AS failure_count,
            COALESCE(SUM(total_tokens), 0) AS total_tokens,
            COALESCE(SUM(input_tokens), 0) AS input_tokens,
            COALESCE(SUM(output_tokens), 0) AS output_tokens,
            COALESCE(SUM(cache_input_tokens), 0) AS cache_input_tokens,
            CAST(COALESCE(SUM(total_cost), 0.0) AS REAL) AS total_cost,
            CAST(COALESCE(SUM(non_success_cost), 0.0) AS REAL) AS non_success_cost,
            COALESCE(SUM(first_response_byte_total_sample_count), 0) AS first_response_byte_total_sample_count,
            CAST(COALESCE(SUM(first_response_byte_total_sum_ms), 0.0) AS REAL) AS first_response_byte_total_sum_ms,
            COALESCE(SUM(total_latency_sample_count), 0) AS total_latency_sample_count,
            CAST(COALESCE(SUM(total_latency_sum_ms), 0.0) AS REAL) AS total_latency_sum_ms
        FROM upstream_account_stats_hourly
        WHERE bucket_start_epoch >=
        "#,
    );
    query
        .push_bind(range_start_epoch)
        .push(" AND bucket_start_epoch < ")
        .push_bind(range_end_epoch);
    if source_scope == InvocationSourceScope::ProxyOnly {
        query.push(" AND source = ").push_bind(SOURCE_PROXY);
    }
    query.push(" GROUP BY upstream_account_id");
    Ok(query
        .build_query_as::<DashboardActivityAccountStatsRollupAggregateRow>()
        .fetch_all(pool)
        .await?)
}

async fn dashboard_activity_materialized_archive_account_fallback_totals(
    state: &AppState,
    source_scope: InvocationSourceScope,
    skipped_materialized_ranges: &[ExactUtcRange],
) -> Result<HashMap<Option<i64>, DashboardActivityAccountFallbackTotals>, ApiError> {
    let mut fallback_by_account =
        HashMap::<Option<i64>, DashboardActivityAccountFallbackTotals>::new();
    let retention_cutoff = shanghai_retention_cutoff(state.config.invocation_max_days);
    for skipped_range in skipped_materialized_ranges {
        let range_plan = build_hourly_rollup_exact_range_plan(
            skipped_range.start,
            skipped_range.end,
            retention_cutoff,
        )?;
        if let Some(full_hour_range) =
            dashboard_activity_full_hour_exact_range(range_plan.full_hour_range)?
        {
            let rollup_rows = query_dashboard_activity_account_stats_rollup_aggregate_rows(
                &state.pool,
                source_scope,
                full_hour_range,
            )
            .await?;
            let live_full_hour_rows = query_live_upstream_account_activity_aggregate_rows(
                &state.pool,
                source_scope,
                full_hour_range,
                true,
                DashboardActivityExcludedInvocationIdsFilter::None,
            )
            .await?;
            let live_full_hour_by_account = live_full_hour_rows
                .into_iter()
                .map(|row| (row.upstream_account_id, row))
                .collect::<HashMap<_, _>>();
            let mut live_full_hour_usage_by_account =
                HashMap::<Option<i64>, DashboardActivityUsageFallbackTotals>::new();
            for row in query_live_upstream_account_usage_breakdown_rows(
                &state.pool,
                source_scope,
                full_hour_range,
                true,
                true,
                DashboardActivityExcludedInvocationIdsFilter::None,
            )
            .await?
            {
                let entry = live_full_hour_usage_by_account
                    .entry(row.upstream_account_id)
                    .or_default();
                entry.cache_write_tokens += row.cache_write_tokens;
                entry.cache_read_tokens += row.cache_read_tokens;
                entry.output_tokens += row.output_tokens;
            }

            for row in rollup_rows {
                let account_id = Some(row.upstream_account_id);
                let totals = DashboardActivityAccountFallbackTotals::from_rollup_minus_live(
                    &row,
                    live_full_hour_by_account.get(&account_id),
                    live_full_hour_usage_by_account
                        .get(&account_id)
                        .copied()
                        .unwrap_or_default(),
                );
                if !dashboard_activity_stats_totals_has_values(totals.stats_totals()) {
                    continue;
                }
                fallback_by_account
                    .entry(account_id)
                    .or_default()
                    .add_assign(totals);
            }
        }
    }
    Ok(fallback_by_account)
}

fn dashboard_activity_merge_account_fallback_totals(
    entry: &mut UpstreamAccountActivityAccumulator,
    totals: DashboardActivityAccountFallbackTotals,
) {
    entry.request_count += totals.request_count;
    entry.success_count += totals.success_count;
    entry.failure_count += totals.failure_count;
    entry.non_success_count += totals.non_success_count;
    entry.total_tokens += totals.total_tokens;
    entry.success_tokens += totals.success_tokens;
    entry.non_success_tokens += totals.non_success_tokens;
    entry.failure_tokens += totals.failure_tokens;
    entry.failure_cost += totals.failure_cost;
    entry.non_success_cost += totals.non_success_cost;
    entry.cache_input_tokens += totals.cache_input_tokens;
    entry.total_cost += totals.total_cost;
    entry.first_response_byte_total_sample_count += totals.first_response_byte_total_sample_count;
    entry.first_response_byte_total_sum_ms += totals.first_response_byte_total_sum_ms;
    entry.total_latency_sample_count += totals.total_latency_sample_count;
    entry.total_latency_sum_ms += totals.total_latency_sum_ms;
    entry.usage_breakdown.add_coarse_rollup_totals(
        totals.cache_write_tokens,
        totals.cache_read_tokens,
        totals.output_tokens,
        totals.total_cost,
    );
}

fn dashboard_activity_source_scope_cache_key(source_scope: InvocationSourceScope) -> &'static str {
    match source_scope {
        InvocationSourceScope::ProxyOnly => "proxy_only",
        InvocationSourceScope::All => "all",
    }
}

pub(crate) fn build_dashboard_activity_snapshot_selection(
    range: &str,
    exact_range: ExactUtcRange,
    reporting_tz: Tz,
    source_scope: InvocationSourceScope,
    recent_limit: usize,
    include_accounts: bool,
    include_recent: bool,
) -> DashboardActivitySnapshotSelection {
    DashboardActivitySnapshotSelection {
        range: range.to_string(),
        range_anchor: dashboard_activity_snapshot_selection_anchor(
            range,
            exact_range,
            reporting_tz,
        ),
        time_zone: reporting_tz.to_string(),
        source_scope: dashboard_activity_source_scope_cache_key(source_scope).to_string(),
        recent_limit,
        include_accounts,
        include_recent,
    }
}

fn dashboard_activity_snapshot_selection_anchor(
    range: &str,
    exact_range: ExactUtcRange,
    reporting_tz: Tz,
) -> String {
    if parse_duration_spec(range).is_ok() {
        return "rolling".to_string();
    }
    exact_range
        .start
        .with_timezone(&reporting_tz)
        .format("%Y-%m-%d")
        .to_string()
}

fn resolve_dashboard_activity_exact_range(
    range_name: &str,
    reporting_tz: Tz,
) -> Result<ExactUtcRange, ApiError> {
    let range_window = resolve_range_window(range_name, reporting_tz).map_err(ApiError::from)?;
    Ok(ExactUtcRange {
        start: range_window.start,
        end: range_window.end,
    })
}

pub(crate) fn resolve_dashboard_activity_cached_range(
    range_name: &str,
    reporting_tz: Tz,
) -> Result<ExactUtcRange, ApiError> {
    resolve_dashboard_activity_exact_range(range_name, reporting_tz)
}

fn dashboard_activity_full_hour_exact_range(
    full_hour_range: Option<(i64, i64)>,
) -> Result<Option<ExactUtcRange>, ApiError> {
    let Some((start_epoch, end_epoch)) = full_hour_range else {
        return Ok(None);
    };
    Ok(Some(ExactUtcRange {
        start: Utc.timestamp_opt(start_epoch, 0).single().ok_or_else(|| {
            ApiError::from(anyhow!("invalid dashboard activity full-hour start epoch"))
        })?,
        end: Utc.timestamp_opt(end_epoch, 0).single().ok_or_else(|| {
            ApiError::from(anyhow!("invalid dashboard activity full-hour end epoch"))
        })?,
    }))
}

pub(crate) fn sort_dashboard_activity_accounts(accounts: &mut [DashboardActivityAccountResponse]) {
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
}

pub(crate) fn dashboard_activity_account_from_live(
    live_account: &DashboardActivityLiveAccount,
    meta: Option<&UpstreamAccountActivityMetaRow>,
    range: ExactUtcRange,
    current_snapshot: DashboardActivityCurrentSnapshot,
    model_performance_available: bool,
    effective_routing_rule: Option<crate::upstream_accounts::EffectiveRoutingRule>,
    recent_invocations: Vec<PromptCacheConversationInvocationPreviewResponse>,
) -> DashboardActivityAccountResponse {
    let status_fields =
        meta.map(|row| build_upstream_account_activity_status_fields(row, Utc::now()));
    let display_name_hint = recent_invocations.iter().find_map(|invocation| {
        invocation
            .upstream_account_name
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    });
    let plan_type_hint = recent_invocations.iter().find_map(|invocation| {
        invocation
            .upstream_account_plan_type
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    });
    let display_name = live_account
        .upstream_account_id
        .map(|id| {
            resolve_upstream_account_activity_display_name(id, meta, display_name_hint.as_deref())
        })
        .unwrap_or_else(|| "未分配上游账号".to_string());

    DashboardActivityAccountResponse {
        account_key: live_account.account_key.clone(),
        upstream_account_id: live_account.upstream_account_id,
        display_name,
        is_unassigned: live_account.upstream_account_id.is_none(),
        latest_conversation_created_at: None,
        last_invocation_at: None,
        group_name: normalize_trimmed_optional_string_local(
            meta.and_then(|row| row.group_name.clone()),
        ),
        plan_type: normalize_trimmed_optional_string_local(
            meta.and_then(|row| row.plan_type.clone())
                .or(plan_type_hint),
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
        request_count: live_account.in_progress_invocation_count.max(0),
        success_count: 0,
        failure_count: 0,
        non_success_count: 0,
        total_tokens: 0,
        success_tokens: 0,
        non_success_tokens: 0,
        failure_tokens: 0,
        failure_cost: 0.0,
        non_success_cost: 0.0,
        total_cost: 0.0,
        usage_breakdown: UsageBreakdownAccumulator::default().into_response(),
        model_performance: ModelPerformanceAccumulator::default()
            .into_response(range, model_performance_available),
        cache_hit_rate: None,
        tokens_per_minute: Some(current_snapshot.qualified_tokens.max(0) as f64),
        spend_rate: Some(current_snapshot.total_cost.max(0.0)),
        first_byte_avg_ms: None,
        first_response_byte_total_avg_ms: None,
        avg_total_ms: None,
        current_first_response_byte_total_avg_ms: current_snapshot
            .first_response_byte_total_avg_ms(),
        current_avg_total_ms: current_snapshot.avg_total_ms(),
        in_progress_invocation_count: Some(live_account.in_progress_invocation_count),
        in_progress_phase_counts: Some(live_account.in_progress_phase_counts),
        retry_invocation_count: Some(live_account.retry_invocation_count),
        upload_bytes_per_second: live_account.upload_bytes_per_second,
        download_bytes_per_second: live_account.download_bytes_per_second,
        in_progress_wait_sum_ms: 0.0,
        in_progress_wait_sample_count: 0,
        effective_routing_rule,
        recent_invocations,
    }
}

fn merge_dashboard_activity_recent_invocations(
    mut recent_invocations: Vec<PromptCacheConversationInvocationPreviewResponse>,
    existing_recent_invocations: Vec<PromptCacheConversationInvocationPreviewResponse>,
    recent_limit: usize,
) -> Vec<PromptCacheConversationInvocationPreviewResponse> {
    recent_invocations.extend(existing_recent_invocations);
    let mut seen_keys = HashSet::with_capacity(recent_invocations.len());
    recent_invocations.retain(|invocation| {
        seen_keys.insert((invocation.invoke_id.clone(), invocation.occurred_at.clone()))
    });
    recent_invocations.sort_by(|left, right| {
        right
            .occurred_at
            .cmp(&left.occurred_at)
            .then_with(|| right.id.cmp(&left.id))
    });
    recent_invocations.truncate(recent_limit);
    recent_invocations
}

async fn load_dashboard_activity_live_recent_invocations_by_account(
    state: &AppState,
    source_scope: InvocationSourceScope,
    range: ExactUtcRange,
    recent_limit: usize,
) -> Result<HashMap<Option<i64>, Vec<PromptCacheConversationInvocationPreviewResponse>>, ApiError> {
    let mut rows = query_live_upstream_account_activity_preview_rows_per_account_limit(
        &state.pool,
        source_scope,
        range,
        recent_limit,
        UpstreamAccountActivityPreviewReadTelemetry {
            route: "dashboard",
            builder: "live_recent_overlay",
            purpose: "bounded_per_account_recent",
        },
    )
    .await?;
    overlay_runtime_upstream_account_activity_preview_rows(state, &mut rows, source_scope, range);
    overlay_runtime_terminal_upstream_account_activity_preview_rows(
        state,
        &mut rows,
        source_scope,
        range,
    )
    .await?;

    let mut recent_invocations_by_account =
        HashMap::<Option<i64>, Vec<PromptCacheConversationInvocationPreviewResponse>>::new();
    for row in rows {
        recent_invocations_by_account
            .entry(row.upstream_account_id)
            .or_default()
            .push(upstream_account_invocation_preview_from_row(row));
    }
    for recent_invocations in recent_invocations_by_account.values_mut() {
        *recent_invocations = merge_dashboard_activity_recent_invocations(
            std::mem::take(recent_invocations),
            Vec::new(),
            recent_limit,
        );
    }

    Ok(recent_invocations_by_account)
}

async fn overlay_dashboard_activity_live_accounts(
    state: &AppState,
    snapshot: &mut DashboardActivitySnapshot,
    live: DashboardActivityLiveSnapshot,
    request_range: ExactUtcRange,
    include_accounts: bool,
    include_recent: bool,
    recent_limit: usize,
) -> Result<(), ApiError> {
    snapshot.summary.stats.in_progress_conversation_count = Some(live.in_progress_invocation_count);
    snapshot.summary.stats.in_progress_retry_conversation_count = Some(live.retry_invocation_count);
    snapshot.summary.stats.in_progress_phase_counts = Some(live.in_progress_phase_counts);

    let current_snapshot_by_account = state
        .dashboard_network_speed_cache
        .snapshot_dashboard_activity_accounts(Utc::now());
    let current_snapshot_summary =
        sum_dashboard_activity_current_snapshots(current_snapshot_by_account.values().copied());
    snapshot.summary.tokens_per_minute =
        Some(current_snapshot_summary.qualified_tokens.max(0) as f64);
    snapshot.summary.spend_rate = Some(current_snapshot_summary.total_cost.max(0.0));
    snapshot.summary.current_first_response_byte_total_avg_ms = current_snapshot_summary
        .first_response_byte_total_avg_ms()
        .or(snapshot.summary.current_first_response_byte_total_avg_ms);
    snapshot.summary.current_avg_total_ms = current_snapshot_summary
        .avg_total_ms()
        .or(snapshot.summary.current_avg_total_ms);

    if !include_accounts {
        return Ok(());
    }

    let recent_source_scope = if include_recent {
        Some(resolve_default_source_scope(&state.pool).await?)
    } else {
        None
    };
    let mut refreshed_recent_invocations_by_account =
        if let Some(source_scope) = recent_source_scope {
            Some(
                load_dashboard_activity_live_recent_invocations_by_account(
                    state,
                    source_scope,
                    request_range,
                    recent_limit,
                )
                .await?,
            )
        } else {
            None
        };
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
        if let Some(live_account) = live_account {
            account.request_count = account
                .request_count
                .max(live_account.in_progress_invocation_count.max(0));
        }
        account.upload_bytes_per_second =
            live_account.map_or(0.0, |row| row.upload_bytes_per_second);
        account.download_bytes_per_second =
            live_account.map_or(0.0, |row| row.download_bytes_per_second);
        let current_snapshot = current_snapshot_by_account
            .get(&account.upstream_account_id)
            .copied()
            .unwrap_or_default();
        account.tokens_per_minute = Some(current_snapshot.qualified_tokens.max(0) as f64);
        account.spend_rate = Some(current_snapshot.total_cost.max(0.0));
        account.current_first_response_byte_total_avg_ms = current_snapshot
            .first_response_byte_total_avg_ms()
            .or(account.current_first_response_byte_total_avg_ms);
        account.current_avg_total_ms = current_snapshot
            .avg_total_ms()
            .or(account.current_avg_total_ms);
        if let Some(refreshed_recent_invocations_by_account) =
            refreshed_recent_invocations_by_account.as_mut()
            && let Some(recent_invocations) =
                refreshed_recent_invocations_by_account.remove(&account.upstream_account_id)
        {
            let existing_recent_invocations = std::mem::take(&mut account.recent_invocations);
            account.recent_invocations = merge_dashboard_activity_recent_invocations(
                recent_invocations,
                existing_recent_invocations,
                recent_limit,
            );
        }
    }

    let existing_account_keys = snapshot
        .accounts
        .iter()
        .map(|account| account.account_key.clone())
        .collect::<HashSet<_>>();
    let missing_live_accounts = live_accounts
        .values()
        .filter(|account| !existing_account_keys.contains(&account.account_key))
        .collect::<Vec<_>>();

    if !missing_live_accounts.is_empty() {
        let missing_account_ids = missing_live_accounts
            .iter()
            .filter_map(|account| account.upstream_account_id)
            .collect::<Vec<_>>();
        let account_meta =
            query_upstream_account_activity_meta(&state.pool, &missing_account_ids).await?;
        let effective_routing_rules =
            crate::upstream_accounts::load_effective_routing_rules_for_accounts(
                &state.pool,
                &missing_account_ids,
            )
            .await?;
        for live_account in missing_live_accounts {
            let meta = live_account
                .upstream_account_id
                .and_then(|id| account_meta.get(&id));
            let effective_routing_rule = live_account.upstream_account_id.map(|id| {
                effective_routing_rules
                    .get(&id)
                    .cloned()
                    .unwrap_or_else(crate::upstream_accounts::default_effective_routing_rule)
            });
            let current_snapshot = current_snapshot_by_account
                .get(&live_account.upstream_account_id)
                .copied()
                .unwrap_or_default();
            let recent_invocations = if let Some(refreshed_recent_invocations_by_account) =
                refreshed_recent_invocations_by_account.as_mut()
            {
                refreshed_recent_invocations_by_account
                    .remove(&live_account.upstream_account_id)
                    .unwrap_or_default()
            } else {
                Vec::new()
            };
            snapshot.accounts.push(dashboard_activity_account_from_live(
                live_account,
                meta,
                request_range,
                current_snapshot,
                snapshot.summary.model_performance.available,
                effective_routing_rule,
                recent_invocations,
            ));
        }
        sort_dashboard_activity_accounts(&mut snapshot.accounts);
    }

    if let Some(refreshed_recent_invocations_by_account) = refreshed_recent_invocations_by_account {
        let existing_account_ids = snapshot
            .accounts
            .iter()
            .map(|account| account.upstream_account_id)
            .collect::<HashSet<_>>();
        let missing_terminal_account_ids = refreshed_recent_invocations_by_account
            .keys()
            .copied()
            .filter(|account_id| !existing_account_ids.contains(account_id))
            .collect::<Vec<_>>();

        if !missing_terminal_account_ids.is_empty() {
            let missing_terminal_account_meta_ids = missing_terminal_account_ids
                .iter()
                .filter_map(|account_id| *account_id)
                .collect::<Vec<_>>();
            let account_meta = if missing_terminal_account_meta_ids.is_empty() {
                HashMap::new()
            } else {
                query_upstream_account_activity_meta(
                    &state.pool,
                    &missing_terminal_account_meta_ids,
                )
                .await?
            };
            let effective_routing_rules = if missing_terminal_account_meta_ids.is_empty() {
                HashMap::new()
            } else {
                crate::upstream_accounts::load_effective_routing_rules_for_accounts(
                    &state.pool,
                    &missing_terminal_account_meta_ids,
                )
                .await?
            };
            let mut refreshed_recent_invocations_by_account =
                refreshed_recent_invocations_by_account;
            for upstream_account_id in missing_terminal_account_ids {
                let recent_invocations = refreshed_recent_invocations_by_account
                    .remove(&upstream_account_id)
                    .unwrap_or_default();
                if recent_invocations.is_empty() {
                    continue;
                }

                let live_account = DashboardActivityLiveAccount {
                    account_key: upstream_account_id
                        .map(|id| format!("upstream:{id}"))
                        .unwrap_or_else(|| "unassigned".to_string()),
                    upstream_account_id,
                    in_progress_invocation_count: 0,
                    in_progress_phase_counts: InvocationPhaseCountsResponse::default(),
                    retry_invocation_count: 0,
                    upload_bytes_per_second: 0.0,
                    download_bytes_per_second: 0.0,
                    network_live_bucket: None,
                };
                let meta = upstream_account_id.and_then(|id| account_meta.get(&id));
                let effective_routing_rule = upstream_account_id.map(|id| {
                    effective_routing_rules
                        .get(&id)
                        .cloned()
                        .unwrap_or_else(crate::upstream_accounts::default_effective_routing_rule)
                });
                let current_snapshot = current_snapshot_by_account
                    .get(&upstream_account_id)
                    .copied()
                    .unwrap_or_default();
                snapshot.accounts.push(dashboard_activity_account_from_live(
                    &live_account,
                    meta,
                    request_range,
                    current_snapshot,
                    snapshot.summary.model_performance.available,
                    effective_routing_rule,
                    recent_invocations,
                ));
            }
            sort_dashboard_activity_accounts(&mut snapshot.accounts);
        }
    }

    snapshot.summary = build_dashboard_activity_summary(
        &snapshot.accounts,
        true,
        current_snapshot_summary,
        snapshot.summary.current_first_response_byte_total_avg_ms,
        snapshot.summary.current_avg_total_ms,
        snapshot.summary.model_performance.clone(),
    );
    dashboard_activity_apply_materialized_archive_fallback_to_stats(
        &mut snapshot.summary.stats,
        snapshot.materialized_archive_fallback_totals,
    );
    if snapshot.materialized_archive_details_limited {
        dashboard_activity_clear_materialized_archive_detail_fields(&mut snapshot.summary.stats);
    }

    Ok(())
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

async fn load_dashboard_activity_current_minute_rows(
    state: &AppState,
    source_scope: InvocationSourceScope,
    range: ExactUtcRange,
) -> Result<Vec<UpstreamAccountInvocationPreviewRow>, ApiError> {
    let current_window = dashboard_activity_last_complete_minute_window(range);
    let mut rows = query_live_upstream_account_activity_preview_rows_with_limit(
        &state.pool,
        source_scope,
        current_window,
        None,
        None,
        false,
        UpstreamAccountActivityPreviewReadTelemetry {
            route: "dashboard",
            builder: "current_minute",
            purpose: "current_minute_preview_rows",
        },
    )
    .await?;
    let mut rows_by_key = rows
        .drain(..)
        .map(|row| ((row.invoke_id.clone(), row.occurred_at.clone()), row))
        .collect::<HashMap<_, _>>();
    let mut unresolved_runtime_account_keys = HashSet::new();

    for record in state.proxy_runtime_invocations.snapshot() {
        let Some(mut row) =
            runtime_upstream_account_activity_preview_row_with_terminal(record, source_scope, true)
        else {
            continue;
        };
        let Some(occurred_at) = parse_to_utc_datetime(&row.occurred_at) else {
            continue;
        };
        if occurred_at < current_window.start || occurred_at >= current_window.end {
            continue;
        }
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
        if row.upstream_account_id.is_none() {
            unresolved_runtime_account_keys.insert(key.clone());
        }
        rows_by_key.insert(key, row);
    }

    if !unresolved_runtime_account_keys.is_empty() {
        for fallback_row in query_runtime_recent_account_fallback_rows(
            &state.pool,
            source_scope,
            &unresolved_runtime_account_keys,
        )
        .await?
        {
            let key = (
                fallback_row.invoke_id.clone(),
                fallback_row.occurred_at.clone(),
            );
            if let Some(row) = rows_by_key.get_mut(&key) {
                if row.upstream_account_id.is_none() {
                    row.upstream_account_id = fallback_row.upstream_account_id;
                }
                if row.upstream_account_name.is_none() {
                    row.upstream_account_name = fallback_row.upstream_account_name;
                }
                if row.upstream_account_plan_type.is_none() {
                    row.upstream_account_plan_type = fallback_row.upstream_account_plan_type;
                }
            }
        }
    }

    Ok(rows_by_key.into_values().collect())
}

async fn load_dashboard_activity_current_minute_accumulators_by_account(
    state: &AppState,
    source_scope: InvocationSourceScope,
    range: ExactUtcRange,
) -> Result<HashMap<Option<i64>, DashboardActivityCurrentMinuteAccumulator>, ApiError> {
    let mut accumulators = HashMap::<Option<i64>, DashboardActivityCurrentMinuteAccumulator>::new();
    for row in load_dashboard_activity_current_minute_rows(state, source_scope, range).await? {
        accumulators
            .entry(row.upstream_account_id)
            .or_default()
            .add_row(&row);
    }
    Ok(accumulators)
}

fn sum_dashboard_activity_current_minute_accumulators(
    accumulators: impl IntoIterator<Item = DashboardActivityCurrentMinuteAccumulator>,
) -> DashboardActivityCurrentMinuteAccumulator {
    let mut total = DashboardActivityCurrentMinuteAccumulator::default();
    for accumulator in accumulators {
        total.merge(accumulator);
    }
    total
}

pub(crate) fn sum_dashboard_activity_current_snapshots(
    snapshots: impl IntoIterator<Item = DashboardActivityCurrentSnapshot>,
) -> DashboardActivityCurrentSnapshot {
    let mut total = DashboardActivityCurrentSnapshot::default();
    for snapshot in snapshots {
        total.add_assign(snapshot);
    }
    total
}

pub(crate) fn build_dashboard_activity_summary(
    accounts: &[DashboardActivityAccountResponse],
    include_live_counts: bool,
    current_snapshot: DashboardActivityCurrentSnapshot,
    latest_first_response_byte_total_in_range: Option<f64>,
    latest_avg_total_in_range: Option<f64>,
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
        tokens_per_minute: Some(
            sum_optional_rates(accounts, |account| account.tokens_per_minute)
                .unwrap_or(current_snapshot.qualified_tokens.max(0) as f64),
        ),
        spend_rate: Some(
            sum_optional_rates(accounts, |account| account.spend_rate)
                .unwrap_or(current_snapshot.total_cost.max(0.0)),
        ),
        current_first_response_byte_total_avg_ms: current_snapshot
            .first_response_byte_total_avg_ms()
            .or(latest_first_response_byte_total_in_range),
        current_avg_total_ms: current_snapshot
            .avg_total_ms()
            .or(latest_avg_total_in_range),
        model_performance,
    }
}

fn build_dashboard_activity_latency_summary(
    current_snapshot: DashboardActivityCurrentSnapshot,
    latest_first_response_byte_total_in_range: Option<f64>,
    latest_avg_total_in_range: Option<f64>,
) -> (Option<f64>, Option<f64>) {
    (
        current_snapshot
            .first_response_byte_total_avg_ms()
            .or(latest_first_response_byte_total_in_range),
        current_snapshot
            .avg_total_ms()
            .or(latest_avg_total_in_range),
    )
}

async fn load_dashboard_activity_current_latency_fallback(
    state: &AppState,
    source_scope: InvocationSourceScope,
    range: ExactUtcRange,
) -> Result<(Option<f64>, Option<f64>), ApiError> {
    let metric_range = ExactUtcRange {
        start: range
            .start
            .max(range.end - ChronoDuration::minutes(DASHBOARD_ACTIVITY_RATE_WINDOW_MINUTES)),
        end: range.end,
    };
    let mut latest_first_response_byte_total = LatestTimedMetricValue::default();
    let mut latest_avg_total = LatestTimedMetricValue::default();
    for row in query_live_upstream_account_activity_aggregate_rows(
        &state.pool,
        source_scope,
        metric_range,
        true,
        DashboardActivityExcludedInvocationIdsFilter::None,
    )
    .await?
    {
        latest_first_response_byte_total.update(
            row.latest_first_response_byte_total_at,
            row.latest_first_response_byte_total_ms,
        );
        latest_avg_total.update(row.latest_avg_total_at, row.latest_avg_total_ms);
    }

    Ok((
        latest_first_response_byte_total.value,
        latest_avg_total.value,
    ))
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

pub(crate) fn dashboard_activity_last_complete_minute_window(
    range: ExactUtcRange,
) -> ExactUtcRange {
    let closed_window_end = range
        .end
        .with_second(0)
        .and_then(|value| value.with_nanosecond(0))
        .expect("valid exact minute");
    let closed_window_start = (closed_window_end - ChronoDuration::minutes(1)).max(range.start);
    ExactUtcRange {
        start: closed_window_start,
        end: closed_window_end.max(closed_window_start),
    }
}

pub(crate) async fn load_dashboard_activity_summary_only_snapshot(
    state: &AppState,
    range_name: &str,
    source_scope: InvocationSourceScope,
    range: ExactUtcRange,
) -> Result<DashboardActivitySnapshot, ApiError> {
    let retention_cutoff = shanghai_retention_cutoff(state.config.invocation_max_days);
    let model_performance_available = range.start >= retention_cutoff;
    let mut totals =
        query_hourly_backed_summary_range(state, range.start, range.end, source_scope).await?;
    totals = totals.add(
        dashboard_activity_historical_live_gap_totals(state, source_scope, range, retention_cutoff)
            .await?,
    );
    let mut stats = totals.into_response();
    stats.non_success_cost = Some(totals.non_success_cost);
    let augmentation = load_summary_live_augmentation(
        state,
        source_scope,
        None,
        Some((range.start, range.end)),
        SummaryLiveAugmentationPolicy {
            include_in_progress: range_name != "yesterday",
            // Hourly summary rollups do not retain non-success token totals; a raw full-range
            // scan here would defeat the summary-only fast path.
            include_non_success_tokens: false,
        },
    )
    .await?;
    apply_summary_live_augmentation(&mut stats, augmentation);
    let current_snapshot = if range_name == "yesterday" {
        let current_minute_by_account =
            load_dashboard_activity_current_minute_accumulators_by_account(
                state,
                source_scope,
                range,
            )
            .await?;
        sum_dashboard_activity_current_minute_accumulators(current_minute_by_account.into_values())
            .into_current_snapshot()
    } else {
        sum_dashboard_activity_current_snapshots(
            state
                .dashboard_network_speed_cache
                .snapshot_dashboard_activity_accounts(range.end)
                .into_values(),
        )
    };
    let snapshot_first_response_byte_total_avg_ms =
        current_snapshot.first_response_byte_total_avg_ms();
    let snapshot_avg_total_ms = current_snapshot.avg_total_ms();
    let (latest_first_response_byte_total_avg_ms, latest_avg_total_ms) =
        if snapshot_first_response_byte_total_avg_ms.is_none() || snapshot_avg_total_ms.is_none() {
            load_dashboard_activity_current_latency_fallback(state, source_scope, range).await?
        } else {
            (None, None)
        };
    let (current_first_response_byte_total_avg_ms, current_avg_total_ms) =
        build_dashboard_activity_latency_summary(
            current_snapshot,
            latest_first_response_byte_total_avg_ms,
            latest_avg_total_ms,
        );

    Ok(DashboardActivitySnapshot {
        range: range_name.to_string(),
        range_start: range.start,
        range_end: range.end,
        accounts: Vec::new(),
        summary: DashboardActivitySummaryResponse {
            stats,
            tokens_per_minute: Some(current_snapshot.qualified_tokens.max(0) as f64),
            spend_rate: Some(current_snapshot.total_cost.max(0.0)),
            current_first_response_byte_total_avg_ms,
            current_avg_total_ms,
            model_performance: ModelPerformanceAccumulator::default()
                .into_response(range, model_performance_available),
        },
        materialized_archive_fallback_totals: StatsTotals::default(),
        materialized_archive_details_limited: false,
        build_telemetry: DashboardActivityBuildTelemetry::default(),
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
    let range = resolve_dashboard_activity_exact_range(range_name, reporting_tz)?;
    load_dashboard_activity_snapshot_for_range(
        state,
        range_name,
        reporting_tz,
        range,
        recent_limit,
        include_accounts,
        include_recent,
        in_progress_counts_override,
    )
    .await
}

async fn load_dashboard_activity_snapshot_for_range(
    state: &AppState,
    range_name: &str,
    _reporting_tz: Tz,
    range: ExactUtcRange,
    recent_limit: usize,
    include_accounts: bool,
    include_recent: bool,
    in_progress_counts_override: Option<HashMap<Option<i64>, UpstreamAccountInProgressSummary>>,
) -> Result<DashboardActivitySnapshot, ApiError> {
    let source_scope = resolve_default_source_scope(&state.pool).await?;
    if !include_accounts {
        return load_dashboard_activity_summary_only_snapshot(
            state,
            range_name,
            source_scope,
            range,
        )
        .await;
    }
    let build = load_dashboard_activity_account_build_result(
        state,
        range_name,
        range,
        recent_limit,
        include_recent,
        in_progress_counts_override,
        DashboardActivityAccountBuilderKind::DashboardFull,
    )
    .await?;
    let mut summary = build_dashboard_activity_summary(
        &build.accounts,
        range_name != "yesterday",
        build.current_snapshot_summary,
        build.latest_first_response_byte_total_in_range,
        build.latest_avg_total_in_range,
        build.summary_model_performance,
    );
    dashboard_activity_apply_materialized_archive_fallback_to_stats(
        &mut summary.stats,
        build.materialized_archive_fallback_totals,
    );
    if build.materialized_archive_details_limited {
        dashboard_activity_clear_materialized_archive_detail_fields(&mut summary.stats);
    }
    Ok(DashboardActivitySnapshot {
        range: range_name.to_string(),
        range_start: range.start,
        range_end: range.end,
        accounts: build.accounts,
        summary,
        materialized_archive_fallback_totals: build.materialized_archive_fallback_totals,
        materialized_archive_details_limited: build.materialized_archive_details_limited,
        build_telemetry: build.build_telemetry,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DashboardActivityAccountBuilderKind {
    DashboardFull,
    UpstreamAccount,
}

impl DashboardActivityAccountBuilderKind {
    fn preview_read_telemetry(self) -> UpstreamAccountActivityPreviewReadTelemetry {
        match self {
            Self::DashboardFull => UpstreamAccountActivityPreviewReadTelemetry {
                route: "dashboard",
                builder: "dashboard_full",
                purpose: "bounded_per_account_recent",
            },
            Self::UpstreamAccount => UpstreamAccountActivityPreviewReadTelemetry {
                route: "upstream_account",
                builder: "upstream_account",
                purpose: "bounded_per_account_recent",
            },
        }
    }
}

#[derive(Debug)]
struct DashboardActivityAccountBuildResult {
    accounts: Vec<DashboardActivityAccountResponse>,
    current_snapshot_summary: DashboardActivityCurrentSnapshot,
    latest_first_response_byte_total_in_range: Option<f64>,
    latest_avg_total_in_range: Option<f64>,
    summary_model_performance: ModelPerformanceResponse,
    materialized_archive_fallback_totals: StatsTotals,
    materialized_archive_details_limited: bool,
    build_telemetry: DashboardActivityBuildTelemetry,
}

fn build_dashboard_activity_account_response(
    range_name: &str,
    range: ExactUtcRange,
    model_performance_available: bool,
    current_snapshot_by_account: &HashMap<Option<i64>, DashboardActivityCurrentSnapshot>,
    account_meta: &HashMap<i64, UpstreamAccountActivityMetaRow>,
    effective_routing_rules: &HashMap<i64, crate::upstream_accounts::EffectiveRoutingRule>,
    in_progress_counts: &HashMap<Option<i64>, UpstreamAccountInProgressSummary>,
    upstream_account_id: Option<i64>,
    aggregate: UpstreamAccountActivityAccumulator,
) -> DashboardActivityAccountResponse {
    let meta = upstream_account_id.and_then(|id| account_meta.get(&id));
    let status_fields =
        meta.map(|row| build_upstream_account_activity_status_fields(row, Utc::now()));
    let model_performance = aggregate
        .model_performance
        .into_response(range, model_performance_available);
    let current_snapshot = current_snapshot_by_account
        .get(&upstream_account_id)
        .copied()
        .unwrap_or_default();
    let (current_first_response_byte_total_avg_ms, current_avg_total_ms) =
        build_dashboard_activity_latency_summary(
            current_snapshot,
            aggregate.latest_first_response_byte_total_ms,
            aggregate.latest_avg_total_ms,
        );
    let tokens_per_minute = Some(current_snapshot.qualified_tokens.max(0) as f64);
    let spend_rate = Some(current_snapshot.total_cost.max(0.0));
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
        is_unassigned: upstream_account_id.is_none(),
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
        first_byte_avg_ms: (aggregate.first_response_byte_total_sample_count > 0).then_some(
            aggregate.first_response_byte_total_sum_ms
                / aggregate.first_response_byte_total_sample_count as f64,
        ),
        first_response_byte_total_avg_ms: (aggregate.first_response_byte_total_sample_count > 0)
            .then_some(
                aggregate.first_response_byte_total_sum_ms
                    / aggregate.first_response_byte_total_sample_count as f64,
            ),
        avg_total_ms: (aggregate.total_latency_sample_count > 0).then_some(
            aggregate.total_latency_sum_ms / aggregate.total_latency_sample_count as f64,
        ),
        current_first_response_byte_total_avg_ms,
        current_avg_total_ms,
        in_progress_invocation_count,
        in_progress_phase_counts,
        retry_invocation_count,
        upload_bytes_per_second: 0.0,
        download_bytes_per_second: 0.0,
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
}

async fn load_dashboard_activity_account_build_result(
    state: &AppState,
    range_name: &str,
    range: ExactUtcRange,
    recent_limit: usize,
    include_recent: bool,
    in_progress_counts_override: Option<HashMap<Option<i64>, UpstreamAccountInProgressSummary>>,
    builder_kind: DashboardActivityAccountBuilderKind,
) -> Result<DashboardActivityAccountBuildResult, ApiError> {
    let retention_cutoff = shanghai_retention_cutoff(state.config.invocation_max_days);
    let model_performance_available = range.start >= retention_cutoff;
    let source_scope = resolve_default_source_scope(&state.pool).await?;
    let mut account_activity = HashMap::<Option<i64>, UpstreamAccountActivityAccumulator>::new();
    let mut model_performance = ModelPerformanceAccumulator::default();
    let materialized_archive_fallback_totals = StatsTotals::default();
    let mut materialized_archive_details_limited = false;
    let current_snapshot_by_account = if range_name == "yesterday" {
        load_dashboard_activity_current_minute_accumulators_by_account(state, source_scope, range)
            .await?
            .into_iter()
            .map(|(upstream_account_id, accumulator)| {
                (upstream_account_id, accumulator.into_current_snapshot())
            })
            .collect::<HashMap<_, _>>()
    } else {
        state
            .dashboard_network_speed_cache
            .snapshot_dashboard_activity_accounts(range.end)
    };
    let current_snapshot_summary =
        sum_dashboard_activity_current_snapshots(current_snapshot_by_account.values().copied());
    let model_performance_duration_overrides = if builder_kind
        == DashboardActivityAccountBuilderKind::DashboardFull
        && model_performance_available
    {
        Some(
            query_live_model_performance_duration_overrides(&state.pool, source_scope, range, true)
                .await?,
        )
    } else {
        None
    };
    for row in query_live_upstream_account_activity_aggregate_rows(
        &state.pool,
        source_scope,
        range,
        true,
        DashboardActivityExcludedInvocationIdsFilter::None,
    )
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
        merge_latest_timed_metric(
            &mut entry.latest_first_response_byte_total_at,
            &mut entry.latest_first_response_byte_total_ms,
            row.latest_first_response_byte_total_at,
            row.latest_first_response_byte_total_ms,
        );
        merge_latest_timed_metric(
            &mut entry.latest_avg_total_at,
            &mut entry.latest_avg_total_ms,
            row.latest_avg_total_at,
            row.latest_avg_total_ms,
        );
    }
    for row in query_live_upstream_account_usage_breakdown_rows(
        &state.pool,
        source_scope,
        range,
        true,
        true,
        DashboardActivityExcludedInvocationIdsFilter::None,
    )
    .await?
    {
        let entry = account_activity.entry(row.upstream_account_id).or_default();
        entry.usage_breakdown.add_aggregate_row(&row);
        entry.model_performance.add_aggregate_row(&row);
        model_performance.add_aggregate_row(&row);
    }
    if range.start < retention_cutoff {
        let archive_rows = query_completed_invocation_archive_activity_aggregate_rows(
            &state.pool,
            source_scope,
            range,
        )
        .await?;
        let QueryCompletedInvocationArchiveActivityAggregateRows {
            aggregates: archived_aggregates,
            usage_breakdowns: archived_usage_breakdowns,
            skipped_materialized_ranges,
        } = archive_rows;
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
            merge_latest_timed_metric(
                &mut entry.latest_first_response_byte_total_at,
                &mut entry.latest_first_response_byte_total_ms,
                row.latest_first_response_byte_total_at,
                row.latest_first_response_byte_total_ms,
            );
            merge_latest_timed_metric(
                &mut entry.latest_avg_total_at,
                &mut entry.latest_avg_total_ms,
                row.latest_avg_total_at,
                row.latest_avg_total_ms,
            );
        }
        for row in archived_usage_breakdowns {
            account_activity
                .entry(row.upstream_account_id)
                .or_default()
                .usage_breakdown
                .add_aggregate_row(&row);
        }
        if !skipped_materialized_ranges.is_empty() {
            let global_fallback_totals = dashboard_activity_materialized_archive_fallback_totals(
                state,
                source_scope,
                skipped_materialized_ranges.clone(),
            )
            .await?;
            if dashboard_activity_stats_totals_has_values(global_fallback_totals) {
                if builder_kind == DashboardActivityAccountBuilderKind::DashboardFull {
                    materialized_archive_details_limited = true;
                }
                let mut account_fallback_totals =
                    dashboard_activity_materialized_archive_account_fallback_totals(
                        state,
                        source_scope,
                        &skipped_materialized_ranges,
                    )
                    .await?;
                let account_fallback_sum = account_fallback_totals
                    .values()
                    .fold(StatsTotals::default(), |total, account_totals| {
                        total.add(account_totals.stats_totals())
                    });
                let residual_fallback_totals = dashboard_activity_stats_totals_subtract(
                    global_fallback_totals,
                    account_fallback_sum,
                );
                if dashboard_activity_stats_totals_has_values(residual_fallback_totals) {
                    account_fallback_totals.entry(None).or_default().add_assign(
                        DashboardActivityAccountFallbackTotals::from_stats_totals(
                            residual_fallback_totals,
                        ),
                    );
                }
                for (upstream_account_id, totals) in account_fallback_totals {
                    dashboard_activity_merge_account_fallback_totals(
                        account_activity.entry(upstream_account_id).or_default(),
                        totals,
                    );
                }
            }
        }
    }
    if let Some(model_performance_duration_overrides) = model_performance_duration_overrides {
        model_performance.wall_clock_usage_duration_ms =
            model_performance_duration_overrides.total_wall_clock_ms;
        for (group, wall_clock_usage_duration_ms) in
            model_performance_duration_overrides.by_group_wall_clock_ms
        {
            if let Some(entry) = model_performance.models.get_mut(&group) {
                entry.wall_clock_usage_duration_ms = Some(wall_clock_usage_duration_ms);
            }
        }
        for (upstream_account_id, wall_clock_usage_duration_ms) in
            model_performance_duration_overrides.by_account_wall_clock_ms
        {
            if let Some(entry) = account_activity.get_mut(&upstream_account_id) {
                entry.model_performance.wall_clock_usage_duration_ms =
                    Some(wall_clock_usage_duration_ms);
            }
        }
        for (key, wall_clock_usage_duration_ms) in
            model_performance_duration_overrides.by_account_group_wall_clock_ms
        {
            if let Some(entry) = account_activity.get_mut(&key.upstream_account_id)
                && let Some(model_entry) = entry.model_performance.models.get_mut(&key.group)
            {
                model_entry.wall_clock_usage_duration_ms = Some(wall_clock_usage_duration_ms);
            }
        }
    }
    let mut build_telemetry = DashboardActivityBuildTelemetry::default();
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

    if include_recent {
        let hydrated_rows =
            query_live_upstream_account_activity_preview_rows_per_account_limit_with_stats(
                &state.pool,
                source_scope,
                range,
                recent_limit,
                builder_kind.preview_read_telemetry(),
            )
            .await?;
        build_telemetry = DashboardActivityBuildTelemetry {
            preview_read_mode: "bounded_per_account",
            candidate_preview_id_count: hydrated_rows.candidate_preview_id_count,
            hydrated_preview_row_count: hydrated_rows.hydrated_preview_row_count,
        };
        let mut recent_rows_by_account =
            HashMap::<Option<i64>, Vec<PromptCacheConversationInvocationPreviewResponse>>::new();
        let mut recent_rows = hydrated_rows.rows;
        overlay_runtime_upstream_account_activity_preview_rows(
            state,
            &mut recent_rows,
            source_scope,
            range,
        );
        overlay_runtime_terminal_upstream_account_activity_preview_rows(
            state,
            &mut recent_rows,
            source_scope,
            range,
        )
        .await?;
        let live_ids = recent_rows.iter().map(|row| row.id).collect::<HashSet<_>>();
        for row in recent_rows {
            merge_upstream_account_activity_recent_row_metadata(&mut account_activity, &row);
            recent_rows_by_account
                .entry(row.upstream_account_id)
                .or_default()
                .push(upstream_account_invocation_preview_from_row(row));
        }
        if range.start < retention_cutoff {
            let mut archived_rows = crate::stats::query_completed_invocation_archive_preview_rows(
                &state.pool,
                source_scope,
                range,
                Some(&live_ids),
            )
            .await?;
            let archived_row_ids = archived_rows.iter().map(|row| row.id).collect::<Vec<_>>();
            let persisted_live_ids = query_live_upstream_account_activity_existing_invocation_ids(
                &state.pool,
                source_scope,
                &archived_row_ids,
            )
            .await?;
            archived_rows.retain(|row| !persisted_live_ids.contains(&row.id));
            for row in archived_rows {
                merge_upstream_account_activity_recent_row_metadata(&mut account_activity, &row);
                recent_rows_by_account
                    .entry(row.upstream_account_id)
                    .or_default()
                    .push(upstream_account_invocation_preview_from_row(row));
            }
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

    let mut latest_first_response_byte_total_in_range = LatestTimedMetricValue::default();
    let mut latest_avg_total_in_range = LatestTimedMetricValue::default();
    for aggregate in account_activity.values() {
        latest_first_response_byte_total_in_range.update(
            aggregate.latest_first_response_byte_total_at.clone(),
            aggregate.latest_first_response_byte_total_ms,
        );
        latest_avg_total_in_range.update(
            aggregate.latest_avg_total_at.clone(),
            aggregate.latest_avg_total_ms,
        );
    }

    let account_ids = account_activity
        .keys()
        .filter_map(|id| *id)
        .collect::<Vec<_>>();
    let account_meta = query_upstream_account_activity_meta(&state.pool, &account_ids).await?;
    let effective_routing_rules =
        crate::upstream_accounts::load_effective_routing_rules_for_accounts(
            &state.pool,
            &account_ids,
        )
        .await?;
    let mut accounts = account_activity
        .into_iter()
        .map(|(upstream_account_id, aggregate)| {
            build_dashboard_activity_account_response(
                range_name,
                range,
                model_performance_available,
                &current_snapshot_by_account,
                &account_meta,
                &effective_routing_rules,
                &in_progress_counts,
                upstream_account_id,
                aggregate,
            )
        })
        .collect::<Vec<_>>();
    sort_dashboard_activity_accounts(&mut accounts);
    Ok(DashboardActivityAccountBuildResult {
        accounts,
        current_snapshot_summary,
        latest_first_response_byte_total_in_range: latest_first_response_byte_total_in_range.value,
        latest_avg_total_in_range: latest_avg_total_in_range.value,
        summary_model_performance: model_performance
            .into_response(range, model_performance_available),
        materialized_archive_fallback_totals,
        materialized_archive_details_limited,
        build_telemetry,
    })
}

async fn load_dashboard_activity_snapshot_cached(
    state: &AppState,
    range_name: &str,
    reporting_tz: Tz,
    recent_limit: usize,
    include_accounts: bool,
    include_recent: bool,
    in_progress_counts_override: Option<HashMap<Option<i64>, UpstreamAccountInProgressSummary>>,
) -> Result<
    (
        DashboardActivitySnapshot,
        DashboardActivitySnapshotCacheOutcome,
    ),
    ApiError,
> {
    if range_name == "yesterday" {
        let started_at = Instant::now();
        let range = resolve_dashboard_activity_exact_range(range_name, reporting_tz)?;
        let snapshot = load_dashboard_activity_snapshot_for_range(
            state,
            range_name,
            reporting_tz,
            range,
            recent_limit,
            include_accounts,
            include_recent,
            in_progress_counts_override.clone(),
        )
        .await?;
        return Ok((
            snapshot,
            DashboardActivitySnapshotCacheOutcome {
                cache_hit_or_miss: "uncached",
                cache_bypass_reason: "yesterday_exact",
                coalesced_waiter_count: 0,
                db_build_elapsed_ms: started_at.elapsed().as_millis() as u64,
                cache_ttl_ms: 0,
                cache_entry_age_ms: 0,
                cache_entry_count: 0,
                in_flight_count: 0,
            },
        ));
    }

    let source_scope = resolve_default_source_scope(&state.pool).await?;
    let range = resolve_dashboard_activity_cached_range(range_name, reporting_tz)?;
    let selection = build_dashboard_activity_snapshot_selection(
        range_name,
        range,
        reporting_tz,
        source_scope,
        recent_limit,
        include_accounts,
        include_recent,
    );
    let mut waited_on_in_flight = false;
    let mut max_waiter_count = 0_usize;
    let cache_ttl = Duration::from_secs(DASHBOARD_ACTIVITY_SNAPSHOT_CACHE_TTL_SECS);
    let cache_ttl_ms = DASHBOARD_ACTIVITY_SNAPSHOT_CACHE_TTL_SECS * 1_000;

    loop {
        let mut wait_on: Option<tokio::sync::watch::Receiver<bool>> = None;
        let mut flight_guard: Option<DashboardActivitySnapshotFlightGuard> = None;
        {
            let mut cache = state.dashboard_activity_snapshot_cache.lock().await;
            cache
                .entries
                .retain(|_, entry| entry.cached_at.elapsed() <= cache_ttl);
            let cache_entry_count = cache.entries.len();
            let in_flight_count = cache.in_flight.len();
            if let Some(entry) = cache.entries.get(&selection)
                && entry.cached_at.elapsed() <= cache_ttl
            {
                let cache_entry_age_ms = entry.cached_at.elapsed().as_millis() as u64;
                return Ok((
                    entry.response.clone(),
                    DashboardActivitySnapshotCacheOutcome {
                        cache_hit_or_miss: if waited_on_in_flight {
                            "wait_on_in_flight"
                        } else {
                            "cache_hit"
                        },
                        cache_bypass_reason: "none",
                        coalesced_waiter_count: max_waiter_count,
                        db_build_elapsed_ms: 0,
                        cache_ttl_ms,
                        cache_entry_age_ms,
                        cache_entry_count,
                        in_flight_count,
                    },
                ));
            }

            if let Some(in_flight) = cache.in_flight.get_mut(&selection) {
                in_flight.waiter_count += 1;
                max_waiter_count = max_waiter_count.max(in_flight.waiter_count);
                wait_on = Some(in_flight.signal.subscribe());
            } else {
                let (signal, _receiver) = tokio::sync::watch::channel(false);
                cache.in_flight.insert(
                    selection.clone(),
                    DashboardActivitySnapshotInFlight {
                        signal,
                        waiter_count: 0,
                    },
                );
                flight_guard = Some(DashboardActivitySnapshotFlightGuard::new(
                    state.dashboard_activity_snapshot_cache.clone(),
                    selection.clone(),
                ));
            }
        }

        if let Some(mut receiver) = wait_on {
            waited_on_in_flight = true;
            if !*receiver.borrow() {
                let _ = receiver.changed().await;
            }
            continue;
        }

        let build_started_at = Instant::now();
        let result = load_dashboard_activity_snapshot_for_range(
            state,
            range_name,
            reporting_tz,
            range,
            recent_limit,
            include_accounts,
            include_recent,
            in_progress_counts_override.clone(),
        )
        .await;
        let db_build_elapsed_ms = build_started_at.elapsed().as_millis() as u64;

        let mut cache = state.dashboard_activity_snapshot_cache.lock().await;
        let in_flight = cache.in_flight.remove(&selection);
        if let Some(guard) = flight_guard.as_mut() {
            guard.disarm();
        }
        let coalesced_waiter_count = in_flight.as_ref().map_or(0, |flight| flight.waiter_count);
        if let Some(in_flight) = in_flight {
            if let Ok(snapshot) = &result {
                cache.entries.insert(
                    selection.clone(),
                    DashboardActivitySnapshotCacheEntry {
                        cached_at: Instant::now(),
                        response: snapshot.clone(),
                    },
                );
            }
            let _ = in_flight.signal.send(true);
        }
        let cache_entry_count = cache.entries.len();
        let in_flight_count = cache.in_flight.len();

        return result.map(|snapshot| {
            (
                snapshot,
                DashboardActivitySnapshotCacheOutcome {
                    cache_hit_or_miss: "cache_miss_build",
                    cache_bypass_reason: "none",
                    coalesced_waiter_count,
                    db_build_elapsed_ms,
                    cache_ttl_ms,
                    cache_entry_age_ms: 0,
                    cache_entry_count,
                    in_flight_count,
                },
            )
        });
    }
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
        current_first_response_byte_total_avg_ms: account.current_first_response_byte_total_avg_ms,
        current_avg_total_ms: account.current_avg_total_ms,
        in_progress_invocation_count: account.in_progress_invocation_count,
        in_progress_phase_counts: account.in_progress_phase_counts,
        retry_invocation_count: account.retry_invocation_count,
        upload_bytes_per_second: account.upload_bytes_per_second,
        download_bytes_per_second: account.download_bytes_per_second,
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
    let started_at = Instant::now();
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
    let network_live_bucket = live
        .as_ref()
        .and_then(|snapshot| snapshot.network_live_bucket.clone());
    let network_realtime_rate = live
        .as_ref()
        .and_then(|snapshot| snapshot.network_realtime_rate.clone());
    let include_recent = params.include_recent.unwrap_or(true);
    let request_range =
        resolve_dashboard_activity_exact_range(params.range.as_str(), reporting_tz)?;
    let (mut snapshot, cache_outcome) = load_dashboard_activity_snapshot_cached(
        state.as_ref(),
        params.range.as_str(),
        reporting_tz,
        recent_limit,
        params.include_accounts,
        include_recent,
        live.as_ref()
            .map(dashboard_live_snapshot_in_progress_counts),
    )
    .await?;
    snapshot.range_start = request_range.start;
    snapshot.range_end = request_range.end;
    let live_revision = live.as_ref().map_or(0, |snapshot| snapshot.revision);
    let live_overlay_started_at = Instant::now();
    if let Some(live) = live {
        overlay_dashboard_activity_live_accounts(
            state.as_ref(),
            &mut snapshot,
            live,
            request_range,
            params.include_accounts,
            include_recent,
            recent_limit,
        )
        .await?;
    }
    let build_telemetry = snapshot.build_telemetry;
    let live_overlay_elapsed_ms = live_overlay_started_at.elapsed().as_millis() as u64;
    let range_start = format_utc_iso_precise(snapshot.range_start);
    let range_end = format_utc_iso_precise(snapshot.range_end);
    let current_rate_window = if params.range == "yesterday" {
        dashboard_activity_last_complete_minute_window(ExactUtcRange {
            start: snapshot.range_start,
            end: snapshot.range_end,
        })
    } else {
        ExactUtcRange {
            start: snapshot.range_end
                - ChronoDuration::seconds(DASHBOARD_ACTIVITY_REALTIME_WINDOW_SECONDS),
            end: snapshot.range_end,
        }
    };
    let account_count = snapshot.accounts.len();
    let accounts = params.include_accounts.then_some(snapshot.accounts);
    let response = DashboardActivityResponse {
        range: snapshot.range,
        range_start: range_start.clone(),
        range_end: range_end.clone(),
        snapshot_id: snapshot.range_end.timestamp_millis(),
        live_revision,
        rate_window: DashboardActivityRateWindowResponse {
            start: format_utc_iso_precise(current_rate_window.start),
            end: format_utc_iso_precise(current_rate_window.end),
            window_minutes: 1,
            mode: if params.range == "yesterday" {
                "last_complete_1m_sma".to_string()
            } else {
                "rolling_60s_live_mean".to_string()
            },
        },
        summary: snapshot.summary,
        network_live_bucket,
        network_realtime_rate,
        accounts,
    };
    let elapsed_ms = started_at.elapsed().as_millis() as u64;
    let builder = if params.include_accounts {
        "dashboard_full"
    } else {
        "summary_only"
    };
    if elapsed_ms >= 250 {
        tracing::warn!(
            route = "dashboard",
            builder,
            endpoint = "/api/stats/dashboard-activity",
            range = %params.range,
            include_accounts = params.include_accounts,
            include_recent,
            recent_limit,
            live_revision,
            account_count,
            build_scope = dashboard_activity_build_scope(params.include_accounts, include_recent),
            cache_hit_or_miss = cache_outcome.cache_hit_or_miss,
            cache_bypass_reason = cache_outcome.cache_bypass_reason,
            coalesced_waiter_count = cache_outcome.coalesced_waiter_count,
            db_build_elapsed_ms = cache_outcome.db_build_elapsed_ms,
            cache_ttl_ms = cache_outcome.cache_ttl_ms,
            cache_entry_age_ms = cache_outcome.cache_entry_age_ms,
            cache_entry_count = cache_outcome.cache_entry_count,
            in_flight_count = cache_outcome.in_flight_count,
            live_overlay_elapsed_ms,
            preview_read_mode = build_telemetry.preview_read_mode,
            candidate_preview_id_count = build_telemetry.candidate_preview_id_count,
            hydrated_preview_row_count = build_telemetry.hydrated_preview_row_count,
            model_performance_available = response.summary.model_performance.available,
            elapsed_ms,
            "dashboard activity snapshot exceeded slow-path threshold"
        );
    } else {
        tracing::debug!(
            route = "dashboard",
            builder,
            endpoint = "/api/stats/dashboard-activity",
            range = %params.range,
            include_accounts = params.include_accounts,
            include_recent,
            recent_limit,
            live_revision,
            account_count,
            build_scope = dashboard_activity_build_scope(params.include_accounts, include_recent),
            cache_hit_or_miss = cache_outcome.cache_hit_or_miss,
            cache_bypass_reason = cache_outcome.cache_bypass_reason,
            coalesced_waiter_count = cache_outcome.coalesced_waiter_count,
            db_build_elapsed_ms = cache_outcome.db_build_elapsed_ms,
            cache_ttl_ms = cache_outcome.cache_ttl_ms,
            cache_entry_age_ms = cache_outcome.cache_entry_age_ms,
            cache_entry_count = cache_outcome.cache_entry_count,
            in_flight_count = cache_outcome.in_flight_count,
            live_overlay_elapsed_ms,
            preview_read_mode = build_telemetry.preview_read_mode,
            candidate_preview_id_count = build_telemetry.candidate_preview_id_count,
            hydrated_preview_row_count = build_telemetry.hydrated_preview_row_count,
            model_performance_available = response.summary.model_performance.available,
            elapsed_ms,
            "dashboard activity snapshot completed"
        );
    }

    Ok(Json(response))
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
    let mut rows = query_live_upstream_account_activity_preview_rows_per_account_limit(
        &state.pool,
        source_scope,
        range,
        recent_limit,
        UpstreamAccountActivityPreviewReadTelemetry {
            route: "dashboard_recent",
            builder: "dashboard_recent",
            purpose: "bounded_per_account_recent",
        },
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
    if range.start < shanghai_retention_cutoff(state.config.invocation_max_days) {
        let live_ids = rows.iter().map(|row| row.id).collect::<HashSet<_>>();
        let mut archived_rows = crate::stats::query_completed_invocation_archive_preview_rows(
            &state.pool,
            source_scope,
            range,
            Some(&live_ids),
        )
        .await?;
        let archived_row_ids = archived_rows.iter().map(|row| row.id).collect::<Vec<_>>();
        let persisted_live_ids = query_live_upstream_account_activity_existing_invocation_ids(
            &state.pool,
            source_scope,
            &archived_row_ids,
        )
        .await?;
        archived_rows.retain(|row| !persisted_live_ids.contains(&row.id));
        rows.extend(archived_rows);
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

#[derive(Debug, Default, Clone, Copy)]
struct DashboardNetworkBucketAggregate {
    upload_bytes: i64,
    download_bytes: i64,
}

#[derive(Debug, FromRow)]
struct DashboardNetworkBucketRow {
    bucket_start_epoch_second: i64,
    upload_bytes: i64,
    download_bytes: i64,
}

fn validate_dashboard_network_timeseries_range(range: &str) -> Result<(), ApiError> {
    if matches!(range, "today" | "yesterday" | "1d") {
        return Ok(());
    }
    Err(ApiError::bad_request(anyhow!(
        "unsupported dashboard network range: {range}"
    )))
}

fn dashboard_network_download_bytes_sql(alias: &str) -> String {
    format!(
        "CASE \
           WHEN COALESCE( \
             CASE \
               WHEN json_valid({alias}.payload) \
                 AND json_type({alias}.payload, '$.forwardedBytes') IN ('integer', 'real') \
               THEN CAST(json_extract({alias}.payload, '$.forwardedBytes') AS INTEGER) \
             END, \
             {alias}.response_raw_size, \
             CAST(LENGTH({alias}.raw_response) AS INTEGER), \
             0 \
           ) < 0 THEN 0 \
           ELSE COALESCE( \
             CASE \
               WHEN json_valid({alias}.payload) \
                 AND json_type({alias}.payload, '$.forwardedBytes') IN ('integer', 'real') \
               THEN CAST(json_extract({alias}.payload, '$.forwardedBytes') AS INTEGER) \
             END, \
             {alias}.response_raw_size, \
             CAST(LENGTH({alias}.raw_response) AS INTEGER), \
             0 \
           ) \
         END"
    )
}

fn dashboard_network_direct_upload_bytes_sql(alias: &str) -> String {
    format!(
        "CASE \
           WHEN COALESCE( \
             CASE \
               WHEN json_valid({alias}.payload) \
                 AND json_type({alias}.payload, '$.upstreamApproxUploadBytes') IN ('integer', 'real') \
               THEN CAST(json_extract({alias}.payload, '$.upstreamApproxUploadBytes') AS INTEGER) \
             END, \
             CASE WHEN {alias}.request_raw_size < 0 THEN 0 ELSE COALESCE({alias}.request_raw_size, 0) END, \
             0 \
           ) < 0 THEN 0 \
           ELSE COALESCE( \
             CASE \
               WHEN json_valid({alias}.payload) \
                 AND json_type({alias}.payload, '$.upstreamApproxUploadBytes') IN ('integer', 'real') \
               THEN CAST(json_extract({alias}.payload, '$.upstreamApproxUploadBytes') AS INTEGER) \
             END, \
             CASE WHEN {alias}.request_raw_size < 0 THEN 0 ELSE COALESCE({alias}.request_raw_size, 0) END, \
             0 \
           ) \
         END"
    )
}

fn dashboard_network_direct_download_bytes_sql(alias: &str) -> String {
    format!(
        "CASE \
           WHEN COALESCE( \
             CASE \
               WHEN json_valid({alias}.payload) \
                 AND json_type({alias}.payload, '$.upstreamApproxDownloadBytes') IN ('integer', 'real') \
               THEN CAST(json_extract({alias}.payload, '$.upstreamApproxDownloadBytes') AS INTEGER) \
             END, \
             {}, \
             0 \
           ) < 0 THEN 0 \
           ELSE COALESCE( \
             CASE \
               WHEN json_valid({alias}.payload) \
                 AND json_type({alias}.payload, '$.upstreamApproxDownloadBytes') IN ('integer', 'real') \
               THEN CAST(json_extract({alias}.payload, '$.upstreamApproxDownloadBytes') AS INTEGER) \
             END, \
             {}, \
             0 \
           ) \
         END",
        dashboard_network_download_bytes_sql(alias),
        dashboard_network_download_bytes_sql(alias),
    )
}

fn dashboard_network_pool_attempt_upload_bytes_sql(alias: &str) -> String {
    format!(
        "CASE \
           WHEN COALESCE({alias}.upstream_request_header_bytes_approx, 0) + COALESCE({alias}.upstream_request_transmitted_body_bytes, 0) < 0 THEN 0 \
           ELSE COALESCE({alias}.upstream_request_header_bytes_approx, 0) + COALESCE({alias}.upstream_request_transmitted_body_bytes, 0) \
         END"
    )
}

fn dashboard_network_pool_attempt_download_bytes_sql(alias: &str) -> String {
    format!(
        "CASE \
           WHEN COALESCE({alias}.upstream_response_header_bytes_approx, 0) + COALESCE({alias}.upstream_response_body_bytes, 0) < 0 THEN 0 \
           ELSE COALESCE({alias}.upstream_response_header_bytes_approx, 0) + COALESCE({alias}.upstream_response_body_bytes, 0) \
         END"
    )
}

fn invocation_payload_upstream_account_id_sql(alias: &str) -> String {
    format!(
        "CASE WHEN json_valid({alias}.payload) \
           THEN CAST(json_extract({alias}.payload, '$.upstreamAccountId') AS INTEGER) \
         END"
    )
}

async fn query_dashboard_network_bucket_rows(
    pool: &Pool<Sqlite>,
    source_scope: InvocationSourceScope,
    range: ExactUtcRange,
    upstream_account_id: Option<Option<i64>>,
    _created_before: Option<&str>,
) -> Result<Vec<DashboardNetworkBucketRow>, ApiError> {
    if range.start >= range.end {
        return Ok(Vec::new());
    }

    let mut query = QueryBuilder::<Sqlite>::new(
        r#"
        SELECT
            ((bucket_start_epoch / 300) * 300) AS bucket_start_epoch_second,
            SUM(upload_bytes) AS upload_bytes,
            SUM(download_bytes) AS download_bytes
        FROM upstream_socket_network_minute
        WHERE bucket_start_epoch >=
        "#,
    );
    query
        .push_bind(range.start.timestamp())
        .push(" AND bucket_start_epoch < ")
        .push_bind(range.end.timestamp());
    if source_scope == InvocationSourceScope::ProxyOnly {
        query.push(" AND source = ").push_bind(SOURCE_PROXY);
    }
    if let Some(upstream_account_id) = upstream_account_id {
        query.push(" AND ");
        if let Some(upstream_account_id) = upstream_account_id {
            query
                .push("upstream_account_id = ")
                .push_bind(upstream_account_id);
        } else {
            query.push("upstream_account_id IS NULL");
        }
    }
    query.push(" GROUP BY bucket_start_epoch_second ORDER BY bucket_start_epoch_second ASC");

    Ok(query
        .build_query_as::<DashboardNetworkBucketRow>()
        .fetch_all(pool)
        .await?)
}

async fn query_dashboard_network_host_minute_bucket_rows(
    pool: &Pool<Sqlite>,
    source_scope: InvocationSourceScope,
    range: ExactUtcRange,
) -> Result<Vec<DashboardNetworkBucketRow>, ApiError> {
    query_dashboard_network_bucket_rows(pool, source_scope, range, None, None).await
}

async fn load_dashboard_network_open_bucket_snapshot_for_scope(
    pool: &Pool<Sqlite>,
    dashboard_network_speed_cache: &DashboardNetworkSpeedCache,
    source_scope: InvocationSourceScope,
    range_end: DateTime<Utc>,
    scope: DashboardNetworkScopeKey,
) -> Result<DashboardNetworkOpenBucketSnapshot, ApiError> {
    let read = dashboard_network_speed_cache.open_bucket_read_state(scope, range_end);
    if read.needs_seed {
        let seed_rows = query_dashboard_network_bucket_rows(
            pool,
            source_scope,
            ExactUtcRange {
                start: read.bucket_start,
                end: range_end.min(read.bucket_end),
            },
            scope.upstream_account_id(),
            None,
        )
        .await?;
        let seed_totals =
            seed_rows
                .into_iter()
                .fold(DashboardNetworkByteTotals::default(), |mut totals, row| {
                    totals.upload_bytes =
                        totals.upload_bytes.saturating_add(row.upload_bytes.max(0));
                    totals.download_bytes = totals
                        .download_bytes
                        .saturating_add(row.download_bytes.max(0));
                    totals
                });
        return Ok(dashboard_network_speed_cache.seed_open_bucket(
            scope,
            read.bucket_start,
            seed_totals,
            range_end,
        ));
    }

    Ok(dashboard_network_speed_cache.snapshot_open_bucket(scope, range_end))
}

async fn load_dashboard_network_open_bucket_snapshot(
    state: &AppState,
    source_scope: InvocationSourceScope,
    range_end: DateTime<Utc>,
    upstream_account_id: Option<i64>,
) -> Result<DashboardNetworkOpenBucketSnapshot, ApiError> {
    load_dashboard_network_open_bucket_snapshot_for_scope(
        &state.pool,
        state.dashboard_network_speed_cache.as_ref(),
        source_scope,
        range_end,
        upstream_account_id
            .map(DashboardNetworkScopeKey::Account)
            .unwrap_or(DashboardNetworkScopeKey::Global),
    )
    .await
}

fn dashboard_network_bucket_rate(
    total_bytes: i64,
    bucket_start: DateTime<Utc>,
    bucket_end: DateTime<Utc>,
    range: ExactUtcRange,
) -> f64 {
    let effective_start = bucket_start.max(range.start);
    let effective_end = bucket_end.min(range.end);
    let effective_millis = effective_end
        .signed_duration_since(effective_start)
        .num_milliseconds()
        .max(1);
    total_bytes.max(0) as f64 / (effective_millis as f64 / 1000.0)
}

fn build_dashboard_network_realtime_rate_response(
    snapshot: DashboardNetworkRealtimeByteSnapshot,
) -> DashboardNetworkRealtimeRateResponse {
    DashboardNetworkRealtimeRateResponse {
        sample_start: format_utc_iso_precise(
            Utc.timestamp_opt(snapshot.sample_start_epoch_second, 0)
                .single()
                .expect("valid realtime sample start"),
        ),
        sample_end: format_utc_iso_precise(
            Utc.timestamp_opt(snapshot.sample_end_epoch_second, 0)
                .single()
                .expect("valid realtime sample end"),
        ),
        sample_seconds: snapshot.sample_seconds,
        upload_bytes_per_second: snapshot.totals.upload_bytes.max(0) as f64
            / snapshot.sample_seconds.max(1) as f64,
        download_bytes_per_second: snapshot.totals.download_bytes.max(0) as f64
            / snapshot.sample_seconds.max(1) as f64,
        upload_bytes: snapshot.totals.upload_bytes.max(0),
        download_bytes: snapshot.totals.download_bytes.max(0),
    }
}

fn build_dashboard_network_timeseries_point_response(
    bucket_start: DateTime<Utc>,
    bucket_end: DateTime<Utc>,
    totals: DashboardNetworkByteTotals,
    range: ExactUtcRange,
    is_live_bucket: bool,
) -> DashboardNetworkTimeseriesPointResponse {
    DashboardNetworkTimeseriesPointResponse {
        bucket_start: format_utc_iso_precise(bucket_start),
        bucket_end: format_utc_iso_precise(bucket_end),
        upload_bytes_per_second: dashboard_network_bucket_rate(
            totals.upload_bytes,
            bucket_start,
            bucket_end,
            range,
        ),
        download_bytes_per_second: dashboard_network_bucket_rate(
            totals.download_bytes,
            bucket_start,
            bucket_end,
            range,
        ),
        upload_bytes: totals.upload_bytes,
        download_bytes: totals.download_bytes,
        is_live_bucket,
    }
}

async fn load_dashboard_network_live_bucket_point(
    pool: &Pool<Sqlite>,
    dashboard_network_speed_cache: &DashboardNetworkSpeedCache,
    source_scope: InvocationSourceScope,
    range_end: DateTime<Utc>,
    scope: DashboardNetworkScopeKey,
) -> Result<DashboardNetworkTimeseriesPointResponse, ApiError> {
    let snapshot = load_dashboard_network_open_bucket_snapshot_for_scope(
        pool,
        dashboard_network_speed_cache,
        source_scope,
        range_end,
        scope,
    )
    .await?;
    Ok(build_dashboard_network_timeseries_point_response(
        snapshot.bucket_start,
        snapshot.bucket_end,
        snapshot.totals,
        ExactUtcRange {
            start: snapshot.bucket_start,
            end: range_end.min(snapshot.bucket_end),
        },
        true,
    ))
}

pub(crate) async fn fetch_dashboard_network_timeseries(
    State(state): State<Arc<AppState>>,
    Query(params): Query<DashboardNetworkTimeseriesQuery>,
) -> Result<Json<DashboardNetworkTimeseriesResponse>, ApiError> {
    let started_at = Instant::now();
    validate_dashboard_network_timeseries_range(params.range.as_str())?;
    let reporting_tz = parse_reporting_tz(params.time_zone.as_deref())?;
    let range_window =
        resolve_range_window(params.range.as_str(), reporting_tz).map_err(ApiError::from)?;
    let range = ExactUtcRange {
        start: range_window.start,
        end: range_window.end,
    };
    let source_scope = resolve_default_source_scope(&state.pool).await?;
    flush_dashboard_network_socket_minute_rollups(
        &state.pool,
        state.dashboard_network_speed_cache.as_ref(),
        range.end,
    )
    .await?;
    let open_bucket_start_epoch_second = range.end.timestamp()
        - range
            .end
            .timestamp()
            .rem_euclid(DASHBOARD_NETWORK_BUCKET_SECONDS);
    let open_bucket_start = Utc
        .timestamp_opt(open_bucket_start_epoch_second, 0)
        .single()
        .expect("valid dashboard network bucket start");
    let closed_range_end = if params.range == "yesterday" {
        range.end
    } else {
        open_bucket_start.min(range.end)
    };
    let bucket_rows = if let Some(upstream_account_id) = params.upstream_account_id {
        query_dashboard_network_bucket_rows(
            &state.pool,
            source_scope,
            ExactUtcRange {
                start: range.start,
                end: closed_range_end,
            },
            Some(Some(upstream_account_id)),
            None,
        )
        .await?
    } else {
        query_dashboard_network_host_minute_bucket_rows(
            &state.pool,
            source_scope,
            ExactUtcRange {
                start: range.start,
                end: closed_range_end,
            },
        )
        .await?
    };

    let mut aggregates = bucket_rows
        .into_iter()
        .map(|row| {
            (
                row.bucket_start_epoch_second,
                DashboardNetworkBucketAggregate {
                    upload_bytes: row.upload_bytes.max(0),
                    download_bytes: row.download_bytes.max(0),
                },
            )
        })
        .collect::<HashMap<_, _>>();

    let live_bucket_epoch_second = open_bucket_start.timestamp();
    let include_live_bucket = params.range != "yesterday"
        && open_bucket_start < range.end
        && open_bucket_start + ChronoDuration::seconds(DASHBOARD_NETWORK_BUCKET_SECONDS)
            > range.start;
    if include_live_bucket {
        let live_bucket = load_dashboard_network_open_bucket_snapshot(
            state.as_ref(),
            source_scope,
            range.end,
            params.upstream_account_id,
        )
        .await?;
        aggregates.insert(
            live_bucket.bucket_start.timestamp(),
            DashboardNetworkBucketAggregate {
                upload_bytes: live_bucket.totals.upload_bytes.max(0),
                download_bytes: live_bucket.totals.download_bytes.max(0),
            },
        );
    }

    let first_bucket_epoch_second = range.start.timestamp()
        - range
            .start
            .timestamp()
            .rem_euclid(DASHBOARD_NETWORK_BUCKET_SECONDS);
    let mut points = Vec::new();
    let mut bucket_epoch_second = first_bucket_epoch_second;
    while bucket_epoch_second < range.end.timestamp() {
        let bucket_start = Utc
            .timestamp_opt(bucket_epoch_second, 0)
            .single()
            .expect("valid dashboard network bucket point start");
        let bucket_end = bucket_start + ChronoDuration::seconds(DASHBOARD_NETWORK_BUCKET_SECONDS);
        if bucket_end <= range.start {
            bucket_epoch_second =
                bucket_epoch_second.saturating_add(DASHBOARD_NETWORK_BUCKET_SECONDS);
            continue;
        }
        if bucket_start >= range.end {
            break;
        }
        let aggregate = aggregates
            .get(&bucket_epoch_second)
            .copied()
            .unwrap_or_default();
        points.push(build_dashboard_network_timeseries_point_response(
            bucket_start,
            bucket_end,
            DashboardNetworkByteTotals {
                upload_bytes: aggregate.upload_bytes,
                download_bytes: aggregate.download_bytes,
            },
            range,
            include_live_bucket && bucket_epoch_second == live_bucket_epoch_second,
        ));
        bucket_epoch_second = bucket_epoch_second.saturating_add(DASHBOARD_NETWORK_BUCKET_SECONDS);
    }

    let elapsed_ms = started_at.elapsed().as_millis() as u64;
    tracing::debug!(
        endpoint = "/api/stats/dashboard-network-timeseries",
        range = %params.range,
        upstream_account_id = params.upstream_account_id,
        bucket_count = points.len(),
        elapsed_ms,
        "dashboard network timeseries completed"
    );

    Ok(Json(DashboardNetworkTimeseriesResponse {
        range: params.range,
        range_start: format_utc_iso_precise(range.start),
        range_end: format_utc_iso_precise(range.end),
        snapshot_id: range.end.timestamp_millis(),
        bucket_seconds: DASHBOARD_NETWORK_BUCKET_SECONDS,
        points,
    }))
}

pub(crate) async fn fetch_upstream_account_activity(
    State(state): State<Arc<AppState>>,
    Query(params): Query<UpstreamAccountActivityQuery>,
) -> Result<Json<UpstreamAccountActivityResponse>, ApiError> {
    let started_at = Instant::now();
    let recent_limit = validate_dashboard_activity_params(
        "upstream-account-activity",
        params.range.as_str(),
        params.recent_limit,
    )?;
    let reporting_tz = parse_reporting_tz(params.time_zone.as_deref())?;
    let range = resolve_dashboard_activity_exact_range(params.range.as_str(), reporting_tz)?;
    let build = load_dashboard_activity_account_build_result(
        state.as_ref(),
        params.range.as_str(),
        range,
        recent_limit,
        true,
        None,
        DashboardActivityAccountBuilderKind::UpstreamAccount,
    )
    .await?;
    let accounts = build
        .accounts
        .into_iter()
        .filter_map(dashboard_account_to_upstream_account)
        .collect::<Vec<_>>();
    let elapsed_ms = started_at.elapsed().as_millis() as u64;
    if elapsed_ms >= 250 {
        tracing::warn!(
            route = "upstream_account",
            builder = "upstream_account",
            endpoint = "/api/stats/upstream-account-activity",
            range = %params.range,
            recent_limit,
            account_count = accounts.len(),
            preview_read_mode = build.build_telemetry.preview_read_mode,
            candidate_preview_id_count = build.build_telemetry.candidate_preview_id_count,
            hydrated_preview_row_count = build.build_telemetry.hydrated_preview_row_count,
            elapsed_ms,
            "upstream account activity exceeded slow-path threshold"
        );
    } else {
        tracing::debug!(
            route = "upstream_account",
            builder = "upstream_account",
            endpoint = "/api/stats/upstream-account-activity",
            range = %params.range,
            recent_limit,
            account_count = accounts.len(),
            preview_read_mode = build.build_telemetry.preview_read_mode,
            candidate_preview_id_count = build.build_telemetry.candidate_preview_id_count,
            hydrated_preview_row_count = build.build_telemetry.hydrated_preview_row_count,
            elapsed_ms,
            "upstream account activity completed"
        );
    }

    Ok(Json(UpstreamAccountActivityResponse {
        range: params.range,
        range_start: format_utc_iso(range.start),
        range_end: format_utc_iso(range.end),
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

#[cfg(test)]
mod dashboard_network_timeseries_tests {
    use super::*;
    use serde_json::json;
    use sqlx::sqlite::SqlitePoolOptions;

    async fn network_pool() -> Pool<Sqlite> {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("in-memory sqlite pool");
        sqlx::query(
            r#"
            CREATE TABLE codex_invocations (
                invoke_id TEXT NOT NULL DEFAULT '',
                occurred_at TEXT NOT NULL,
                payload TEXT,
                response_raw_size INTEGER,
                raw_response TEXT NOT NULL DEFAULT '',
                request_raw_size INTEGER,
                source TEXT NOT NULL,
                created_at TEXT NOT NULL
            )
            "#,
        )
        .execute(&pool)
        .await
        .expect("create codex_invocations table");
        sqlx::query(
            r#"
            CREATE TABLE pool_upstream_request_attempts (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                invoke_id TEXT NOT NULL,
                occurred_at TEXT NOT NULL,
                upstream_account_id INTEGER,
                upstream_request_header_bytes_approx INTEGER,
                upstream_request_transmitted_body_bytes INTEGER,
                upstream_response_header_bytes_approx INTEGER,
                upstream_response_body_bytes INTEGER
            )
            "#,
        )
        .execute(&pool)
        .await
        .expect("create pool_upstream_request_attempts table");
        sqlx::query(
            r#"
            CREATE TABLE upstream_host_network_minute (
                bucket_start_epoch INTEGER NOT NULL,
                source TEXT NOT NULL,
                upstream_base_url_host TEXT NOT NULL,
                upload_bytes INTEGER NOT NULL DEFAULT 0,
                download_bytes INTEGER NOT NULL DEFAULT 0,
                updated_at TEXT NOT NULL DEFAULT (datetime('now')),
                PRIMARY KEY (bucket_start_epoch, source, upstream_base_url_host)
            )
            "#,
        )
        .execute(&pool)
        .await
        .expect("create upstream_host_network_minute table");
        sqlx::query(
            r#"
            CREATE TABLE upstream_socket_network_minute (
                bucket_start_epoch INTEGER NOT NULL,
                source TEXT NOT NULL,
                upstream_base_url_host TEXT NOT NULL,
                upstream_account_id INTEGER,
                upload_bytes INTEGER NOT NULL DEFAULT 0,
                download_bytes INTEGER NOT NULL DEFAULT 0,
                updated_at TEXT NOT NULL DEFAULT (datetime('now')),
                PRIMARY KEY (
                    bucket_start_epoch,
                    source,
                    upstream_base_url_host,
                    upstream_account_id
                )
            )
            "#,
        )
        .execute(&pool)
        .await
        .expect("create upstream_socket_network_minute table");
        pool
    }

    #[tokio::test]
    async fn dashboard_network_bucket_rows_read_real_socket_minute_rows() {
        let pool = network_pool().await;
        sqlx::query(
            r#"
            INSERT INTO upstream_socket_network_minute (
                bucket_start_epoch,
                source,
                upstream_base_url_host,
                upstream_account_id,
                upload_bytes,
                download_bytes
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6)
            "#,
        )
        .bind(
            Utc.with_ymd_and_hms(2026, 7, 16, 4, 35, 0)
                .single()
                .expect("valid bucket start")
                .timestamp(),
        )
        .bind(SOURCE_PROXY)
        .bind("api.openai.com")
        .bind(42_i64)
        .bind(128_i64)
        .bind(256_i64)
        .execute(&pool)
        .await
        .expect("insert dashboard socket minute row");

        let rows = query_dashboard_network_bucket_rows(
            &pool,
            InvocationSourceScope::ProxyOnly,
            ExactUtcRange {
                start: Utc
                    .with_ymd_and_hms(2026, 7, 16, 4, 34, 0)
                    .single()
                    .expect("valid range start"),
                end: Utc
                    .with_ymd_and_hms(2026, 7, 16, 4, 40, 0)
                    .single()
                    .expect("valid range end"),
            },
            Some(Some(42)),
            None,
        )
        .await
        .expect("query dashboard socket minute rows");

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].upload_bytes, 128);
        assert_eq!(rows[0].download_bytes, 256);
    }

    #[tokio::test]
    async fn dashboard_network_host_minute_rows_roll_up_into_five_minute_buckets() {
        let pool = network_pool().await;
        let bucket_start_epoch = Utc
            .with_ymd_and_hms(2026, 7, 16, 4, 35, 0)
            .single()
            .expect("valid bucket start")
            .timestamp();
        sqlx::query(
            r#"
            INSERT INTO upstream_socket_network_minute (
                bucket_start_epoch,
                source,
                upstream_base_url_host,
                upstream_account_id,
                upload_bytes,
                download_bytes
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6), (?7, ?8, ?9, ?10, ?11, ?12)
            "#,
        )
        .bind(bucket_start_epoch)
        .bind(SOURCE_PROXY)
        .bind("api.openai.com")
        .bind(Some(42_i64))
        .bind(100_i64)
        .bind(200_i64)
        .bind(bucket_start_epoch + 60)
        .bind(SOURCE_PROXY)
        .bind("backup.openai.com")
        .bind(Some(77_i64))
        .bind(40_i64)
        .bind(80_i64)
        .execute(&pool)
        .await
        .expect("insert upstream socket minute rows");

        let rows = query_dashboard_network_host_minute_bucket_rows(
            &pool,
            InvocationSourceScope::ProxyOnly,
            ExactUtcRange {
                start: Utc
                    .timestamp_opt(bucket_start_epoch, 0)
                    .single()
                    .expect("valid start"),
                end: Utc
                    .timestamp_opt(bucket_start_epoch + 300, 0)
                    .single()
                    .expect("valid end"),
            },
        )
        .await
        .expect("query socket minute network rows");

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].bucket_start_epoch_second, bucket_start_epoch);
        assert_eq!(rows[0].upload_bytes, 140);
        assert_eq!(rows[0].download_bytes, 280);
    }

    #[test]
    fn dashboard_network_realtime_rate_response_serializes_complete_second_snapshot() {
        let response =
            build_dashboard_network_realtime_rate_response(DashboardNetworkRealtimeByteSnapshot {
                sample_start_epoch_second: Utc
                    .with_ymd_and_hms(2026, 7, 19, 18, 3, 59)
                    .single()
                    .expect("valid sample start")
                    .timestamp(),
                sample_end_epoch_second: Utc
                    .with_ymd_and_hms(2026, 7, 19, 18, 4, 0)
                    .single()
                    .expect("valid sample end")
                    .timestamp(),
                sample_seconds: 1,
                totals: DashboardNetworkByteTotals {
                    upload_bytes: 2048,
                    download_bytes: 4096,
                },
            });

        assert_eq!(response.sample_start, "2026-07-19T18:03:59Z");
        assert_eq!(response.sample_end, "2026-07-19T18:04:00Z");
        assert_eq!(response.sample_seconds, 1);
        assert_eq!(response.upload_bytes_per_second, 2048.0);
        assert_eq!(response.download_bytes_per_second, 4096.0);

        let payload =
            serde_json::to_value(&response).expect("serialize dashboard network realtime rate");
        assert_eq!(
            payload,
            json!({
                "sampleStart": "2026-07-19T18:03:59Z",
                "sampleEnd": "2026-07-19T18:04:00Z",
                "sampleSeconds": 1,
                "uploadBytesPerSecond": 2048.0,
                "downloadBytesPerSecond": 4096.0,
                "uploadBytes": 2048,
                "downloadBytes": 4096
            })
        );
    }
}

#[cfg(test)]
mod model_performance_duration_override_tests {
    use super::*;

    fn group(model: &str, reasoning_effort: Option<&str>) -> UsageBreakdownGroupKey {
        UsageBreakdownGroupKey {
            model: model.to_string(),
            reasoning_effort: reasoning_effort.map(str::to_string),
        }
    }

    fn utc_at(
        year: i32,
        month: u32,
        day: u32,
        hour: u32,
        minute: u32,
        second: u32,
    ) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(year, month, day, hour, minute, second)
            .single()
            .expect("valid test time")
    }

    #[test]
    fn wall_clock_duration_dedupes_overlaps_per_account_model_and_total_scopes() {
        let rows = vec![
            SuccessfulBilledUsageDurationIntervalRow {
                upstream_account_id: Some(42),
                model: "gpt-5.4".to_string(),
                reasoning_effort: Some("high".to_string()),
                start_epoch_ms: utc_at(2026, 7, 15, 12, 0, 10).timestamp_millis() as f64,
                end_epoch_ms: utc_at(2026, 7, 15, 12, 0, 14).timestamp_millis() as f64,
            },
            SuccessfulBilledUsageDurationIntervalRow {
                upstream_account_id: Some(42),
                model: "gpt-5.4".to_string(),
                reasoning_effort: Some("high".to_string()),
                start_epoch_ms: utc_at(2026, 7, 15, 12, 0, 12).timestamp_millis() as f64,
                end_epoch_ms: utc_at(2026, 7, 15, 12, 0, 13).timestamp_millis() as f64,
            },
            SuccessfulBilledUsageDurationIntervalRow {
                upstream_account_id: Some(77),
                model: "gpt-5.4".to_string(),
                reasoning_effort: Some("high".to_string()),
                start_epoch_ms: utc_at(2026, 7, 15, 12, 0, 13).timestamp_millis() as f64,
                end_epoch_ms: utc_at(2026, 7, 15, 12, 0, 17).timestamp_millis() as f64,
            },
        ];

        let overrides = compute_model_performance_duration_overrides(&rows);

        assert_eq!(overrides.total_wall_clock_ms, Some(7_000.0));
        assert_eq!(
            overrides.by_account_wall_clock_ms.get(&Some(42)).copied(),
            Some(4_000.0)
        );
        assert_eq!(
            overrides.by_account_wall_clock_ms.get(&Some(77)).copied(),
            Some(4_000.0)
        );
        assert_eq!(
            overrides
                .by_group_wall_clock_ms
                .get(&group("gpt-5.4", Some("high")))
                .copied(),
            Some(7_000.0)
        );
        assert_eq!(
            overrides
                .by_account_group_wall_clock_ms
                .get(&AccountModelGroupKey {
                    upstream_account_id: Some(42),
                    group: group("gpt-5.4", Some("high")),
                })
                .copied(),
            Some(4_000.0)
        );
        assert_eq!(
            overrides
                .by_account_group_wall_clock_ms
                .get(&AccountModelGroupKey {
                    upstream_account_id: Some(77),
                    group: group("gpt-5.4", Some("high")),
                })
                .copied(),
            Some(4_000.0)
        );
    }

    #[test]
    fn wall_clock_duration_keeps_cross_model_overlap_local_to_each_model_group() {
        let rows = vec![
            SuccessfulBilledUsageDurationIntervalRow {
                upstream_account_id: Some(42),
                model: "gpt-5.4".to_string(),
                reasoning_effort: Some("high".to_string()),
                start_epoch_ms: utc_at(2026, 7, 15, 12, 0, 10).timestamp_millis() as f64,
                end_epoch_ms: utc_at(2026, 7, 15, 12, 0, 14).timestamp_millis() as f64,
            },
            SuccessfulBilledUsageDurationIntervalRow {
                upstream_account_id: Some(77),
                model: "gpt-5.6-sol".to_string(),
                reasoning_effort: Some("low".to_string()),
                start_epoch_ms: utc_at(2026, 7, 15, 12, 0, 13).timestamp_millis() as f64,
                end_epoch_ms: utc_at(2026, 7, 15, 12, 0, 17).timestamp_millis() as f64,
            },
        ];

        let overrides = compute_model_performance_duration_overrides(&rows);

        assert_eq!(overrides.total_wall_clock_ms, Some(7_000.0));
        assert_eq!(
            overrides
                .by_group_wall_clock_ms
                .get(&group("gpt-5.4", Some("high")))
                .copied(),
            Some(4_000.0)
        );
        assert_eq!(
            overrides
                .by_group_wall_clock_ms
                .get(&group("gpt-5.6-sol", Some("low")))
                .copied(),
            Some(4_000.0)
        );
        assert_eq!(
            overrides
                .by_group_wall_clock_ms
                .values()
                .copied()
                .sum::<f64>(),
            8_000.0
        );
    }

    #[test]
    fn wall_clock_duration_clips_tail_to_selected_range() {
        let rows = vec![
            SuccessfulBilledUsageDurationIntervalRow {
                upstream_account_id: Some(42),
                model: "gpt-5.4".to_string(),
                reasoning_effort: None,
                start_epoch_ms: utc_at(2026, 7, 15, 12, 0, 58).timestamp_millis() as f64,
                end_epoch_ms: utc_at(2026, 7, 15, 12, 1, 0).timestamp_millis() as f64,
            },
            SuccessfulBilledUsageDurationIntervalRow {
                upstream_account_id: Some(42),
                model: "gpt-5.4".to_string(),
                reasoning_effort: None,
                start_epoch_ms: utc_at(2026, 7, 15, 12, 0, 59).timestamp_millis() as f64,
                end_epoch_ms: utc_at(2026, 7, 15, 12, 1, 0).timestamp_millis() as f64,
            },
        ];

        let overrides = compute_model_performance_duration_overrides(&rows);

        assert_eq!(overrides.total_wall_clock_ms, Some(2_000.0));
        assert_eq!(
            overrides.by_account_wall_clock_ms.get(&Some(42)).copied(),
            Some(2_000.0)
        );
        assert_eq!(
            overrides
                .by_group_wall_clock_ms
                .get(&group("gpt-5.4", None))
                .copied(),
            Some(2_000.0)
        );
        assert_eq!(
            overrides
                .by_account_group_wall_clock_ms
                .get(&AccountModelGroupKey {
                    upstream_account_id: Some(42),
                    group: group("gpt-5.4", None),
                })
                .copied(),
            Some(2_000.0)
        );
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

#[cfg(test)]
mod invocation_cost_audit_tests {
    use super::*;
    use std::collections::HashMap;

    fn sample_pricing_catalog() -> PricingCatalog {
        PricingCatalog {
            version: "unit-test".to_string(),
            models: HashMap::from([(
                "gpt-5.4".to_string(),
                ModelPricing {
                    input_per_1m: 1.0,
                    output_per_1m: 2.0,
                    cache_input_per_1m: Some(0.1),
                    cache_read_per_1m: Some(0.2),
                    cache_write_per_1m: Some(1.25),
                    reasoning_per_1m: Some(3.0),
                    source: "unit-test".to_string(),
                },
            )]),
        }
    }

    fn sample_invocation(reasoning_tokens: Option<i64>) -> ApiInvocation {
        ApiInvocation {
            id: 7,
            invoke_id: "invocation-cost-audit".to_string(),
            occurred_at: "2026-07-20 10:25:09".to_string(),
            source: SOURCE_PROXY.to_string(),
            proxy_display_name: Some("ciii".to_string()),
            model: Some("gpt-5.4".to_string()),
            request_model: Some("gpt-5.4".to_string()),
            response_model: Some("gpt-5.4".to_string()),
            input_tokens: Some(1_000),
            output_tokens: Some(200),
            cache_input_tokens: Some(400),
            reasoning_tokens,
            reasoning_effort: Some("medium".to_string()),
            total_tokens: Some(1_200),
            cost: Some(0.0099),
            cost_input: None,
            cost_cache_write: None,
            cost_cache_read: None,
            cost_output: None,
            cost_reasoning: None,
            cache_write_tokens: None,
            status: Some("success".to_string()),
            live_phase: None,
            error_message: None,
            downstream_status_code: Some(200),
            failure_kind: None,
            blocked_binding: None,
            blocked_binding_json: None,
            stream_terminal_event: None,
            upstream_error_code: None,
            upstream_error_message: None,
            downstream_error_message: None,
            upstream_request_id: Some("req_cost_audit".to_string()),
            failure_class: Some("none".to_string()),
            is_actionable: Some(false),
            endpoint: Some("/v1/responses".to_string()),
            compaction_request_kind: Some("remote_v2".to_string()),
            compaction_response_kind: Some("remote_v2".to_string()),
            image_intent: Some("no".to_string()),
            requester_ip: Some("192.168.31.6".to_string()),
            prompt_cache_key: Some("pck-cost-audit".to_string()),
            sticky_key: None,
            route_mode: Some("pool".to_string()),
            upstream_account_id: Some(17),
            upstream_account_name: Some("Pool 17".to_string()),
            response_content_encoding: Some("identity".to_string()),
            transport: Some("http".to_string()),
            pool_attempt_count: Some(3),
            pool_distinct_account_count: Some(2),
            pool_attempt_terminal_reason: Some("success".to_string()),
            requested_service_tier: Some("default".to_string()),
            service_tier: Some("default".to_string()),
            billing_service_tier: Some("default".to_string()),
            proxy_weight_delta: None,
            cost_estimated: Some(0),
            price_version: Some("legacy@response-tier".to_string()),
            cost_audit: None,
            request_raw_path: None,
            request_raw_size: None,
            request_raw_truncated: None,
            request_raw_truncated_reason: None,
            response_raw_path: None,
            response_raw_size: None,
            response_raw_truncated: None,
            response_raw_truncated_reason: None,
            detail_level: "full".to_string(),
            detail_pruned_at: None,
            detail_prune_reason: None,
            t_total_ms: Some(3_450.0),
            t_req_read_ms: Some(20.0),
            t_req_parse_ms: Some(8.0),
            t_upstream_connect_ms: Some(190.0),
            t_upstream_ttfb_ms: Some(330.0),
            t_upstream_stream_ms: Some(2_800.0),
            t_resp_parse_ms: Some(12.0),
            t_persist_ms: Some(6.0),
            created_at: "2026-07-20 10:25:09".to_string(),
        }
    }

    fn sample_attempt_row(attempt_index: i64, status: &str) -> InvocationWorkflowAttemptRow {
        InvocationWorkflowAttemptRow {
            attempt_id: Some(format!("attempt-{attempt_index}")),
            invoke_id: "invocation-cost-audit".to_string(),
            occurred_at: "2026-07-20 10:25:09".to_string(),
            endpoint: "/v1/responses".to_string(),
            sticky_key: None,
            upstream_account_id: Some(17),
            upstream_account_name: Some("Pool 17".to_string()),
            upstream_route_key: Some("route-17".to_string()),
            proxy_binding_key_snapshot: Some("binding-17".to_string()),
            attempt_index,
            distinct_account_index: attempt_index,
            same_account_retry_index: 0,
            requester_ip: Some("192.168.31.6".to_string()),
            started_at: Some("2026-07-20 10:25:09".to_string()),
            finished_at: Some("2026-07-20 10:25:12".to_string()),
            status: status.to_string(),
            phase: Some(if status == "failed" {
                "streaming".to_string()
            } else {
                "completed".to_string()
            }),
            http_status: Some(if status == "failed" { 500 } else { 200 }),
            downstream_http_status: Some(200),
            failure_kind: (status == "failed").then_some("upstream_error".to_string()),
            error_message: (status == "failed").then_some("upstream error".to_string()),
            downstream_error_message: None,
            connect_latency_ms: Some(120.0),
            first_byte_latency_ms: Some(240.0),
            stream_latency_ms: Some(1_500.0),
            upstream_request_id: Some(format!("req-{attempt_index}")),
            upstream_request_compression_algorithm: None,
            upstream_request_compression_mode: None,
            upstream_request_logical_body_bytes: None,
            upstream_request_transmitted_body_bytes: None,
            upstream_request_header_bytes_approx: None,
            upstream_response_body_bytes: None,
            upstream_response_header_bytes_approx: None,
            compact_support_status: None,
            compact_support_reason: None,
            request_summary_json: None,
            response_summary_json: None,
        }
    }

    fn assert_close(left: f64, right: f64) {
        assert!(
            (left - right).abs() < 1e-12,
            "expected {left} to be close to {right}"
        );
    }

    #[test]
    fn build_invocation_cost_audit_flags_price_version_change_and_keeps_total_only_history() {
        let record = sample_invocation(None);
        let catalog = sample_pricing_catalog();

        let audit =
            build_invocation_cost_audit(&record, &catalog, true).expect("cost audit should exist");

        assert!(audit.mismatch);
        assert_eq!(
            audit.reason.as_deref(),
            Some(INVOCATION_COST_AUDIT_REASON_PRICE_VERSION_CHANGED)
        );
        assert_eq!(
            audit.recorded_price_version.as_deref(),
            Some("legacy@response-tier")
        );
        assert_eq!(
            audit.local_price_version.as_deref(),
            Some("unit-test@response-tier")
        );

        let recorded = audit.recorded.expect("recorded breakdown");
        assert_eq!(recorded.total, Some(0.0099));
        assert_eq!(recorded.input, None);
        assert_eq!(recorded.cache_write, None);
        assert_eq!(recorded.cache_read, None);
        assert_eq!(recorded.output, None);
        assert_eq!(recorded.reasoning, None);

        let local = audit.local.expect("local breakdown");
        assert_close(local.input.expect("input cost"), 0.0);
        assert_close(local.cache_write.expect("cache write cost"), 0.00075);
        assert_close(local.cache_read.expect("cache read cost"), 0.00008);
        assert_close(local.output.expect("output cost"), 0.0004);
        assert_close(local.reasoning.expect("reasoning cost"), 0.0);
        assert_close(local.total.expect("total cost"), 0.00123);
        assert!(
            audit.absolute_diff_usd.expect("absolute diff")
                > INVOCATION_COST_AUDIT_MISMATCH_EPSILON_USD
        );
    }

    #[test]
    fn build_invocation_usage_summary_preserves_reasoning_null_vs_zero() {
        let catalog = sample_pricing_catalog();

        let record_without_reasoning = sample_invocation(None);
        let audit_without_reasoning =
            build_invocation_cost_audit(&record_without_reasoning, &catalog, true)
                .expect("cost audit for null reasoning");
        let usage_without_reasoning =
            build_invocation_usage_summary(&record_without_reasoning, &audit_without_reasoning);
        assert!(usage_without_reasoning["reasoningTokens"].is_null());
        assert!(usage_without_reasoning["tokens"]["reasoning"].is_null());

        let record_with_zero_reasoning = sample_invocation(Some(0));
        let audit_with_zero_reasoning =
            build_invocation_cost_audit(&record_with_zero_reasoning, &catalog, true)
                .expect("cost audit for zero reasoning");
        let usage_with_zero_reasoning =
            build_invocation_usage_summary(&record_with_zero_reasoning, &audit_with_zero_reasoning);
        assert_eq!(
            usage_with_zero_reasoning["reasoningTokens"].as_i64(),
            Some(0)
        );
        assert_eq!(
            usage_with_zero_reasoning["tokens"]["reasoning"].as_i64(),
            Some(0)
        );
    }

    #[test]
    fn workflow_usage_audit_only_attaches_to_last_success_like_attempt() {
        let record = sample_invocation(Some(0));
        let catalog = sample_pricing_catalog();
        let usage_cost_audit =
            build_invocation_cost_audit(&record, &catalog, true).expect("cost audit");

        let attempt_rows = [
            sample_attempt_row(1, "failed"),
            sample_attempt_row(2, "success"),
            sample_attempt_row(3, "warning_success"),
        ];
        let attempt_refs = attempt_rows.iter().collect::<Vec<_>>();
        let last_success_attempt_index = last_success_like_attempt_index(&attempt_refs);
        assert_eq!(last_success_attempt_index, Some(3));

        let attempts = attempt_rows
            .iter()
            .map(|attempt| {
                build_workflow_attempt_from_row(
                    &record,
                    attempt,
                    None,
                    (last_success_attempt_index == Some(attempt.attempt_index))
                        .then_some(&usage_cost_audit),
                )
            })
            .collect::<Vec<_>>();

        for attempt in &attempts[..2] {
            let response_summary = attempt.response_summary.as_ref().expect("response summary");
            assert!(
                response_summary.get("usage").is_some_and(Value::is_null),
                "non-terminal success attempts should not receive usage audit"
            );
        }

        let final_response_summary = attempts[2]
            .response_summary
            .as_ref()
            .expect("final response summary");
        assert_eq!(
            final_response_summary["usage"]["reasoningTokens"].as_i64(),
            Some(0)
        );
        assert_eq!(
            final_response_summary["usage"]["audit"]["reason"].as_str(),
            Some(INVOCATION_COST_AUDIT_REASON_PRICE_VERSION_CHANGED)
        );
    }
}
