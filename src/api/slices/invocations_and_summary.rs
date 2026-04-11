use crate::forward_proxy::*;
use crate::stats::*;
use crate::*;
use chrono::Offset;
use chrono::Timelike;

pub(crate) const INVOCATION_PROXY_DISPLAY_SQL: &str = "COALESCE(NULLIF(TRIM(CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.proxyDisplayName') AS TEXT) END), ''), CASE WHEN TRIM(source) != 'proxy' THEN TRIM(source) END)";
pub(crate) const INVOCATION_ENDPOINT_SQL: &str =
    "CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.endpoint') AS TEXT) END";
pub(crate) const INVOCATION_FAILURE_KIND_SQL: &str = "COALESCE(CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.failureKind') AS TEXT) END, failure_kind)";
pub(crate) const INVOCATION_REQUESTER_IP_SQL: &str =
    "CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.requesterIp') AS TEXT) END";
const INVOCATION_PROMPT_CACHE_KEY_SQL: &str = "CASE WHEN json_valid(payload) THEN TRIM(CAST(json_extract(payload, '$.promptCacheKey') AS TEXT)) END";
pub(crate) const INVOCATION_STICKY_KEY_SQL: &str = "CASE WHEN json_valid(payload) THEN TRIM(COALESCE(CAST(json_extract(payload, '$.stickyKey') AS TEXT), CAST(json_extract(payload, '$.promptCacheKey') AS TEXT))) END";
const INVOCATION_UPSTREAM_SCOPE_SQL: &str = "COALESCE(CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.upstreamScope') AS TEXT) END, 'external')";
pub(crate) const INVOCATION_ROUTE_MODE_SQL: &str =
    "CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.routeMode') AS TEXT) END";
pub(crate) const INVOCATION_UPSTREAM_ACCOUNT_ID_SQL: &str = "CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.upstreamAccountId') AS INTEGER) END";
pub(crate) const INVOCATION_UPSTREAM_ACCOUNT_NAME_SQL: &str = "CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.upstreamAccountName') AS TEXT) END";
pub(crate) const INVOCATION_REASONING_EFFORT_SQL: &str = "CASE WHEN json_valid(payload) AND json_type(payload, '$.reasoningEffort') = 'text' THEN json_extract(payload, '$.reasoningEffort') END";
pub(crate) const INVOCATION_RESPONSE_CONTENT_ENCODING_SQL: &str = "CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.responseContentEncoding') AS TEXT) END";
pub(crate) const INVOCATION_DOWNSTREAM_STATUS_CODE_SQL: &str = "CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.downstreamStatusCode') AS INTEGER) END";
pub(crate) const INVOCATION_DOWNSTREAM_ERROR_MESSAGE_SQL: &str = "CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.downstreamErrorMessage') AS TEXT) END";
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
         model, input_tokens, output_tokens, \
         cache_input_tokens, reasoning_tokens, \
         ",
    );
    query
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

#[derive(Debug, Clone)]
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

