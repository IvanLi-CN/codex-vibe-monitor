use chrono::Offset;
use chrono::Timelike;

pub(crate) const INVOCATION_PROXY_DISPLAY_SQL: &str =
    "NULLIF(TRIM(CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.proxyDisplayName') AS TEXT) END), '')";
pub(crate) const INVOCATION_ENDPOINT_SQL: &str =
    "CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.endpoint') AS TEXT) END";
pub(crate) const INVOCATION_COMPACTION_REQUEST_KIND_SQL: &str =
    "CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.compactionRequestKind') AS TEXT) END";
pub(crate) const INVOCATION_COMPACTION_RESPONSE_KIND_SQL: &str =
    "CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.compactionResponseKind') AS TEXT) END";
pub(crate) const INVOCATION_IMAGE_INTENT_SQL: &str =
    "CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.imageIntent') AS TEXT) END";
pub(crate) const INVOCATION_FAILURE_KIND_SQL: &str = "COALESCE(CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.failureKind') AS TEXT) END, failure_kind)";
pub(crate) const INVOCATION_REQUESTER_IP_SQL: &str =
    "CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.requesterIp') AS TEXT) END";
const INVOCATION_PROMPT_CACHE_KEY_SQL: &str = "CASE WHEN json_valid(payload) THEN TRIM(CAST(json_extract(payload, '$.promptCacheKey') AS TEXT)) END";
pub(crate) const INVOCATION_STICKY_KEY_SQL: &str = "CASE WHEN json_valid(payload) THEN TRIM(COALESCE(CAST(json_extract(payload, '$.stickyKey') AS TEXT), CAST(json_extract(payload, '$.promptCacheKey') AS TEXT))) END";
const INVOCATION_UPSTREAM_SCOPE_SQL: &str = "COALESCE(CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.upstreamScope') AS TEXT) END, 'external')";
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
const INVOCATION_POOL_ATTEMPT_COUNT_SQL: &str = "CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.poolAttemptCount') AS INTEGER) END";
const INVOCATION_POOL_DISTINCT_ACCOUNT_COUNT_SQL: &str = "CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.poolDistinctAccountCount') AS INTEGER) END";
const INVOCATION_POOL_ATTEMPT_TERMINAL_REASON_SQL: &str = "CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.poolAttemptTerminalReason') AS TEXT) END";
const PROMPT_CACHE_CONVERSATION_UPSTREAM_ACCOUNT_LIMIT: usize = 3;
const PROMPT_CACHE_CONVERSATION_INVOCATION_PREVIEW_LIMIT: usize = 5;
const INVOCATION_STATUS_NORMALIZED_SQL: &str = "LOWER(TRIM(COALESCE(status, '')))";
const INVOCATION_RESPONSE_BODY_PREVIEW_CHAR_LIMIT: usize = 2_000;

// Legacy records can carry `failure_class=none` or NULL while still representing failures.
// Keep classification consistent with `resolve_failure_classification` without requiring a
// backfill pass to complete before the summary + filters become accurate.
pub(crate) const INVOCATION_RESOLVED_FAILURE_CLASS_SQL: &str = "CASE   WHEN LOWER(TRIM(COALESCE(failure_class, ''))) IN ('service_failure', 'client_failure', 'client_abort')     THEN LOWER(TRIM(COALESCE(failure_class, '')))   ELSE     CASE       WHEN LOWER(TRIM(COALESCE(status, ''))) IN ('success', 'completed')         AND LOWER(TRIM(COALESCE(error_message, ''))) = ''         AND LOWER(TRIM(COALESCE(CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.downstreamErrorMessage') AS TEXT) END, ''))) = ''         AND LOWER(TRIM(COALESCE(CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.failureKind') AS TEXT) END, failure_kind, ''))) = '' THEN 'none'       WHEN LOWER(TRIM(COALESCE(status, ''))) IN ('running', 'pending')         AND LOWER(TRIM(COALESCE(error_message, ''))) = '' THEN 'none'       WHEN LOWER(TRIM(COALESCE(status, ''))) = ''         AND LOWER(TRIM(COALESCE(error_message, ''))) = ''         AND LOWER(TRIM(COALESCE(CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.downstreamErrorMessage') AS TEXT) END, ''))) = ''         AND LOWER(TRIM(COALESCE(CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.failureKind') AS TEXT) END, failure_kind, ''))) = '' THEN 'none'       WHEN LOWER(TRIM(COALESCE(CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.failureKind') AS TEXT) END, failure_kind, ''))) = 'downstream_closed'         OR LOWER(TRIM(COALESCE(error_message, ''))) LIKE '[downstream_closed]%'         OR LOWER(TRIM(COALESCE(error_message, ''))) LIKE '%downstream closed while streaming upstream response%'         OR LOWER(TRIM(COALESCE(CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.downstreamErrorMessage') AS TEXT) END, ''))) LIKE '%downstream closed while streaming upstream response%'         THEN 'client_abort'       WHEN LOWER(TRIM(COALESCE(status, ''))) = 'http_429'         OR LOWER(TRIM(COALESCE(CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.failureKind') AS TEXT) END, failure_kind, ''))) = 'upstream_http_429'         THEN 'service_failure'       WHEN LOWER(TRIM(COALESCE(CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.failureKind') AS TEXT) END, failure_kind, ''))) IN ('request_body_stream_error_client_closed', 'invalid_api_key', 'api_key_not_found', 'api_key_missing')         OR LOWER(TRIM(COALESCE(error_message, ''))) LIKE '[request_body_stream_error_client_closed]%'         OR LOWER(TRIM(COALESCE(error_message, ''))) LIKE '%failed to read request body stream%'         OR LOWER(TRIM(COALESCE(error_message, ''))) LIKE '%invalid api key format%'         OR LOWER(TRIM(COALESCE(error_message, ''))) LIKE '%api key format is invalid%'         OR LOWER(TRIM(COALESCE(error_message, ''))) LIKE '%incorrect api key provided%'         OR LOWER(TRIM(COALESCE(error_message, ''))) LIKE '%api key not found%'         OR LOWER(TRIM(COALESCE(error_message, ''))) LIKE '%please provide an api key%'         OR (LOWER(TRIM(COALESCE(status, ''))) LIKE 'http_4%' AND LOWER(TRIM(COALESCE(status, ''))) != 'http_429')         OR LOWER(TRIM(COALESCE(status, ''))) IN ('http_401', 'http_403')         THEN 'client_failure'       WHEN LOWER(TRIM(COALESCE(CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.failureKind') AS TEXT) END, failure_kind, ''))) IN ('failed_contact_upstream', 'upstream_response_failed', 'upstream_stream_error', 'request_body_read_timeout', 'upstream_handshake_timeout')         OR LOWER(TRIM(COALESCE(error_message, ''))) LIKE '[failed_contact_upstream]%'         OR LOWER(TRIM(COALESCE(error_message, ''))) LIKE '[upstream_response_failed]%'         OR LOWER(TRIM(COALESCE(error_message, ''))) LIKE '[upstream_stream_error]%'         OR LOWER(TRIM(COALESCE(error_message, ''))) LIKE '[request_body_read_timeout]%'         OR LOWER(TRIM(COALESCE(error_message, ''))) LIKE '[upstream_handshake_timeout]%'         OR LOWER(TRIM(COALESCE(error_message, ''))) LIKE '%failed to contact upstream%'         OR LOWER(TRIM(COALESCE(error_message, ''))) LIKE '%upstream response stream reported failure%'         OR LOWER(TRIM(COALESCE(error_message, ''))) LIKE '%upstream stream error%'         OR LOWER(TRIM(COALESCE(error_message, ''))) LIKE '%request body read timed out%'         OR LOWER(TRIM(COALESCE(error_message, ''))) LIKE '%upstream handshake timed out%'         OR LOWER(TRIM(COALESCE(status, ''))) LIKE 'http_5%'         THEN 'service_failure'       WHEN LOWER(TRIM(COALESCE(status, ''))) IN ('success', 'completed') THEN 'none'       WHEN LOWER(TRIM(COALESCE(status, ''))) = 'http_200'         AND LOWER(TRIM(COALESCE(error_message, ''))) = ''         AND LOWER(TRIM(COALESCE(CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.downstreamErrorMessage') AS TEXT) END, ''))) = ''         AND LOWER(TRIM(COALESCE(CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.failureKind') AS TEXT) END, failure_kind, ''))) = '' THEN 'none'       ELSE 'service_failure'     END END";

