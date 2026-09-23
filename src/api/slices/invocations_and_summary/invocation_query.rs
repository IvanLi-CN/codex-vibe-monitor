use super::*;

pub(crate) fn build_invocation_select_query() -> QueryBuilder<'static, Sqlite> {
    let final_first_token_timing_sql =
        final_pool_invocation_timing_sql("codex_invocations", "first_token_ms");
    let final_live_phase_sql = invocation_live_phase_sql_with_timing_sql(
        "codex_invocations",
        final_first_token_timing_sql.as_str(),
    );
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
         cache_input_tokens, reported_cache_write_tokens, reasoning_tokens, \
         ",
        )
        .push(INVOCATION_REASONING_EFFORT_SQL)
        .push(
            " AS reasoning_effort, \
         total_tokens, cost, status, \
         ",
        )
        .push(final_live_phase_sql.as_str())
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
        .push(invocation_request_compression_algorithm_with_attempt_fallback_sql(
            "codex_invocations",
        ))
        .push(
            " AS request_compression_algorithm, \
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
         ",
        )
        .push(final_first_token_timing_sql.as_str())
        .push(
            " AS first_token_ms, \
         ",
        )
        .push(final_pool_invocation_timing_sql("codex_invocations", "t_upstream_stream_ms").as_str())
        .push(
            " AS t_upstream_stream_ms, t_resp_parse_ms, t_persist_ms, \
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum InvocationModelTarget {
    Request,
    Response,
}

