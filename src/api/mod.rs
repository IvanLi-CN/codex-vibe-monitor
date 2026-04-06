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
pub(crate) const INVOCATION_RESOLVED_FAILURE_CLASS_SQL: &str = "CASE   WHEN LOWER(TRIM(COALESCE(failure_class, ''))) IN ('service_failure', 'client_failure', 'client_abort')     THEN LOWER(TRIM(COALESCE(failure_class, '')))   ELSE     CASE       WHEN LOWER(TRIM(COALESCE(status, ''))) = 'success'         AND LOWER(TRIM(COALESCE(error_message, ''))) = '' THEN 'none'       WHEN LOWER(TRIM(COALESCE(status, ''))) IN ('running', 'pending')         AND LOWER(TRIM(COALESCE(error_message, ''))) = '' THEN 'none'       WHEN LOWER(TRIM(COALESCE(CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.failureKind') AS TEXT) END, failure_kind, ''))) = 'downstream_closed'         OR LOWER(TRIM(COALESCE(error_message, ''))) LIKE '[downstream_closed]%'         OR LOWER(TRIM(COALESCE(error_message, ''))) LIKE '%downstream closed while streaming upstream response%'         THEN 'client_abort'       WHEN LOWER(TRIM(COALESCE(status, ''))) = 'http_429'         OR LOWER(TRIM(COALESCE(CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.failureKind') AS TEXT) END, failure_kind, ''))) = 'upstream_http_429'         THEN 'service_failure'       WHEN LOWER(TRIM(COALESCE(CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.failureKind') AS TEXT) END, failure_kind, ''))) IN ('request_body_stream_error_client_closed', 'invalid_api_key', 'api_key_not_found', 'api_key_missing')         OR LOWER(TRIM(COALESCE(error_message, ''))) LIKE '[request_body_stream_error_client_closed]%'         OR LOWER(TRIM(COALESCE(error_message, ''))) LIKE '%failed to read request body stream%'         OR LOWER(TRIM(COALESCE(error_message, ''))) LIKE '%invalid api key format%'         OR LOWER(TRIM(COALESCE(error_message, ''))) LIKE '%api key format is invalid%'         OR LOWER(TRIM(COALESCE(error_message, ''))) LIKE '%incorrect api key provided%'         OR LOWER(TRIM(COALESCE(error_message, ''))) LIKE '%api key not found%'         OR LOWER(TRIM(COALESCE(error_message, ''))) LIKE '%please provide an api key%'         OR (LOWER(TRIM(COALESCE(status, ''))) LIKE 'http_4%' AND LOWER(TRIM(COALESCE(status, ''))) != 'http_429')         OR LOWER(TRIM(COALESCE(status, ''))) IN ('http_401', 'http_403')         THEN 'client_failure'       WHEN LOWER(TRIM(COALESCE(CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.failureKind') AS TEXT) END, failure_kind, ''))) IN ('failed_contact_upstream', 'upstream_response_failed', 'upstream_stream_error', 'request_body_read_timeout', 'upstream_handshake_timeout')         OR LOWER(TRIM(COALESCE(error_message, ''))) LIKE '[failed_contact_upstream]%'         OR LOWER(TRIM(COALESCE(error_message, ''))) LIKE '[upstream_response_failed]%'         OR LOWER(TRIM(COALESCE(error_message, ''))) LIKE '[upstream_stream_error]%'         OR LOWER(TRIM(COALESCE(error_message, ''))) LIKE '[request_body_read_timeout]%'         OR LOWER(TRIM(COALESCE(error_message, ''))) LIKE '[upstream_handshake_timeout]%'         OR LOWER(TRIM(COALESCE(error_message, ''))) LIKE '%failed to contact upstream%'         OR LOWER(TRIM(COALESCE(error_message, ''))) LIKE '%upstream response stream reported failure%'         OR LOWER(TRIM(COALESCE(error_message, ''))) LIKE '%upstream stream error%'         OR LOWER(TRIM(COALESCE(error_message, ''))) LIKE '%request body read timed out%'         OR LOWER(TRIM(COALESCE(error_message, ''))) LIKE '%upstream handshake timed out%'         OR LOWER(TRIM(COALESCE(status, ''))) LIKE 'http_5%'         THEN 'service_failure'       WHEN LOWER(TRIM(COALESCE(status, ''))) = 'success' THEN 'none'       ELSE 'service_failure'     END END";

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
         CASE WHEN json_valid(payload) THEN json_extract(payload, '$.endpoint') END AS endpoint, \
         ",
        )
        .push(INVOCATION_FAILURE_KIND_SQL)
        .push(
            " AS failure_kind, \
         CASE WHEN json_valid(payload) THEN json_extract(payload, '$.streamTerminalEvent') END AS stream_terminal_event, \
         CASE WHEN json_valid(payload) THEN json_extract(payload, '$.upstreamErrorCode') END AS upstream_error_code, \
         CASE WHEN json_valid(payload) THEN json_extract(payload, '$.upstreamErrorMessage') END AS upstream_error_message, \
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
        "CASE WHEN {resolved_failure} IN ('service_failure', 'client_failure', 'client_abort') THEN 'failed' WHEN {status_norm} = '' THEN 'unknown' ELSE {status_norm} END",
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
        .push(" ESCAPE '\\\\'");
    query
        .push(" OR LOWER(COALESCE(model, '')) LIKE ")
        .push_bind(like_pattern.clone())
        .push(" ESCAPE '\\\\'");
    query.push(" OR LOWER(TRIM(COALESCE(");
    query.push(INVOCATION_PROXY_DISPLAY_SQL);
    query.push(", ''))) LIKE ");
    query.push_bind(like_pattern.clone()).push(" ESCAPE '\\\\'");
    query.push(" OR LOWER(TRIM(COALESCE(");
    query.push(INVOCATION_ENDPOINT_SQL);
    query.push(", ''))) LIKE ");
    query.push_bind(like_pattern.clone()).push(" ESCAPE '\\\\'");
    query.push(" OR LOWER(TRIM(COALESCE(");
    query.push(INVOCATION_FAILURE_KIND_SQL);
    query.push(", ''))) LIKE ");
    query.push_bind(like_pattern.clone()).push(" ESCAPE '\\\\'");
    query
        .push(" OR LOWER(COALESCE(error_message, '')) LIKE ")
        .push_bind(like_pattern.clone())
        .push(" ESCAPE '\\\\'");
    query.push(" OR LOWER(TRIM(COALESCE(");
    query.push(INVOCATION_PROMPT_CACHE_KEY_SQL);
    query.push(", ''))) LIKE ");
    query.push_bind(like_pattern.clone()).push(" ESCAPE '\\\\'");
    query.push(" OR LOWER(TRIM(COALESCE(");
    query.push(INVOCATION_REQUESTER_IP_SQL);
    query.push(", ''))) LIKE ");
    query.push_bind(like_pattern).push(" ESCAPE '\\\\'");
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
    let total = count_query
        .build_query_as::<CountRow>()
        .fetch_one(&state.pool)
        .await?
        .total;

    let offset = (request.page - 1).saturating_mul(request.page_size);
    let mut query = build_invocation_select_query();
    apply_invocation_records_filters(
        &mut query,
        &request.filters,
        source_scope,
        Some(SnapshotConstraint::UpTo(snapshot_id)),
    );
    append_invocation_order_clause(&mut query, request.sort_by, request.sort_order);
    query
        .push(" LIMIT ")
        .push_bind(request.page_size)
        .push(" OFFSET ")
        .push_bind(offset);

    let records = query
        .build_query_as::<ApiInvocation>()
        .fetch_all(&state.pool)
        .await?;

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
         COALESCE(SUM(CASE WHEN {resolved_failure} = 'none' AND {status_norm} = 'success' THEN 1 ELSE 0 END), 0) AS success_count, \
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
            let now = Utc::now();
            let start = named_range_start(spec.as_str(), now, reporting_tz).ok_or_else(|| {
                ApiError::bad_request(anyhow!("unsupported calendar window: {spec}"))
            })?;
            query_hourly_backed_summary_since(state.as_ref(), start, source_scope).await?
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
struct HourlyRollupExactRangePlan {
    full_hour_range: Option<(i64, i64)>,
    live_exact_ranges: Vec<ExactUtcRange>,
}

#[derive(Debug, Clone, sqlx::FromRow)]
struct InvocationAggregateRecord {
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

fn split_exact_range_by_retention(
    live_ranges: &mut Vec<ExactUtcRange>,
    range: ExactUtcRange,
    raw_cutoff: DateTime<Utc>,
) -> Result<(), ApiError> {
    if range.end > raw_cutoff {
        push_exact_range(live_ranges, range.start.max(raw_cutoff), range.end)?;
    }
    Ok(())
}

fn build_hourly_rollup_exact_range_plan(
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    raw_cutoff: DateTime<Utc>,
) -> Result<HourlyRollupExactRangePlan, ApiError> {
    let mut plan = HourlyRollupExactRangePlan::default();
    let start_epoch = start.timestamp();
    let end_epoch = end.timestamp();
    // Archived history is only available as hourly buckets. Keep only the full hours that are
    // completely contained in the requested range so historical queries never overstate totals.
    let full_hour_start_epoch = ceil_hour_epoch(start_epoch);
    let full_hour_end_epoch = align_bucket_epoch(end_epoch, 3_600, 0);
    let full_hour_start = Utc
        .timestamp_opt(full_hour_start_epoch, 0)
        .single()
        .ok_or_else(|| ApiError::from(anyhow!("invalid full-hour start epoch")))?;
    let full_hour_end = Utc
        .timestamp_opt(full_hour_end_epoch, 0)
        .single()
        .ok_or_else(|| ApiError::from(anyhow!("invalid full-hour end epoch")))?;
    if full_hour_start_epoch < full_hour_end_epoch {
        plan.full_hour_range = Some((full_hour_start_epoch, full_hour_end_epoch));
    }
    if let Some(range) = exact_utc_range(start, end.min(full_hour_start))? {
        split_exact_range_by_retention(&mut plan.live_exact_ranges, range, raw_cutoff)?;
    }
    if let Some(range) = exact_utc_range(start.max(full_hour_end), end)? {
        split_exact_range_by_retention(&mut plan.live_exact_ranges, range, raw_cutoff)?;
    }
    Ok(plan)
}

fn effective_range_for_hourly_rollup_plan(
    plan: &HourlyRollupExactRangePlan,
) -> Result<Option<ExactUtcRange>, ApiError> {
    let mut range: Option<ExactUtcRange> = None;
    if let Some((start_epoch, end_epoch)) = plan.full_hour_range {
        let start = Utc
            .timestamp_opt(start_epoch, 0)
            .single()
            .ok_or_else(|| ApiError::from(anyhow!("invalid effective range start epoch")))?;
        let end = Utc
            .timestamp_opt(end_epoch, 0)
            .single()
            .ok_or_else(|| ApiError::from(anyhow!("invalid effective range end epoch")))?;
        range = Some(ExactUtcRange { start, end });
    }
    for exact_range in &plan.live_exact_ranges {
        range = Some(match range {
            Some(existing) => ExactUtcRange {
                start: existing.start.min(exact_range.start),
                end: existing.end.max(exact_range.end),
            },
            None => *exact_range,
        });
    }
    Ok(range.filter(|value| value.start < value.end))
}

async fn load_pool_attempt_account_names(
    pool: &Pool<Sqlite>,
    records: &mut [ApiPoolUpstreamRequestAttempt],
) -> Result<(), ApiError> {
    let account_ids = records
        .iter()
        .filter_map(|record| record.upstream_account_id)
        .collect::<HashSet<_>>();
    if account_ids.is_empty() {
        return Ok(());
    }

    #[derive(Debug, FromRow)]
    struct AccountNameRow {
        id: i64,
        display_name: String,
    }

    let mut query = QueryBuilder::<Sqlite>::new(
        "SELECT id, display_name FROM pool_upstream_accounts WHERE id IN (",
    );
    {
        let mut separated = query.separated(", ");
        for account_id in account_ids {
            separated.push_bind(account_id);
        }
    }
    query.push(")");
    let rows = query
        .build_query_as::<AccountNameRow>()
        .fetch_all(pool)
        .await?;
    let name_map = rows
        .into_iter()
        .map(|row| (row.id, row.display_name))
        .collect::<HashMap<_, _>>();
    for record in records {
        if record.upstream_account_name.is_none()
            && let Some(account_id) = record.upstream_account_id
        {
            record.upstream_account_name = name_map.get(&account_id).cloned();
        }
    }
    Ok(())
}

pub(crate) async fn query_pool_attempt_records_from_live(
    pool: &Pool<Sqlite>,
    invoke_id: &str,
) -> Result<Vec<ApiPoolUpstreamRequestAttempt>, ApiError> {
    let mut records = sqlx::query_as::<_, ApiPoolUpstreamRequestAttempt>(
        r#"
        SELECT
            attempts.id,
            attempts.invoke_id,
            attempts.occurred_at,
            attempts.endpoint,
            attempts.sticky_key,
            attempts.upstream_account_id,
            accounts.display_name AS upstream_account_name,
            attempts.upstream_route_key,
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
            attempts.failure_kind,
            attempts.error_message,
            attempts.connect_latency_ms,
            attempts.first_byte_latency_ms,
            attempts.stream_latency_ms,
            attempts.upstream_request_id,
            attempts.compact_support_status,
            attempts.compact_support_reason,
            attempts.created_at
        FROM pool_upstream_request_attempts AS attempts
        LEFT JOIN pool_upstream_accounts AS accounts
            ON accounts.id = attempts.upstream_account_id
        WHERE attempts.invoke_id = ?1
        ORDER BY attempts.attempt_index ASC, attempts.id ASC
        "#,
    )
    .bind(invoke_id)
    .fetch_all(pool)
    .await?;
    load_pool_attempt_account_names(pool, &mut records).await?;
    Ok(records)
}

async fn query_invocation_aggregate_records_from_live_range(
    pool: &Pool<Sqlite>,
    range: ExactUtcRange,
    source_scope: InvocationSourceScope,
) -> Result<Vec<InvocationAggregateRecord>, ApiError> {
    let mut query = QueryBuilder::<Sqlite>::new(
        "SELECT \
            id, occurred_at, status, total_tokens, cost, error_message, failure_kind, \
            failure_class, is_actionable, t_total_ms, t_req_read_ms, t_req_parse_ms, \
            t_upstream_connect_ms, t_upstream_ttfb_ms, t_upstream_stream_ms, \
            t_resp_parse_ms, t_persist_ms \
         FROM codex_invocations \
         WHERE occurred_at >= ",
    );
    query
        .push_bind(db_occurred_at_lower_bound(range.start))
        .push(" AND occurred_at < ")
        .push_bind(db_occurred_at_upper_bound(range.end));
    if source_scope == InvocationSourceScope::ProxyOnly {
        query.push(" AND source = ").push_bind(SOURCE_PROXY);
    }
    query.push(" ORDER BY occurred_at ASC, id ASC");
    query
        .build_query_as::<InvocationAggregateRecord>()
        .fetch_all(pool)
        .await
        .map_err(Into::into)
}

fn extend_unique_invocation_records(
    records: &mut Vec<InvocationAggregateRecord>,
    seen_ids: &mut HashSet<i64>,
    candidates: Vec<InvocationAggregateRecord>,
) {
    for record in candidates {
        if seen_ids.insert(record.id) {
            records.push(record);
        }
    }
}

async fn query_invocation_exact_records(
    pool: &Pool<Sqlite>,
    range_plan: &HourlyRollupExactRangePlan,
    source_scope: InvocationSourceScope,
) -> Result<Vec<InvocationAggregateRecord>, ApiError> {
    let mut records = Vec::new();
    let mut seen_ids = HashSet::new();

    for range in &range_plan.live_exact_ranges {
        extend_unique_invocation_records(
            &mut records,
            &mut seen_ids,
            query_invocation_aggregate_records_from_live_range(pool, *range, source_scope).await?,
        );
    }

    records.sort_by(|left, right| {
        left.occurred_at
            .cmp(&right.occurred_at)
            .then_with(|| left.id.cmp(&right.id))
    });
    Ok(records)
}

fn add_invocation_record_to_summary_totals(
    totals: &mut StatsTotals,
    record: &InvocationAggregateRecord,
) {
    totals.total_count += 1;
    match record.status.as_deref() {
        Some("success") => totals.success_count += 1,
        Some(_) => totals.failure_count += 1,
        None => {}
    }
    totals.total_tokens += record.total_tokens.unwrap_or_default();
    totals.total_cost += record.cost.unwrap_or_default();
}

fn db_occurred_at_upper_bound(end_utc: DateTime<Utc>) -> String {
    if end_utc.timestamp_subsec_nanos() > 0 {
        return db_occurred_at_lower_bound(end_utc + ChronoDuration::seconds(1));
    }
    db_occurred_at_lower_bound(end_utc)
}

fn record_perf_stage_sample(
    by_stage: &mut BTreeMap<String, (i64, f64, f64, ApproxHistogramCounts)>,
    stage: &str,
    value: Option<f64>,
) {
    let Some(value) = value else {
        return;
    };
    let entry = by_stage
        .entry(stage.to_string())
        .or_insert_with(|| (0, 0.0, 0.0, empty_approx_histogram()));
    entry.0 += 1;
    entry.1 += value;
    entry.2 = entry.2.max(value);
    add_approx_histogram_sample(&mut entry.3, value);
}

pub(crate) async fn query_hourly_backed_summary_since_with_config(
    pool: &Pool<Sqlite>,
    relay: Option<&CrsStatsConfig>,
    invocation_max_days: u64,
    start: DateTime<Utc>,
    source_scope: InvocationSourceScope,
) -> Result<StatsTotals, ApiError> {
    let retention_cutoff = shanghai_retention_cutoff(invocation_max_days);
    if start >= retention_cutoff {
        return query_combined_totals(pool, relay, StatsFilter::Since(start), source_scope)
            .await
            .map_err(Into::into);
    }

    let mut totals = StatsTotals::default();
    let now = Utc::now();
    let range_plan = build_hourly_rollup_exact_range_plan(start, now, retention_cutoff)?;
    if let Some((range_start_epoch, range_end_epoch)) = range_plan.full_hour_range {
        let rows = query_invocation_hourly_rollup_range(
            pool,
            range_start_epoch,
            range_end_epoch,
            source_scope,
        )
        .await?;
        for row in rows {
            totals.total_count += row.total_count;
            totals.success_count += row.success_count;
            totals.failure_count += row.failure_count;
            totals.total_tokens += row.total_tokens;
            totals.total_cost += row.total_cost;
        }
    }
    let exact_records = query_invocation_exact_records(pool, &range_plan, source_scope).await?;
    for record in &exact_records {
        add_invocation_record_to_summary_totals(&mut totals, record);
    }
    let relay_totals =
        if let Some(effective_range) = effective_range_for_hourly_rollup_plan(&range_plan)? {
            query_crs_totals(
                pool,
                relay,
                &StatsFilter::Since(effective_range.start),
                source_scope,
            )
            .await?
        } else {
            StatsTotals::default()
        };
    Ok(totals.add(relay_totals))
}

pub(crate) async fn query_hourly_backed_summary_since(
    state: &AppState,
    start: DateTime<Utc>,
    source_scope: InvocationSourceScope,
) -> Result<StatsTotals, ApiError> {
    query_hourly_backed_summary_since_with_config(
        &state.pool,
        state.config.crs_stats.as_ref(),
        state.config.invocation_max_days,
        start,
        source_scope,
    )
    .await
    .map_err(Into::into)
}

pub(crate) async fn fetch_forward_proxy_live_stats(
    State(state): State<Arc<AppState>>,
) -> Result<Json<ForwardProxyLiveStatsResponse>, ApiError> {
    ensure_hourly_rollups_caught_up(state.as_ref()).await?;
    let response = build_forward_proxy_live_stats_response(state.as_ref()).await?;
    Ok(Json(response))
}

pub(crate) async fn fetch_forward_proxy_timeseries(
    State(state): State<Arc<AppState>>,
    Query(params): Query<TimeseriesQuery>,
) -> Result<Json<ForwardProxyTimeseriesResponse>, ApiError> {
    let reporting_tz = parse_reporting_tz(params.time_zone.as_deref())?;
    let range_window = resolve_range_window(&params.range, reporting_tz)?;
    ensure_forward_proxy_hourly_tz_supported(reporting_tz, &range_window)?;
    let bucket_spec = params.bucket.as_deref().unwrap_or("1h");
    if bucket_seconds_from_spec(bucket_spec) != Some(3_600) {
        return Err(ApiError::bad_request(anyhow!(
            "unsupported forward proxy bucket specification: {bucket_spec}; only 1h is supported"
        )));
    }
    ensure_hourly_rollups_caught_up(state.as_ref()).await?;
    let response = build_forward_proxy_timeseries_response(state.as_ref(), range_window).await?;
    Ok(Json(response))
}

fn ensure_forward_proxy_hourly_tz_supported(
    reporting_tz: Tz,
    range_window: &RangeWindow,
) -> Result<(), ApiError> {
    if reporting_tz_has_whole_hour_offsets(reporting_tz, range_window) {
        return Ok(());
    }
    Err(ApiError::bad_request(anyhow!(
        "unsupported timeZone for forward proxy hourly timeseries: {reporting_tz}; hourly buckets require whole-hour UTC offsets"
    )))
}

pub(crate) fn reporting_tz_has_whole_hour_offsets(
    reporting_tz: Tz,
    range_window: &RangeWindow,
) -> bool {
    const SAMPLE_STEP_DAYS: i64 = 1;

    fn offset_is_hour_aligned(reporting_tz: Tz, instant: DateTime<Utc>) -> bool {
        instant
            .with_timezone(&reporting_tz)
            .offset()
            .fix()
            .local_minus_utc()
            .rem_euclid(3_600)
            == 0
    }

    let mut cursor = range_window.start;
    while cursor < range_window.end {
        if !offset_is_hour_aligned(reporting_tz, cursor) {
            return false;
        }
        let Some(next) = cursor.checked_add_signed(ChronoDuration::days(SAMPLE_STEP_DAYS)) else {
            break;
        };
        if next >= range_window.end {
            break;
        }
        cursor = next;
    }
    if let Some(last_instant) = range_window
        .end
        .checked_sub_signed(ChronoDuration::nanoseconds(1))
        .filter(|instant| *instant >= range_window.start)
    {
        return offset_is_hour_aligned(reporting_tz, last_instant);
    }
    true
}

pub(crate) async fn fetch_prompt_cache_conversations(
    State(state): State<Arc<AppState>>,
    Query(params): Query<PromptCacheConversationsQuery>,
) -> Result<Json<PromptCacheConversationsResponse>, ApiError> {
    ensure_hourly_rollups_caught_up(state.as_ref()).await?;
    let selection = resolve_prompt_cache_conversation_selection(params)?;
    let response = fetch_prompt_cache_conversations_cached(state.as_ref(), selection).await?;
    Ok(Json(response))
}

pub(crate) fn normalize_prompt_cache_conversation_limit(raw: Option<i64>) -> i64 {
    match raw {
        Some(value @ (20 | 50 | 100)) => value,
        _ => PROMPT_CACHE_CONVERSATION_DEFAULT_LIMIT,
    }
}

pub(crate) fn normalize_prompt_cache_conversation_activity_hours(raw: Option<i64>) -> Option<i64> {
    match raw {
        Some(value @ (1 | 3 | 6 | 12 | 24)) => Some(value),
        _ => None,
    }
}

pub(crate) fn normalize_prompt_cache_conversation_activity_minutes(
    raw: Option<i64>,
) -> Option<i64> {
    match raw {
        Some(5) => Some(5),
        _ => None,
    }
}

pub(crate) fn resolve_prompt_cache_conversation_selection(
    params: PromptCacheConversationsQuery,
) -> Result<PromptCacheConversationSelection, ApiError> {
    let activity_param_count =
        i64::from(params.activity_hours.is_some()) + i64::from(params.activity_minutes.is_some());
    if params.limit.is_some() && activity_param_count > 0 {
        return Err(ApiError::bad_request(anyhow!(
            "limit, activityHours, and activityMinutes are mutually exclusive"
        )));
    }
    if params.activity_hours.is_some() && params.activity_minutes.is_some() {
        return Err(ApiError::bad_request(anyhow!(
            "activityHours and activityMinutes are mutually exclusive"
        )));
    }

    if let Some(hours) = normalize_prompt_cache_conversation_activity_hours(params.activity_hours) {
        return Ok(PromptCacheConversationSelection::ActivityWindowHours(hours));
    }

    if let Some(minutes) =
        normalize_prompt_cache_conversation_activity_minutes(params.activity_minutes)
    {
        return Ok(PromptCacheConversationSelection::ActivityWindowMinutes(
            minutes,
        ));
    }

    Ok(PromptCacheConversationSelection::Count(
        normalize_prompt_cache_conversation_limit(params.limit),
    ))
}

pub(crate) async fn fetch_prompt_cache_conversations_cached(
    state: &AppState,
    selection: PromptCacheConversationSelection,
) -> Result<PromptCacheConversationsResponse> {
    loop {
        let mut wait_on: Option<watch::Receiver<bool>> = None;
        let mut flight_guard: Option<PromptCacheConversationFlightGuard> = None;
        let build_generation: u64;
        {
            let mut cache = state.prompt_cache_conversation_cache.lock().await;
            let generation = cache.generation;
            if let Some(entry) = cache.entries.get(&selection)
                && entry.generation == generation
                && entry.cached_at.elapsed()
                    <= Duration::from_secs(PROMPT_CACHE_CONVERSATION_CACHE_TTL_SECS)
            {
                return Ok(entry.response.clone());
            }

            let in_flight_generation = cache
                .in_flight
                .get(&selection)
                .map(|flight| flight.generation);
            match in_flight_generation {
                Some(current_generation) if current_generation == generation => {
                    if let Some(in_flight) = cache.in_flight.get(&selection) {
                        wait_on = Some(in_flight.signal.subscribe());
                    }
                }
                Some(_) => {
                    cache.in_flight.remove(&selection);
                }
                None => {}
            }

            if wait_on.is_none() {
                let (signal, _receiver) = watch::channel(false);
                cache.in_flight.insert(
                    selection,
                    PromptCacheConversationInFlight { signal, generation },
                );
                build_generation = generation;
                flight_guard = Some(PromptCacheConversationFlightGuard::new(
                    state.prompt_cache_conversation_cache.clone(),
                    selection,
                    generation,
                ));
            } else {
                build_generation = generation;
            }
        }

        if let Some(mut receiver) = wait_on {
            if !*receiver.borrow() {
                let _ = receiver.changed().await;
            }
            continue;
        }

        let result = build_prompt_cache_conversations_response(state, selection).await;

        if let Some(guard) = flight_guard.as_mut() {
            guard.disarm();
        }

        let mut cache = state.prompt_cache_conversation_cache.lock().await;
        let stale_result = result.is_ok() && cache.generation != build_generation;
        let in_flight = match cache.in_flight.remove(&selection) {
            Some(in_flight) if in_flight.generation == build_generation => Some(in_flight),
            Some(in_flight) => {
                cache.in_flight.insert(selection, in_flight);
                None
            }
            None => None,
        };
        if let Some(in_flight) = in_flight {
            if let Ok(response) = &result {
                if !stale_result && cache.generation == build_generation {
                    cache.entries.insert(
                        selection,
                        PromptCacheConversationsCacheEntry {
                            cached_at: Instant::now(),
                            generation: build_generation,
                            response: response.clone(),
                        },
                    );
                }
            }
            let _ = in_flight.signal.send(true);
        }

        return result;
    }
}

pub(crate) async fn build_prompt_cache_conversations_response(
    state: &AppState,
    selection: PromptCacheConversationSelection,
) -> Result<PromptCacheConversationsResponse> {
    let source_scope = resolve_default_source_scope(&state.pool).await?;
    let range_end = Utc::now();
    let range_start = range_end - selection.activity_window_duration();
    let range_start_bound = db_occurred_at_lower_bound(range_start);
    let display_limit = selection.display_limit();

    let (aggregates, active_filtered_count) = match selection {
        PromptCacheConversationSelection::Count(limit) => {
            let aggregates = query_prompt_cache_conversation_aggregates(
                &state.pool,
                &range_start_bound,
                source_scope,
                display_limit,
            )
            .await?;
            let filtered_count = query_prompt_cache_conversation_hidden_count(
                &state.pool,
                &range_start_bound,
                source_scope,
                limit,
                aggregates.len() as i64,
            )
            .await?;
            (aggregates, filtered_count)
        }
        PromptCacheConversationSelection::ActivityWindowHours(_) => {
            let aggregates = query_prompt_cache_conversation_aggregates(
                &state.pool,
                &range_start_bound,
                source_scope,
                display_limit,
            )
            .await?;
            let matched_count = query_active_prompt_cache_conversation_count(
                &state.pool,
                &range_start_bound,
                source_scope,
            )
            .await?;
            (aggregates, matched_count.saturating_sub(display_limit))
        }
        PromptCacheConversationSelection::ActivityWindowMinutes(_) => {
            let aggregates = query_prompt_cache_working_conversation_aggregates(
                &state.pool,
                &range_start_bound,
                source_scope,
                display_limit,
            )
            .await?;
            let matched_count = query_working_prompt_cache_conversation_count(
                &state.pool,
                &range_start_bound,
                source_scope,
            )
            .await?;
            (aggregates, matched_count.saturating_sub(display_limit))
        }
    };
    let implicit_filter = selection.implicit_filter(active_filtered_count);

    if aggregates.is_empty() {
        return Ok(PromptCacheConversationsResponse {
            range_start: format_utc_iso(range_start),
            range_end: format_utc_iso(range_end),
            selection_mode: selection.selection_mode(),
            selected_limit: selection.selected_limit(),
            selected_activity_hours: selection.selected_activity_hours(),
            selected_activity_minutes: selection.selected_activity_minutes(),
            implicit_filter,
            conversations: Vec::new(),
        });
    }

    let selected_keys = aggregates
        .iter()
        .map(|row| row.prompt_cache_key.clone())
        .collect::<Vec<_>>();
    let chart_range_start_bound = resolve_prompt_cache_conversation_chart_range_start(
        range_end,
        aggregates.iter().map(|row| row.created_at.as_str()).min(),
    );
    let events = query_prompt_cache_conversation_events(
        &state.pool,
        &chart_range_start_bound,
        source_scope,
        &selected_keys,
    )
    .await?;
    let upstream_account_rows = query_prompt_cache_conversation_upstream_account_summaries(
        &state.pool,
        source_scope,
        &selected_keys,
    )
    .await?;
    let recent_invocation_rows = query_prompt_cache_conversation_recent_invocations(
        &state.pool,
        source_scope,
        &selected_keys,
        PROMPT_CACHE_CONVERSATION_INVOCATION_PREVIEW_LIMIT as i64,
    )
    .await?;

    let mut grouped_events: HashMap<String, Vec<PromptCacheConversationRequestPointResponse>> =
        HashMap::new();
    for row in events {
        let status = row.status.trim().to_string();
        let status = if status.is_empty() {
            "unknown".to_string()
        } else {
            status
        };
        let is_success = status.eq_ignore_ascii_case("success");
        let request_tokens = row.request_tokens.max(0);
        let points = grouped_events.entry(row.prompt_cache_key).or_default();
        let cumulative_tokens = points
            .last()
            .map(|point| point.cumulative_tokens)
            .unwrap_or(0)
            + request_tokens;
        points.push(PromptCacheConversationRequestPointResponse {
            occurred_at: row.occurred_at,
            status,
            is_success,
            request_tokens,
            cumulative_tokens,
        });
    }

    let mut upstream_account_rows_by_key: HashMap<
        String,
        Vec<PromptCacheConversationUpstreamAccountSummaryRow>,
    > = HashMap::new();
    for row in upstream_account_rows {
        upstream_account_rows_by_key
            .entry(row.prompt_cache_key.clone())
            .or_default()
            .push(row);
    }
    let mut grouped_recent_invocations: HashMap<
        String,
        Vec<PromptCacheConversationInvocationPreviewResponse>,
    > = HashMap::new();
    for row in recent_invocation_rows {
        grouped_recent_invocations
            .entry(row.prompt_cache_key.clone())
            .or_default()
            .push(PromptCacheConversationInvocationPreviewResponse {
                id: row.id,
                invoke_id: row.invoke_id,
                occurred_at: row.occurred_at,
                status: row.status,
                failure_class: normalize_trimmed_optional_string(row.failure_class),
                route_mode: normalize_trimmed_optional_string(row.route_mode),
                model: normalize_trimmed_optional_string(row.model),
                total_tokens: row.total_tokens.max(0),
                cost: row.cost,
                proxy_display_name: normalize_trimmed_optional_string(row.proxy_display_name),
                upstream_account_id: row.upstream_account_id,
                upstream_account_name: normalize_trimmed_optional_string(row.upstream_account_name),
                endpoint: normalize_trimmed_optional_string(row.endpoint),
                source: normalize_trimmed_optional_string(row.source),
                input_tokens: row.input_tokens,
                output_tokens: row.output_tokens,
                cache_input_tokens: row.cache_input_tokens,
                reasoning_tokens: row.reasoning_tokens,
                reasoning_effort: normalize_trimmed_optional_string(row.reasoning_effort),
                error_message: normalize_trimmed_optional_string(row.error_message),
                failure_kind: normalize_trimmed_optional_string(row.failure_kind),
                is_actionable: row.is_actionable.map(|value| value != 0),
                response_content_encoding: normalize_trimmed_optional_string(
                    row.response_content_encoding,
                ),
                requested_service_tier: normalize_trimmed_optional_string(
                    row.requested_service_tier,
                ),
                service_tier: normalize_trimmed_optional_string(row.service_tier),
                t_req_read_ms: row.t_req_read_ms,
                t_req_parse_ms: row.t_req_parse_ms,
                t_upstream_connect_ms: row.t_upstream_connect_ms,
                t_upstream_ttfb_ms: row.t_upstream_ttfb_ms,
                t_upstream_stream_ms: row.t_upstream_stream_ms,
                t_resp_parse_ms: row.t_resp_parse_ms,
                t_persist_ms: row.t_persist_ms,
                t_total_ms: row.t_total_ms,
            });
    }

    let mut grouped_upstream_accounts: HashMap<
        String,
        Vec<PromptCacheConversationUpstreamAccountResponse>,
    > = HashMap::new();
    for (prompt_cache_key, rows) in upstream_account_rows_by_key {
        let mut unique_ids_by_name: HashMap<String, Option<i64>> = HashMap::new();
        for row in &rows {
            let Some(normalized_name) =
                normalize_trimmed_optional_string(row.upstream_account_name.clone())
            else {
                continue;
            };
            let Some(upstream_account_id) = row.upstream_account_id else {
                continue;
            };
            match unique_ids_by_name.entry(normalized_name) {
                std::collections::hash_map::Entry::Vacant(entry) => {
                    entry.insert(Some(upstream_account_id));
                }
                std::collections::hash_map::Entry::Occupied(mut entry) => {
                    if entry
                        .get()
                        .is_some_and(|existing_id| existing_id != upstream_account_id)
                    {
                        entry.insert(None);
                    }
                }
            }
        }

        let mut account_entries: HashMap<String, PromptCacheConversationUpstreamAccountResponse> =
            HashMap::new();
        for row in rows {
            let normalized_name =
                normalize_trimmed_optional_string(row.upstream_account_name.clone());
            let resolved_upstream_account_id = row.upstream_account_id.or_else(|| {
                normalized_name
                    .as_ref()
                    .and_then(|name| unique_ids_by_name.get(name).copied().flatten())
            });
            let account_group_key = resolve_prompt_cache_upstream_account_group_key(
                resolved_upstream_account_id,
                normalized_name.as_deref(),
            );
            let entry = account_entries.entry(account_group_key).or_insert_with(|| {
                PromptCacheConversationUpstreamAccountResponse {
                    upstream_account_id: resolved_upstream_account_id,
                    upstream_account_name: normalized_name.clone(),
                    request_count: 0,
                    total_tokens: 0,
                    total_cost: 0.0,
                    last_activity_at: row.last_activity_at.clone(),
                }
            });

            if entry.upstream_account_id.is_none() && resolved_upstream_account_id.is_some() {
                entry.upstream_account_id = resolved_upstream_account_id;
            }
            if entry.upstream_account_name.is_none() && normalized_name.is_some() {
                entry.upstream_account_name = normalized_name;
            }
            entry.request_count += row.request_count;
            entry.total_tokens += row.total_tokens.max(0);
            entry.total_cost += row.total_cost;
            if row.last_activity_at > entry.last_activity_at {
                entry.last_activity_at = row.last_activity_at;
            }
        }
        grouped_upstream_accounts.insert(
            prompt_cache_key,
            account_entries.into_values().collect::<Vec<_>>(),
        );
    }

    for accounts in grouped_upstream_accounts.values_mut() {
        accounts.sort_by(|left, right| {
            right
                .last_activity_at
                .cmp(&left.last_activity_at)
                .then_with(|| {
                    resolve_prompt_cache_upstream_account_label(
                        right.upstream_account_name.as_deref(),
                        right.upstream_account_id,
                    )
                    .cmp(&resolve_prompt_cache_upstream_account_label(
                        left.upstream_account_name.as_deref(),
                        left.upstream_account_id,
                    ))
                })
                .then_with(|| {
                    right
                        .upstream_account_id
                        .unwrap_or(i64::MIN)
                        .cmp(&left.upstream_account_id.unwrap_or(i64::MIN))
                })
                .then_with(|| right.total_tokens.cmp(&left.total_tokens))
                .then_with(|| right.request_count.cmp(&left.request_count))
        });
        accounts.truncate(PROMPT_CACHE_CONVERSATION_UPSTREAM_ACCOUNT_LIMIT);
    }

    let conversations = aggregates
        .into_iter()
        .map(|row| PromptCacheConversationResponse {
            prompt_cache_key: row.prompt_cache_key.clone(),
            request_count: row.request_count,
            total_tokens: row.total_tokens,
            total_cost: row.total_cost,
            created_at: row.created_at,
            last_activity_at: row.last_activity_at,
            upstream_accounts: grouped_upstream_accounts
                .remove(&row.prompt_cache_key)
                .unwrap_or_default(),
            recent_invocations: grouped_recent_invocations
                .remove(&row.prompt_cache_key)
                .unwrap_or_default(),
            last24h_requests: grouped_events
                .remove(&row.prompt_cache_key)
                .unwrap_or_default(),
        })
        .collect::<Vec<_>>();

    Ok(PromptCacheConversationsResponse {
        range_start: format_utc_iso(range_start),
        range_end: format_utc_iso(range_end),
        selection_mode: selection.selection_mode(),
        selected_limit: selection.selected_limit(),
        selected_activity_hours: selection.selected_activity_hours(),
        selected_activity_minutes: selection.selected_activity_minutes(),
        implicit_filter,
        conversations,
    })
}

fn resolve_prompt_cache_conversation_chart_range_start(
    range_end: DateTime<Utc>,
    earliest_created_at: Option<&str>,
) -> String {
    let floor = range_end - ChronoDuration::hours(PROMPT_CACHE_CONVERSATION_CHART_MAX_HOURS);
    let created_at = earliest_created_at
        .and_then(|raw| DateTime::parse_from_rfc3339(raw).ok())
        .map(|value| value.with_timezone(&Utc));
    let chart_start = match created_at {
        Some(created_at) if created_at > floor => created_at,
        _ => floor,
    };
    format_utc_iso(chart_start)
}

fn normalize_trimmed_optional_string(raw: Option<String>) -> Option<String> {
    raw.and_then(|value| {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    })
}

fn resolve_prompt_cache_upstream_account_label(
    upstream_account_name: Option<&str>,
    upstream_account_id: Option<i64>,
) -> String {
    if let Some(name) = upstream_account_name
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return name.to_string();
    }
    if let Some(account_id) = upstream_account_id {
        return format!("账号 #{account_id}");
    }
    "—".to_string()
}

fn resolve_prompt_cache_upstream_account_group_key(
    upstream_account_id: Option<i64>,
    upstream_account_name: Option<&str>,
) -> String {
    if let Some(account_id) = upstream_account_id {
        return format!("id:{account_id}");
    }
    if let Some(name) = upstream_account_name
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return format!("name:{name}");
    }
    "unknown".to_string()
}