fn build_invocation_select_query() -> QueryBuilder<'static, Sqlite> {
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
         total_tokens, cost, status, error_message, \
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
enum InvocationSortBy {
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
enum InvocationSortOrder {
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
enum SnapshotConstraint {
    UpTo(i64),
    After(i64),
}

#[derive(Debug, Clone, Default)]
struct InvocationRecordsFilters {
    occurred_from: Option<String>,
    occurred_to: Option<String>,
    status: Option<String>,
    model: Option<String>,
    endpoint: Option<String>,
    request_id: Option<String>,
    failure_class: Option<String>,
    failure_kind: Option<String>,
    prompt_cache_key: Option<String>,
    sticky_key: Option<String>,
    upstream_scope: Option<String>,
    upstream_account_id: Option<i64>,
    requester_ip: Option<String>,
    keyword: Option<String>,
    min_total_tokens: Option<i64>,
    max_total_tokens: Option<i64>,
    min_total_ms: Option<f64>,
    max_total_ms: Option<f64>,
}

#[derive(Debug, Clone)]
struct InvocationListRequest {
    filters: InvocationRecordsFilters,
    page: i64,
    page_size: i64,
    sort_by: InvocationSortBy,
    sort_order: InvocationSortOrder,
    snapshot_id: Option<i64>,
}

#[derive(Debug, FromRow)]
struct InvocationSummaryAggRow {
    total_count: i64,
    success_count: i64,
    failure_count: i64,
    total_tokens: i64,
    total_cost: f64,
    cache_input_tokens: i64,
}

#[derive(Debug, FromRow)]
struct InvocationNetworkAggRow {
    avg_ttfb_ms: Option<f64>,
    ttfb_count: i64,
    avg_total_ms: Option<f64>,
    total_count: i64,
}

#[derive(Debug, FromRow)]
struct InvocationExceptionAggRow {
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

fn normalize_query_text(raw: Option<&str>) -> Option<String> {
    raw.map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn escape_sql_like(raw: &str) -> String {
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

fn parse_invocation_bound(raw: Option<&str>, field_name: &str) -> Result<Option<String>, ApiError> {
    let Some(raw_value) = normalize_query_text(raw) else {
        return Ok(None);
    };
    let parsed = DateTime::parse_from_rfc3339(&raw_value)
        .with_context(|| format!("invalid {field_name}: {raw_value}"))
        .map_err(ApiError::bad_request)?
        .with_timezone(&Utc);
    Ok(Some(db_occurred_at_lower_bound(parsed)))
}

fn build_invocation_filters(params: &ListQuery) -> Result<InvocationRecordsFilters, ApiError> {
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

fn build_invocation_list_request(
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

fn push_exact_text_filter(query: &mut QueryBuilder<Sqlite>, sql_expr: &str, value: &str) {
    query.push(" AND LOWER(TRIM(COALESCE(");
    query.push(sql_expr);
    query.push(", ''))) = ");
    query.push_bind(value.to_lowercase());
}

fn push_keyword_filter(query: &mut QueryBuilder<Sqlite>, keyword: &str) {
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

fn apply_invocation_records_filters(
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

fn append_invocation_order_clause(
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

fn normalized_runtime_text(value: Option<&str>) -> String {
    value.map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or_default()
        .to_lowercase()
}

fn runtime_text_equals(value: Option<&str>, expected: &str) -> bool {
    normalized_runtime_text(value) == expected.trim().to_lowercase()
}

fn runtime_keyword_matches(record: &ApiInvocation, keyword: &str) -> bool {
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

fn runtime_sticky_key(record: &ApiInvocation) -> Option<&str> {
    record
        .sticky_key
        .as_deref()
        .or(record.prompt_cache_key.as_deref())
}

fn runtime_upstream_scope(record: &ApiInvocation) -> &'static str {
    if runtime_text_equals(record.route_mode.as_deref(), "pool") {
        "internal"
    } else {
        "external"
    }
}

fn runtime_record_is_retry(record: &ApiInvocation) -> bool {
    record.pool_attempt_count.unwrap_or(1) > 1
}

fn runtime_record_is_in_flight(record: &ApiInvocation) -> bool {
    matches!(
        normalized_runtime_text(record.status.as_deref()).as_str(),
        "running" | "pending"
    )
}

fn runtime_record_matches_filters(
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

fn runtime_in_flight_record_matches_filters(
    record: &ApiInvocation,
    filters: &InvocationRecordsFilters,
    source_scope: InvocationSourceScope,
) -> bool {
    runtime_record_is_in_flight(record)
        && runtime_record_matches_filters(record, filters, source_scope)
}

fn option_presence_order(left_some: bool, right_some: bool) -> Option<std::cmp::Ordering> {
    match (left_some, right_some) {
        (true, false) => Some(std::cmp::Ordering::Less),
        (false, true) => Some(std::cmp::Ordering::Greater),
        (false, false) => Some(std::cmp::Ordering::Equal),
        (true, true) => None,
    }
}

fn apply_runtime_sort_order(
    ordering: std::cmp::Ordering,
    sort_order: InvocationSortOrder,
) -> std::cmp::Ordering {
    match sort_order {
        InvocationSortOrder::Asc => ordering,
        InvocationSortOrder::Desc => ordering.reverse(),
    }
}

fn compare_runtime_option_i64(
    left: Option<i64>,
    right: Option<i64>,
    sort_order: InvocationSortOrder,
) -> std::cmp::Ordering {
    option_presence_order(left.is_some(), right.is_some()).unwrap_or_else(|| {
        apply_runtime_sort_order(left.unwrap_or_default().cmp(&right.unwrap_or_default()), sort_order)
    })
}

fn compare_runtime_option_f64(
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

fn compare_runtime_option_str(
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

fn invocation_display_status_value(record: &ApiInvocation) -> Option<&str> {
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

fn compare_runtime_invocation_records(
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

fn should_overlay_runtime_records(request: &InvocationListRequest) -> bool {
    request.snapshot_id.is_none()
}

fn runtime_overlay_snapshot(state: &AppState) -> Vec<ApiInvocation> {
    state.proxy_runtime_invocations.snapshot()
}

async fn query_current_runtime_db_keys(
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

async fn query_terminal_db_keys_for_runtime_records(
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

fn overlay_runtime_records_for_current_page(
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
        .map(|record| ((record.invoke_id.clone(), record.occurred_at.clone()), record))
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
                    if runtime_record_matches_filters(runtime_record, &request.filters, source_scope)
                    {
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

fn runtime_overlay_total_delta(
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
        .map(|record| ((record.invoke_id.clone(), record.occurred_at.clone()), record))
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
    (
        delta,
        runtime_new_count,
        stale_db_runtime_count,
    )
}

#[derive(Debug, Default)]
struct RuntimeSummaryOverlayDelta {
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

fn runtime_record_is_success_for_summary(record: &ApiInvocation) -> bool {
    let status = normalized_runtime_text(record.status.as_deref());
    status == "success"
        || status == "completed"
        || (status == "http_200"
            && normalized_runtime_text(record.error_message.as_deref()).is_empty())
}

async fn query_invocation_network_summary(
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

async fn query_invocation_new_records_count(
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
enum InvocationSuggestionField {
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

fn empty_invocation_suggestion_bucket() -> InvocationSuggestionBucket {
    InvocationSuggestionBucket {
        items: Vec::new(),
        has_more: false,
    }
}

fn suggestion_response_for_field(
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

async fn query_invocation_suggestion_bucket(
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

fn is_legacy_invocation_stream_query(params: &ListQuery) -> bool {
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

async fn query_invocation_exception_summary(
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
    let request = build_invocation_list_request(&params, state.config.list_limit_max as i64)?;
    let source_scope = resolve_default_source_scope(&state.pool).await?;
    let runtime_overlay_records = if should_overlay_runtime_records(&request) {
        runtime_overlay_snapshot(state.as_ref())
    } else {
        Vec::new()
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

pub(crate) async fn fetch_invocation_pool_attempts(
    State(state): State<Arc<AppState>>,
    axum::extract::Path(invoke_id): axum::extract::Path<String>,
) -> Result<Json<Vec<ApiPoolUpstreamRequestAttempt>>, ApiError> {
    Ok(Json(
        query_pool_attempt_records_from_live(&state.pool, &invoke_id).await?,
    ))
}

#[derive(Debug, FromRow)]
struct InvocationResponseBodyRow {
    id: i64,
    raw_response: String,
    response_raw_path: Option<String>,
    response_raw_size: Option<i64>,
    response_raw_truncated: Option<i64>,
    response_raw_truncated_reason: Option<String>,
    detail_level: String,
    detail_prune_reason: Option<String>,
    response_content_encoding: Option<String>,
    failure_class: Option<String>,
}

fn is_abnormal_invocation_failure(failure_class: Option<&str>) -> bool {
    matches!(
        failure_class
            .map(str::trim)
            .filter(|value| !value.is_empty()),
        Some("service_failure" | "client_failure" | "client_abort")
    )
}

fn truncate_response_preview_text(value: &str) -> (String, bool) {
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

fn raw_response_fallback_reason(row: &InvocationResponseBodyRow) -> String {
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

fn resolve_response_body_text_from_row(
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
                let preview_len = row.raw_response.as_bytes().len() as i64;
                if !raw_preview.is_empty()
                    && row.response_raw_size.unwrap_or(preview_len) <= preview_len
                {
                    return Ok((row.raw_response.clone(), false));
                }
                return Err("raw_file_missing".to_string());
            }
            Err(err) => {
                let raw_preview = row.raw_response.trim();
                let preview_len = row.raw_response.as_bytes().len() as i64;
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

    let preview_len = row.raw_response.as_bytes().len() as i64;
    let fits_in_preview = row.response_raw_size.unwrap_or(preview_len) <= preview_len;
    if fits_in_preview && row.response_raw_truncated.unwrap_or_default() == 0 {
        return Ok((row.raw_response.clone(), false));
    }

    Err(raw_response_fallback_reason(row))
}

async fn fetch_invocation_response_body_row_by_id(
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
        let db_runtime_keys =
            query_current_runtime_db_keys(
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
        let (delta, runtime_new_count, stale_db_runtime_count) =
            runtime_overlay_total_delta(
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
    let totals = query_combined_totals(
        &state.pool,
        state.config.crs_stats.as_ref(),
        StatsFilter::All,
        source_scope,
    )
    .await?;
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

async fn load_in_progress_conversation_count(
    state: &AppState,
    source_scope: InvocationSourceScope,
    upstream_account_id: Option<i64>,
) -> Result<i64, ApiError> {
    Ok(load_in_progress_summary_snapshot(state, source_scope, upstream_account_id)
        .await?
        .0)
}

#[derive(Debug, Clone, Copy, Default)]
struct SummaryLiveAugmentation {
    in_progress_conversation_count: Option<i64>,
    in_progress_retry_conversation_count: Option<i64>,
    in_progress_avg_wait_ms: Option<f64>,
    non_success_tokens: Option<i64>,
}

#[derive(Debug, Clone, Copy)]
struct SummaryLiveAugmentationPolicy {
    include_in_progress: bool,
    include_non_success_tokens: bool,
}

fn summary_window_range(
    window: &SummaryWindow,
    reporting_tz: Tz,
    now: DateTime<Utc>,
) -> Result<Option<(DateTime<Utc>, DateTime<Utc>)>, ApiError> {
    match window {
        SummaryWindow::All | SummaryWindow::Current(_) => Ok(None),
        SummaryWindow::Duration(duration) => Ok(Some((now - *duration, now))),
        SummaryWindow::Calendar(spec) => {
            let range = resolve_range_window(spec.as_str(), reporting_tz).map_err(ApiError::from)?;
            Ok(Some((range.start, range.end)))
        }
        SummaryWindow::PreviousFullDays(day_count) => {
            let (start, end) = previous_full_days_range_bounds(*day_count, now, reporting_tz)
                .ok_or_else(|| ApiError::bad_request(anyhow!("invalid previous full days window")))?;
            Ok(Some((start, end)))
        }
    }
}

async fn load_summary_live_augmentation(
    state: &AppState,
    source_scope: InvocationSourceScope,
    upstream_account_id: Option<i64>,
    range: Option<(DateTime<Utc>, DateTime<Utc>)>,
    policy: SummaryLiveAugmentationPolicy,
) -> Result<SummaryLiveAugmentation, ApiError> {
    let in_progress = if policy.include_in_progress {
        let snapshot = load_in_progress_summary_snapshot(state, source_scope, upstream_account_id).await?;
        (Some(snapshot.0), Some(snapshot.1), snapshot.2)
    } else {
        (None, None, None)
    };
    let non_success_tokens = if policy.include_non_success_tokens {
        if let Some((start, end)) = range {
            load_non_success_tokens_snapshot(
                state,
                source_scope,
                upstream_account_id,
                start,
                end,
            )
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
        non_success_tokens,
    })
}

fn apply_summary_live_augmentation(
    response: &mut StatsResponse,
    augmentation: SummaryLiveAugmentation,
) {
    response.in_progress_conversation_count = augmentation.in_progress_conversation_count;
    response.in_progress_retry_conversation_count =
        augmentation.in_progress_retry_conversation_count;
    response.in_progress_avg_wait_ms = augmentation.in_progress_avg_wait_ms;
    response.non_success_tokens = augmentation.non_success_tokens;
}

async fn load_in_progress_summary_snapshot(
    state: &AppState,
    source_scope: InvocationSourceScope,
    upstream_account_id: Option<i64>,
) -> Result<(i64, i64, Option<f64>), ApiError> {
    #[derive(Debug, FromRow)]
    struct RuntimeKeyRow {
        invoke_id: String,
        occurred_at: String,
        retry_count: i64,
        upstream_ttfb_ms: Option<f64>,
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
            InvocationSourceScope::ProxyOnly => "live.is_retry_after_failure_proxy_only".to_string(),
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
    db_key_query.push(retry_sql.as_str()).push(
        " AS retry_count, live.upstream_ttfb_ms AS upstream_ttfb_ms \
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
    let mut db_ttfb_sum = avg_wait_ms.unwrap_or_default() * avg_wait_sample_count as f64;
    for row in &db_runtime_rows {
        let key = (row.invoke_id.clone(), row.occurred_at.clone());
        let Some(runtime_record) = runtime_by_key.get(&key) else {
            continue;
        };
        if runtime_in_flight_record_matches_filters(runtime_record, &filter, source_scope) {
            continue;
        }
        db_in_progress_count = db_in_progress_count.saturating_sub(1);
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
    let (ttfb_sum, ttfb_count) = runtime_records
        .iter()
        .filter(|record| {
            !db_runtime_keys.contains_key(&(record.invoke_id.clone(), record.occurred_at.clone()))
        })
        .filter_map(|record| record.t_upstream_ttfb_ms.filter(|value| value.is_finite() && *value >= 0.0))
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
    Ok((
        db_in_progress_count + runtime_in_progress_count,
        retry_count + runtime_retry_count,
        combined_avg_wait_ms,
    ))
}

async fn load_live_invocation_ids_in_range(
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

    let mut query = QueryBuilder::<Sqlite>::new("SELECT id FROM codex_invocations WHERE occurred_at >= ");
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

fn summary_live_augmentation_policy(
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

async fn load_non_success_tokens_snapshot(
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
        in_progress_conversation_count: None,
        in_progress_retry_conversation_count: None,
        in_progress_avg_wait_ms: None,
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
struct UpstreamAccountActivityMetaRow {
    id: i64,
    display_name: Option<String>,
    group_name: Option<String>,
    plan_type: Option<String>,
}

#[derive(Debug, Clone, FromRow)]
struct UpstreamAccountInProgressRow {
    upstream_account_id: i64,
    in_progress_count: i64,
    retry_count: i64,
}

#[derive(Debug, FromRow)]
struct UpstreamAccountActiveConversationCountRow {
    account_id: i64,
    active_conversation_count: i64,
}

#[derive(Debug, Default)]
struct UpstreamAccountActivityAccumulator {
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
    cache_input_tokens: i64,
    total_cost: f64,
    first_response_byte_total_sample_count: i64,
    first_response_byte_total_sum_ms: f64,
    total_latency_sample_count: i64,
    total_latency_sum_ms: f64,
    last_occurred_at_epoch_ms: i64,
    rate_usage_events: Vec<UpstreamAccountRateUsageEvent>,
    recent_invocations: Vec<PromptCacheConversationInvocationPreviewResponse>,
}

#[derive(Debug, Clone, Copy)]
struct UpstreamAccountRateUsageEvent {
    occurred_at_epoch_ms: i64,
    total_tokens: i64,
    total_cost: f64,
}

fn normalize_trimmed_optional_string_local(raw: Option<String>) -> Option<String> {
    raw.and_then(|value| {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    })
}

fn resolve_upstream_account_activity_display_name(
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

fn invocation_upstream_account_id_with_attempt_fallback_sql(invocation_ref: &str) -> String {
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

fn invocation_account_retry_after_failure_with_attempt_fallback_sql(
    current_upstream_account_id_sql: &str,
    source_scope: InvocationSourceScope,
) -> String {
    let previous_upstream_account_id_sql =
        invocation_upstream_account_id_with_attempt_fallback_sql("codex_invocations");
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
                  AND {previous_upstream_account_id_sql} = {current_upstream_account_id_sql}
                  AND id < live.invocation_id
                  {source_filter}
                  AND LOWER(TRIM({display_status_sql})) NOT IN ('running', 'pending')
                ORDER BY id DESC
                LIMIT 1
            ) AS previous_terminal
        ), 0)",
        prompt_cache_key_sql = INVOCATION_PROMPT_CACHE_KEY_SQL,
    )
}

fn compute_upstream_account_activity_rates(
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

fn compute_upstream_account_activity_tail_rate(
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

async fn query_live_upstream_account_activity_preview_rows(
    pool: &Pool<Sqlite>,
    source_scope: InvocationSourceScope,
    range: ExactUtcRange,
) -> Result<Vec<UpstreamAccountInvocationPreviewRow>, ApiError> {
    let resolved_upstream_account_id_sql =
        invocation_upstream_account_id_with_attempt_fallback_sql("codex_invocations");
    let mut query =
        QueryBuilder::<Sqlite>::new("SELECT id, invoke_id, ");
    query
        .push(INVOCATION_PROMPT_CACHE_KEY_SQL)
        .push(" AS prompt_cache_key, occurred_at, ")
        .push(invocation_display_status_sql())
        .push(" AS status, ")
        .push(INVOCATION_RESOLVED_FAILURE_CLASS_SQL)
        .push(" AS failure_class, ")
        .push(INVOCATION_ROUTE_MODE_SQL)
        .push(" AS route_mode, model, ")
        .push(INVOCATION_REQUEST_MODEL_SQL)
        .push(" AS request_model, ")
        .push(INVOCATION_RESPONSE_MODEL_SQL)
        .push(" AS response_model, COALESCE(total_tokens, 0) AS total_tokens, cost, source, input_tokens, output_tokens, cache_input_tokens, reasoning_tokens, ")
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
        .push_bind(db_occurred_at_upper_bound(range.end))
        .push(" AND ")
        .push(resolved_upstream_account_id_sql.as_str())
        .push(" IS NOT NULL");
    if source_scope == InvocationSourceScope::ProxyOnly {
        query.push(" AND source = ").push_bind(SOURCE_PROXY);
    }
    query.push(" ORDER BY occurred_at DESC, id DESC");

    Ok(query
        .build_query_as::<UpstreamAccountInvocationPreviewRow>()
        .fetch_all(pool)
        .await?)
}

fn runtime_upstream_account_activity_preview_row(
    record: ApiInvocation,
    source_scope: InvocationSourceScope,
) -> Option<UpstreamAccountInvocationPreviewRow> {
    if source_scope == InvocationSourceScope::ProxyOnly && record.source != SOURCE_PROXY {
        return None;
    }
    if !matches!(
        normalized_runtime_text(record.status.as_deref()).as_str(),
        "running" | "pending"
    ) {
        return None;
    }
    let Some(upstream_account_id) = record.upstream_account_id else {
        return None;
    };
    Some(UpstreamAccountInvocationPreviewRow {
        upstream_account_id,
        id: record.id,
        invoke_id: record.invoke_id,
        prompt_cache_key: record.prompt_cache_key,
        occurred_at: record.occurred_at,
        status: record.status.unwrap_or_else(|| "running".to_string()),
        failure_class: record.failure_class,
        route_mode: record.route_mode,
        model: record.model,
        request_model: record.request_model,
        response_model: record.response_model,
        total_tokens: record.total_tokens.unwrap_or_default(),
        cost: record.cost,
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
        is_actionable: record
            .is_actionable
            .map(|value| if value { 1 } else { 0 }),
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

fn overlay_runtime_upstream_account_activity_preview_rows(
    state: &AppState,
    rows: &mut Vec<UpstreamAccountInvocationPreviewRow>,
    source_scope: InvocationSourceScope,
) {
    let mut runtime_overlay_row_count = 0_i64;
    for record in state.proxy_runtime_invocations.snapshot() {
        let Some(row) = runtime_upstream_account_activity_preview_row(record, source_scope) else {
            continue;
        };
        let key = (row.invoke_id.clone(), row.occurred_at.clone());
        if let Some(existing) = rows
            .iter_mut()
            .find(|existing| (existing.invoke_id.clone(), existing.occurred_at.clone()) == key)
        {
            if matches!(
                normalized_runtime_text(Some(existing.status.as_str())).as_str(),
                "running" | "pending"
            ) {
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

async fn query_upstream_account_activity_meta(
    pool: &Pool<Sqlite>,
    account_ids: &[i64],
) -> Result<HashMap<i64, UpstreamAccountActivityMetaRow>, ApiError> {
    if account_ids.is_empty() {
        return Ok(HashMap::new());
    }
    let mut query = QueryBuilder::<Sqlite>::new(
        "SELECT id, display_name, group_name, plan_type FROM pool_upstream_accounts WHERE id IN (",
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

async fn query_upstream_account_in_progress_counts(
    state: &AppState,
    source_scope: InvocationSourceScope,
) -> Result<HashMap<i64, (i64, i64)>, ApiError> {
    #[derive(Debug, FromRow)]
    struct RuntimeKeyRow {
        invoke_id: String,
        occurred_at: String,
        upstream_account_id: i64,
        retry_count: i64,
    }

    let resolved_upstream_account_id_sql =
        invocation_upstream_account_id_with_attempt_fallback_sql("inv");
    let retry_sql = invocation_account_retry_after_failure_with_attempt_fallback_sql(
        resolved_upstream_account_id_sql.as_str(),
        source_scope,
    );
    let mut query = QueryBuilder::<Sqlite>::new("SELECT ");
    query
        .push(resolved_upstream_account_id_sql.as_str())
        .push(
            " AS upstream_account_id, \
                COUNT(*) AS in_progress_count, \
                COALESCE(SUM(",
        );
    query.push(retry_sql.as_str()).push(
        "), 0) AS retry_count \
         FROM invocation_in_progress_live live \
         JOIN codex_invocations inv ON inv.id = live.invocation_id \
         WHERE ",
    );
    query
        .push(resolved_upstream_account_id_sql.as_str())
        .push(" IS NOT NULL");
    if source_scope == InvocationSourceScope::ProxyOnly {
        query.push(" AND live.source = ").push_bind(SOURCE_PROXY);
    }
    query
        .push(" GROUP BY ")
        .push(resolved_upstream_account_id_sql.as_str());

    let mut counts = query
        .build_query_as::<UpstreamAccountInProgressRow>()
        .fetch_all(&state.pool)
        .await?
        .into_iter()
        .map(|row| {
            (
                row.upstream_account_id,
                (row.in_progress_count.max(0), row.retry_count.max(0)),
            )
        })
        .collect::<HashMap<_, _>>();
    let mut db_key_query = QueryBuilder::<Sqlite>::new("SELECT inv.invoke_id AS invoke_id, inv.occurred_at AS occurred_at, ");
    db_key_query
        .push(resolved_upstream_account_id_sql.as_str())
        .push(" AS upstream_account_id, ")
        .push(retry_sql.as_str())
        .push(
            " AS retry_count \
         FROM invocation_in_progress_live live \
         JOIN codex_invocations inv ON inv.id = live.invocation_id \
         WHERE ",
        );
    db_key_query
        .push(resolved_upstream_account_id_sql.as_str())
        .push(" IS NOT NULL");
    if source_scope == InvocationSourceScope::ProxyOnly {
        db_key_query
            .push(" AND live.source = ")
            .push_bind(SOURCE_PROXY);
    }
    let db_runtime_keys = db_key_query
        .build_query_as::<RuntimeKeyRow>()
        .fetch_all(&state.pool)
        .await?
        .into_iter()
        .map(|row| {
            (
                (row.invoke_id, row.occurred_at),
                (row.upstream_account_id, row.retry_count > 0),
            )
        })
        .collect::<HashMap<_, _>>();
    let mut runtime_overlay_row_count = 0_i64;
    for record in state.proxy_runtime_invocations.snapshot() {
        if source_scope == InvocationSourceScope::ProxyOnly && record.source != SOURCE_PROXY {
            continue;
        }
        if !matches!(
            normalized_runtime_text(record.status.as_deref()).as_str(),
            "running" | "pending"
        ) {
            continue;
        }
        let Some(upstream_account_id) = record.upstream_account_id else {
            continue;
        };
        let key = (record.invoke_id.clone(), record.occurred_at.clone());
        if let Some((db_upstream_account_id, db_is_retry)) = db_runtime_keys.get(&key).copied() {
            let runtime_is_retry = runtime_record_is_retry(&record);
            if db_upstream_account_id != upstream_account_id {
                if let Some(entry) = counts.get_mut(&db_upstream_account_id) {
                    entry.0 = entry.0.saturating_sub(1);
                    if db_is_retry {
                        entry.1 = entry.1.saturating_sub(1);
                    }
                }
                let entry = counts.entry(upstream_account_id).or_insert((0, 0));
                entry.0 += 1;
                if runtime_is_retry {
                    entry.1 += 1;
                }
                runtime_overlay_row_count += 1;
            } else if runtime_is_retry && !db_is_retry {
                let entry = counts.entry(upstream_account_id).or_insert((0, 0));
                entry.1 += 1;
            }
            continue;
        }
        let entry = counts.entry(upstream_account_id).or_insert((0, 0));
        entry.0 += 1;
        if runtime_record_is_retry(&record) {
            entry.1 += 1;
        }
        runtime_overlay_row_count += 1;
    }
    if runtime_overlay_row_count > 0 {
        debug!(
            endpoint = "/api/upstream-account-activity",
            runtime_overlay_row_count,
            "overlayed memory runtime account in-progress counts"
        );
    }
    Ok(counts)
}

async fn query_upstream_account_active_conversation_counts(
    pool: &Pool<Sqlite>,
    account_ids: &[i64],
    now: DateTime<Utc>,
) -> Result<HashMap<i64, i64>, ApiError> {
    if account_ids.is_empty() {
        return Ok(HashMap::new());
    }

    let active_cutoff =
        format_utc_iso(now - ChronoDuration::minutes(POOL_ROUTE_ACTIVE_STICKY_WINDOW_MINUTES));
    let mut query = QueryBuilder::<Sqlite>::new(
        r#"
        SELECT
            account_id,
            COUNT(*) AS active_conversation_count
        FROM pool_sticky_routes
        WHERE last_seen_at >=
        "#,
    );
    query.push_bind(&active_cutoff).push(" AND account_id IN (");
    {
        let mut separated = query.separated(", ");
        for account_id in account_ids {
            separated.push_bind(account_id);
        }
    }
    query.push(") GROUP BY account_id");

    Ok(query
        .build_query_as::<UpstreamAccountActiveConversationCountRow>()
        .fetch_all(pool)
        .await?
        .into_iter()
        .map(|row| (row.account_id, row.active_conversation_count))
        .collect())
}

pub(crate) async fn fetch_upstream_account_activity(
    State(state): State<Arc<AppState>>,
    Query(params): Query<UpstreamAccountActivityQuery>,
) -> Result<Json<UpstreamAccountActivityResponse>, ApiError> {
    if !matches!(params.range.as_str(), "today" | "yesterday" | "1d" | "7d") {
        return Err(ApiError::bad_request(anyhow!(
            "unsupported upstream-account-activity range: {}",
            params.range
        )));
    }

    let recent_limit = match params.recent_limit {
        Some(value) if !(1..=16).contains(&value) => {
            return Err(ApiError::bad_request(anyhow!(
                "recentLimit must be between 1 and 16"
            )));
        }
        Some(value) => value as usize,
        None => 4,
    };
    let reporting_tz = parse_reporting_tz(params.time_zone.as_deref())?;
    let range_window = resolve_range_window(&params.range, reporting_tz).map_err(ApiError::from)?;
    let source_scope = resolve_default_source_scope(&state.pool).await?;
    let range = ExactUtcRange {
        start: range_window.start,
        end: range_window.end,
    };
    let mut live_rows =
        query_live_upstream_account_activity_preview_rows(&state.pool, source_scope, range).await?;
    overlay_runtime_upstream_account_activity_preview_rows(
        state.as_ref(),
        &mut live_rows,
        source_scope,
    );
    let live_ids = live_rows.iter().map(|row| row.id).collect::<HashSet<_>>();
    let retention_cutoff = shanghai_retention_cutoff(state.config.invocation_max_days);
    let mut combined_rows = live_rows;
    if range.start < retention_cutoff {
        combined_rows.extend(
            crate::stats::query_completed_invocation_archive_preview_rows(
                &state.pool,
                source_scope,
                range,
                Some(&live_ids),
            )
            .await?,
        );
    }

    combined_rows.sort_by(|left, right| {
        right
            .occurred_at
            .cmp(&left.occurred_at)
            .then_with(|| right.id.cmp(&left.id))
    });

    let mut account_activity = HashMap::<i64, UpstreamAccountActivityAccumulator>::new();
    for row in combined_rows {
        let Some(occurred_at) = parse_to_utc_datetime(&row.occurred_at) else {
            continue;
        };
        let classification = resolve_failure_classification(
            Some(row.status.as_str()),
            row.error_message.as_deref(),
            row.failure_kind.as_deref(),
            row.failure_class.as_deref(),
            row.is_actionable,
        );
        let is_success = prompt_cache_and_timeseries_shared::invocation_status_is_success_like(
            Some(row.status.as_str()),
            row.error_message.as_deref(),
        ) && classification.failure_class == FailureClass::None;
        let counts_toward_failure =
            prompt_cache_and_timeseries_shared::invocation_status_counts_toward_terminal_totals(
                Some(row.status.as_str()),
            ) && classification.failure_class != FailureClass::None;
        let counts_toward_non_success = invocation_counts_toward_non_success_usage(
            Some(row.status.as_str()),
            row.error_message.as_deref(),
            row.failure_kind.as_deref(),
            row.failure_class.as_deref(),
            row.is_actionable,
        );

        let entry = account_activity
            .entry(row.upstream_account_id)
            .or_default();
        if entry.last_occurred_at_epoch_ms == 0 {
            entry.last_occurred_at_epoch_ms = occurred_at.timestamp_millis();
        }
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
            if let Some(total_ms) = row.t_total_ms.filter(|value| value.is_finite() && *value >= 0.0) {
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
        }
        entry.rate_usage_events.push(UpstreamAccountRateUsageEvent {
            occurred_at_epoch_ms: occurred_at.timestamp_millis(),
            total_tokens: row.total_tokens.max(0),
            total_cost: row.cost.unwrap_or_default().max(0.0),
        });
        if entry.recent_invocations.len() < recent_limit {
            entry.recent_invocations.push(
                upstream_account_invocation_preview_from_row(row.clone()),
            );
        }
    }

    if account_activity.is_empty() {
        return Ok(Json(UpstreamAccountActivityResponse {
            range: params.range,
            range_start: format_utc_iso(range.start),
            range_end: format_utc_iso(range.end),
            accounts: Vec::new(),
        }));
    }

    let account_ids = account_activity.keys().copied().collect::<Vec<_>>();
    let account_meta = query_upstream_account_activity_meta(&state.pool, &account_ids).await?;
    let now = Utc::now();
    let active_conversation_counts =
        query_upstream_account_active_conversation_counts(&state.pool, &account_ids, now).await?;
    let effective_routing_rules =
        crate::upstream_accounts::load_effective_routing_rules_for_accounts(
            &state.pool,
            &account_ids,
        )
        .await?;
    let in_progress_counts = if params.range == "yesterday" {
        HashMap::new()
    } else {
        query_upstream_account_in_progress_counts(state.as_ref(), source_scope).await?
    };

    let mut accounts = account_activity
        .into_iter()
        .map(|(upstream_account_id, aggregate)| {
            let meta = account_meta.get(&upstream_account_id);
            let (tokens_per_minute, spend_rate) = compute_upstream_account_activity_rates(
                &aggregate.rate_usage_events,
                range.start,
                range.end,
            );
            let (in_progress_invocation_count, retry_invocation_count) =
                if params.range == "yesterday" {
                    (None, None)
                } else {
                    let (in_progress_count, retry_count) = in_progress_counts
                        .get(&upstream_account_id)
                        .copied()
                        .unwrap_or((0, 0));
                    (Some(in_progress_count), Some(retry_count))
                };

            UpstreamAccountActivityAccountResponse {
                upstream_account_id,
                display_name: resolve_upstream_account_activity_display_name(
                    upstream_account_id,
                    meta,
                    aggregate.display_name_hint.as_deref(),
                ),
                group_name: normalize_trimmed_optional_string_local(
                    meta.and_then(|row| row.group_name.clone()),
                ),
                plan_type: normalize_trimmed_optional_string_local(
                    meta.and_then(|row| row.plan_type.clone())
                        .or(aggregate.plan_type_hint),
                ),
                request_count: aggregate.request_count,
                success_count: aggregate.success_count,
                failure_count: aggregate.failure_count,
                non_success_count: aggregate.non_success_count,
                total_tokens: aggregate.total_tokens,
                success_tokens: aggregate.success_tokens,
                non_success_tokens: aggregate.non_success_tokens,
                failure_tokens: aggregate.failure_tokens,
                failure_cost: aggregate.failure_cost,
                total_cost: aggregate.total_cost,
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
                retry_invocation_count,
                active_conversation_count: active_conversation_counts
                    .get(&upstream_account_id)
                    .copied()
                    .unwrap_or(0),
                effective_routing_rule: effective_routing_rules
                    .get(&upstream_account_id)
                    .cloned()
                    .unwrap_or_else(crate::upstream_accounts::default_effective_routing_rule),
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
                    .cmp(&left.recent_invocations.first().map(|row| row.occurred_at.as_str()))
            })
            .then_with(|| right.upstream_account_id.cmp(&left.upstream_account_id))
    });

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
                let start = Utc
                    .timestamp_opt(0, 0)
                    .single()
                    .ok_or_else(|| ApiError::from(anyhow!("invalid account all-time summary start")))?;
                query_hourly_backed_summary_range_for_account(
                    state.as_ref(),
                    start,
                    Utc::now(),
                    source_scope,
                    upstream_account_id,
                )
                .await?
            } else {
                query_combined_totals(
                    &state.pool,
                    state.config.crs_stats.as_ref(),
                    StatsFilter::All,
                    source_scope,
                )
                .await?
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
                query_combined_totals(
                    &state.pool,
                    state.config.crs_stats.as_ref(),
                    StatsFilter::RecentLimit(limit),
                    source_scope,
                )
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
            let range_window = resolve_range_window(spec.as_str(), reporting_tz).map_err(ApiError::from)?;
            if range_window.start >= range_window.end {
                return Ok(Json(
                    build_empty_summary_response(
                        state.as_ref(),
                        source_scope,
                        upstream_account_id,
                    )
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
                .ok_or_else(|| ApiError::bad_request(anyhow!("invalid previous full days window")))?;
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
                query_hourly_backed_summary_range(
                    state.as_ref(),
                    start,
                    end,
                    source_scope,
                )
                .await?
            }
        }
    };

    let mut response = totals.into_response();
    response.non_success_cost = Some(totals.non_success_cost);
    let range = summary_window_range(&window, reporting_tz, now)?;
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

async fn load_stats_maintenance_response(
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