impl InvocationModelTarget {
    fn parse(raw: Option<&str>) -> Result<Option<Self>, ApiError> {
        let Some(value) = normalize_query_text(raw) else {
            return Ok(None);
        };
        match value.to_ascii_lowercase().as_str() {
            "request" => Ok(Some(Self::Request)),
            "response" => Ok(Some(Self::Response)),
            _ => Err(ApiError::bad_request(anyhow!(
                "unsupported modelTarget: {value}"
            ))),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum InvocationModelRerouteFilter {
    Rerouted,
    NotRerouted,
}

impl InvocationModelRerouteFilter {
    fn parse(raw: Option<&str>) -> Result<Option<Self>, ApiError> {
        let Some(value) = normalize_query_text(raw) else {
            return Ok(None);
        };
        match value.to_ascii_lowercase().as_str() {
            "1" | "true" | "yes" => Ok(Some(Self::Rerouted)),
            "0" | "false" | "no" => Ok(Some(Self::NotRerouted)),
            _ => Err(ApiError::bad_request(anyhow!(
                "invalid modelRerouted: {value}"
            ))),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub(crate) struct InvocationRecordsFilters {
    pub(crate) occurred_from: Option<String>,
    pub(crate) occurred_to: Option<String>,
    pub(crate) status: Option<String>,
    pub(crate) model: Option<String>,
    pub(crate) model_values: Vec<String>,
    pub(crate) model_target: Option<InvocationModelTarget>,
    pub(crate) model_rerouted: Option<InvocationModelRerouteFilter>,
    pub(crate) endpoint: Option<String>,
    pub(crate) request_id: Option<String>,
    pub(crate) failure_class: Option<String>,
    pub(crate) failure_kind: Option<String>,
    pub(crate) prompt_cache_key: Option<String>,
    pub(crate) sticky_key: Option<String>,
    pub(crate) upstream_scope: Option<String>,
    pub(crate) upstream_account_id: Option<i64>,
    pub(crate) proxy_display_name: Option<String>,
    pub(crate) transport: Option<String>,
    pub(crate) service_tier: Option<String>,
    pub(crate) reasoning_effort: Option<String>,
    pub(crate) reasoning_effort_values: Vec<String>,
    pub(crate) requester_ip: Option<String>,
    pub(crate) keyword: Option<String>,
    pub(crate) min_total_tokens: Option<i64>,
    pub(crate) max_total_tokens: Option<i64>,
    pub(crate) min_total_ms: Option<f64>,
    pub(crate) max_total_ms: Option<f64>,
}

#[derive(Debug, Clone)]
pub(crate) struct InvocationListRequest {
    pub(super) filters: InvocationRecordsFilters,
    pub(super) page: i64,
    pub(super) page_size: i64,
    pub(super) sort_by: InvocationSortBy,
    pub(super) sort_order: InvocationSortOrder,
    pub(super) snapshot_id: Option<i64>,
}

#[derive(Debug, FromRow)]
pub(crate) struct InvocationSummaryAggRow {
    pub(super) total_count: i64,
    pub(super) success_count: i64,
    pub(super) failure_count: i64,
    pub(super) total_tokens: i64,
    pub(super) total_cost: f64,
    pub(super) cache_write_tokens: i64,
    pub(super) cache_input_tokens: i64,
    pub(super) output_tokens: i64,
    pub(super) max_total_tokens: Option<i64>,
}

#[derive(Debug, FromRow)]
pub(crate) struct InvocationNetworkAggRow {
    pub(super) avg_ttfb_ms: Option<f64>,
    pub(super) ttfb_count: i64,
    pub(super) avg_first_token_ms: Option<f64>,
    pub(super) first_token_count: i64,
    pub(super) avg_response_duration_ms: Option<f64>,
    pub(super) response_duration_count: i64,
    pub(super) avg_total_ms: Option<f64>,
    pub(super) total_count: i64,
    pub(super) max_total_ms: Option<f64>,
}

#[derive(Debug, FromRow)]
pub(crate) struct InvocationExceptionAggRow {
    pub(super) failure_count: i64,
    pub(super) service_failure_count: i64,
    pub(super) client_failure_count: i64,
    pub(super) client_abort_count: i64,
    pub(super) actionable_failure_count: i64,
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
    pub(crate) max_tokens_per_request: Option<i64>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct InvocationNetworkSummary {
    pub(crate) avg_ttfb_ms: Option<f64>,
    pub(crate) p95_ttfb_ms: Option<f64>,
    pub(crate) avg_first_token_ms: Option<f64>,
    pub(crate) p95_first_token_ms: Option<f64>,
    pub(crate) avg_response_duration_ms: Option<f64>,
    pub(crate) p95_response_duration_ms: Option<f64>,
    pub(crate) avg_total_ms: Option<f64>,
    pub(crate) p95_total_ms: Option<f64>,
    pub(crate) max_total_ms: Option<f64>,
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
pub(crate) struct InvocationHistoryOverviewResponse {
    pub(crate) summary: InvocationSummaryResponse,
    pub(crate) records: Vec<ApiInvocation>,
    pub(crate) chart_total: i64,
    pub(crate) chart_is_sampled: bool,
    pub(crate) chart_range_start: Option<String>,
    pub(crate) chart_range_end: Option<String>,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) label: Option<String>,
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
    pub(crate) request_model: InvocationSuggestionBucket,
    pub(crate) response_model: InvocationSuggestionBucket,
    pub(crate) endpoint: InvocationSuggestionBucket,
    pub(crate) failure_kind: InvocationSuggestionBucket,
    pub(crate) sticky_key: InvocationSuggestionBucket,
    pub(crate) prompt_cache_key: InvocationSuggestionBucket,
    pub(crate) requester_ip: InvocationSuggestionBucket,
    pub(crate) proxy_display_name: InvocationSuggestionBucket,
    pub(crate) upstream_account: InvocationSuggestionBucket,
    pub(crate) service_tier: InvocationSuggestionBucket,
    pub(crate) reasoning_effort: InvocationSuggestionBucket,
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

pub(crate) fn parse_query_text_list(raw: Option<&str>) -> Vec<String> {
    let mut deduped = Vec::new();
    let mut seen = HashSet::new();
    for value in raw.into_iter().flat_map(|value| value.split(',')) {
        let Some(normalized) = normalize_query_text(Some(value)) else {
            continue;
        };
        let key = normalized.to_ascii_lowercase();
        if seen.insert(key) {
            deduped.push(normalized);
        }
    }
    deduped
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

    let model_values = parse_query_text_list(params.models.as_deref());
    let model_target = InvocationModelTarget::parse(params.model_target.as_deref())?;
    let model_rerouted = InvocationModelRerouteFilter::parse(params.model_rerouted.as_deref())?;
    let reasoning_effort_values = parse_query_text_list(params.reasoning_efforts.as_deref());

    Ok(InvocationRecordsFilters {
        occurred_from,
        occurred_to,
        status: normalize_query_text(params.status.as_deref()),
        model: normalize_query_text(params.model.as_deref()),
        model_values,
        model_target,
        model_rerouted,
        endpoint: normalize_query_text(params.endpoint.as_deref()),
        request_id: normalize_query_text(params.invoke_id.as_deref())
            .or_else(|| normalize_query_text(params.request_id.as_deref())),
        failure_class: normalize_query_text(params.failure_class.as_deref()),
        failure_kind: normalize_query_text(params.failure_kind.as_deref()),
        prompt_cache_key: normalize_query_text(params.prompt_cache_key.as_deref()),
        sticky_key: normalize_query_text(params.sticky_key.as_deref()),
        upstream_scope: match normalize_query_text(params.upstream_scope.as_deref()) {
            Some(value) if value.eq_ignore_ascii_case("all") => None,
            other => other,
        },
        upstream_account_id: params.upstream_account_id,
        proxy_display_name: normalize_query_text(params.proxy_display_name.as_deref()),
        transport: match normalize_query_text(params.transport.as_deref()) {
            Some(value) if value.eq_ignore_ascii_case("all") => None,
            other => other,
        },
        service_tier: normalize_query_text(params.service_tier.as_deref()),
        reasoning_effort: normalize_query_text(params.reasoning_effort.as_deref()),
        reasoning_effort_values,
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

async fn resolve_attempt_public_id_to_invoke_id(
    pool: &Pool<Sqlite>,
    attempt_public_id: &str,
) -> Result<Option<String>, ApiError> {
    #[derive(Debug, FromRow)]
    struct AttemptLookupRow {
        invoke_id: String,
    }

    sqlx::query_as::<_, AttemptLookupRow>(
        r#"
        SELECT invoke_id
        FROM pool_upstream_request_attempts
        WHERE attempt_public_id = ?1
        LIMIT 1
        "#,
    )
    .bind(attempt_public_id)
    .fetch_optional(pool)
    .await
    .map(|row| row.map(|value| value.invoke_id))
    .map_err(ApiError::from)
}

pub(crate) async fn build_resolved_invocation_list_request(
    pool: &Pool<Sqlite>,
    params: &ListQuery,
    list_limit_max: i64,
) -> Result<InvocationListRequest, ApiError> {
    let mut request = build_invocation_list_request(params, list_limit_max)?;
    let Some(attempt_id) = normalize_query_text(params.attempt_id.as_deref()) else {
        return Ok(request);
    };

    let Some(resolved_invoke_id) =
        resolve_attempt_public_id_to_invoke_id(pool, &attempt_id).await?
    else {
        request.filters.request_id = Some(format!("__missing_attempt__:{attempt_id}"));
        return Ok(request);
    };

    if request
        .filters
        .request_id
        .as_deref()
        .is_some_and(|invoke_id| !invoke_id.eq_ignore_ascii_case(&resolved_invoke_id))
    {
        request.filters.request_id = Some(format!("__invoke_attempt_mismatch__:{attempt_id}"));
        return Ok(request);
    }

    request.filters.request_id = Some(resolved_invoke_id);
    Ok(request)
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

pub(crate) fn push_exact_text_any_filter(
    query: &mut QueryBuilder<Sqlite>,
    sql_expr: &str,
    values: &[String],
) {
    if values.is_empty() {
        return;
    }
    query.push(" AND (");
    for (index, value) in values.iter().enumerate() {
        if index > 0 {
            query.push(" OR ");
        }
        query.push("LOWER(TRIM(COALESCE(");
        query.push(sql_expr);
        query.push(", ''))) = ");
        query.push_bind(value.to_lowercase());
    }
    query.push(")");
}

pub(crate) fn invocation_model_sql_expr(target: InvocationModelTarget) -> String {
    let target_sql = match target {
        InvocationModelTarget::Request => INVOCATION_REQUEST_MODEL_SQL,
        InvocationModelTarget::Response => INVOCATION_RESPONSE_MODEL_SQL,
    };
    format!("COALESCE(NULLIF(TRIM({target_sql}), ''), NULLIF(TRIM(model), ''))")
}

pub(crate) fn invocation_model_rerouted_sql() -> String {
    let request_sql = invocation_model_sql_expr(InvocationModelTarget::Request);
    let response_sql = invocation_model_sql_expr(InvocationModelTarget::Response);
    format!(
        "NULLIF(TRIM(COALESCE({request_sql}, '')), '') IS NOT NULL \
         AND NULLIF(TRIM(COALESCE({response_sql}, '')), '') IS NOT NULL \
         AND LOWER(TRIM(COALESCE({request_sql}, ''))) != LOWER(TRIM(COALESCE({response_sql}, '')))"
    )
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

    if !filters.model_values.is_empty() {
        let target = filters
            .model_target
            .unwrap_or(InvocationModelTarget::Request);
        let model_sql = invocation_model_sql_expr(target);
        push_exact_text_any_filter(query, &model_sql, &filters.model_values);
    } else if let Some(model) = filters.model.as_deref() {
        push_exact_text_filter(query, "model", model);
    }

    if let Some(model_rerouted) = filters.model_rerouted {
        query.push(" AND ");
        match model_rerouted {
            InvocationModelRerouteFilter::Rerouted => {
                query.push(invocation_model_rerouted_sql());
            }
            InvocationModelRerouteFilter::NotRerouted => {
                query.push("NOT (");
                query.push(invocation_model_rerouted_sql());
                query.push(")");
            }
        }
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

    if let Some(proxy_display_name) = filters.proxy_display_name.as_deref() {
        push_exact_text_filter(query, INVOCATION_PROXY_DISPLAY_SQL, proxy_display_name);
    }

    if let Some(transport) = filters.transport.as_deref() {
        push_exact_text_filter(query, INVOCATION_TRANSPORT_SQL, transport);
    }

    if let Some(service_tier) = filters.service_tier.as_deref() {
        push_exact_text_filter(query, INVOCATION_SERVICE_TIER_SQL, service_tier);
    }

    if !filters.reasoning_effort_values.is_empty() {
        push_exact_text_any_filter(
            query,
            INVOCATION_REASONING_EFFORT_SQL,
            &filters.reasoning_effort_values,
        );
    } else if let Some(reasoning_effort) = filters.reasoning_effort.as_deref() {
        push_exact_text_filter(query, INVOCATION_REASONING_EFFORT_SQL, reasoning_effort);
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
    let mut connection = pool.acquire().await?;
    resolve_invocation_snapshot_id_on_connection(&mut connection, source_scope).await
}

pub(crate) async fn resolve_invocation_snapshot_id_on_connection(
    connection: &mut SqliteConnection,
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
        .fetch_one(connection)
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