pub(crate) async fn query_prompt_cache_conversation_aggregates(
    pool: &Pool<Sqlite>,
    range_start_bound: &str,
    source_scope: InvocationSourceScope,
    limit: i64,
) -> Result<Vec<PromptCacheConversationAggregateRow>> {
    let mut query = QueryBuilder::<Sqlite>::new(
        "WITH active AS (\
            SELECT prompt_cache_key, MIN(first_seen_at) AS first_seen_24h \
             FROM prompt_cache_rollup_hourly \
             WHERE last_seen_at >= ",
    );
    query.push_bind(range_start_bound);
    if source_scope == InvocationSourceScope::ProxyOnly {
        query.push(" AND source = ").push_bind(SOURCE_PROXY);
    }

    query.push(
        " GROUP BY prompt_cache_key\
         ), aggregates AS (\
            SELECT prompt_cache_key, \
                 SUM(request_count) AS request_count, \
                 SUM(total_tokens) AS total_tokens, \
                 SUM(total_cost) AS total_cost, \
                 MIN(first_seen_at) AS created_at, \
                 MAX(last_seen_at) AS last_activity_at \
             FROM prompt_cache_rollup_hourly \
             WHERE prompt_cache_key IN (SELECT prompt_cache_key FROM active)",
    );

    if source_scope == InvocationSourceScope::ProxyOnly {
        query.push(" AND source = ").push_bind(SOURCE_PROXY);
    }

    query
        .push(
            " GROUP BY prompt_cache_key\
         ) \
         SELECT prompt_cache_key, request_count, total_tokens, total_cost, created_at, last_activity_at \
         FROM aggregates \
         ORDER BY created_at DESC, prompt_cache_key DESC \
         LIMIT ",
        )
        .push_bind(limit);

    query
        .build_query_as::<PromptCacheConversationAggregateRow>()
        .fetch_all(pool)
        .await
        .map_err(Into::into)
}