async fn resolve_invocation_snapshot_id(
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

    if is_legacy_invocation_stream_query(&params) {
        let mut query = build_invocation_select_query();
        apply_invocation_records_filters(&mut query, &request.filters, source_scope, None);
        append_invocation_order_clause(&mut query, request.sort_by, request.sort_order);
        query.push(" LIMIT ").push_bind(request.page_size);

        let records = query
            .build_query_as::<ApiInvocation>()
            .fetch_all(&state.pool)
            .await?;

        return Ok(Json(ListResponse {
            snapshot_id: 0,
            total: records.len() as i64,
            page: 1,
            page_size: request.page_size,
            records,
        }));
    }

    let snapshot_id = request
        .snapshot_id
        .unwrap_or(resolve_invocation_snapshot_id(&state.pool, source_scope).await?);

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
    page_id_query
        .push(" LIMIT ")
        .push_bind(request.page_size)
        .push(" OFFSET ")
        .push_bind(offset);
    let page_ids = page_id_query
        .build_query_as::<PageIdRow>()
        .fetch_all(&mut *tx)
        .await?
        .into_iter()
        .map(|row| row.id)
        .collect::<Vec<_>>();

    if page_ids.is_empty() {
        return Ok(Json(ListResponse {
            snapshot_id,
            total,
            page: request.page,
            page_size: request.page_size,
            records: Vec::new(),
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

    let avg_tokens_per_request = if totals.total_count <= 0 {
        0.0
    } else {
        totals.total_tokens as f64 / totals.total_count as f64
    };

    Ok(Json(InvocationSummaryResponse {
        snapshot_id,
        new_records_count,
        total_count: totals.total_count,
        success_count: totals.success_count,
        failure_count: totals.failure_count,
        total_tokens: totals.total_tokens,
        total_cost: totals.total_cost,
        token: InvocationTokenSummary {
            request_count: totals.total_count,
            total_tokens: totals.total_tokens,
            avg_tokens_per_request,
            cache_input_tokens: totals.cache_input_tokens,
            total_cost: totals.total_cost,
        },
        network,
        exception,
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
    ensure_hourly_rollups_caught_up(state.as_ref()).await?;
    let source_scope = resolve_default_source_scope(&state.pool).await?;
    let totals = query_combined_totals(
        &state.pool,
        state.config.crs_stats.as_ref(),
        StatsFilter::All,
        source_scope,
    )
    .await?;
    let mut response = totals.into_response();
    response.maintenance = Some(load_stats_maintenance_response(state.as_ref()).await?);
    Ok(Json(response))
}

pub(crate) async fn fetch_summary(
    State(state): State<Arc<AppState>>,
    Query(params): Query<SummaryQuery>,
) -> Result<Json<StatsResponse>, ApiError> {
    let default_limit = state.config.list_limit_max as i64;
    let window = parse_summary_window(&params, default_limit)?;
    let reporting_tz = parse_reporting_tz(params.time_zone.as_deref())?;
    ensure_hourly_rollups_caught_up(state.as_ref()).await?;
    let source_scope = resolve_default_source_scope(&state.pool).await?;

    let totals = match window {
        SummaryWindow::All => {
            query_combined_totals(
                &state.pool,
                state.config.crs_stats.as_ref(),
                StatsFilter::All,
                source_scope,
            )
            .await?
        }
        SummaryWindow::Current(limit) => {
            query_combined_totals(
                &state.pool,
                state.config.crs_stats.as_ref(),
                StatsFilter::RecentLimit(limit),
                source_scope,
            )
            .await?
        }
        SummaryWindow::Duration(duration) => {
            let start = Utc::now() - duration;
            query_hourly_backed_summary_since(state.as_ref(), start, source_scope).await?
        }
        SummaryWindow::Calendar(spec) => {
            let range_window = resolve_range_window(spec.as_str(), reporting_tz).map_err(ApiError::from)?;
            if range_window.start >= range_window.end {
                return Ok(Json(StatsResponse {
                    total_count: 0,
                    success_count: 0,
                    failure_count: 0,
                    total_cost: 0.0,
                    total_tokens: 0,
                    maintenance: Some(load_stats_maintenance_response(state.as_ref()).await?),
                }));
            }
            query_hourly_backed_summary_range(
                state.as_ref(),
                range_window.start,
                range_window.end,
                source_scope,
            )
            .await?
        }
    };

    let mut response = totals.into_response();
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
struct ExactUtcRange {
    start: DateTime<Utc>,
    end: DateTime<Utc>,
}

#[derive(Debug, Default)]
pub(crate) struct HourlyRollupExactRangePlan {
    full_hour_range: Option<(i64, i64)>,
    live_exact_ranges: Vec<ExactUtcRange>,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub(crate) struct InvocationAggregateRecord {
    id: i64,
    occurred_at: String,
    status: Option<String>,
    total_tokens: Option<i64>,
    cost: Option<f64>,
    error_message: Option<String>,
    failure_kind: Option<String>,
    failure_class: Option<String>,
    is_actionable: Option<i64>,
    t_total_ms: Option<f64>,
    t_req_read_ms: Option<f64>,
    t_req_parse_ms: Option<f64>,
    t_upstream_connect_ms: Option<f64>,
    t_upstream_ttfb_ms: Option<f64>,
    t_upstream_stream_ms: Option<f64>,
    t_resp_parse_ms: Option<f64>,
    t_persist_ms: Option<f64>,
}

fn ceil_hour_epoch(epoch: i64) -> i64 {
    let floor = align_bucket_epoch(epoch, 3_600, 0);
    if floor < epoch { floor + 3_600 } else { floor }
}

fn exact_utc_range(
    start: DateTime<Utc>,
    end: DateTime<Utc>,
) -> Result<Option<ExactUtcRange>, ApiError> {
    if start >= end {
        return Ok(None);
    }
    Ok(Some(ExactUtcRange { start, end }))
}

fn push_exact_range(
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
