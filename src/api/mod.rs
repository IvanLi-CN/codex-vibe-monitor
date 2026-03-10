use super::*;
use crate::forward_proxy::*;
use crate::stats::*;

const INVOCATION_PROXY_DISPLAY_SQL: &str = "COALESCE(NULLIF(TRIM(CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.proxyDisplayName') AS TEXT) END), ''), CASE WHEN TRIM(source) != 'proxy' THEN TRIM(source) END)";
const INVOCATION_ENDPOINT_SQL: &str =
    "CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.endpoint') AS TEXT) END";
const INVOCATION_FAILURE_KIND_SQL: &str = "COALESCE(CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.failureKind') AS TEXT) END, failure_kind)";
const INVOCATION_REQUESTER_IP_SQL: &str =
    "CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.requesterIp') AS TEXT) END";
const INVOCATION_PROMPT_CACHE_KEY_SQL: &str = "CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.promptCacheKey') AS TEXT) END";
const INVOCATION_SELECT_SQL: &str = "SELECT id, invoke_id, occurred_at, source, \
     CASE WHEN json_valid(payload) THEN json_extract(payload, '$.proxyDisplayName') END AS proxy_display_name, \
     model, input_tokens, output_tokens, \
     cache_input_tokens, reasoning_tokens, \
     CASE WHEN json_valid(payload) THEN json_extract(payload, '$.reasoningEffort') END AS reasoning_effort, \
     total_tokens, cost, status, error_message, \
     CASE WHEN json_valid(payload) THEN json_extract(payload, '$.endpoint') END AS endpoint, \
     COALESCE(CASE WHEN json_valid(payload) THEN json_extract(payload, '$.failureKind') END, failure_kind) AS failure_kind, \
     failure_class, is_actionable, \
     CASE WHEN json_valid(payload) THEN json_extract(payload, '$.requesterIp') END AS requester_ip, \
     CASE WHEN json_valid(payload) THEN json_extract(payload, '$.promptCacheKey') END AS prompt_cache_key, \
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
     raw_expires_at, detail_level, detail_pruned_at, detail_prune_reason, \
     t_total_ms, t_req_read_ms, t_req_parse_ms, t_upstream_connect_ms, t_upstream_ttfb_ms, \
     t_upstream_stream_ms, t_resp_parse_ms, t_persist_ms, \
     created_at \
     FROM codex_invocations WHERE 1 = 1";

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
            Self::Status => "status",
        }
    }
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
    proxy: Option<String>,
    endpoint: Option<String>,
    failure_class: Option<String>,
    failure_kind: Option<String>,
    prompt_cache_key: Option<String>,
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
struct InvocationNetworkTimingRow {
    t_upstream_ttfb_ms: Option<f64>,
    t_total_ms: Option<f64>,
}