pub(crate) async fn query_active_prompt_cache_conversation_count(
    pool: &Pool<Sqlite>,
    range_start_bound: &str,
    source_scope: InvocationSourceScope,
) -> Result<i64> {
    let mut query = QueryBuilder::<Sqlite>::new(
        "SELECT COUNT(DISTINCT prompt_cache_key) AS count \
         FROM prompt_cache_rollup_hourly \
         WHERE last_seen_at >= ",
    );
    query.push_bind(range_start_bound);

    if source_scope == InvocationSourceScope::ProxyOnly {
        query.push(" AND source = ").push_bind(SOURCE_PROXY);
    }

    let (count,) = query.build_query_as::<(i64,)>().fetch_one(pool).await?;
    Ok(count)
}

pub(crate) async fn query_working_prompt_cache_conversation_count(
    pool: &Pool<Sqlite>,
    range_start_bound: &str,
    source_scope: InvocationSourceScope,
) -> Result<i64> {
    const KEY_EXPR: &str = "CASE WHEN json_valid(payload) THEN TRIM(CAST(json_extract(payload, '$.promptCacheKey') AS TEXT)) END";

    let mut query = QueryBuilder::<Sqlite>::new(
        "WITH recent_terminal AS (\
            SELECT ",
    );
    query
        .push(KEY_EXPR)
        .push(
            " AS prompt_cache_key \
             FROM codex_invocations \
             WHERE occurred_at >= ",
        )
        .push_bind(range_start_bound)
        .push(" AND ")
        .push(KEY_EXPR)
        .push(" IS NOT NULL AND ")
        .push(KEY_EXPR)
        .push(" <> '' AND LOWER(TRIM(")
        .push(invocation_display_status_sql())
        .push(")) NOT IN ('running', 'pending')");

    if source_scope == InvocationSourceScope::ProxyOnly {
        query.push(" AND source = ").push_bind(SOURCE_PROXY);
    }

    query.push(
        " GROUP BY prompt_cache_key\
         ), in_flight AS (\
            SELECT ",
    );
    query
        .push(KEY_EXPR)
        .push(
            " AS prompt_cache_key \
             FROM codex_invocations \
             WHERE ",
        )
        .push(KEY_EXPR)
        .push(" IS NOT NULL AND ")
        .push(KEY_EXPR)
        .push(" <> '' AND LOWER(TRIM(")
        .push(invocation_display_status_sql())
        .push(")) IN ('running', 'pending')");

    if source_scope == InvocationSourceScope::ProxyOnly {
        query.push(" AND source = ").push_bind(SOURCE_PROXY);
    }

    query.push(
        " GROUP BY prompt_cache_key\
         ), working AS (\
            SELECT prompt_cache_key FROM recent_terminal \
            UNION \
            SELECT prompt_cache_key FROM in_flight\
         ) \
         SELECT COUNT(*) AS count FROM working",
    );

    let (count,) = query.build_query_as::<(i64,)>().fetch_one(pool).await?;
    Ok(count)
}

pub(crate) async fn query_prompt_cache_conversation_hidden_count(
    pool: &Pool<Sqlite>,
    range_start_bound: &str,
    source_scope: InvocationSourceScope,
    requested_limit: i64,
    selected_active_count: i64,
) -> Result<i64> {
    if requested_limit <= 0 {
        return Ok(0);
    }

    let mut query = QueryBuilder::<Sqlite>::new(
        "WITH active AS (\
            SELECT DISTINCT prompt_cache_key \
         FROM prompt_cache_rollup_hourly \
         WHERE last_seen_at >= ",
    );
    query.push_bind(range_start_bound);

    if source_scope == InvocationSourceScope::ProxyOnly {
        query.push(" AND source = ").push_bind(SOURCE_PROXY);
    }

    query.push(
        " ), history AS (\
            SELECT prompt_cache_key, MIN(first_seen_at) AS created_at \
             FROM prompt_cache_rollup_hourly",
    );

    if source_scope == InvocationSourceScope::ProxyOnly {
        query.push(" WHERE source = ").push_bind(SOURCE_PROXY);
    }

    query.push(
        " GROUP BY prompt_cache_key\
         ), ranked AS (\
            SELECT history.prompt_cache_key, \
                   CASE WHEN active.prompt_cache_key IS NULL THEN 0 ELSE 1 END AS is_active, \
                   ROW_NUMBER() OVER (\
                       ORDER BY history.created_at DESC, history.prompt_cache_key DESC\
                   ) AS history_rank, \
                   SUM(CASE WHEN active.prompt_cache_key IS NULL THEN 0 ELSE 1 END) OVER (\
                       ORDER BY history.created_at DESC, history.prompt_cache_key DESC \
                       ROWS BETWEEN UNBOUNDED PRECEDING AND CURRENT ROW\
                   ) AS active_rank \
            FROM history \
            LEFT JOIN active ON active.prompt_cache_key = history.prompt_cache_key\
         ) \
         SELECT COUNT(*) AS count \
         FROM ranked \
         WHERE is_active = 0 AND ((",
    );
    query
        .push_bind(selected_active_count)
        .push(" < ")
        .push_bind(requested_limit)
        .push(" AND history_rank <= ")
        .push_bind(requested_limit)
        .push(") OR (")
        .push_bind(selected_active_count)
        .push(" >= ")
        .push_bind(requested_limit)
        .push(" AND active_rank < ")
        .push_bind(requested_limit)
        .push("))");

    let (count,) = query.build_query_as::<(i64,)>().fetch_one(pool).await?;
    Ok(count)
}

pub(crate) async fn query_prompt_cache_working_conversation_aggregates(
    pool: &Pool<Sqlite>,
    range_start_bound: &str,
    source_scope: InvocationSourceScope,
    limit: i64,
) -> Result<Vec<PromptCacheConversationAggregateRow>> {
    const KEY_EXPR: &str = "CASE WHEN json_valid(payload) THEN TRIM(CAST(json_extract(payload, '$.promptCacheKey') AS TEXT)) END";

    let mut query = QueryBuilder::<Sqlite>::new(
        "WITH recent_terminal AS (\
            SELECT ",
    );
    query
        .push(KEY_EXPR)
        .push(
            " AS prompt_cache_key, MAX(occurred_at) AS last_terminal_at \
             FROM codex_invocations \
             WHERE occurred_at >= ",
        )
        .push_bind(range_start_bound)
        .push(" AND ")
        .push(KEY_EXPR)
        .push(" IS NOT NULL AND ")
        .push(KEY_EXPR)
        .push(" <> '' AND LOWER(TRIM(")
        .push(invocation_display_status_sql())
        .push(")) NOT IN ('running', 'pending')");

    if source_scope == InvocationSourceScope::ProxyOnly {
        query.push(" AND source = ").push_bind(SOURCE_PROXY);
    }

    query.push(
        " GROUP BY prompt_cache_key\
         ), in_flight AS (\
            SELECT ",
    );
    query
        .push(KEY_EXPR)
        .push(
            " AS prompt_cache_key, MAX(occurred_at) AS last_in_flight_at \
             FROM codex_invocations \
             WHERE ",
        )
        .push(KEY_EXPR)
        .push(" IS NOT NULL AND ")
        .push(KEY_EXPR)
        .push(" <> '' AND LOWER(TRIM(")
        .push(invocation_display_status_sql())
        .push(")) IN ('running', 'pending')");

    if source_scope == InvocationSourceScope::ProxyOnly {
        query.push(" AND source = ").push_bind(SOURCE_PROXY);
    }

    query.push(
        " GROUP BY prompt_cache_key\
         ), working AS (\
            SELECT prompt_cache_key, last_terminal_at, NULL AS last_in_flight_at \
              FROM recent_terminal \
            UNION ALL \
            SELECT prompt_cache_key, NULL AS last_terminal_at, last_in_flight_at \
              FROM in_flight \
         ), collapsed_working AS (\
            SELECT prompt_cache_key, \
                   MAX(last_terminal_at) AS last_terminal_at, \
                   MAX(last_in_flight_at) AS last_in_flight_at, \
                   COALESCE(MAX(last_terminal_at), MAX(last_in_flight_at)) AS sort_anchor_at \
              FROM working \
              GROUP BY prompt_cache_key\
         ), aggregates AS (\
            SELECT prompt_cache_key, \
                   SUM(request_count) AS request_count, \
                   SUM(total_tokens) AS total_tokens, \
                   SUM(total_cost) AS total_cost, \
                   MIN(first_seen_at) AS created_at, \
                   MAX(last_seen_at) AS last_activity_at \
              FROM prompt_cache_rollup_hourly \
             WHERE prompt_cache_key IN (SELECT prompt_cache_key FROM collapsed_working)",
    );

    if source_scope == InvocationSourceScope::ProxyOnly {
        query.push(" AND source = ").push_bind(SOURCE_PROXY);
    }

    query
        .push(
            " GROUP BY prompt_cache_key\
         ) \
         SELECT aggregates.prompt_cache_key, aggregates.request_count, aggregates.total_tokens, \
                aggregates.total_cost, aggregates.created_at, aggregates.last_activity_at \
           FROM aggregates \
           INNER JOIN collapsed_working ON collapsed_working.prompt_cache_key = aggregates.prompt_cache_key \
          ORDER BY collapsed_working.sort_anchor_at DESC, aggregates.created_at DESC, aggregates.prompt_cache_key DESC \
          LIMIT ",
        )
        .push_bind(limit);

    query
        .build_query_as::<PromptCacheConversationAggregateRow>()
        .fetch_all(pool)
        .await
        .map_err(Into::into)
}

pub(crate) async fn query_prompt_cache_conversation_events(
    pool: &Pool<Sqlite>,
    range_start_bound: &str,
    source_scope: InvocationSourceScope,
    selected_keys: &[String],
) -> Result<Vec<PromptCacheConversationEventRow>> {
    if selected_keys.is_empty() {
        return Ok(Vec::new());
    }

    const KEY_EXPR: &str = "CASE WHEN json_valid(payload) THEN TRIM(CAST(json_extract(payload, '$.promptCacheKey') AS TEXT)) END";

    let mut query = QueryBuilder::<Sqlite>::new(
        "SELECT occurred_at, COALESCE(status, 'unknown') AS status, \
         COALESCE(total_tokens, 0) AS request_tokens, ",
    );
    query
        .push(KEY_EXPR)
        .push(
            " AS prompt_cache_key \
             FROM codex_invocations \
             WHERE occurred_at >= ",
        )
        .push_bind(range_start_bound)
        .push(" AND ")
        .push(KEY_EXPR)
        .push(" IN (");

    {
        let mut separated = query.separated(", ");
        for key in selected_keys {
            separated.push_bind(key);
        }
    }
    query.push(")");

    if source_scope == InvocationSourceScope::ProxyOnly {
        query.push(" AND source = ").push_bind(SOURCE_PROXY);
    }

    query.push(" ORDER BY prompt_cache_key ASC, occurred_at ASC, id ASC");

    query
        .build_query_as::<PromptCacheConversationEventRow>()
        .fetch_all(pool)
        .await
        .map_err(Into::into)
}

pub(crate) async fn query_prompt_cache_conversation_recent_invocations(
    pool: &Pool<Sqlite>,
    source_scope: InvocationSourceScope,
    selected_keys: &[String],
    limit_per_key: i64,
) -> Result<Vec<PromptCacheConversationInvocationPreviewRow>> {
    if selected_keys.is_empty() || limit_per_key <= 0 {
        return Ok(Vec::new());
    }

    const KEY_EXPR: &str = "CASE WHEN json_valid(payload) THEN TRIM(CAST(json_extract(payload, '$.promptCacheKey') AS TEXT)) END";
    let mut query =
        QueryBuilder::<Sqlite>::new("WITH ranked AS (SELECT id, invoke_id, occurred_at, ");
    query
        .push(invocation_display_status_sql())
        .push(" AS status, ")
        .push(INVOCATION_RESOLVED_FAILURE_CLASS_SQL)
        .push(" AS failure_class, ")
        .push(INVOCATION_ROUTE_MODE_SQL)
        .push(" AS route_mode, model, COALESCE(total_tokens, 0) AS total_tokens, cost, source, input_tokens, output_tokens, cache_input_tokens, reasoning_tokens, ")
        .push(INVOCATION_REASONING_EFFORT_SQL)
        .push(" AS reasoning_effort, error_message, ")
        .push(INVOCATION_FAILURE_KIND_SQL)
        .push(" AS failure_kind, CASE WHEN ")
        .push(INVOCATION_RESOLVED_FAILURE_CLASS_SQL)
        .push(" = 'service_failure' THEN 1 ELSE 0 END AS is_actionable, ")
        .push(INVOCATION_PROXY_DISPLAY_SQL)
        .push(" AS proxy_display_name, ")
        .push(INVOCATION_UPSTREAM_ACCOUNT_ID_SQL)
        .push(" AS upstream_account_id, ")
        .push(INVOCATION_UPSTREAM_ACCOUNT_NAME_SQL)
        .push(" AS upstream_account_name, ")
        .push(INVOCATION_RESPONSE_CONTENT_ENCODING_SQL)
        .push(
            " AS response_content_encoding, \
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
             t_req_read_ms, t_req_parse_ms, t_upstream_connect_ms, t_upstream_ttfb_ms, \
             t_upstream_stream_ms, t_resp_parse_ms, t_persist_ms, t_total_ms, ",
        )
        .push(INVOCATION_ENDPOINT_SQL)
        .push(" AS endpoint, ")
        .push(KEY_EXPR)
        .push(" AS prompt_cache_key, ROW_NUMBER() OVER (PARTITION BY ")
        .push(KEY_EXPR)
        .push(" ORDER BY occurred_at DESC, id DESC) AS row_number FROM codex_invocations WHERE ")
        .push(KEY_EXPR)
        .push(" IN (");

    {
        let mut separated = query.separated(", ");
        for key in selected_keys {
            separated.push_bind(key);
        }
    }
    query.push(")");

    if source_scope == InvocationSourceScope::ProxyOnly {
        query.push(" AND source = ").push_bind(SOURCE_PROXY);
    }

    query
        .push(") SELECT prompt_cache_key, id, invoke_id, occurred_at, status, failure_class, route_mode, model, total_tokens, cost, source, input_tokens, output_tokens, cache_input_tokens, reasoning_tokens, reasoning_effort, error_message, failure_kind, is_actionable, proxy_display_name, upstream_account_id, upstream_account_name, response_content_encoding, requested_service_tier, service_tier, t_req_read_ms, t_req_parse_ms, t_upstream_connect_ms, t_upstream_ttfb_ms, t_upstream_stream_ms, t_resp_parse_ms, t_persist_ms, t_total_ms, endpoint FROM ranked WHERE row_number <= ")
        .push_bind(limit_per_key)
        .push(" ORDER BY prompt_cache_key ASC, occurred_at DESC, id DESC");

    query
        .build_query_as::<PromptCacheConversationInvocationPreviewRow>()
        .fetch_all(pool)
        .await
        .map_err(Into::into)
}

pub(crate) async fn query_prompt_cache_conversation_upstream_account_summaries(
    pool: &Pool<Sqlite>,
    source_scope: InvocationSourceScope,
    selected_keys: &[String],
) -> Result<Vec<PromptCacheConversationUpstreamAccountSummaryRow>> {
    if selected_keys.is_empty() {
        return Ok(Vec::new());
    }

    let mut query = QueryBuilder::<Sqlite>::new(
        "SELECT prompt_cache_key, \
             upstream_account_id, \
             upstream_account_name, \
             SUM(request_count) AS request_count, \
             SUM(total_tokens) AS total_tokens, \
             SUM(total_cost) AS total_cost, \
             MAX(last_seen_at) AS last_activity_at \
         FROM prompt_cache_upstream_account_hourly \
         WHERE prompt_cache_key IN (",
    );

    {
        let mut separated = query.separated(", ");
        for key in selected_keys {
            separated.push_bind(key);
        }
    }
    query.push(")");
    if source_scope == InvocationSourceScope::ProxyOnly {
        query.push(" AND source = ").push_bind(SOURCE_PROXY);
    }

    query
        .push(
            " GROUP BY prompt_cache_key, upstream_account_key, upstream_account_id, upstream_account_name \
              ORDER BY prompt_cache_key ASC, last_activity_at DESC, upstream_account_name DESC, upstream_account_id DESC",
        )
        .build_query_as::<PromptCacheConversationUpstreamAccountSummaryRow>()
        .fetch_all(pool)
        .await
        .map_err(Into::into)
}

pub(crate) async fn fetch_timeseries(
    State(state): State<Arc<AppState>>,
    Query(params): Query<TimeseriesQuery>,
) -> Result<Json<TimeseriesResponse>, ApiError> {
    ensure_hourly_rollups_caught_up(state.as_ref()).await?;
    let reporting_tz = parse_reporting_tz(params.time_zone.as_deref())?;
    let source_scope = resolve_default_source_scope(&state.pool).await?;
    let range_window = resolve_range_window(&params.range, reporting_tz)?;
    let bucket_selection = resolve_timeseries_bucket_selection(
        &params,
        &range_window,
        state.config.invocation_max_days,
    )?;
    let bucket_seconds = bucket_selection.bucket_seconds;

    if bucket_seconds >= 3_600 {
        let tz_is_hour_aligned = reporting_tz_has_whole_hour_offsets(reporting_tz, &range_window);
        let needs_historical_rollups =
            range_window.start < shanghai_retention_cutoff(state.config.invocation_max_days);
        if !tz_is_hour_aligned {
            if needs_historical_rollups {
                return Err(ApiError::bad_request(anyhow!(
                    "unsupported timeZone for historical hourly timeseries: {reporting_tz}; historical hourly buckets require whole-hour UTC offsets"
                )));
            }
        } else {
            return fetch_timeseries_from_hourly_rollups(
                state,
                params,
                reporting_tz,
                source_scope,
                range_window,
                bucket_selection,
            )
            .await;
        }
    }

    let end_dt = range_window.end;
    let start_dt = range_window.start;
    let start_str_iso = format_utc_iso(start_dt);

    let mut records_query = QueryBuilder::new(
        "SELECT occurred_at, status, total_tokens, cost, t_req_read_ms, t_req_parse_ms, t_upstream_connect_ms, t_upstream_ttfb_ms FROM codex_invocations WHERE occurred_at >= ",
    );
    records_query.push_bind(db_occurred_at_lower_bound(start_dt));
    if source_scope == InvocationSourceScope::ProxyOnly {
        records_query.push(" AND source = ").push_bind(SOURCE_PROXY);
    }
    records_query.push(" ORDER BY occurred_at ASC");
    let records = records_query
        .build_query_as::<TimeseriesRecord>()
        .fetch_all(&state.pool)
        .await?;

    let mut aggregates: BTreeMap<i64, BucketAggregate> = BTreeMap::new();

    let start_epoch = start_dt.timestamp();

    for record in records {
        let naive = NaiveDateTime::parse_from_str(&record.occurred_at, "%Y-%m-%d %H:%M:%S")
            .map_err(|err| anyhow!("failed to parse occurred_at: {err}"))?;
        // Interpret stored naive time as local Asia/Shanghai and convert to UTC epoch
        let epoch = Shanghai
            .from_local_datetime(&naive)
            .single()
            .map(|dt| dt.with_timezone(&Utc).timestamp())
            .unwrap_or_else(|| naive.and_utc().timestamp());
        let bucket_epoch = align_reporting_bucket_epoch(epoch, bucket_seconds, reporting_tz)?;
        let entry = aggregates.entry(bucket_epoch).or_default();
        entry.total_count += 1;
        match record.status.as_deref() {
            Some("success") => entry.success_count += 1,
            Some(_) => entry.failure_count += 1,
            None => {}
        }
        entry.record_ttfb_sample(record.status.as_deref(), record.t_upstream_ttfb_ms);
        entry.record_first_response_byte_total_sample(
            record.t_req_read_ms,
            record.t_req_parse_ms,
            record.t_upstream_connect_ms,
            record.t_upstream_ttfb_ms,
        );
        entry.total_tokens += record.total_tokens.unwrap_or(0);
        entry.total_cost += record.cost.unwrap_or(0.0);
    }

    let relay_deltas = if source_scope == InvocationSourceScope::All
        && let Some(relay) = state.config.crs_stats.as_ref()
    {
        query_crs_deltas(&state.pool, relay, start_epoch, end_dt.timestamp()).await?
    } else {
        Vec::new()
    };

    for delta in relay_deltas {
        let bucket_epoch =
            align_reporting_bucket_epoch(delta.captured_at_epoch, bucket_seconds, reporting_tz)?;
        let entry = aggregates.entry(bucket_epoch).or_default();
        entry.total_count += delta.total_count;
        entry.success_count += delta.success_count;
        entry.failure_count += delta.failure_count;
        entry.total_tokens += delta.total_tokens;
        entry.total_cost += delta.total_cost;
    }

    // Fill every bucket that intersects the requested range using reporting-timezone
    // boundaries rather than fixed UTC-duration strides. This keeps DST transition
    // days aligned to local clock buckets.
    let fill_start_epoch = align_reporting_bucket_epoch(start_epoch, bucket_seconds, reporting_tz)?;
    let fill_end_epoch = next_reporting_bucket_epoch(
        align_reporting_bucket_epoch(end_dt.timestamp(), bucket_seconds, reporting_tz)?,
        bucket_seconds,
        reporting_tz,
    )?;
    let mut bucket_cursor = fill_start_epoch;
    while bucket_cursor < fill_end_epoch {
        aggregates.entry(bucket_cursor).or_default();
        bucket_cursor = next_reporting_bucket_epoch(bucket_cursor, bucket_seconds, reporting_tz)?;
    }

    let mut points = Vec::with_capacity(aggregates.len());
    for (bucket_epoch, agg) in aggregates {
        let bucket_end_epoch =
            next_reporting_bucket_epoch(bucket_epoch, bucket_seconds, reporting_tz)?;
        // Skip any buckets outside the desired window. This guards against
        // future-dated records leaking past the clamped end.
        if bucket_epoch < fill_start_epoch || bucket_end_epoch > fill_end_epoch {
            continue;
        }
        let start = Utc
            .timestamp_opt(bucket_epoch, 0)
            .single()
            .ok_or_else(|| anyhow!("invalid bucket epoch"))?;
        let end = Utc
            .timestamp_opt(bucket_end_epoch, 0)
            .single()
            .ok_or_else(|| anyhow!("invalid bucket epoch"))?;
        let first_byte_avg_ms = agg.first_byte_avg_ms();
        let first_byte_p95_ms = agg.first_byte_p95_ms();
        let first_response_byte_total_avg_ms = agg.first_response_byte_total_avg_ms();
        let first_response_byte_total_p95_ms = agg.first_response_byte_total_p95_ms();
        points.push(TimeseriesPoint {
            bucket_start: format_utc_iso(start),
            bucket_end: format_utc_iso(end),
            total_count: agg.total_count,
            success_count: agg.success_count,
            failure_count: agg.failure_count,
            total_tokens: agg.total_tokens,
            total_cost: agg.total_cost,
            first_byte_sample_count: agg.first_byte_sample_count,
            first_byte_avg_ms,
            first_byte_p95_ms,
            first_response_byte_total_sample_count: agg.first_response_byte_total_sample_count,
            first_response_byte_total_avg_ms,
            first_response_byte_total_p95_ms,
        });
    }

    let response = TimeseriesResponse {
        range_start: start_str_iso,
        range_end: {
            let end = Utc
                .timestamp_opt(fill_end_epoch, 0)
                .single()
                .unwrap_or_else(Utc::now);
            format_utc_iso(end)
        },
        bucket_seconds,
        effective_bucket: bucket_selection.effective_bucket,
        available_buckets: bucket_selection.available_buckets,
        bucket_limited_to_daily: bucket_selection.bucket_limited_to_daily,
        points,
    };

    Ok(Json(response))
}

pub(crate) async fn fetch_timeseries_from_hourly_rollups(
    state: Arc<AppState>,
    _params: TimeseriesQuery,
    reporting_tz: Tz,
    source_scope: InvocationSourceScope,
    range_window: RangeWindow,
    bucket_selection: TimeseriesBucketSelection,
) -> Result<Json<TimeseriesResponse>, ApiError> {
    let bucket_seconds = bucket_selection.bucket_seconds;
    let start_epoch = range_window.start.timestamp();
    let end_epoch = range_window.end.timestamp();
    let range_plan = build_hourly_rollup_exact_range_plan(
        range_window.start,
        range_window.end,
        shanghai_retention_cutoff(state.config.invocation_max_days),
    )?;

    let mut aggregates: BTreeMap<i64, BucketAggregate> = BTreeMap::new();
    let fill_start_epoch = align_reporting_bucket_epoch(start_epoch, bucket_seconds, reporting_tz)?;
    let fill_end_epoch = next_reporting_bucket_epoch(
        align_reporting_bucket_epoch(end_epoch, bucket_seconds, reporting_tz)?,
        bucket_seconds,
        reporting_tz,
    )?;
    let mut bucket_cursor = fill_start_epoch;
    while bucket_cursor < fill_end_epoch {
        aggregates.entry(bucket_cursor).or_default();
        bucket_cursor = next_reporting_bucket_epoch(bucket_cursor, bucket_seconds, reporting_tz)?;
    }

    if let Some((hourly_cursor, hourly_end_epoch)) = range_plan.full_hour_range {
        let rows = query_invocation_hourly_rollup_range(
            &state.pool,
            hourly_cursor,
            hourly_end_epoch,
            source_scope,
        )
        .await?;
        for row in rows {
            let bucket_epoch =
                align_reporting_bucket_epoch(row.bucket_start_epoch, bucket_seconds, reporting_tz)?;
            let entry = aggregates.entry(bucket_epoch).or_default();
            entry.total_count += row.total_count;
            entry.success_count += row.success_count;
            entry.failure_count += row.failure_count;
            entry.total_tokens += row.total_tokens;
            entry.total_cost += row.total_cost;
            entry.first_byte_sample_count += row.first_byte_sample_count;
            entry.first_byte_ttfb_sum_ms += row.first_byte_sum_ms;
            entry.first_byte_histogram = if entry.first_byte_histogram.is_empty() {
                decode_approx_histogram(&row.first_byte_histogram)
            } else {
                let mut merged = entry.first_byte_histogram.clone();
                merge_approx_histogram_into(
                    &mut merged,
                    &decode_approx_histogram(&row.first_byte_histogram),
                )?;
                merged
            };
            entry.first_response_byte_total_sample_count +=
                row.first_response_byte_total_sample_count;
            entry.first_response_byte_total_sum_ms += row.first_response_byte_total_sum_ms;
            entry.first_response_byte_total_histogram =
                if entry.first_response_byte_total_histogram.is_empty() {
                    decode_approx_histogram(&row.first_response_byte_total_histogram)
                } else {
                    let mut merged = entry.first_response_byte_total_histogram.clone();
                    merge_approx_histogram_into(
                        &mut merged,
                        &decode_approx_histogram(&row.first_response_byte_total_histogram),
                    )?;
                    merged
                };
        }
    }

    let exact_records =
        query_invocation_exact_records(&state.pool, &range_plan, source_scope).await?;
    for record in exact_records {
        let Some(occurred_utc) = parse_to_utc_datetime(&record.occurred_at) else {
            continue;
        };
        let bucket_epoch =
            align_reporting_bucket_epoch(occurred_utc.timestamp(), bucket_seconds, reporting_tz)?;
        if let Some(entry) = aggregates.get_mut(&bucket_epoch) {
            entry.total_count += 1;
            match record.status.as_deref() {
                Some("success") => entry.success_count += 1,
                Some(_) => entry.failure_count += 1,
                None => {}
            }
            entry.record_exact_ttfb_sample(record.status.as_deref(), record.t_upstream_ttfb_ms);
            entry.record_exact_first_response_byte_total_sample(
                record.t_req_read_ms,
                record.t_req_parse_ms,
                record.t_upstream_connect_ms,
                record.t_upstream_ttfb_ms,
            );
            entry.total_tokens += record.total_tokens.unwrap_or_default();
            entry.total_cost += record.cost.unwrap_or_default();
        }
    }

    let relay_deltas = if source_scope == InvocationSourceScope::All
        && let Some(relay) = state.config.crs_stats.as_ref()
        && let Some(effective_range) = effective_range_for_hourly_rollup_plan(&range_plan)?
    {
        query_crs_deltas(
            &state.pool,
            relay,
            effective_range.start.timestamp(),
            effective_range.end.timestamp(),
        )
        .await?
    } else {
        Vec::new()
    };
    for delta in relay_deltas {
        let bucket_epoch =
            align_reporting_bucket_epoch(delta.captured_at_epoch, bucket_seconds, reporting_tz)?;
        if let Some(entry) = aggregates.get_mut(&bucket_epoch) {
            entry.total_count += delta.total_count;
            entry.success_count += delta.success_count;
            entry.failure_count += delta.failure_count;
            entry.total_tokens += delta.total_tokens;
            entry.total_cost += delta.total_cost;
        }
    }

    let mut points = Vec::with_capacity(aggregates.len());
    for (bucket_epoch, agg) in aggregates {
        let bucket_end_epoch =
            next_reporting_bucket_epoch(bucket_epoch, bucket_seconds, reporting_tz)?;
        if bucket_epoch < fill_start_epoch || bucket_end_epoch > fill_end_epoch {
            continue;
        }
        let start = Utc
            .timestamp_opt(bucket_epoch, 0)
            .single()
            .ok_or_else(|| anyhow!("invalid bucket epoch"))?;
        let end = Utc
            .timestamp_opt(bucket_end_epoch, 0)
            .single()
            .ok_or_else(|| anyhow!("invalid bucket epoch"))?;
        points.push(TimeseriesPoint {
            bucket_start: format_utc_iso(start),
            bucket_end: format_utc_iso(end),
            total_count: agg.total_count,
            success_count: agg.success_count,
            failure_count: agg.failure_count,
            total_tokens: agg.total_tokens,
            total_cost: agg.total_cost,
            first_byte_sample_count: agg.first_byte_sample_count,
            first_byte_avg_ms: agg.first_byte_avg_ms(),
            first_byte_p95_ms: agg.first_byte_p95_ms(),
            first_response_byte_total_sample_count: agg.first_response_byte_total_sample_count,
            first_response_byte_total_avg_ms: agg.first_response_byte_total_avg_ms(),
            first_response_byte_total_p95_ms: agg.first_response_byte_total_p95_ms(),
        });
    }

    Ok(Json(TimeseriesResponse {
        range_start: format_utc_iso(range_window.start),
        range_end: {
            let end = Utc
                .timestamp_opt(fill_end_epoch, 0)
                .single()
                .unwrap_or_else(Utc::now);
            format_utc_iso(end)
        },
        bucket_seconds,
        effective_bucket: bucket_selection.effective_bucket,
        available_buckets: bucket_selection.available_buckets,
        bucket_limited_to_daily: bucket_selection.bucket_limited_to_daily,
        points,
    }))
}

pub(crate) fn align_reporting_bucket_epoch(
    epoch: i64,
    bucket_seconds: i64,
    reporting_tz: Tz,
) -> Result<i64> {
    let timestamp = Utc
        .timestamp_opt(epoch, 0)
        .single()
        .ok_or_else(|| anyhow!("invalid bucket epoch"))?;
    let local = timestamp.with_timezone(&reporting_tz);
    let elapsed_seconds = i64::from(local.time().num_seconds_from_midnight());
    let remainder = elapsed_seconds.rem_euclid(bucket_seconds);
    let bucket_start_local = local.naive_local() - ChronoDuration::seconds(remainder);
    Ok(
        local_naive_to_utc_not_after_reference(bucket_start_local, reporting_tz, timestamp)
            .timestamp(),
    )
}

pub(crate) fn next_reporting_bucket_epoch(
    bucket_start_epoch: i64,
    bucket_seconds: i64,
    reporting_tz: Tz,
) -> Result<i64> {
    let bucket_start = Utc
        .timestamp_opt(bucket_start_epoch, 0)
        .single()
        .ok_or_else(|| anyhow!("invalid bucket epoch"))?;
    let next_start = if bucket_seconds == 3_600 {
        bucket_start + ChronoDuration::seconds(bucket_seconds)
    } else {
        let local_start = bucket_start.with_timezone(&reporting_tz).naive_local();
        local_naive_to_utc(
            local_start + ChronoDuration::seconds(bucket_seconds),
            reporting_tz,
        )
    };
    if next_start.timestamp() <= bucket_start_epoch {
        return Err(anyhow!(
            "non-increasing reporting bucket progression for {reporting_tz} at {bucket_start_epoch}"
        ));
    }
    Ok(next_start.timestamp())
}