#[derive(Debug, FromRow)]
struct InvocationFailureSummaryRow {
    status: Option<String>,
    error_message: Option<String>,
    failure_kind: Option<String>,
    failure_class: Option<String>,
    is_actionable: Option<i64>,
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

fn parse_invocation_bound(raw: Option<&str>, field_name: &str) -> Result<Option<String>> {
    let Some(raw_value) = normalize_query_text(raw) else {
        return Ok(None);
    };
    let parsed = DateTime::parse_from_rfc3339(&raw_value)
        .with_context(|| format!("invalid {field_name}: {raw_value}"))?
        .with_timezone(&Utc);
    Ok(Some(db_occurred_at_lower_bound(parsed)))
}

fn build_invocation_filters(params: &ListQuery) -> Result<InvocationRecordsFilters> {
    let occurred_from = parse_invocation_bound(params.from.as_deref(), "from")?;
    let occurred_to = parse_invocation_bound(params.to.as_deref(), "to")?;

    if let (Some(min_tokens), Some(max_tokens)) = (params.min_total_tokens, params.max_total_tokens)
        && min_tokens > max_tokens
    {
        return Err(anyhow!("minTotalTokens must be <= maxTotalTokens"));
    }

    if let (Some(min_ms), Some(max_ms)) = (params.min_total_ms, params.max_total_ms)
        && min_ms > max_ms
    {
        return Err(anyhow!("minTotalMs must be <= maxTotalMs"));
    }

    Ok(InvocationRecordsFilters {
        occurred_from,
        occurred_to,
        status: normalize_query_text(params.status.as_deref()),
        model: normalize_query_text(params.model.as_deref()),
        proxy: normalize_query_text(params.proxy.as_deref()),
        endpoint: normalize_query_text(params.endpoint.as_deref()),
        failure_class: normalize_query_text(params.failure_class.as_deref()),
        failure_kind: normalize_query_text(params.failure_kind.as_deref()),
        prompt_cache_key: normalize_query_text(params.prompt_cache_key.as_deref()),
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
) -> Result<InvocationListRequest> {
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
        push_exact_text_filter(query, "status", status);
    }

    if let Some(proxy) = filters.proxy.as_deref() {
        push_exact_text_filter(query, INVOCATION_PROXY_DISPLAY_SQL, proxy);
    }

    if let Some(endpoint) = filters.endpoint.as_deref() {
        push_exact_text_filter(query, INVOCATION_ENDPOINT_SQL, endpoint);
    }

    if let Some(failure_class) = filters.failure_class.as_deref() {
        push_exact_text_filter(query, "failure_class", failure_class);
    }

    if let Some(failure_kind) = filters.failure_kind.as_deref() {
        push_exact_text_filter(query, INVOCATION_FAILURE_KIND_SQL, failure_kind);
    }

    if let Some(prompt_cache_key) = filters.prompt_cache_key.as_deref() {
        push_exact_text_filter(query, INVOCATION_PROMPT_CACHE_KEY_SQL, prompt_cache_key);
    }

    if let Some(requester_ip) = filters.requester_ip.as_deref() {
        push_exact_text_filter(query, INVOCATION_REQUESTER_IP_SQL, requester_ip);
    }

    if let Some(keyword) = filters.keyword.as_deref() {
        push_keyword_filter(query, keyword);
    }

    if let Some(min_total_tokens) = filters.min_total_tokens {
        query
            .push(" AND COALESCE(total_tokens, 0) >= ")
            .push_bind(min_total_tokens);
    }

    if let Some(max_total_tokens) = filters.max_total_tokens {
        query
            .push(" AND COALESCE(total_tokens, 0) <= ")
            .push_bind(max_total_tokens);
    }

    if let Some(min_total_ms) = filters.min_total_ms {
        query
            .push(" AND COALESCE(t_total_ms, 0) >= ")
            .push_bind(min_total_ms);
    }

    if let Some(max_total_ms) = filters.max_total_ms {
        query
            .push(" AND COALESCE(t_total_ms, 0) <= ")
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
    query.push(sort_by.sql_expr());
    query.push(" IS NULL ASC, ");
    query.push(sort_by.sql_expr());
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

fn collect_non_negative_finite(values: impl Iterator<Item = Option<f64>>) -> Vec<f64> {
    let mut collected = values
        .flatten()
        .filter(|value| value.is_finite() && *value >= 0.0)
        .collect::<Vec<_>>();
    collected.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    collected
}

fn summarize_network_timings(rows: &[InvocationNetworkTimingRow]) -> InvocationNetworkSummary {
    let ttfb_values = collect_non_negative_finite(rows.iter().map(|row| row.t_upstream_ttfb_ms));
    let total_values = collect_non_negative_finite(rows.iter().map(|row| row.t_total_ms));

    let avg = |values: &[f64]| {
        if values.is_empty() {
            None
        } else {
            Some(values.iter().copied().sum::<f64>() / values.len() as f64)
        }
    };

    InvocationNetworkSummary {
        avg_ttfb_ms: avg(&ttfb_values),
        p95_ttfb_ms: (!ttfb_values.is_empty()).then(|| percentile_sorted_f64(&ttfb_values, 0.95)),
        avg_total_ms: avg(&total_values),
        p95_total_ms: (!total_values.is_empty())
            .then(|| percentile_sorted_f64(&total_values, 0.95)),
    }
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
        && params.proxy.is_none()
        && params.endpoint.is_none()
        && params.failure_class.is_none()
        && params.failure_kind.is_none()
        && params.prompt_cache_key.is_none()
        && params.requester_ip.is_none()
        && params.keyword.is_none()
        && params.min_total_tokens.is_none()
        && params.max_total_tokens.is_none()
        && params.min_total_ms.is_none()
        && params.max_total_ms.is_none()
}

fn summarize_exception_rows(rows: &[InvocationFailureSummaryRow]) -> InvocationExceptionSummary {
    let mut summary = InvocationExceptionSummary {
        failure_count: 0,
        service_failure_count: 0,
        client_failure_count: 0,
        client_abort_count: 0,
        actionable_failure_count: 0,
    };

    for row in rows {
        let resolved = resolve_failure_classification(
            row.status.as_deref(),
            row.error_message.as_deref(),
            row.failure_kind.as_deref(),
            row.failure_class.as_deref(),
            row.is_actionable,
        );
        if resolved.failure_class == FailureClass::None {
            continue;
        }
        summary.failure_count += 1;
        match resolved.failure_class {
            FailureClass::ServiceFailure => summary.service_failure_count += 1,
            FailureClass::ClientFailure => summary.client_failure_count += 1,
            FailureClass::ClientAbort => summary.client_abort_count += 1,
            FailureClass::None => {}
        }
        if resolved.is_actionable {
            summary.actionable_failure_count += 1;
        }
    }

    summary
}

pub(crate) async fn list_invocations(
    State(state): State<Arc<AppState>>,
    Query(params): Query<ListQuery>,
) -> Result<Json<ListResponse>, ApiError> {
    let request = build_invocation_list_request(&params, state.config.list_limit_max as i64)?;
    let source_scope = resolve_default_source_scope(&state.pool).await?;

    if is_legacy_invocation_stream_query(&params) {
        let mut query = QueryBuilder::new(INVOCATION_SELECT_SQL);
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
    let mut query = QueryBuilder::new(INVOCATION_SELECT_SQL);
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

pub(crate) async fn fetch_invocation_summary(
    State(state): State<Arc<AppState>>,
    Query(params): Query<ListQuery>,
) -> Result<Json<InvocationSummaryResponse>, ApiError> {
    let request = build_invocation_list_request(&params, state.config.list_limit_max as i64)?;
    let source_scope = resolve_default_source_scope(&state.pool).await?;
    let snapshot_id = request
        .snapshot_id
        .unwrap_or(resolve_invocation_snapshot_id(&state.pool, source_scope).await?);

    let mut totals_query = QueryBuilder::new(
        "SELECT \
         COUNT(*) AS total_count, \
         COALESCE(SUM(CASE WHEN status = 'success' THEN 1 ELSE 0 END), 0) AS success_count, \
         COALESCE(SUM(CASE WHEN status IS NOT NULL AND status != 'success' THEN 1 ELSE 0 END), 0) AS failure_count, \
         COALESCE(SUM(total_tokens), 0) AS total_tokens, \
         COALESCE(SUM(cost), 0.0) AS total_cost, \
         COALESCE(SUM(cache_input_tokens), 0) AS cache_input_tokens \
         FROM codex_invocations WHERE 1 = 1",
    );
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

    let mut timing_query = QueryBuilder::new(
        "SELECT t_upstream_ttfb_ms, t_total_ms FROM codex_invocations WHERE 1 = 1",
    );
    apply_invocation_records_filters(
        &mut timing_query,
        &request.filters,
        source_scope,
        Some(SnapshotConstraint::UpTo(snapshot_id)),
    );
    let timing_rows = timing_query
        .build_query_as::<InvocationNetworkTimingRow>()
        .fetch_all(&state.pool)
        .await?;

    let mut failure_query = QueryBuilder::new("SELECT status, error_message, ");
    failure_query
        .push(INVOCATION_FAILURE_KIND_SQL)
        .push(" AS failure_kind, failure_class, is_actionable FROM codex_invocations WHERE 1 = 1");
    apply_invocation_records_filters(
        &mut failure_query,
        &request.filters,
        source_scope,
        Some(SnapshotConstraint::UpTo(snapshot_id)),
    );
    let failure_rows = failure_query
        .build_query_as::<InvocationFailureSummaryRow>()
        .fetch_all(&state.pool)
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
        network: summarize_network_timings(&timing_rows),
        exception: summarize_exception_rows(&failure_rows),
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
        .unwrap_or(resolve_invocation_snapshot_id(&state.pool, source_scope).await?);
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
    Ok(Json(totals.into_response()))
}

pub(crate) async fn fetch_summary(
    State(state): State<Arc<AppState>>,
    Query(params): Query<SummaryQuery>,
) -> Result<Json<StatsResponse>, ApiError> {
    let default_limit = state.config.list_limit_max as i64;
    let window = parse_summary_window(&params, default_limit)?;
    let reporting_tz = parse_reporting_tz(params.time_zone.as_deref())?;
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
            query_combined_totals(
                &state.pool,
                state.config.crs_stats.as_ref(),
                StatsFilter::Since(start),
                source_scope,
            )
            .await?
        }
        SummaryWindow::Calendar(spec) => {
            let now = Utc::now();
            let start = named_range_start(spec.as_str(), now, reporting_tz)
                .ok_or_else(|| ApiError(anyhow!("unsupported calendar window: {spec}")))?;
            query_combined_totals(
                &state.pool,
                state.config.crs_stats.as_ref(),
                StatsFilter::Since(start),
                source_scope,
            )
            .await?
        }
    };

    Ok(Json(totals.into_response()))
}

pub(crate) async fn fetch_forward_proxy_live_stats(
    State(state): State<Arc<AppState>>,
) -> Result<Json<ForwardProxyLiveStatsResponse>, ApiError> {
    let response = build_forward_proxy_live_stats_response(state.as_ref()).await?;
    Ok(Json(response))
}

pub(crate) async fn fetch_prompt_cache_conversations(
    State(state): State<Arc<AppState>>,
    Query(params): Query<PromptCacheConversationsQuery>,
) -> Result<Json<PromptCacheConversationsResponse>, ApiError> {
    let limit = normalize_prompt_cache_conversation_limit(params.limit);
    let response = fetch_prompt_cache_conversations_cached(state.as_ref(), limit).await?;
    Ok(Json(response))
}

pub(crate) fn normalize_prompt_cache_conversation_limit(raw: Option<i64>) -> i64 {
    match raw {
        Some(value @ (20 | 50 | 100)) => value,
        _ => PROMPT_CACHE_CONVERSATION_DEFAULT_LIMIT,
    }
}

pub(crate) async fn fetch_prompt_cache_conversations_cached(
    state: &AppState,
    limit: i64,
) -> Result<PromptCacheConversationsResponse> {
    loop {
        let mut wait_on: Option<watch::Receiver<bool>> = None;
        let mut flight_guard: Option<PromptCacheConversationFlightGuard> = None;
        {
            let mut cache = state.prompt_cache_conversation_cache.lock().await;
            if let Some(entry) = cache.entries.get(&limit)
                && entry.cached_at.elapsed()
                    <= Duration::from_secs(PROMPT_CACHE_CONVERSATION_CACHE_TTL_SECS)
            {
                return Ok(entry.response.clone());
            }

            if let Some(in_flight) = cache.in_flight.get(&limit) {
                wait_on = Some(in_flight.signal.subscribe());
            } else {
                let (signal, _receiver) = watch::channel(false);
                cache
                    .in_flight
                    .insert(limit, PromptCacheConversationInFlight { signal });
                flight_guard = Some(PromptCacheConversationFlightGuard::new(
                    state.prompt_cache_conversation_cache.clone(),
                    limit,
                ));
            }
        }

        if let Some(mut receiver) = wait_on {
            if !*receiver.borrow() {
                let _ = receiver.changed().await;
            }
            continue;
        }

        let result = build_prompt_cache_conversations_response(state, limit).await;

        if let Some(guard) = flight_guard.as_mut() {
            guard.disarm();
        }

        let mut cache = state.prompt_cache_conversation_cache.lock().await;
        if let Some(in_flight) = cache.in_flight.remove(&limit) {
            if let Ok(response) = &result {
                cache.entries.insert(
                    limit,
                    PromptCacheConversationsCacheEntry {
                        cached_at: Instant::now(),
                        response: response.clone(),
                    },
                );
            }
            let _ = in_flight.signal.send(true);
        }

        return result;
    }
}

pub(crate) async fn build_prompt_cache_conversations_response(
    state: &AppState,
    limit: i64,
) -> Result<PromptCacheConversationsResponse> {
    let source_scope = resolve_default_source_scope(&state.pool).await?;
    let range_end = Utc::now();
    let range_start = range_end - ChronoDuration::hours(24);
    let range_start_bound = db_occurred_at_lower_bound(range_start);

    let aggregates = query_prompt_cache_conversation_aggregates(
        &state.pool,
        &range_start_bound,
        source_scope,
        limit,
    )
    .await?;
    if aggregates.is_empty() {
        return Ok(PromptCacheConversationsResponse {
            range_start: format_utc_iso(range_start),
            range_end: format_utc_iso(range_end),
            conversations: Vec::new(),
        });
    }

    let selected_keys = aggregates
        .iter()
        .map(|row| row.prompt_cache_key.clone())
        .collect::<Vec<_>>();
    let events = query_prompt_cache_conversation_events(
        &state.pool,
        &range_start_bound,
        source_scope,
        &selected_keys,
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

    let conversations = aggregates
        .into_iter()
        .map(|row| PromptCacheConversationResponse {
            prompt_cache_key: row.prompt_cache_key.clone(),
            request_count: row.request_count,
            total_tokens: row.total_tokens,
            total_cost: row.total_cost,
            created_at: row.created_at,
            last_activity_at: row.last_activity_at,
            last24h_requests: grouped_events
                .remove(&row.prompt_cache_key)
                .unwrap_or_default(),
        })
        .collect::<Vec<_>>();

    Ok(PromptCacheConversationsResponse {
        range_start: format_utc_iso(range_start),
        range_end: format_utc_iso(range_end),
        conversations,
    })
}

pub(crate) async fn query_prompt_cache_conversation_aggregates(
    pool: &Pool<Sqlite>,
    range_start_bound: &str,
    source_scope: InvocationSourceScope,
    limit: i64,
) -> Result<Vec<PromptCacheConversationAggregateRow>> {
    const KEY_EXPR: &str = "CASE WHEN json_valid(payload) THEN TRIM(CAST(json_extract(payload, '$.promptCacheKey') AS TEXT)) END";

    let mut query = QueryBuilder::<Sqlite>::new(
        "WITH active AS (\
            SELECT ",
    );
    query
        .push(KEY_EXPR)
        .push(
            " AS prompt_cache_key, MIN(occurred_at) AS first_seen_24h \
             FROM codex_invocations \
             WHERE occurred_at >= ",
        )
        .push_bind(range_start_bound);

    if source_scope == InvocationSourceScope::ProxyOnly {
        query.push(" AND source = ").push_bind(SOURCE_PROXY);
    }

    query
        .push(" AND ")
        .push(KEY_EXPR)
        .push(" IS NOT NULL AND ")
        .push(KEY_EXPR)
        .push(
            " <> '' \
             GROUP BY prompt_cache_key\
         ), aggregates AS (\
            SELECT ",
        )
        .push(KEY_EXPR)
        .push(
            " AS prompt_cache_key, \
                 COUNT(*) AS request_count, \
                 COALESCE(SUM(total_tokens), 0) AS total_tokens, \
                 COALESCE(SUM(cost), 0.0) AS total_cost, \
                 MIN(occurred_at) AS created_at, \
                 MAX(occurred_at) AS last_activity_at \
             FROM codex_invocations \
             WHERE ",
        )
        .push(KEY_EXPR)
        .push(" IN (SELECT prompt_cache_key FROM active)");

    if source_scope == InvocationSourceScope::ProxyOnly {
        query.push(" AND source = ").push_bind(SOURCE_PROXY);
    }

    query
        .push(
            " GROUP BY prompt_cache_key\
         ) \
         SELECT prompt_cache_key, request_count, total_tokens, total_cost, created_at, last_activity_at \
         FROM aggregates \
         ORDER BY created_at DESC \
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

pub(crate) async fn fetch_timeseries(
    State(state): State<Arc<AppState>>,
    Query(params): Query<TimeseriesQuery>,
) -> Result<Json<TimeseriesResponse>, ApiError> {
    let reporting_tz = parse_reporting_tz(params.time_zone.as_deref())?;
    let source_scope = resolve_default_source_scope(&state.pool).await?;
    let range_window = resolve_range_window(&params.range, reporting_tz)?;
    let mut bucket_seconds = if let Some(spec) = params.bucket.as_deref() {
        bucket_seconds_from_spec(spec)
            .ok_or_else(|| anyhow!("unsupported bucket specification: {spec}"))?
    } else {
        default_bucket_seconds(range_window.duration)
    };

    if bucket_seconds <= 0 {
        return Err(ApiError(anyhow!("bucket seconds must be positive")));
    }

    let range_seconds = range_window.duration.num_seconds();

    if range_seconds / bucket_seconds > 10_000 {
        // avoid accidentally returning extremely large payloads
        bucket_seconds = range_seconds / 10_000;
    }

    if bucket_seconds == 86_400 {
        return fetch_timeseries_daily(state, params, reporting_tz).await;
    }

    let offset_seconds = 0;

    let end_dt = range_window.end;
    let start_dt = range_window.start;
    let start_str_iso = format_utc_iso(start_dt);

    let mut records_query = QueryBuilder::new(
        "SELECT occurred_at, status, total_tokens, cost, t_upstream_ttfb_ms FROM codex_invocations WHERE occurred_at >= ",
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
    // Track the latest record timestamp only for internal stats, but do not
    // let it extend the visible range beyond "now". Some providers or clock
    // skews can produce future-dated records which previously caused the
    // time-series to expand past the requested window.
    let mut latest_record_epoch = end_dt.timestamp();

    for record in records {
        let naive = NaiveDateTime::parse_from_str(&record.occurred_at, "%Y-%m-%d %H:%M:%S")
            .map_err(|err| anyhow!("failed to parse occurred_at: {err}"))?;
        // Interpret stored naive time as local Asia/Shanghai and convert to UTC epoch
        let epoch = Shanghai
            .from_local_datetime(&naive)
            .single()
            .map(|dt| dt.with_timezone(&Utc).timestamp())
            .unwrap_or_else(|| naive.and_utc().timestamp());
        if epoch > latest_record_epoch {
            latest_record_epoch = epoch;
        }
        let bucket_epoch = align_bucket_epoch(epoch, bucket_seconds, offset_seconds);
        let entry = aggregates.entry(bucket_epoch).or_default();
        entry.total_count += 1;
        match record.status.as_deref() {
            Some("success") => entry.success_count += 1,
            _ => entry.failure_count += 1,
        }
        entry.record_ttfb_sample(record.status.as_deref(), record.t_upstream_ttfb_ms);
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
            align_bucket_epoch(delta.captured_at_epoch, bucket_seconds, offset_seconds);
        let entry = aggregates.entry(bucket_epoch).or_default();
        entry.total_count += delta.total_count;
        entry.success_count += delta.success_count;
        entry.failure_count += delta.failure_count;
        entry.total_tokens += delta.total_tokens;
        entry.total_cost += delta.total_cost;
    }

    // Compute the inclusive fill range [fill_start_epoch, fill_end_epoch].
    // Start from the aligned bucket that intersects the requested start time.
    let mut bucket_cursor = align_bucket_epoch(start_epoch, bucket_seconds, offset_seconds);
    if bucket_cursor > start_epoch {
        bucket_cursor -= bucket_seconds;
    }
    let fill_start_epoch = bucket_cursor;

    // Clamp the filled range end to the current time (aligned to the next bucket).
    // This prevents future-dated records from pushing the chart beyond the
    // intended window (e.g., "last 24 hours").
    let fill_end_epoch =
        align_bucket_epoch(end_dt.timestamp(), bucket_seconds, offset_seconds) + bucket_seconds;
    while bucket_cursor <= fill_end_epoch {
        aggregates.entry(bucket_cursor).or_default();
        bucket_cursor += bucket_seconds;
    }

    let mut points = Vec::with_capacity(aggregates.len());
    for (bucket_epoch, agg) in aggregates {
        // Skip any buckets outside the desired window. This guards against
        // future-dated records leaking past the clamped end.
        if bucket_epoch < fill_start_epoch || bucket_epoch + bucket_seconds > fill_end_epoch {
            continue;
        }
        let start = Utc
            .timestamp_opt(bucket_epoch, 0)
            .single()
            .ok_or_else(|| anyhow!("invalid bucket epoch"))?;
        let end = Utc
            .timestamp_opt(bucket_epoch + bucket_seconds, 0)
            .single()
            .ok_or_else(|| anyhow!("invalid bucket epoch"))?;
        let first_byte_avg_ms = agg.first_byte_avg_ms();
        let first_byte_p95_ms = agg.first_byte_p95_ms();
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
        points,
    };

    Ok(Json(response))
}

pub(crate) fn resolve_daily_date_range(
    spec: &str,
    now: DateTime<Utc>,
    tz: Tz,
) -> Result<(NaiveDate, NaiveDate)> {
    if let Some((start, _raw_end)) = named_range_bounds(spec, now, tz) {
        let start_local = start.with_timezone(&tz).date_naive();
        let end_local = now.with_timezone(&tz).date_naive();
        return Ok((start_local, end_local));
    }

    let duration = parse_duration_spec(spec)?;
    let mut days = duration.num_days();
    if days <= 0 {
        days = 1;
    }
    let end_local = now.with_timezone(&tz).date_naive();
    let start_local = if days <= 1 {
        end_local
    } else {
        end_local - ChronoDuration::days(days - 1)
    };

    Ok((start_local, end_local))
}

pub(crate) async fn fetch_timeseries_daily(
    state: Arc<AppState>,
    params: TimeseriesQuery,
    reporting_tz: Tz,
) -> Result<Json<TimeseriesResponse>, ApiError> {
    let now = Utc::now();
    let source_scope = resolve_default_source_scope(&state.pool).await?;
    let (start_date, end_date) = resolve_daily_date_range(&params.range, now, reporting_tz)?;

    let start_naive = start_date
        .and_hms_opt(0, 0, 0)
        .expect("midnight should be representable");
    let start_dt = local_naive_to_utc(start_naive, reporting_tz);

    let mut aggregates: BTreeMap<NaiveDate, BucketAggregate> = BTreeMap::new();
    let mut cursor = start_date;
    while cursor <= end_date {
        aggregates.entry(cursor).or_default();
        cursor = cursor
            .succ_opt()
            .unwrap_or(cursor + ChronoDuration::days(1));
    }

    let mut records_query = QueryBuilder::new(
        "SELECT occurred_at, status, total_tokens, cost, t_upstream_ttfb_ms FROM codex_invocations WHERE occurred_at >= ",
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

    for record in records {
        let occurred_utc = match parse_to_utc_datetime(&record.occurred_at) {
            Some(dt) => dt,
            None => continue,
        };
        let local_date = occurred_utc.with_timezone(&reporting_tz).date_naive();
        if local_date < start_date || local_date > end_date {
            continue;
        }
        let entry = aggregates.entry(local_date).or_default();
        entry.total_count += 1;
        match record.status.as_deref() {
            Some("success") => entry.success_count += 1,
            _ => entry.failure_count += 1,
        }
        entry.record_ttfb_sample(record.status.as_deref(), record.t_upstream_ttfb_ms);
        entry.total_tokens += record.total_tokens.unwrap_or(0);
        entry.total_cost += record.cost.unwrap_or(0.0);
    }

    if source_scope == InvocationSourceScope::All
        && let Some(relay) = state.config.crs_stats.as_ref()
    {
        let deltas =
            query_crs_deltas(&state.pool, relay, start_dt.timestamp(), now.timestamp()).await?;

        for delta in deltas {
            let captured = match Utc.timestamp_opt(delta.captured_at_epoch, 0).single() {
                Some(dt) => dt,
                None => continue,
            };
            let local_date = captured.with_timezone(&reporting_tz).date_naive();
            if local_date < start_date || local_date > end_date {
                continue;
            }
            let entry = aggregates.entry(local_date).or_default();
            entry.total_count += delta.total_count;
            entry.success_count += delta.success_count;
            entry.failure_count += delta.failure_count;
            entry.total_tokens += delta.total_tokens;
            entry.total_cost += delta.total_cost;
        }
    }

    let mut points = Vec::with_capacity(aggregates.len());
    for (date, agg) in aggregates {
        let start_naive = date
            .and_hms_opt(0, 0, 0)
            .expect("midnight should be representable");
        let end_naive = (date + ChronoDuration::days(1))
            .and_hms_opt(0, 0, 0)
            .expect("midnight should be representable");
        let start = local_naive_to_utc(start_naive, reporting_tz);
        let end = local_naive_to_utc(end_naive, reporting_tz);
        let first_byte_avg_ms = agg.first_byte_avg_ms();
        let first_byte_p95_ms = agg.first_byte_p95_ms();
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
        });
    }

    let range_start = {
        let naive = start_date
            .and_hms_opt(0, 0, 0)
            .expect("midnight should be representable");
        format_utc_iso(local_naive_to_utc(naive, reporting_tz))
    };
    let range_end = {
        let next = end_date + ChronoDuration::days(1);
        let naive = next
            .and_hms_opt(0, 0, 0)
            .expect("midnight should be representable");
        format_utc_iso(local_naive_to_utc(naive, reporting_tz))
    };

    Ok(Json(TimeseriesResponse {
        range_start,
        range_end,
        bucket_seconds: 86_400,
        points,
    }))
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
            _ => Err(ApiError(anyhow!(
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
    if err_lower.contains("upstream stream error") {
        return Some(PROXY_FAILURE_UPSTREAM_STREAM_ERROR.to_string());
    }
    if err_lower.contains("failed to contact upstream") {
        return Some(PROXY_FAILURE_FAILED_CONTACT_UPSTREAM.to_string());
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

    let failure_kind = extract_failure_kind_prefix(err)
        .or_else(|| derive_failure_kind(&status_norm, err, &err_lower));

    let failure_kind_lower = failure_kind
        .as_deref()
        .unwrap_or_default()
        .to_ascii_lowercase();
    let is_http_4xx =
        status_norm.starts_with("http_4") || status_norm == "http_401" || status_norm == "http_403";
    let is_http_5xx = status_norm.starts_with("http_5");

    let failure_class = if failure_kind_lower == PROXY_STREAM_TERMINAL_DOWNSTREAM_CLOSED
        || err_lower.contains("downstream closed while streaming upstream response")
    {
        FailureClass::ClientAbort
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
        || failure_kind_lower == PROXY_FAILURE_UPSTREAM_STREAM_ERROR
        || failure_kind_lower == PROXY_FAILURE_REQUEST_BODY_READ_TIMEOUT
        || failure_kind_lower == PROXY_FAILURE_UPSTREAM_HANDSHAKE_TIMEOUT
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
    let reporting_tz = parse_reporting_tz(params.time_zone.as_deref())?;
    let range_window = resolve_range_window(&params.range, reporting_tz)?;
    let start_dt = range_window.start;
    let display_end = range_window.display_end;
    let scope = FailureScope::parse(params.scope.as_deref())?;
    let source_scope = resolve_default_source_scope(&state.pool).await?;

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
    let reporting_tz = parse_reporting_tz(params.time_zone.as_deref())?;
    let range_window = resolve_range_window(&params.range, reporting_tz)?;
    let start_dt = range_window.start;
    let display_end = range_window.display_end;
    let source_scope = resolve_default_source_scope(&state.pool).await?;

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

pub(crate) async fn get_settings(
    State(state): State<Arc<AppState>>,
) -> Result<Json<SettingsResponse>, ApiError> {
    let proxy = state.proxy_model_settings.read().await.clone();
    let pricing = state.pricing_catalog.read().await.clone();
    let forward_proxy = build_forward_proxy_settings_response(state.as_ref()).await?;
    Ok(Json(SettingsResponse {
        proxy: proxy.into(),
        forward_proxy,
        pricing: PricingSettingsResponse::from_catalog(&pricing),
    }))
}

pub(crate) async fn removed_proxy_model_settings_endpoint() -> (StatusCode, &'static str) {
    (
        StatusCode::NOT_FOUND,
        "endpoint removed; use /api/settings and /api/settings/proxy",
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

    let next = ProxyModelSettings {
        hijack_enabled: payload.hijack_enabled,
        merge_upstream_enabled: payload.merge_upstream_enabled,
        fast_mode_rewrite_mode: payload.fast_mode_rewrite_mode,
        enabled_preset_models: payload.enabled_models,
    }
    .normalized();
    let _update_guard = state.proxy_model_settings_update_lock.lock().await;
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
    #[sqlx(default)]
    pub(crate) raw_expires_at: Option<String>,
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

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StatsResponse {
    pub(crate) total_count: i64,
    pub(crate) success_count: i64,
    pub(crate) failure_count: i64,
    pub(crate) total_cost: f64,
    pub(crate) total_tokens: i64,
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
    pub(crate) points: Vec<TimeseriesPoint>,
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
    pub(crate) conversations: Vec<PromptCacheConversationResponse>,
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
    pub(crate) last24h_requests: Vec<PromptCacheConversationRequestPointResponse>,
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
    pub(crate) response: PromptCacheConversationsResponse,
}

#[derive(Debug)]
pub(crate) struct PromptCacheConversationInFlight {
    pub(crate) signal: watch::Sender<bool>,
}

#[derive(Debug, Default)]
pub(crate) struct PromptCacheConversationsCacheState {
    pub(crate) entries: HashMap<i64, PromptCacheConversationsCacheEntry>,
    pub(crate) in_flight: HashMap<i64, PromptCacheConversationInFlight>,
}

#[derive(Debug)]
pub(crate) struct PromptCacheConversationFlightGuard {
    pub(crate) cache: Arc<Mutex<PromptCacheConversationsCacheState>>,
    pub(crate) limit: i64,
    pub(crate) active: bool,
}

impl PromptCacheConversationFlightGuard {
    pub(crate) fn new(cache: Arc<Mutex<PromptCacheConversationsCacheState>>, limit: i64) -> Self {
        Self {
            cache,
            limit,
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
        let limit = self.limit;
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            handle.spawn(async move {
                let mut state = cache.lock().await;
                if let Some(in_flight) = state.in_flight.remove(&limit) {
                    let _ = in_flight.signal.send(true);
                }
            });
            return;
        }

        if let Ok(mut state) = cache.try_lock()
            && let Some(in_flight) = state.in_flight.remove(&limit)
        {
            let _ = in_flight.signal.send(true);
        }
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
    pub(crate) payload: Option<String>,
    pub(crate) raw_response: String,
    pub(crate) req_raw: RawPayloadMeta,
    pub(crate) resp_raw: RawPayloadMeta,
    pub(crate) raw_expires_at: Option<String>,
    pub(crate) timings: StageTimings,
}

#[derive(Debug)]
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
    pub(crate) proxy: Option<String>,
    pub(crate) endpoint: Option<String>,
    pub(crate) failure_class: Option<String>,
    pub(crate) failure_kind: Option<String>,
    pub(crate) prompt_cache_key: Option<String>,
    pub(crate) requester_ip: Option<String>,
    pub(crate) keyword: Option<String>,
    pub(crate) min_total_tokens: Option<i64>,
    pub(crate) max_total_tokens: Option<i64>,
    pub(crate) min_total_ms: Option<f64>,
    pub(crate) max_total_ms: Option<f64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PromptCacheConversationsQuery {
    pub(crate) limit: Option<i64>,
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