fn local_naive_to_utc_not_after_reference(
    naive: NaiveDateTime,
    tz: Tz,
    reference_utc: DateTime<Utc>,
) -> DateTime<Utc> {
    match tz.from_local_datetime(&naive) {
        LocalResult::Single(dt) => dt.with_timezone(&Utc),
        LocalResult::Ambiguous(first, second) => {
            let first_utc = first.with_timezone(&Utc);
            let second_utc = second.with_timezone(&Utc);
            [first_utc, second_utc]
                .into_iter()
                .filter(|candidate| *candidate <= reference_utc)
                .max()
                .unwrap_or(first_utc.min(second_utc))
        }
        LocalResult::None => local_naive_to_utc(naive, tz),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FailureScope {
    All,
    Service,
    Client,
    Abort,
}

impl FailureScope {
    pub(crate) fn parse(raw: Option<&str>) -> Result<Self, ApiError> {
        let Some(scope) = raw.map(str::trim).filter(|v| !v.is_empty()) else {
            return Ok(FailureScope::Service);
        };
        match scope.to_ascii_lowercase().as_str() {
            "all" => Ok(FailureScope::All),
            "service" => Ok(FailureScope::Service),
            "client" => Ok(FailureScope::Client),
            "abort" => Ok(FailureScope::Abort),
            _ => Err(ApiError::bad_request(anyhow!(
                "unsupported failure scope: {scope}; expected one of all|service|client|abort"
            ))),
        }
    }
}

pub(crate) fn failure_scope_matches(scope: FailureScope, class: FailureClass) -> bool {
    match scope {
        FailureScope::All => class != FailureClass::None,
        FailureScope::Service => class == FailureClass::ServiceFailure,
        FailureScope::Client => class == FailureClass::ClientFailure,
        FailureScope::Abort => class == FailureClass::ClientAbort,
    }
}

pub(crate) fn extract_failure_kind_prefix(input: &str) -> Option<String> {
    let trimmed = input.trim();
    if !trimmed.starts_with('[') {
        return None;
    }
    let closing = trimmed.find(']')?;
    if closing <= 1 {
        return None;
    }
    Some(trimmed[1..closing].trim().to_string())
}

pub(crate) fn derive_failure_kind(status_norm: &str, err: &str, err_lower: &str) -> Option<String> {
    if err_lower.contains("downstream closed while streaming upstream response") {
        return Some(PROXY_STREAM_TERMINAL_DOWNSTREAM_CLOSED.to_string());
    }
    if err_lower.contains("upstream response stream reported failure") {
        return Some(PROXY_FAILURE_UPSTREAM_RESPONSE_FAILED.to_string());
    }
    if err_lower.contains("upstream stream error") {
        return Some(PROXY_FAILURE_UPSTREAM_STREAM_ERROR.to_string());
    }
    if err_lower.contains("failed to contact upstream") {
        return Some(PROXY_FAILURE_FAILED_CONTACT_UPSTREAM.to_string());
    }
    if err_lower.contains("[upstream_response_failed]")
        || err_lower.contains("upstream response failed")
    {
        return Some(PROXY_FAILURE_UPSTREAM_RESPONSE_FAILED.to_string());
    }
    if err_lower.contains("upstream handshake timed out") {
        return Some(PROXY_FAILURE_UPSTREAM_HANDSHAKE_TIMEOUT.to_string());
    }
    if err_lower.contains("request body read timed out") {
        return Some(PROXY_FAILURE_REQUEST_BODY_READ_TIMEOUT.to_string());
    }
    if err_lower.contains("failed to read request body stream") {
        return Some(PROXY_FAILURE_REQUEST_BODY_STREAM_ERROR_CLIENT_CLOSED.to_string());
    }
    if err_lower.contains("invalid api key format")
        || err_lower.contains("api key format is invalid")
        || err_lower.contains("incorrect api key provided")
    {
        return Some("invalid_api_key".to_string());
    }
    if err_lower.contains("api key not found") {
        return Some("api_key_not_found".to_string());
    }
    if err_lower.contains("please provide an api key") {
        return Some("api_key_missing".to_string());
    }
    if status_norm.starts_with("http_") {
        return Some(status_norm.to_string());
    }
    if !err.is_empty() {
        return Some("untyped_failure".to_string());
    }
    None
}

pub(crate) fn classify_invocation_failure(
    status: Option<&str>,
    error_message: Option<&str>,
) -> FailureClassification {
    let status_norm = status.unwrap_or_default().trim().to_ascii_lowercase();
    let err = error_message.unwrap_or_default().trim();
    let err_lower = err.to_ascii_lowercase();

    if status_norm == "success" && err.is_empty() {
        return FailureClassification {
            failure_kind: None,
            failure_class: FailureClass::None,
            is_actionable: false,
        };
    }
    if (status_norm == "running" || status_norm == "pending") && err.is_empty() {
        return FailureClassification {
            failure_kind: None,
            failure_class: FailureClass::None,
            is_actionable: false,
        };
    }

    let failure_kind = extract_failure_kind_prefix(err)
        .or_else(|| derive_failure_kind(&status_norm, err, &err_lower));

    let failure_kind_lower = failure_kind
        .as_deref()
        .unwrap_or_default()
        .to_ascii_lowercase();
    let is_http_429 =
        status_norm == "http_429" || failure_kind_lower == FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429;
    let is_http_4xx = (status_norm.starts_with("http_4")
        || status_norm == "http_401"
        || status_norm == "http_403")
        && !is_http_429;
    let is_http_5xx = status_norm.starts_with("http_5");

    let failure_class = if failure_kind_lower == PROXY_STREAM_TERMINAL_DOWNSTREAM_CLOSED
        || err_lower.contains("downstream closed while streaming upstream response")
    {
        FailureClass::ClientAbort
    } else if is_http_429 {
        // Upstream rate limiting is retryable and should be surfaced as service-impacting.
        FailureClass::ServiceFailure
    } else if failure_kind_lower == PROXY_FAILURE_REQUEST_BODY_STREAM_ERROR_CLIENT_CLOSED
        || err_lower.contains("invalid api key format")
        || err_lower.contains("api key format is invalid")
        || err_lower.contains("incorrect api key provided")
        || err_lower.contains("api key not found")
        || err_lower.contains("please provide an api key")
        || is_http_4xx
    {
        FailureClass::ClientFailure
    } else if failure_kind_lower == PROXY_FAILURE_FAILED_CONTACT_UPSTREAM
        || failure_kind_lower == PROXY_FAILURE_UPSTREAM_RESPONSE_FAILED
        || failure_kind_lower == PROXY_FAILURE_UPSTREAM_STREAM_ERROR
        || failure_kind_lower == PROXY_FAILURE_REQUEST_BODY_READ_TIMEOUT
        || failure_kind_lower == PROXY_FAILURE_UPSTREAM_HANDSHAKE_TIMEOUT
        || err_lower.contains("upstream response stream reported failure")
        || err_lower.contains("failed to contact upstream")
        || err_lower.contains("upstream stream error")
        || err_lower.contains("request body read timed out")
        || err_lower.contains("upstream handshake timed out")
        || is_http_5xx
    {
        FailureClass::ServiceFailure
    } else if status_norm == "success" {
        FailureClass::None
    } else {
        // Conservative fallback: unknown non-success records are treated as service-impacting.
        FailureClass::ServiceFailure
    };

    FailureClassification {
        failure_kind: if failure_class == FailureClass::None {
            None
        } else {
            failure_kind
        },
        failure_class,
        is_actionable: failure_class == FailureClass::ServiceFailure,
    }
}

pub(crate) fn resolve_failure_classification(
    status: Option<&str>,
    error_message: Option<&str>,
    failure_kind: Option<&str>,
    failure_class: Option<&str>,
    is_actionable: Option<i64>,
) -> FailureClassification {
    let derived = classify_invocation_failure(status, error_message);
    let stored_class = failure_class.and_then(FailureClass::from_db_str);
    let resolved_class = match stored_class {
        // Legacy rows can carry migration defaults (`none`/`0`) for non-success records.
        Some(FailureClass::None) if derived.failure_class != FailureClass::None => {
            derived.failure_class
        }
        Some(value) => value,
        None => derived.failure_class,
    };
    let resolved_kind = failure_kind
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(ToOwned::to_owned)
        .or(derived.failure_kind);
    let expected_actionable = resolved_class == FailureClass::ServiceFailure;
    let resolved_actionable = is_actionable
        .map(|value| value != 0)
        .filter(|value| *value == expected_actionable)
        .unwrap_or(expected_actionable);

    FailureClassification {
        failure_kind: resolved_kind,
        failure_class: resolved_class,
        is_actionable: resolved_actionable,
    }
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ErrorQuery {
    pub(crate) range: String,
    pub(crate) top: Option<i64>,
    pub(crate) scope: Option<String>,
    pub(crate) time_zone: Option<String>,
}

#[derive(serde::Serialize)]
pub(crate) struct ErrorDistributionItem {
    pub(crate) reason: String,
    pub(crate) count: i64,
}

#[derive(serde::Serialize)]
pub(crate) struct ErrorDistributionResponse {
    pub(crate) range_start: String,
    pub(crate) range_end: String,
    pub(crate) items: Vec<ErrorDistributionItem>,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct OtherErrorsQuery {
    pub(crate) range: String,
    pub(crate) page: Option<i64>,
    pub(crate) limit: Option<i64>,
    pub(crate) scope: Option<String>,
    pub(crate) time_zone: Option<String>,
}

#[derive(serde::Serialize)]
pub(crate) struct OtherErrorItem {
    pub(crate) id: i64,
    pub(crate) occurred_at: String,
    pub(crate) error_message: Option<String>,
}

#[derive(serde::Serialize)]
pub(crate) struct OtherErrorsResponse {
    pub(crate) total: i64,
    pub(crate) page: i64,
    pub(crate) limit: i64,
    pub(crate) items: Vec<OtherErrorItem>,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct FailureSummaryQuery {
    pub(crate) range: String,
    pub(crate) time_zone: Option<String>,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct FailureSummaryResponse {
    pub(crate) range_start: String,
    pub(crate) range_end: String,
    pub(crate) total_failures: i64,
    pub(crate) service_failure_count: i64,
    pub(crate) client_failure_count: i64,
    pub(crate) client_abort_count: i64,
    pub(crate) actionable_failure_count: i64,
    pub(crate) actionable_failure_rate: f64,
}

pub(crate) async fn fetch_error_distribution(
    State(state): State<Arc<AppState>>,
    Query(params): Query<ErrorQuery>,
) -> Result<Json<ErrorDistributionResponse>, ApiError> {
    ensure_hourly_rollups_caught_up(state.as_ref()).await?;
    let reporting_tz = parse_reporting_tz(params.time_zone.as_deref())?;
    let range_window = resolve_range_window(&params.range, reporting_tz)?;
    let start_dt = range_window.start;
    let display_end = range_window.display_end;
    let scope = FailureScope::parse(params.scope.as_deref())?;
    let source_scope = resolve_default_source_scope(&state.pool).await?;
    if start_dt < shanghai_retention_cutoff(state.config.invocation_max_days) {
        let mut counts: HashMap<String, i64> = HashMap::new();
        let range_plan = build_hourly_rollup_exact_range_plan(
            start_dt,
            display_end,
            shanghai_retention_cutoff(state.config.invocation_max_days),
        )?;
        if let Some((range_start_epoch, range_end_epoch)) = range_plan.full_hour_range {
            let rows = query_invocation_failure_hourly_rollup_range(
                &state.pool,
                range_start_epoch,
                range_end_epoch,
                source_scope,
            )
            .await?;
            for row in rows {
                let Some(class) = FailureClass::from_db_str(&row.failure_class) else {
                    continue;
                };
                if !failure_scope_matches(scope, class) {
                    continue;
                }
                *counts.entry(row.error_category).or_default() += row.failure_count;
            }
        }
        let exact_records =
            query_invocation_exact_records(&state.pool, &range_plan, source_scope).await?;
        for record in exact_records {
            let classification = resolve_failure_classification(
                record.status.as_deref(),
                record.error_message.as_deref(),
                record.failure_kind.as_deref(),
                record.failure_class.as_deref(),
                record.is_actionable,
            );
            if !failure_scope_matches(scope, classification.failure_class) {
                continue;
            }
            let raw = record.error_message.unwrap_or_default();
            let key = categorize_error(&raw);
            *counts.entry(key).or_default() += 1;
        }
        let mut items: Vec<ErrorDistributionItem> = counts
            .into_iter()
            .map(|(reason, count)| ErrorDistributionItem { reason, count })
            .collect();
        items.sort_by(|a, b| b.count.cmp(&a.count));
        if let Some(top) = params.top {
            let limited = top.clamp(1, 50) as usize;
            if items.len() > limited {
                items.truncate(limited);
            }
        }
        return Ok(Json(ErrorDistributionResponse {
            range_start: format_utc_iso(start_dt),
            range_end: format_utc_iso(display_end),
            items,
        }));
    }

    #[derive(sqlx::FromRow)]
    struct RawErr {
        status: Option<String>,
        error_message: Option<String>,
        failure_kind: Option<String>,
        failure_class: Option<String>,
        is_actionable: Option<i64>,
    }

    let mut query = QueryBuilder::new(
        "SELECT status, error_message, failure_kind, failure_class, is_actionable FROM codex_invocations WHERE occurred_at >= ",
    );
    query.push_bind(db_occurred_at_lower_bound(start_dt));
    if source_scope == InvocationSourceScope::ProxyOnly {
        query.push(" AND source = ").push_bind(SOURCE_PROXY);
    }
    query.push(" AND (status IS NULL OR status != 'success')");
    let rows: Vec<RawErr> = query.build_query_as().fetch_all(&state.pool).await?;

    let mut counts: std::collections::HashMap<String, i64> = std::collections::HashMap::new();
    for r in rows {
        let classification = resolve_failure_classification(
            r.status.as_deref(),
            r.error_message.as_deref(),
            r.failure_kind.as_deref(),
            r.failure_class.as_deref(),
            r.is_actionable,
        );
        if !failure_scope_matches(scope, classification.failure_class) {
            continue;
        }
        let raw = r.error_message.unwrap_or_default();
        let key = categorize_error(&raw);
        *counts.entry(key).or_insert(0) += 1;
    }

    let mut items: Vec<ErrorDistributionItem> = counts
        .into_iter()
        .map(|(reason, count)| ErrorDistributionItem { reason, count })
        .collect();
    items.sort_by(|a, b| b.count.cmp(&a.count));
    if let Some(top) = params.top {
        let limited = top.clamp(1, 50) as usize;
        if items.len() > limited {
            items.truncate(limited);
        }
    }

    Ok(Json(ErrorDistributionResponse {
        range_start: format_utc_iso(start_dt),
        range_end: format_utc_iso(display_end),
        items,
    }))
}

// Classify error message by rules:
// - If contains HTTP code >= 501, group as "HTTP <code>"
// - If 4xx: try to extract concrete type (json error.type or regex phrases); otherwise "HTTP <code>"
// - Otherwise: normalize message and if still not matched, return "Other"
pub(crate) fn categorize_error(input: &str) -> String {
    let s = input.trim();
    if s.is_empty() {
        return "Other".to_string();
    }

    if let Some(code) = extract_http_code(s) {
        if code >= 501 {
            return format!("HTTP {}", code);
        }
        if (400..500).contains(&code) {
            if let Some(t) = extract_json_error_type(s) {
                return t.to_string();
            }
            if RE_USAGE_NOT_INCLUDED.is_match(s) {
                return "usage_not_included".to_string();
            }
            if RE_USAGE_LIMIT_REACHED.is_match(s) {
                return "usage_limit_reached".to_string();
            }
            if code == 429 {
                if RE_TOO_MANY_REQUESTS.is_match(s) {
                    return "too_many_requests".to_string();
                }
                return "http_429".to_string();
            }
            if code == 401 {
                return "unauthorized".to_string();
            }
            if code == 403 {
                return "forbidden".to_string();
            }
            if code == 404 {
                return "not_found".to_string();
            }
            return format!("HTTP {}", code);
        }
    }

    // Fallback to normalized text; if empty -> Other
    let norm = normalize_error_reason(s);
    if norm == "Unknown" || norm.is_empty() {
        "Other".to_string()
    } else {
        norm
    }
}

pub(crate) fn normalize_error_reason(input: &str) -> String {
    let s = input.trim();
    if s.is_empty() {
        return "Unknown".to_string();
    }
    // Extract stable info from JSON payloads if present
    if s.starts_with('{')
        && let Ok(v) = serde_json::from_str::<serde_json::Value>(s)
        && let Some(err) = v.get("error")
        && let Some(ty) = err.get("type").and_then(|x| x.as_str())
    {
        return format!("json error: {ty}");
    }

    let mut out = s.to_lowercase();

    static RE_HTTP: Lazy<Regex> =
        Lazy::new(|| Regex::new(r"(?i)\bhttp\s*(\d{3})\b").expect("valid regex"));
    let status = RE_HTTP
        .captures(&out)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().to_string());

    static RE_ISO_DT: Lazy<Regex> = Lazy::new(|| {
        Regex::new(r"\b\d{4}-\d{2}-\d{2}[ t]\d{2}:\d{2}:\d{2}(?:\.\d+)?z?\b").expect("valid regex")
    });
    out = RE_ISO_DT.replace_all(&out, "").into_owned();

    static RE_UUID: Lazy<Regex> = Lazy::new(|| {
        Regex::new(r"\b[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}\b")
            .expect("valid regex")
    });
    out = RE_UUID.replace_all(&out, "").into_owned();

    static RE_LONG_ID: Lazy<Regex> =
        Lazy::new(|| Regex::new(r"\b[a-z0-9_\-]{10,}\b").expect("valid regex"));
    out = RE_LONG_ID.replace_all(&out, "").into_owned();

    static RE_URL: Lazy<Regex> =
        Lazy::new(|| Regex::new(r#"https?://[^\s'\"]+"#).expect("valid regex"));
    out = RE_URL
        .replace_all(&out, |caps: &regex::Captures| {
            let url = &caps[0];
            if let Ok(u) = reqwest::Url::parse(url) {
                format!(
                    "{}://{}{}",
                    u.scheme(),
                    u.host_str().unwrap_or(""),
                    u.path()
                )
            } else {
                String::new()
            }
        })
        .into_owned();

    static RE_BIG_NUM: Lazy<Regex> = Lazy::new(|| Regex::new(r"\b\d{4,}\b").expect("valid regex"));
    out = RE_BIG_NUM.replace_all(&out, "").into_owned();

    out = out.replace("request failed:", "request failed");
    out = out.replace("exception recovered:", "exception");

    static RE_WS: Lazy<Regex> = Lazy::new(|| Regex::new(r"\s+").expect("valid regex"));
    out = RE_WS.replace_all(&out, " ").trim().to_string();

    if let Some(code) = status.as_ref().filter(|c| !out.contains(&c[..])) {
        out = format!("http {code}: {out}");
    }

    if out.is_empty() {
        "Unknown".to_string()
    } else {
        out.chars().take(160).collect()
    }
}

pub(crate) fn extract_http_code(s: &str) -> Option<u16> {
    static RE: Lazy<Regex> =
        Lazy::new(|| Regex::new(r"(?i)\bhttp\s*:?\s*(\d{3})\b").expect("valid regex"));
    RE.captures(s)
        .and_then(|c| c.get(1))
        .and_then(|m| m.as_str().parse::<u16>().ok())
}

pub(crate) fn extract_json_error_type(s: &str) -> Option<String> {
    if !s.trim_start().starts_with('{') {
        return None;
    }
    let v: serde_json::Value = serde_json::from_str(s).ok()?;
    let ty = v
        .get("error")
        .and_then(|e| e.get("type"))
        .and_then(|t| t.as_str())?;
    Some(ty.to_string())
}

static RE_USAGE_NOT_INCLUDED: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?i)usage[_\s-]*not[_\s-]*included").expect("valid regex"));
static RE_USAGE_LIMIT_REACHED: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?i)usage[_\s-]*limit[_\s-]*reached").expect("valid regex"));
static RE_TOO_MANY_REQUESTS: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?i)too\s+many\s+requests").expect("valid regex"));

pub(crate) async fn fetch_other_errors(
    State(state): State<Arc<AppState>>,
    Query(params): Query<OtherErrorsQuery>,
) -> Result<Json<OtherErrorsResponse>, ApiError> {
    let reporting_tz = parse_reporting_tz(params.time_zone.as_deref())?;
    let range_window = resolve_range_window(&params.range, reporting_tz)?;
    let start_dt = range_window.start;
    let scope = FailureScope::parse(params.scope.as_deref())?;
    let source_scope = resolve_default_source_scope(&state.pool).await?;

    #[derive(sqlx::FromRow)]
    struct RowItem {
        id: i64,
        occurred_at: String,
        status: Option<String>,
        error_message: Option<String>,
        failure_kind: Option<String>,
        failure_class: Option<String>,
        is_actionable: Option<i64>,
    }
    let mut query = QueryBuilder::new(
        "SELECT id, occurred_at, status, error_message, failure_kind, failure_class, is_actionable FROM codex_invocations WHERE occurred_at >= ",
    );
    query.push_bind(db_occurred_at_lower_bound(start_dt));
    if source_scope == InvocationSourceScope::ProxyOnly {
        query.push(" AND source = ").push_bind(SOURCE_PROXY);
    }
    query.push(" AND (status IS NULL OR status != 'success') ORDER BY occurred_at DESC");
    let rows: Vec<RowItem> = query.build_query_as().fetch_all(&state.pool).await?;

    let mut others: Vec<RowItem> = Vec::new();
    for r in rows.into_iter() {
        let classification = resolve_failure_classification(
            r.status.as_deref(),
            r.error_message.as_deref(),
            r.failure_kind.as_deref(),
            r.failure_class.as_deref(),
            r.is_actionable,
        );
        if !failure_scope_matches(scope, classification.failure_class) {
            continue;
        }
        let msg = r.error_message.clone().unwrap_or_default();
        let cat = categorize_error(&msg);
        if cat == "Other" {
            others.push(r);
        }
    }

    let total = others.len() as i64;
    let limit = params.limit.unwrap_or(50).clamp(1, 200);
    let page = params.page.unwrap_or(1).max(1);
    let start = ((page - 1) * limit) as usize;
    let end = (start + limit as usize).min(others.len());
    let slice = if start < end {
        &others[start..end]
    } else {
        &[]
    };

    let items = slice
        .iter()
        .map(|r| OtherErrorItem {
            id: r.id,
            occurred_at: r.occurred_at.clone(),
            error_message: r.error_message.clone(),
        })
        .collect();

    Ok(Json(OtherErrorsResponse {
        total,
        page,
        limit,
        items,
    }))
}

pub(crate) async fn fetch_failure_summary(
    State(state): State<Arc<AppState>>,
    Query(params): Query<FailureSummaryQuery>,
) -> Result<Json<FailureSummaryResponse>, ApiError> {
    ensure_hourly_rollups_caught_up(state.as_ref()).await?;
    let reporting_tz = parse_reporting_tz(params.time_zone.as_deref())?;
    let range_window = resolve_range_window(&params.range, reporting_tz)?;
    let start_dt = range_window.start;
    let display_end = range_window.display_end;
    let source_scope = resolve_default_source_scope(&state.pool).await?;
    if start_dt < shanghai_retention_cutoff(state.config.invocation_max_days) {
        let mut total_failures = 0_i64;
        let mut service_failure_count = 0_i64;
        let mut client_failure_count = 0_i64;
        let mut client_abort_count = 0_i64;
        let mut actionable_failure_count = 0_i64;
        let range_plan = build_hourly_rollup_exact_range_plan(
            start_dt,
            display_end,
            shanghai_retention_cutoff(state.config.invocation_max_days),
        )?;
        if let Some((range_start_epoch, range_end_epoch)) = range_plan.full_hour_range {
            let rows = query_invocation_failure_hourly_rollup_range(
                &state.pool,
                range_start_epoch,
                range_end_epoch,
                source_scope,
            )
            .await?;
            for row in rows {
                let Some(class) = FailureClass::from_db_str(&row.failure_class) else {
                    continue;
                };
                total_failures += row.failure_count;
                match class {
                    FailureClass::ServiceFailure => service_failure_count += row.failure_count,
                    FailureClass::ClientFailure => client_failure_count += row.failure_count,
                    FailureClass::ClientAbort => client_abort_count += row.failure_count,
                    FailureClass::None => {}
                }
                if row.is_actionable != 0 {
                    actionable_failure_count += row.failure_count;
                }
            }
        }
        let exact_records =
            query_invocation_exact_records(&state.pool, &range_plan, source_scope).await?;
        for record in exact_records {
            let classification = resolve_failure_classification(
                record.status.as_deref(),
                record.error_message.as_deref(),
                record.failure_kind.as_deref(),
                record.failure_class.as_deref(),
                record.is_actionable,
            );
            if classification.failure_class == FailureClass::None {
                continue;
            }
            total_failures += 1;
            match classification.failure_class {
                FailureClass::ServiceFailure => service_failure_count += 1,
                FailureClass::ClientFailure => client_failure_count += 1,
                FailureClass::ClientAbort => client_abort_count += 1,
                FailureClass::None => {}
            }
            if classification.is_actionable {
                actionable_failure_count += 1;
            }
        }
        let actionable_failure_rate = if total_failures > 0 {
            actionable_failure_count as f64 / total_failures as f64
        } else {
            0.0
        };
        return Ok(Json(FailureSummaryResponse {
            range_start: format_utc_iso(start_dt),
            range_end: format_utc_iso(display_end),
            total_failures,
            service_failure_count,
            client_failure_count,
            client_abort_count,
            actionable_failure_count,
            actionable_failure_rate,
        }));
    }

    #[derive(sqlx::FromRow)]
    struct Row {
        status: Option<String>,
        error_message: Option<String>,
        failure_kind: Option<String>,
        failure_class: Option<String>,
        is_actionable: Option<i64>,
    }

    let mut query = QueryBuilder::new(
        "SELECT status, error_message, failure_kind, failure_class, is_actionable FROM codex_invocations WHERE occurred_at >= ",
    );
    query.push_bind(db_occurred_at_lower_bound(start_dt));
    if source_scope == InvocationSourceScope::ProxyOnly {
        query.push(" AND source = ").push_bind(SOURCE_PROXY);
    }
    query.push(" AND (status IS NULL OR status != 'success')");

    let rows: Vec<Row> = query.build_query_as().fetch_all(&state.pool).await?;
    let total_failures = rows.len() as i64;

    let mut service_failure_count = 0_i64;
    let mut client_failure_count = 0_i64;
    let mut client_abort_count = 0_i64;
    let mut actionable_failure_count = 0_i64;

    for row in rows {
        let classification = resolve_failure_classification(
            row.status.as_deref(),
            row.error_message.as_deref(),
            row.failure_kind.as_deref(),
            row.failure_class.as_deref(),
            row.is_actionable,
        );
        match classification.failure_class {
            FailureClass::ServiceFailure => service_failure_count += 1,
            FailureClass::ClientFailure => client_failure_count += 1,
            FailureClass::ClientAbort => client_abort_count += 1,
            FailureClass::None => {}
        }
        if classification.is_actionable {
            actionable_failure_count += 1;
        }
    }

    let actionable_failure_rate = if total_failures > 0 {
        actionable_failure_count as f64 / total_failures as f64
    } else {
        0.0
    };

    Ok(Json(FailureSummaryResponse {
        range_start: format_utc_iso(start_dt),
        range_end: format_utc_iso(display_end),
        total_failures,
        service_failure_count,
        client_failure_count,
        client_abort_count,
        actionable_failure_count,
        actionable_failure_rate,
    }))
}

pub(crate) async fn fetch_perf_stats(
    State(state): State<Arc<AppState>>,
    Query(params): Query<PerfQuery>,
) -> Result<Json<PerfStatsResponse>, ApiError> {
    ensure_hourly_rollups_caught_up(state.as_ref()).await?;
    #[derive(sqlx::FromRow)]
    struct PerfTimingRow {
        t_total_ms: Option<f64>,
        t_req_read_ms: Option<f64>,
        t_req_parse_ms: Option<f64>,
        t_upstream_connect_ms: Option<f64>,
        t_upstream_ttfb_ms: Option<f64>,
        t_upstream_stream_ms: Option<f64>,
        t_resp_parse_ms: Option<f64>,
        t_persist_ms: Option<f64>,
    }

    let reporting_tz = parse_reporting_tz(params.time_zone.as_deref())?;
    let range_window = resolve_range_window(&params.range, reporting_tz)?;
    if range_window.start < shanghai_retention_cutoff(state.config.invocation_max_days) {
        let range_plan = build_hourly_rollup_exact_range_plan(
            range_window.start,
            range_window.display_end,
            shanghai_retention_cutoff(state.config.invocation_max_days),
        )?;
        let mut by_stage: BTreeMap<String, (i64, f64, f64, ApproxHistogramCounts)> =
            BTreeMap::new();
        if let Some((range_start_epoch, range_end_epoch)) = range_plan.full_hour_range {
            let rows = query_proxy_perf_stage_hourly_rollup_range(
                &state.pool,
                range_start_epoch,
                range_end_epoch,
            )
            .await?;
            for row in rows {
                let entry = by_stage
                    .entry(row.stage)
                    .or_insert_with(|| (0, 0.0, 0.0, empty_approx_histogram()));
                entry.0 += row.sample_count;
                entry.1 += row.sum_ms;
                entry.2 = entry.2.max(row.max_ms);
                merge_approx_histogram_into(
                    &mut entry.3,
                    &decode_approx_histogram(&row.histogram),
                )?;
            }
        }
        let exact_records = query_invocation_exact_records(
            &state.pool,
            &range_plan,
            InvocationSourceScope::ProxyOnly,
        )
        .await?;
        for record in exact_records {
            record_perf_stage_sample(&mut by_stage, "total", record.t_total_ms);
            record_perf_stage_sample(&mut by_stage, "requestRead", record.t_req_read_ms);
            record_perf_stage_sample(&mut by_stage, "requestParse", record.t_req_parse_ms);
            record_perf_stage_sample(
                &mut by_stage,
                "upstreamConnect",
                record.t_upstream_connect_ms,
            );
            record_perf_stage_sample(
                &mut by_stage,
                "upstreamFirstByte",
                record.t_upstream_ttfb_ms,
            );
            record_perf_stage_sample(&mut by_stage, "upstreamStream", record.t_upstream_stream_ms);
            record_perf_stage_sample(&mut by_stage, "responseParse", record.t_resp_parse_ms);
            record_perf_stage_sample(&mut by_stage, "persistence", record.t_persist_ms);
        }
        let mut stages = Vec::new();
        for (stage, (count, sum_ms, max_ms, histogram)) in by_stage {
            if count <= 0 {
                continue;
            }
            stages.push(PerfStageStats {
                stage,
                count,
                avg_ms: sum_ms / count as f64,
                p50_ms: approx_histogram_percentile_ms(&histogram, 0.50).unwrap_or(max_ms),
                p90_ms: approx_histogram_percentile_ms(&histogram, 0.90).unwrap_or(max_ms),
                p99_ms: approx_histogram_percentile_ms(&histogram, 0.99).unwrap_or(max_ms),
                max_ms,
            });
        }
        return Ok(Json(PerfStatsResponse {
            range_start: format_utc_iso(range_window.start),
            range_end: format_utc_iso(range_window.display_end),
            source: SOURCE_PROXY.to_string(),
            stages,
        }));
    }
    let mut query = QueryBuilder::new(
        "SELECT \
            t_total_ms, t_req_read_ms, t_req_parse_ms, \
            t_upstream_connect_ms, t_upstream_ttfb_ms, t_upstream_stream_ms, \
            t_resp_parse_ms, t_persist_ms \
         FROM codex_invocations \
         WHERE source = ",
    );
    query
        .push_bind(SOURCE_PROXY)
        .push(" AND occurred_at >= ")
        .push_bind(db_occurred_at_lower_bound(range_window.start))
        .push(" AND occurred_at <= ")
        .push_bind(db_occurred_at_lower_bound(range_window.display_end));
    let rows: Vec<PerfTimingRow> = query.build_query_as().fetch_all(&state.pool).await?;

    let stage_series: Vec<(&str, Vec<f64>)> = vec![
        (
            "total",
            rows.iter()
                .filter_map(|row| row.t_total_ms)
                .collect::<Vec<_>>(),
        ),
        (
            "requestRead",
            rows.iter()
                .filter_map(|row| row.t_req_read_ms)
                .collect::<Vec<_>>(),
        ),
        (
            "requestParse",
            rows.iter()
                .filter_map(|row| row.t_req_parse_ms)
                .collect::<Vec<_>>(),
        ),
        (
            "upstreamConnect",
            rows.iter()
                .filter_map(|row| row.t_upstream_connect_ms)
                .collect::<Vec<_>>(),
        ),
        (
            "upstreamFirstByte",
            rows.iter()
                .filter_map(|row| row.t_upstream_ttfb_ms)
                .collect::<Vec<_>>(),
        ),
        (
            "upstreamStream",
            rows.iter()
                .filter_map(|row| row.t_upstream_stream_ms)
                .collect::<Vec<_>>(),
        ),
        (
            "responseParse",
            rows.iter()
                .filter_map(|row| row.t_resp_parse_ms)
                .collect::<Vec<_>>(),
        ),
        (
            "persistence",
            rows.iter()
                .filter_map(|row| row.t_persist_ms)
                .collect::<Vec<_>>(),
        ),
    ];

    let mut stages = Vec::new();
    for (stage, mut values) in stage_series {
        if values.is_empty() {
            continue;
        }
        values.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let count = values.len() as i64;
        let sum = values.iter().copied().sum::<f64>();
        let max_ms = values.last().copied().unwrap_or(0.0);
        stages.push(PerfStageStats {
            stage: stage.to_string(),
            count,
            avg_ms: sum / count as f64,
            p50_ms: percentile_sorted_f64(&values, 0.50),
            p90_ms: percentile_sorted_f64(&values, 0.90),
            p99_ms: percentile_sorted_f64(&values, 0.99),
            max_ms,
        });
    }

    Ok(Json(PerfStatsResponse {
        range_start: format_utc_iso(range_window.start),
        range_end: format_utc_iso(range_window.display_end),
        source: SOURCE_PROXY.to_string(),
        stages,
    }))
}

pub(crate) async fn latest_quota_snapshot(
    State(state): State<Arc<AppState>>,
) -> Result<Json<QuotaSnapshotResponse>, ApiError> {
    let snapshot = QuotaSnapshotResponse::fetch_latest(&state.pool)
        .await?
        .unwrap_or_else(QuotaSnapshotResponse::degraded_default);
    Ok(Json(snapshot))
}

pub(crate) async fn broadcast_summary_if_changed(
    broadcaster: &broadcast::Sender<BroadcastPayload>,
    cache: &Mutex<BroadcastStateCache>,
    window: &str,
    summary: StatsResponse,
) -> Result<bool, broadcast::error::SendError<BroadcastPayload>> {
    let mut cache = cache.lock().await;
    if cache
        .summaries
        .get(window)
        .is_some_and(|current| current == &summary)
    {
        return Ok(false);
    }

    broadcaster.send(BroadcastPayload::Summary {
        window: window.to_string(),
        summary: summary.clone(),
    })?;
    cache.summaries.insert(window.to_string(), summary);
    Ok(true)
}

pub(crate) async fn broadcast_quota_if_changed(
    broadcaster: &broadcast::Sender<BroadcastPayload>,
    cache: &Mutex<BroadcastStateCache>,
    snapshot: QuotaSnapshotResponse,
) -> Result<bool, broadcast::error::SendError<BroadcastPayload>> {
    let mut cache = cache.lock().await;
    if cache
        .quota
        .as_ref()
        .is_some_and(|current| current == &snapshot)
    {
        return Ok(false);
    }

    broadcaster.send(BroadcastPayload::Quota {
        snapshot: Box::new(snapshot.clone()),
    })?;
    cache.quota = Some(snapshot);
    Ok(true)
}

pub(crate) async fn sse_stream(
    State(state): State<Arc<AppState>>,
) -> Sse<impl futures_util::Stream<Item = Result<Event, Infallible>>> {
    let rx = state.broadcaster.subscribe();
    let broadcast = BroadcastStream::new(rx).filter_map(|res| async {
        match res {
            Ok(payload) => match Event::default().json_data(&payload) {
                Ok(event) => Some(Ok(event)),
                Err(err) => {
                    warn!(?err, "failed to serialize sse payload");
                    None
                }
            },
            Err(err) => {
                warn!(?err, "sse broadcast stream lagging");
                None
            }
        }
    });
    // Seed a version event on connect so clients know the current server version immediately
    let initial = {
        let (backend, _frontend) = detect_versions(state.config.static_dir.as_deref());
        let payload = BroadcastPayload::Version { version: backend };
        let ev = Event::default().json_data(&payload);
        match ev {
            Ok(event) => stream::iter(vec![Ok(event)]),
            Err(_) => stream::iter(Vec::<Result<Event, Infallible>>::new()),
        }
    };

    let merged = initial.chain(broadcast);
    Sse::new(merged).keep_alive(KeepAlive::new().interval(Duration::from_secs(15)))
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct VersionResponse {
    pub(crate) backend: String,
    pub(crate) frontend: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_invocation_filters_normalizes_request_id() {
        let params = ListQuery {
            request_id: Some(" invoke-123 ".to_string()),
            ..Default::default()
        };

        let filters = build_invocation_filters(&params).expect("filters should build");

        assert_eq!(filters.request_id.as_deref(), Some("invoke-123"));
    }

    #[test]
    fn build_invocation_filters_ignores_legacy_proxy_param() {
        let params = ListQuery {
            proxy: Some(" tokyo-edge-01 ".to_string()),
            ..Default::default()
        };

        let filters = build_invocation_filters(&params).expect("filters should build");

        assert_eq!(params.proxy.as_deref(), Some(" tokyo-edge-01 "));
        assert_eq!(filters.endpoint, None);
        assert_eq!(filters.request_id, None);
    }

    #[test]
    fn response_body_falls_back_to_preview_when_complete() {
        let row = InvocationResponseBodyRow {
            id: 1,
            raw_response: "{\"error\":\"preview\"}".to_string(),
            response_raw_path: None,
            response_raw_size: Some(19),
            response_raw_truncated: Some(0),
            response_raw_truncated_reason: None,
            detail_level: "full".to_string(),
            detail_prune_reason: None,
            response_content_encoding: None,
            failure_class: Some("service_failure".to_string()),
        };

        let (body, from_full_body) =
            resolve_response_body_text_from_row(&row, None).expect("preview should be reusable");

        assert_eq!(body, "{\"error\":\"preview\"}");
        assert!(!from_full_body);
    }

    #[test]
    fn response_body_reports_detail_pruned_when_structured_only_preview_missing() {
        let row = InvocationResponseBodyRow {
            id: 2,
            raw_response: String::new(),
            response_raw_path: None,
            response_raw_size: None,
            response_raw_truncated: Some(0),
            response_raw_truncated_reason: None,
            detail_level: DETAIL_LEVEL_STRUCTURED_ONLY.to_string(),
            detail_prune_reason: Some("success_over_30d".to_string()),
            response_content_encoding: None,
            failure_class: Some("client_failure".to_string()),
        };

        let err = resolve_response_body_text_from_row(&row, None)
            .expect_err("structured-only rows should not expose a full body");

        assert_eq!(err, "detail_pruned");
    }
}

pub(crate) async fn get_settings(
    State(state): State<Arc<AppState>>,
) -> Result<Json<SettingsResponse>, ApiError> {
    let pricing = state.pricing_catalog.read().await.clone();
    let forward_proxy = build_forward_proxy_settings_response(state.as_ref()).await?;
    Ok(Json(SettingsResponse {
        forward_proxy,
        pricing: PricingSettingsResponse::from_catalog(&pricing),
    }))
}

pub(crate) async fn removed_proxy_model_settings_endpoint() -> (StatusCode, &'static str) {
    (
        StatusCode::NOT_FOUND,
        "endpoint removed; legacy reverse proxy settings are no longer supported",
    )
}

pub(crate) async fn put_proxy_settings(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(payload): Json<ProxyModelSettingsUpdateRequest>,
) -> Result<Json<ProxyModelSettingsResponse>, (StatusCode, String)> {
    if !is_same_origin_settings_write(&headers) {
        return Err((
            StatusCode::FORBIDDEN,
            "cross-origin settings writes are forbidden".to_string(),
        ));
    }

    let _update_guard = state.proxy_model_settings_update_lock.lock().await;
    let current = state.proxy_model_settings.read().await.clone();
    let next = ProxyModelSettings {
        hijack_enabled: payload.hijack_enabled,
        merge_upstream_enabled: payload.merge_upstream_enabled,
        upstream_429_max_retries: payload
            .upstream_429_max_retries
            .unwrap_or(current.upstream_429_max_retries),
        enabled_preset_models: payload.enabled_models,
    }
    .normalized();
    save_proxy_model_settings(&state.pool, next.clone())
        .await
        .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?;
    let mut guard = state.proxy_model_settings.write().await;
    *guard = next.clone();
    Ok(Json(next.into()))
}

pub(crate) async fn put_pricing_settings(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(payload): Json<PricingSettingsUpdateRequest>,
) -> Result<Json<PricingSettingsResponse>, (StatusCode, String)> {
    if !is_same_origin_settings_write(&headers) {
        return Err((
            StatusCode::FORBIDDEN,
            "cross-origin settings writes are forbidden".to_string(),
        ));
    }

    let next = payload.normalized()?;
    let _update_guard = state.pricing_settings_update_lock.lock().await;
    save_pricing_catalog(&state.pool, &next)
        .await
        .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?;

    let mut guard = state.pricing_catalog.write().await;
    *guard = next.clone();
    Ok(Json(PricingSettingsResponse::from_catalog(&next)))
}

pub(crate) async fn get_versions(
    State(state): State<Arc<AppState>>,
) -> Result<Json<VersionResponse>, ApiError> {
    let (backend, frontend) = detect_versions(state.config.static_dir.as_deref());
    Ok(Json(VersionResponse { backend, frontend }))
}

#[derive(Debug, Default)]
pub(crate) struct BroadcastStateCache {
    pub(crate) summaries: HashMap<String, StatsResponse>,
    pub(crate) quota: Option<QuotaSnapshotResponse>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub(crate) enum BroadcastPayload {
    Version {
        version: String,
    },
    Records {
        records: Vec<ApiInvocation>,
    },
    #[serde(rename = "pool_attempts")]
    PoolAttempts {
        invoke_id: String,
        attempts: Vec<ApiPoolUpstreamRequestAttempt>,
    },
    Summary {
        window: String,
        summary: StatsResponse,
    },
    Quota {
        snapshot: Box<QuotaSnapshotResponse>,
    },
}

#[derive(Debug, Clone, Serialize, FromRow)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ApiInvocation {
    pub(crate) id: i64,
    pub(crate) invoke_id: String,
    #[serde(serialize_with = "serialize_local_naive_to_utc_iso")]
    pub(crate) occurred_at: String,
    pub(crate) source: String,
    #[sqlx(default)]
    pub(crate) proxy_display_name: Option<String>,
    pub(crate) model: Option<String>,
    pub(crate) input_tokens: Option<i64>,
    pub(crate) output_tokens: Option<i64>,
    pub(crate) cache_input_tokens: Option<i64>,
    pub(crate) reasoning_tokens: Option<i64>,
    #[sqlx(default)]
    pub(crate) reasoning_effort: Option<String>,
    pub(crate) total_tokens: Option<i64>,
    pub(crate) cost: Option<f64>,
    pub(crate) status: Option<String>,
    pub(crate) error_message: Option<String>,
    #[sqlx(default)]
    pub(crate) failure_kind: Option<String>,
    #[sqlx(default)]
    pub(crate) stream_terminal_event: Option<String>,
    #[sqlx(default)]
    pub(crate) upstream_error_code: Option<String>,
    #[sqlx(default)]
    pub(crate) upstream_error_message: Option<String>,
    #[sqlx(default)]
    pub(crate) upstream_request_id: Option<String>,
    #[sqlx(default)]
    pub(crate) failure_class: Option<String>,
    #[sqlx(default)]
    pub(crate) is_actionable: Option<bool>,
    #[sqlx(default)]
    pub(crate) endpoint: Option<String>,
    #[sqlx(default)]
    pub(crate) requester_ip: Option<String>,
    #[sqlx(default)]
    pub(crate) prompt_cache_key: Option<String>,
    #[sqlx(default)]
    pub(crate) route_mode: Option<String>,
    #[sqlx(default)]
    pub(crate) upstream_account_id: Option<i64>,
    #[sqlx(default)]
    pub(crate) upstream_account_name: Option<String>,
    #[sqlx(default)]
    pub(crate) response_content_encoding: Option<String>,
    #[sqlx(default)]
    pub(crate) pool_attempt_count: Option<i64>,
    #[sqlx(default)]
    pub(crate) pool_distinct_account_count: Option<i64>,
    #[sqlx(default)]
    pub(crate) pool_attempt_terminal_reason: Option<String>,
    #[sqlx(default)]
    pub(crate) requested_service_tier: Option<String>,
    #[sqlx(default)]
    pub(crate) service_tier: Option<String>,
    #[sqlx(default)]
    pub(crate) proxy_weight_delta: Option<f64>,
    #[sqlx(default)]
    pub(crate) cost_estimated: Option<i64>,
    #[sqlx(default)]
    pub(crate) price_version: Option<String>,
    #[sqlx(default)]
    pub(crate) request_raw_path: Option<String>,
    #[sqlx(default)]
    pub(crate) request_raw_size: Option<i64>,
    #[sqlx(default)]
    pub(crate) request_raw_truncated: Option<i64>,
    #[sqlx(default)]
    pub(crate) request_raw_truncated_reason: Option<String>,
    #[sqlx(default)]
    pub(crate) response_raw_path: Option<String>,
    #[sqlx(default)]
    pub(crate) response_raw_size: Option<i64>,
    #[sqlx(default)]
    pub(crate) response_raw_truncated: Option<i64>,
    #[sqlx(default)]
    pub(crate) response_raw_truncated_reason: Option<String>,
    pub(crate) detail_level: String,
    #[sqlx(default)]
    #[serde(serialize_with = "serialize_opt_local_or_utc_to_utc_iso")]
    pub(crate) detail_pruned_at: Option<String>,
    #[sqlx(default)]
    pub(crate) detail_prune_reason: Option<String>,
    #[sqlx(default)]
    pub(crate) t_total_ms: Option<f64>,
    #[sqlx(default)]
    pub(crate) t_req_read_ms: Option<f64>,
    #[sqlx(default)]
    pub(crate) t_req_parse_ms: Option<f64>,
    #[sqlx(default)]
    pub(crate) t_upstream_connect_ms: Option<f64>,
    #[sqlx(default)]
    pub(crate) t_upstream_ttfb_ms: Option<f64>,
    #[sqlx(default)]
    pub(crate) t_upstream_stream_ms: Option<f64>,
    #[sqlx(default)]
    pub(crate) t_resp_parse_ms: Option<f64>,
    #[sqlx(default)]
    pub(crate) t_persist_ms: Option<f64>,
    #[serde(serialize_with = "serialize_local_naive_to_utc_iso")]
    pub(crate) created_at: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ListResponse {
    pub(crate) snapshot_id: i64,
    pub(crate) total: i64,
    pub(crate) page: i64,
    pub(crate) page_size: i64,
    pub(crate) records: Vec<ApiInvocation>,
}

#[derive(Debug, Clone, Serialize, FromRow)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ApiPoolUpstreamRequestAttempt {
    pub(crate) id: i64,
    pub(crate) invoke_id: String,
    #[serde(serialize_with = "serialize_local_naive_to_utc_iso")]
    pub(crate) occurred_at: String,
    pub(crate) endpoint: String,
    #[sqlx(default)]
    pub(crate) sticky_key: Option<String>,
    #[sqlx(default)]
    pub(crate) upstream_account_id: Option<i64>,
    #[sqlx(default)]
    pub(crate) upstream_account_name: Option<String>,
    #[sqlx(default)]
    pub(crate) upstream_route_key: Option<String>,
    pub(crate) attempt_index: i64,
    pub(crate) distinct_account_index: i64,
    pub(crate) same_account_retry_index: i64,
    #[sqlx(default)]
    pub(crate) requester_ip: Option<String>,
    #[sqlx(default)]
    #[serde(serialize_with = "serialize_opt_local_or_utc_to_utc_iso")]
    pub(crate) started_at: Option<String>,
    #[sqlx(default)]
    #[serde(serialize_with = "serialize_opt_local_or_utc_to_utc_iso")]
    pub(crate) finished_at: Option<String>,
    pub(crate) status: String,
    pub(crate) phase: String,
    #[sqlx(default)]
    pub(crate) http_status: Option<i64>,
    #[sqlx(default)]
    pub(crate) failure_kind: Option<String>,
    #[sqlx(default)]
    pub(crate) error_message: Option<String>,
    #[sqlx(default)]
    pub(crate) connect_latency_ms: Option<f64>,
    #[sqlx(default)]
    pub(crate) first_byte_latency_ms: Option<f64>,
    #[sqlx(default)]
    pub(crate) stream_latency_ms: Option<f64>,
    #[sqlx(default)]
    pub(crate) upstream_request_id: Option<String>,
    #[sqlx(default)]
    pub(crate) compact_support_status: Option<String>,
    #[sqlx(default)]
    pub(crate) compact_support_reason: Option<String>,
    #[serde(serialize_with = "serialize_local_naive_to_utc_iso")]
    pub(crate) created_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StatsResponse {
    pub(crate) total_count: i64,
    pub(crate) success_count: i64,
    pub(crate) failure_count: i64,
    pub(crate) total_cost: f64,
    pub(crate) total_tokens: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) maintenance: Option<StatsMaintenanceResponse>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StatsMaintenanceResponse {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) raw_compression_backlog: Option<RawCompressionBacklogResponse>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) startup_backfill: Option<StartupBackfillMaintenanceResponse>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) historical_rollup_backfill: Option<HistoricalRollupBackfillMaintenanceResponse>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RawCompressionBacklogResponse {
    pub(crate) oldest_uncompressed_age_secs: u64,
    pub(crate) uncompressed_count: u64,
    pub(crate) uncompressed_bytes: u64,
    pub(crate) alert_level: RawCompressionAlertLevel,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StartupBackfillMaintenanceResponse {
    pub(crate) upstream_activity_archive_pending_accounts: u64,
    pub(crate) zero_update_streak: u32,
    #[serde(serialize_with = "serialize_opt_local_or_utc_to_utc_iso")]
    pub(crate) next_run_after: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct HistoricalRollupBackfillMaintenanceResponse {
    pub(crate) pending_buckets: u64,
    pub(crate) legacy_archive_pending: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) last_materialized_hour: Option<String>,
    pub(crate) alert_level: HistoricalRollupBackfillAlertLevel,
}

#[derive(Debug, FromRow)]
pub(crate) struct StatsRow {
    pub(crate) total_count: i64,
    pub(crate) success_count: Option<i64>,
    pub(crate) failure_count: Option<i64>,
    pub(crate) total_cost: f64,
    pub(crate) total_tokens: i64,
}

#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct StatsTotals {
    pub(crate) total_count: i64,
    pub(crate) success_count: i64,
    pub(crate) failure_count: i64,
    pub(crate) total_cost: f64,
    pub(crate) total_tokens: i64,
}

impl StatsTotals {
    pub(crate) fn add(self, other: StatsTotals) -> StatsTotals {
        StatsTotals {
            total_count: self.total_count + other.total_count,
            success_count: self.success_count + other.success_count,
            failure_count: self.failure_count + other.failure_count,
            total_cost: self.total_cost + other.total_cost,
            total_tokens: self.total_tokens + other.total_tokens,
        }
    }

    pub(crate) fn into_response(self) -> StatsResponse {
        StatsResponse {
            total_count: self.total_count,
            success_count: self.success_count,
            failure_count: self.failure_count,
            total_cost: self.total_cost,
            total_tokens: self.total_tokens,
            maintenance: None,
        }
    }
}

impl From<StatsRow> for StatsTotals {
    fn from(value: StatsRow) -> Self {
        Self {
            total_count: value.total_count,
            success_count: value.success_count.unwrap_or(0),
            failure_count: value.failure_count.unwrap_or(0),
            total_cost: value.total_cost,
            total_tokens: value.total_tokens,
        }
    }
}

impl From<StatsRow> for StatsResponse {
    fn from(value: StatsRow) -> Self {
        StatsTotals::from(value).into_response()
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TimeseriesResponse {
    pub(crate) range_start: String,
    pub(crate) range_end: String,
    pub(crate) bucket_seconds: i64,
    pub(crate) effective_bucket: String,
    pub(crate) available_buckets: Vec<String>,
    pub(crate) bucket_limited_to_daily: bool,
    pub(crate) points: Vec<TimeseriesPoint>,
}

#[derive(Debug, Clone)]
pub(crate) struct TimeseriesBucketSelection {
    pub(crate) bucket_seconds: i64,
    pub(crate) effective_bucket: String,
    pub(crate) available_buckets: Vec<String>,
    pub(crate) bucket_limited_to_daily: bool,
}

fn resolve_timeseries_bucket_selection(
    params: &TimeseriesQuery,
    range_window: &RangeWindow,
    invocation_max_days: u64,
) -> Result<TimeseriesBucketSelection, ApiError> {
    let mut bucket_seconds = if let Some(spec) = params.bucket.as_deref() {
        bucket_seconds_from_spec(spec)
            .ok_or_else(|| anyhow!("unsupported bucket specification: {spec}"))?
    } else {
        default_bucket_seconds(range_window.duration)
    };

    if bucket_seconds <= 0 {
        return Err(ApiError::bad_request(anyhow!(
            "bucket seconds must be positive"
        )));
    }

    let range_seconds = range_window.duration.num_seconds();
    if range_seconds / bucket_seconds > 10_000 {
        // avoid accidentally returning extremely large payloads
        bucket_seconds = range_seconds / 10_000;
    }

    let subhour_supported = range_window.start >= shanghai_retention_cutoff(invocation_max_days);
    let bucket_limited_to_daily = false;
    let effective_bucket_seconds = if bucket_seconds < 3_600 && !subhour_supported {
        3_600
    } else {
        bucket_seconds
    };
    let effective_bucket = bucket_spec_from_seconds(effective_bucket_seconds)
        .map(str::to_string)
        .unwrap_or_else(|| format!("{effective_bucket_seconds}s"));

    Ok(TimeseriesBucketSelection {
        bucket_seconds: effective_bucket_seconds,
        effective_bucket,
        available_buckets: available_timeseries_bucket_specs(subhour_supported),
        bucket_limited_to_daily,
    })
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TimeseriesPoint {
    pub(crate) bucket_start: String,
    pub(crate) bucket_end: String,
    pub(crate) total_count: i64,
    pub(crate) success_count: i64,
    pub(crate) failure_count: i64,
    pub(crate) total_tokens: i64,
    pub(crate) total_cost: f64,
    pub(crate) first_byte_sample_count: i64,
    pub(crate) first_byte_avg_ms: Option<f64>,
    pub(crate) first_byte_p95_ms: Option<f64>,
    pub(crate) first_response_byte_total_sample_count: i64,
    pub(crate) first_response_byte_total_avg_ms: Option<f64>,
    pub(crate) first_response_byte_total_p95_ms: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct QuotaSnapshotResponse {
    #[serde(serialize_with = "serialize_local_or_utc_to_utc_iso")]
    pub(crate) captured_at: String,
    pub(crate) amount_limit: Option<f64>,
    pub(crate) used_amount: Option<f64>,
    pub(crate) remaining_amount: Option<f64>,
    pub(crate) period: Option<String>,
    #[serde(serialize_with = "serialize_opt_local_or_utc_to_utc_iso")]
    pub(crate) period_reset_time: Option<String>,
    #[serde(serialize_with = "serialize_opt_local_or_utc_to_utc_iso")]
    pub(crate) expire_time: Option<String>,
    pub(crate) is_active: bool,
    pub(crate) total_cost: f64,
    pub(crate) total_requests: i64,
    pub(crate) total_tokens: i64,
    #[serde(serialize_with = "serialize_opt_local_or_utc_to_utc_iso")]
    pub(crate) last_request_time: Option<String>,
    pub(crate) billing_type: Option<String>,
    pub(crate) remaining_count: Option<i64>,
    pub(crate) used_count: Option<i64>,
    pub(crate) sub_type_name: Option<String>,
}

#[derive(Debug, FromRow)]
pub(crate) struct QuotaSnapshotRow {
    pub(crate) captured_at: String,
    pub(crate) amount_limit: Option<f64>,
    pub(crate) used_amount: Option<f64>,
    pub(crate) remaining_amount: Option<f64>,
    pub(crate) period: Option<String>,
    pub(crate) period_reset_time: Option<String>,
    pub(crate) expire_time: Option<String>,
    pub(crate) is_active: Option<i64>,
    pub(crate) total_cost: f64,
    pub(crate) total_requests: i64,
    pub(crate) total_tokens: i64,
    pub(crate) last_request_time: Option<String>,
    pub(crate) billing_type: Option<String>,
    pub(crate) remaining_count: Option<i64>,
    pub(crate) used_count: Option<i64>,
    pub(crate) sub_type_name: Option<String>,
}

impl From<QuotaSnapshotRow> for QuotaSnapshotResponse {
    fn from(value: QuotaSnapshotRow) -> Self {
        Self {
            captured_at: value.captured_at,
            amount_limit: value.amount_limit,
            used_amount: value.used_amount,
            remaining_amount: value.remaining_amount,
            period: value.period,
            period_reset_time: value.period_reset_time,
            expire_time: value.expire_time,
            is_active: value.is_active.unwrap_or(0) != 0,
            total_cost: value.total_cost,
            total_requests: value.total_requests,
            total_tokens: value.total_tokens,
            last_request_time: value.last_request_time,
            billing_type: value.billing_type,
            remaining_count: value.remaining_count,
            used_count: value.used_count,
            sub_type_name: value.sub_type_name,
        }
    }
}

impl QuotaSnapshotResponse {
    pub(crate) async fn fetch_latest(pool: &Pool<Sqlite>) -> Result<Option<Self>> {
        let row = sqlx::query_as::<_, QuotaSnapshotRow>(
            r#"
            SELECT
                captured_at,
                amount_limit,
                used_amount,
                remaining_amount,
                period,
                period_reset_time,
                expire_time,
                is_active,
                total_cost,
                total_requests,
                total_tokens,
                last_request_time,
                billing_type,
                remaining_count,
                used_count,
                sub_type_name
            FROM codex_quota_snapshots
            ORDER BY captured_at DESC
            LIMIT 1
            "#,
        )
        .fetch_optional(pool)
        .await?;

        Ok(row.map(Into::into))
    }

    pub(crate) fn degraded_default() -> Self {
        Self {
            captured_at: format_utc_iso(Utc::now()),
            amount_limit: None,
            used_amount: None,
            remaining_amount: None,
            period: None,
            period_reset_time: None,
            expire_time: None,
            is_active: false,
            total_cost: 0.0,
            total_requests: 0,
            total_tokens: 0,
            last_request_time: None,
            billing_type: None,
            remaining_count: None,
            used_count: None,
            sub_type_name: None,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PromptCacheConversationsResponse {
    pub(crate) range_start: String,
    pub(crate) range_end: String,
    pub(crate) selection_mode: PromptCacheConversationSelectionMode,
    pub(crate) selected_limit: Option<i64>,
    pub(crate) selected_activity_hours: Option<i64>,
    pub(crate) selected_activity_minutes: Option<i64>,
    pub(crate) implicit_filter: PromptCacheConversationImplicitFilter,
    pub(crate) conversations: Vec<PromptCacheConversationResponse>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum PromptCacheConversationSelectionMode {
    Count,
    ActivityWindow,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum PromptCacheConversationSelection {
    Count(i64),
    ActivityWindowHours(i64),
    ActivityWindowMinutes(i64),
}

impl PromptCacheConversationSelection {
    pub(crate) fn selection_mode(self) -> PromptCacheConversationSelectionMode {
        match self {
            Self::Count(_) => PromptCacheConversationSelectionMode::Count,
            Self::ActivityWindowHours(_) | Self::ActivityWindowMinutes(_) => {
                PromptCacheConversationSelectionMode::ActivityWindow
            }
        }
    }

    pub(crate) fn activity_window_duration(self) -> ChronoDuration {
        match self {
            Self::Count(_) => ChronoDuration::hours(24),
            Self::ActivityWindowHours(hours) => ChronoDuration::hours(hours),
            Self::ActivityWindowMinutes(minutes) => ChronoDuration::minutes(minutes),
        }
    }

    pub(crate) fn display_limit(self) -> i64 {
        match self {
            Self::Count(limit) => limit,
            Self::ActivityWindowHours(_) | Self::ActivityWindowMinutes(_) => {
                PROMPT_CACHE_CONVERSATION_ACTIVITY_MODE_LIMIT
            }
        }
    }

    pub(crate) fn selected_limit(self) -> Option<i64> {
        match self {
            Self::Count(limit) => Some(limit),
            Self::ActivityWindowHours(_) | Self::ActivityWindowMinutes(_) => None,
        }
    }

    pub(crate) fn selected_activity_hours(self) -> Option<i64> {
        match self {
            Self::Count(_) => None,
            Self::ActivityWindowHours(hours) => Some(hours),
            Self::ActivityWindowMinutes(_) => None,
        }
    }

    pub(crate) fn selected_activity_minutes(self) -> Option<i64> {
        match self {
            Self::Count(_) | Self::ActivityWindowHours(_) => None,
            Self::ActivityWindowMinutes(minutes) => Some(minutes),
        }
    }

    pub(crate) fn implicit_filter(
        self,
        filtered_count: i64,
    ) -> PromptCacheConversationImplicitFilter {
        let kind = if filtered_count > 0 {
            Some(match self {
                Self::Count(_) => PromptCacheConversationImplicitFilterKind::InactiveOutside24h,
                Self::ActivityWindowHours(_) | Self::ActivityWindowMinutes(_) => {
                    PromptCacheConversationImplicitFilterKind::CappedTo50
                }
            })
        } else {
            None
        };

        PromptCacheConversationImplicitFilter {
            kind,
            filtered_count: filtered_count.max(0),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PromptCacheConversationImplicitFilter {
    pub(crate) kind: Option<PromptCacheConversationImplicitFilterKind>,
    pub(crate) filtered_count: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum PromptCacheConversationImplicitFilterKind {
    InactiveOutside24h,
    CappedTo50,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PromptCacheConversationResponse {
    pub(crate) prompt_cache_key: String,
    pub(crate) request_count: i64,
    pub(crate) total_tokens: i64,
    pub(crate) total_cost: f64,
    #[serde(serialize_with = "serialize_local_naive_to_utc_iso")]
    pub(crate) created_at: String,
    #[serde(serialize_with = "serialize_local_naive_to_utc_iso")]
    pub(crate) last_activity_at: String,
    pub(crate) upstream_accounts: Vec<PromptCacheConversationUpstreamAccountResponse>,
    pub(crate) recent_invocations: Vec<PromptCacheConversationInvocationPreviewResponse>,
    pub(crate) last24h_requests: Vec<PromptCacheConversationRequestPointResponse>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PromptCacheConversationInvocationPreviewResponse {
    pub(crate) id: i64,
    pub(crate) invoke_id: String,
    #[serde(serialize_with = "serialize_local_naive_to_utc_iso")]
    pub(crate) occurred_at: String,
    pub(crate) status: String,
    pub(crate) failure_class: Option<String>,
    pub(crate) route_mode: Option<String>,
    pub(crate) model: Option<String>,
    pub(crate) total_tokens: i64,
    pub(crate) cost: Option<f64>,
    pub(crate) proxy_display_name: Option<String>,
    pub(crate) upstream_account_id: Option<i64>,
    pub(crate) upstream_account_name: Option<String>,
    pub(crate) endpoint: Option<String>,
    pub(crate) source: Option<String>,
    pub(crate) input_tokens: Option<i64>,
    pub(crate) output_tokens: Option<i64>,
    pub(crate) cache_input_tokens: Option<i64>,
    pub(crate) reasoning_tokens: Option<i64>,
    pub(crate) reasoning_effort: Option<String>,
    pub(crate) error_message: Option<String>,
    pub(crate) failure_kind: Option<String>,
    pub(crate) is_actionable: Option<bool>,
    pub(crate) response_content_encoding: Option<String>,
    pub(crate) requested_service_tier: Option<String>,
    pub(crate) service_tier: Option<String>,
    pub(crate) t_req_read_ms: Option<f64>,
    pub(crate) t_req_parse_ms: Option<f64>,
    pub(crate) t_upstream_connect_ms: Option<f64>,
    pub(crate) t_upstream_ttfb_ms: Option<f64>,
    pub(crate) t_upstream_stream_ms: Option<f64>,
    pub(crate) t_resp_parse_ms: Option<f64>,
    pub(crate) t_persist_ms: Option<f64>,
    pub(crate) t_total_ms: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PromptCacheConversationUpstreamAccountResponse {
    pub(crate) upstream_account_id: Option<i64>,
    pub(crate) upstream_account_name: Option<String>,
    pub(crate) request_count: i64,
    pub(crate) total_tokens: i64,
    pub(crate) total_cost: f64,
    #[serde(serialize_with = "serialize_local_naive_to_utc_iso")]
    pub(crate) last_activity_at: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PromptCacheConversationRequestPointResponse {
    #[serde(serialize_with = "serialize_local_naive_to_utc_iso")]
    pub(crate) occurred_at: String,
    pub(crate) status: String,
    pub(crate) is_success: bool,
    pub(crate) request_tokens: i64,
    pub(crate) cumulative_tokens: i64,
}

#[derive(Debug, Clone)]
pub(crate) struct PromptCacheConversationsCacheEntry {
    pub(crate) cached_at: Instant,
    pub(crate) generation: u64,
    pub(crate) response: PromptCacheConversationsResponse,
}

#[derive(Debug)]
pub(crate) struct PromptCacheConversationInFlight {
    pub(crate) signal: watch::Sender<bool>,
    pub(crate) generation: u64,
}

#[derive(Debug, Default)]
pub(crate) struct PromptCacheConversationsCacheState {
    pub(crate) entries:
        HashMap<PromptCacheConversationSelection, PromptCacheConversationsCacheEntry>,
    pub(crate) in_flight:
        HashMap<PromptCacheConversationSelection, PromptCacheConversationInFlight>,
    pub(crate) generation: u64,
}

#[derive(Debug)]
pub(crate) struct PromptCacheConversationFlightGuard {
    pub(crate) cache: Arc<Mutex<PromptCacheConversationsCacheState>>,
    pub(crate) selection: PromptCacheConversationSelection,
    pub(crate) generation: u64,
    pub(crate) active: bool,
}

impl PromptCacheConversationFlightGuard {
    pub(crate) fn new(
        cache: Arc<Mutex<PromptCacheConversationsCacheState>>,
        selection: PromptCacheConversationSelection,
        generation: u64,
    ) -> Self {
        Self {
            cache,
            selection,
            generation,
            active: true,
        }
    }

    pub(crate) fn disarm(&mut self) {
        self.active = false;
    }
}

impl Drop for PromptCacheConversationFlightGuard {
    fn drop(&mut self) {
        if !self.active {
            return;
        }

        let cache = self.cache.clone();
        let selection = self.selection;
        let generation = self.generation;
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            handle.spawn(async move {
                let mut state = cache.lock().await;
                if let Some(in_flight) = state.in_flight.remove(&selection) {
                    if in_flight.generation != generation {
                        state.in_flight.insert(selection, in_flight);
                        return;
                    }
                    let _ = in_flight.signal.send(true);
                }
            });
            return;
        }

        if let Ok(mut state) = cache.try_lock()
            && let Some(in_flight) = state.in_flight.remove(&selection)
        {
            if in_flight.generation != generation {
                state.in_flight.insert(selection, in_flight);
                return;
            }
            let _ = in_flight.signal.send(true);
        }
    }
}

pub(crate) async fn invalidate_prompt_cache_conversations_cache(
    cache: &Arc<Mutex<PromptCacheConversationsCacheState>>,
) {
    let in_flight = {
        let mut state = cache.lock().await;
        state.generation = state.generation.wrapping_add(1);
        state.entries.clear();
        std::mem::take(&mut state.in_flight)
    };

    for flight in in_flight.into_values() {
        let _ = flight.signal.send(true);
    }
}

#[derive(Debug, Clone, Default)]
pub(crate) struct ParsedUsage {
    pub(crate) input_tokens: Option<i64>,
    pub(crate) output_tokens: Option<i64>,
    pub(crate) cache_input_tokens: Option<i64>,
    pub(crate) reasoning_tokens: Option<i64>,
    pub(crate) total_tokens: Option<i64>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct RawPayloadMeta {
    pub(crate) path: Option<String>,
    pub(crate) size_bytes: i64,
    pub(crate) truncated: bool,
    pub(crate) truncated_reason: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct RequestCaptureInfo {
    pub(crate) model: Option<String>,
    pub(crate) sticky_key: Option<String>,
    pub(crate) prompt_cache_key: Option<String>,
    pub(crate) requested_service_tier: Option<String>,
    pub(crate) reasoning_effort: Option<String>,
    pub(crate) is_stream: bool,
    pub(crate) parse_error: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct ResponseCaptureInfo {
    pub(crate) model: Option<String>,
    pub(crate) usage: ParsedUsage,
    pub(crate) usage_missing_reason: Option<String>,
    pub(crate) service_tier: Option<String>,
    pub(crate) stream_terminal_event: Option<String>,
    pub(crate) upstream_error_code: Option<String>,
    pub(crate) upstream_error_message: Option<String>,
    pub(crate) upstream_request_id: Option<String>,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct StageTimings {
    pub(crate) t_total_ms: f64,
    pub(crate) t_req_read_ms: f64,
    pub(crate) t_req_parse_ms: f64,
    pub(crate) t_upstream_connect_ms: f64,
    pub(crate) t_upstream_ttfb_ms: f64,
    pub(crate) t_upstream_stream_ms: f64,
    pub(crate) t_resp_parse_ms: f64,
    pub(crate) t_persist_ms: f64,
}

#[derive(Debug, Clone)]
pub(crate) struct ProxyCaptureRecord {
    pub(crate) invoke_id: String,
    pub(crate) occurred_at: String,
    pub(crate) model: Option<String>,
    pub(crate) usage: ParsedUsage,
    pub(crate) cost: Option<f64>,
    pub(crate) cost_estimated: bool,
    pub(crate) price_version: Option<String>,
    pub(crate) status: String,
    pub(crate) error_message: Option<String>,
    pub(crate) failure_kind: Option<String>,
    pub(crate) payload: Option<String>,
    pub(crate) raw_response: String,
    pub(crate) req_raw: RawPayloadMeta,
    pub(crate) resp_raw: RawPayloadMeta,
    pub(crate) timings: StageTimings,
}

#[derive(Debug, Clone)]
pub(crate) struct RequestBodyReadError {
    pub(crate) status: StatusCode,
    pub(crate) message: String,
    pub(crate) failure_kind: &'static str,
    pub(crate) partial_body: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProxyCaptureTarget {
    ChatCompletions,
    Responses,
    ResponsesCompact,
}

impl ProxyCaptureTarget {
    pub(crate) fn endpoint(self) -> &'static str {
        match self {
            Self::ChatCompletions => "/v1/chat/completions",
            Self::Responses => "/v1/responses",
            Self::ResponsesCompact => "/v1/responses/compact",
        }
    }

    pub(crate) fn allows_fast_mode_rewrite(self) -> bool {
        matches!(self, Self::ChatCompletions | Self::Responses)
    }

    pub(crate) fn should_auto_include_usage(self) -> bool {
        matches!(self, Self::ChatCompletions)
    }

    pub(crate) fn from_endpoint(endpoint: &str) -> Self {
        match endpoint {
            "/v1/chat/completions" => Self::ChatCompletions,
            "/v1/responses/compact" => Self::ResponsesCompact,
            "/v1/responses" => Self::Responses,
            _ => Self::Responses,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum InvocationSourceScope {
    ProxyOnly,
    All,
}

#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct ProxyUsageBackfillSummary {
    pub(crate) scanned: u64,
    pub(crate) updated: u64,
    pub(crate) skipped_missing_file: u64,
    pub(crate) skipped_without_usage: u64,
    pub(crate) skipped_decode_error: u64,
}

#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct ProxyCostBackfillSummary {
    pub(crate) scanned: u64,
    pub(crate) updated: u64,
    pub(crate) skipped_unpriced_model: u64,
}

#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct ProxyPromptCacheKeyBackfillSummary {
    pub(crate) scanned: u64,
    pub(crate) updated: u64,
    pub(crate) skipped_missing_file: u64,
    pub(crate) skipped_invalid_json: u64,
    pub(crate) skipped_missing_key: u64,
}

#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct ProxyRequestedServiceTierBackfillSummary {
    pub(crate) scanned: u64,
    pub(crate) updated: u64,
    pub(crate) skipped_missing_file: u64,
    pub(crate) skipped_invalid_json: u64,
    pub(crate) skipped_missing_tier: u64,
}

#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct InvocationServiceTierBackfillSummary {
    pub(crate) scanned: u64,
    pub(crate) updated: u64,
    pub(crate) skipped_missing_file: u64,
    pub(crate) skipped_missing_tier: u64,
}

#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct ProxyReasoningEffortBackfillSummary {
    pub(crate) scanned: u64,
    pub(crate) updated: u64,
    pub(crate) skipped_missing_file: u64,
    pub(crate) skipped_invalid_json: u64,
    pub(crate) skipped_missing_effort: u64,
}

#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct FailureClassificationBackfillSummary {
    pub(crate) scanned: u64,
    pub(crate) updated: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FailureClass {
    None,
    ServiceFailure,
    ClientFailure,
    ClientAbort,
}

impl FailureClass {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            FailureClass::None => FAILURE_CLASS_NONE,
            FailureClass::ServiceFailure => FAILURE_CLASS_SERVICE,
            FailureClass::ClientFailure => FAILURE_CLASS_CLIENT,
            FailureClass::ClientAbort => FAILURE_CLASS_ABORT,
        }
    }

    pub(crate) fn from_db_str(raw: &str) -> Option<Self> {
        match raw {
            FAILURE_CLASS_NONE => Some(FailureClass::None),
            FAILURE_CLASS_SERVICE => Some(FailureClass::ServiceFailure),
            FAILURE_CLASS_CLIENT => Some(FailureClass::ClientFailure),
            FAILURE_CLASS_ABORT => Some(FailureClass::ClientAbort),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct FailureClassification {
    pub(crate) failure_kind: Option<String>,
    pub(crate) failure_class: FailureClass,
    pub(crate) is_actionable: bool,
}

#[derive(Debug, FromRow)]
pub(crate) struct ProxyUsageBackfillCandidate {
    pub(crate) id: i64,
    pub(crate) response_raw_path: String,
    pub(crate) payload: Option<String>,
}

#[derive(Debug, FromRow)]
pub(crate) struct ProxyCostBackfillCandidate {
    pub(crate) id: i64,
    pub(crate) model: Option<String>,
    pub(crate) input_tokens: Option<i64>,
    pub(crate) output_tokens: Option<i64>,
    pub(crate) cache_input_tokens: Option<i64>,
    pub(crate) reasoning_tokens: Option<i64>,
    pub(crate) total_tokens: Option<i64>,
}

#[derive(Debug, FromRow)]
pub(crate) struct ProxyPromptCacheKeyBackfillCandidate {
    pub(crate) id: i64,
    pub(crate) request_raw_path: String,
}

#[derive(Debug, FromRow)]
pub(crate) struct ProxyRequestedServiceTierBackfillCandidate {
    pub(crate) id: i64,
    pub(crate) request_raw_path: String,
}

#[derive(Debug, FromRow)]
pub(crate) struct ProxyReasoningEffortBackfillCandidate {
    pub(crate) id: i64,
    pub(crate) request_raw_path: String,
}

#[derive(Debug)]
pub(crate) struct ProxyUsageBackfillUpdate {
    pub(crate) id: i64,
    pub(crate) usage: ParsedUsage,
}

#[derive(Debug)]
pub(crate) struct ProxyCostBackfillUpdate {
    pub(crate) id: i64,
    pub(crate) cost: Option<f64>,
    pub(crate) cost_estimated: bool,
    pub(crate) price_version: Option<String>,
}

#[derive(Debug, FromRow)]
pub(crate) struct PromptCacheConversationAggregateRow {
    pub(crate) prompt_cache_key: String,
    pub(crate) request_count: i64,
    pub(crate) total_tokens: i64,
    pub(crate) total_cost: f64,
    pub(crate) created_at: String,
    pub(crate) last_activity_at: String,
}

#[derive(Debug, FromRow)]
pub(crate) struct PromptCacheConversationEventRow {
    pub(crate) occurred_at: String,
    pub(crate) status: String,
    pub(crate) request_tokens: i64,
    pub(crate) prompt_cache_key: String,
}

#[derive(Debug, FromRow)]
pub(crate) struct PromptCacheConversationInvocationPreviewRow {
    pub(crate) prompt_cache_key: String,
    pub(crate) id: i64,
    pub(crate) invoke_id: String,
    pub(crate) occurred_at: String,
    pub(crate) status: String,
    pub(crate) failure_class: Option<String>,
    pub(crate) route_mode: Option<String>,
    pub(crate) model: Option<String>,
    pub(crate) total_tokens: i64,
    pub(crate) cost: Option<f64>,
    pub(crate) source: Option<String>,
    pub(crate) input_tokens: Option<i64>,
    pub(crate) output_tokens: Option<i64>,
    pub(crate) cache_input_tokens: Option<i64>,
    pub(crate) reasoning_tokens: Option<i64>,
    pub(crate) reasoning_effort: Option<String>,
    pub(crate) error_message: Option<String>,
    pub(crate) failure_kind: Option<String>,
    pub(crate) is_actionable: Option<i64>,
    pub(crate) proxy_display_name: Option<String>,
    pub(crate) upstream_account_id: Option<i64>,
    pub(crate) upstream_account_name: Option<String>,
    pub(crate) response_content_encoding: Option<String>,
    pub(crate) requested_service_tier: Option<String>,
    pub(crate) service_tier: Option<String>,
    pub(crate) t_req_read_ms: Option<f64>,
    pub(crate) t_req_parse_ms: Option<f64>,
    pub(crate) t_upstream_connect_ms: Option<f64>,
    pub(crate) t_upstream_ttfb_ms: Option<f64>,
    pub(crate) t_upstream_stream_ms: Option<f64>,
    pub(crate) t_resp_parse_ms: Option<f64>,
    pub(crate) t_persist_ms: Option<f64>,
    pub(crate) t_total_ms: Option<f64>,
    pub(crate) endpoint: Option<String>,
}

#[derive(Debug, FromRow)]
pub(crate) struct PromptCacheConversationUpstreamAccountSummaryRow {
    pub(crate) prompt_cache_key: String,
    pub(crate) upstream_account_id: Option<i64>,
    pub(crate) upstream_account_name: Option<String>,
    pub(crate) request_count: i64,
    pub(crate) total_tokens: i64,
    pub(crate) total_cost: f64,
    pub(crate) last_activity_at: String,
}

#[derive(Debug, Deserialize, Default, Clone)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ListQuery {
    pub(crate) limit: Option<i64>,
    pub(crate) page: Option<i64>,
    pub(crate) page_size: Option<i64>,
    pub(crate) snapshot_id: Option<i64>,
    pub(crate) sort_by: Option<String>,
    pub(crate) sort_order: Option<String>,
    #[allow(dead_code)]
    pub(crate) range_preset: Option<String>,
    pub(crate) from: Option<String>,
    pub(crate) to: Option<String>,
    pub(crate) model: Option<String>,
    pub(crate) status: Option<String>,
    // Kept for compatibility so stale /records URLs with `?proxy=...` deserialize cleanly,
    // but records queries intentionally ignore this field.
    #[allow(dead_code)]
    pub(crate) proxy: Option<String>,
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
    pub(crate) suggest_field: Option<String>,
    pub(crate) suggest_query: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PromptCacheConversationsQuery {
    pub(crate) limit: Option<i64>,
    pub(crate) activity_hours: Option<i64>,
    pub(crate) activity_minutes: Option<i64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SummaryQuery {
    pub(crate) window: Option<String>,
    pub(crate) limit: Option<i64>,
    pub(crate) time_zone: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TimeseriesQuery {
    #[serde(default = "default_range")]
    pub(crate) range: String,
    pub(crate) bucket: Option<String>,
    #[allow(dead_code)]
    pub(crate) settlement_hour: Option<u8>,
    pub(crate) time_zone: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PerfQuery {
    #[serde(default = "default_range")]
    pub(crate) range: String,
    pub(crate) time_zone: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PerfStatsResponse {
    pub(crate) range_start: String,
    pub(crate) range_end: String,
    pub(crate) source: String,
    pub(crate) stages: Vec<PerfStageStats>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PerfStageStats {
    pub(crate) stage: String,
    pub(crate) count: i64,
    pub(crate) avg_ms: f64,
    pub(crate) p50_ms: f64,
    pub(crate) p90_ms: f64,
    pub(crate) p99_ms: f64,
    pub(crate) max_ms: f64,
}
