pub(crate) struct InvocationRuntimeOverlayInput<'a> {
    pub(crate) request: &'a InvocationListRequest,
    pub(crate) source_scope: InvocationSourceScope,
    pub(crate) runtime_records: Vec<ApiInvocation>,
    pub(crate) db_runtime_keys: &'a HashSet<(String, String)>,
    pub(crate) db_terminal_keys: &'a HashSet<(String, String)>,
    pub(crate) db_records_are_prefix: bool,
    pub(crate) records: Vec<ApiInvocation>,
    pub(crate) total: i64,
    pub(crate) endpoint: &'static str,
}

pub(crate) fn overlay_runtime_records_for_current_page(
    input: InvocationRuntimeOverlayInput<'_>,
) -> (Vec<ApiInvocation>, i64) {
    overlay_runtime_records_for_current_page_inner(input)
}

fn overlay_runtime_records_for_current_page_inner(
    InvocationRuntimeOverlayInput {
        request,
        source_scope,
        runtime_records,
        db_runtime_keys,
        db_terminal_keys,
        db_records_are_prefix,
        mut records,
        total,
        endpoint,
    }: InvocationRuntimeOverlayInput<'_>,
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
    records =
        paginate_overlayed_runtime_invocation_records(records, request, db_records_are_prefix);
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

fn paginate_overlayed_runtime_invocation_records(
    records: Vec<ApiInvocation>,
    request: &InvocationListRequest,
    db_records_are_prefix: bool,
) -> Vec<ApiInvocation> {
    if db_records_are_prefix {
        let offset = (request.page - 1).saturating_mul(request.page_size) as usize;
        records
            .into_iter()
            .skip(offset)
            .take(request.page_size as usize)
            .collect()
    } else {
        records
            .into_iter()
            .take(request.page_size as usize)
            .collect()
    }
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
    max_total_tokens: Option<i64>,
    max_total_ms: Option<f64>,
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

        if let Some(total_tokens) = record.total_tokens.filter(|value| *value >= 0) {
            self.max_total_tokens = match self.max_total_tokens {
                Some(current) => Some(current.max(total_tokens)),
                None => Some(total_tokens),
            };
        }

        if let Some(total_ms) = record
            .t_total_ms
            .filter(|value| value.is_finite() && *value >= 0.0)
        {
            self.max_total_ms = match self.max_total_ms {
                Some(current) => Some(current.max(total_ms)),
                None => Some(total_ms),
            };
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
    let mut connection = pool.acquire().await?;
    query_invocation_network_summary_on_connection(
        &mut connection,
        filters,
        source_scope,
        snapshot_id,
    )
    .await
}

async fn query_invocation_network_summary_on_connection(
    connection: &mut SqliteConnection,
    filters: &InvocationRecordsFilters,
    source_scope: InvocationSourceScope,
    snapshot_id: i64,
) -> Result<InvocationNetworkSummary> {
    let agg = query_invocation_network_aggregate_on_connection(
        connection,
        filters,
        source_scope,
        snapshot_id,
    )
    .await?;
    Ok(InvocationNetworkSummary {
        avg_ttfb_ms: agg.avg_ttfb_ms,
        p95_ttfb_ms: query_invocation_network_p95(
            connection,
            filters,
            source_scope,
            snapshot_id,
            "t_upstream_ttfb_ms",
            agg.ttfb_count,
        )
        .await?,
        avg_first_token_ms: agg.avg_first_token_ms,
        p95_first_token_ms: query_invocation_network_p95(
            connection,
            filters,
            source_scope,
            snapshot_id,
            "first_token_ms",
            agg.first_token_count,
        )
        .await?,
        avg_response_duration_ms: agg.avg_response_duration_ms,
        p95_response_duration_ms: query_invocation_network_p95(
            connection,
            filters,
            source_scope,
            snapshot_id,
            "t_upstream_stream_ms",
            agg.response_duration_count,
        )
        .await?,
        avg_total_ms: agg.avg_total_ms,
        p95_total_ms: query_invocation_network_p95(
            connection,
            filters,
            source_scope,
            snapshot_id,
            "t_total_ms",
            agg.total_count,
        )
        .await?,
        max_total_ms: agg.max_total_ms,
    })
}

#[derive(Debug, FromRow)]
struct InvocationNetworkValueRow {
    value: Option<f64>,
}

async fn query_invocation_network_aggregate_on_connection(
    connection: &mut SqliteConnection,
    filters: &InvocationRecordsFilters,
    source_scope: InvocationSourceScope,
    snapshot_id: i64,
) -> Result<InvocationNetworkAggRow> {
    let ttfb_nonnegative_sql = sqlite_nonnegative_timing_sql("t_upstream_ttfb_ms");
    let final_first_token_sql =
        final_pool_invocation_timing_sql("codex_invocations", "first_token_ms");
    let final_stream_sql =
        final_pool_invocation_timing_sql("codex_invocations", "t_upstream_stream_ms");
    let total_nonnegative_sql = sqlite_nonnegative_timing_sql("t_total_ms");
    let mut query = QueryBuilder::new(format!(
        "SELECT \
         AVG(CASE WHEN {ttfb_nonnegative_sql} THEN t_upstream_ttfb_ms END) AS avg_ttfb_ms, \
         COALESCE(SUM(CASE WHEN {ttfb_nonnegative_sql} THEN 1 ELSE 0 END), 0) AS ttfb_count, \
         AVG({final_first_token_sql}) AS avg_first_token_ms, \
         COALESCE(SUM(CASE WHEN ({final_first_token_sql}) IS NOT NULL THEN 1 ELSE 0 END), 0) AS first_token_count, \
         AVG({final_stream_sql}) AS avg_response_duration_ms, \
         COALESCE(SUM(CASE WHEN ({final_stream_sql}) IS NOT NULL THEN 1 ELSE 0 END), 0) AS response_duration_count, \
         AVG(CASE WHEN {total_nonnegative_sql} THEN t_total_ms END) AS avg_total_ms, \
         COALESCE(SUM(CASE WHEN {total_nonnegative_sql} THEN 1 ELSE 0 END), 0) AS total_count, \
         MAX(CASE WHEN {total_nonnegative_sql} THEN t_total_ms END) AS max_total_ms \
         FROM codex_invocations WHERE 1 = 1"
    ));
    apply_invocation_records_filters(
        &mut query,
        filters,
        source_scope,
        Some(SnapshotConstraint::UpTo(snapshot_id)),
    );
    Ok(query
        .build_query_as::<InvocationNetworkAggRow>()
        .fetch_one(&mut *connection)
        .await?)
}

async fn query_invocation_network_sorted_value(
    connection: &mut SqliteConnection,
    filters: &InvocationRecordsFilters,
    source_scope: InvocationSourceScope,
    snapshot_id: i64,
    column: &'static str,
    offset: i64,
) -> Result<Option<f64>> {
    let timing_value_sql = match column {
        "first_token_ms" | "t_upstream_stream_ms" => {
            final_pool_invocation_timing_sql("codex_invocations", column)
        }
        _ => column.to_string(),
    };
    let mut query = QueryBuilder::new("SELECT ");
    query
        .push(timing_value_sql.as_str())
        .push(" AS value FROM codex_invocations WHERE 1 = 1");
    apply_invocation_records_filters(
        &mut query,
        filters,
        source_scope,
        Some(SnapshotConstraint::UpTo(snapshot_id)),
    );
    let timing_sql = match column {
        "first_token_ms" | "t_upstream_stream_ms" => {
            format!("({timing_value_sql}) IS NOT NULL")
        }
        _ => sqlite_nonnegative_timing_sql(column),
    };
    query.push(" AND ").push(timing_sql.as_str());
    query.push(" ORDER BY value ASC");
    query.push(" LIMIT 1 OFFSET ").push_bind(offset.max(0));

    Ok(query
        .build_query_as::<InvocationNetworkValueRow>()
        .fetch_optional(connection)
        .await?
        .and_then(|row| row.value))
}

fn resolve_invocation_network_p95_offsets(count: i64) -> Option<(i64, i64, f64)> {
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

async fn query_invocation_network_p95(
    connection: &mut SqliteConnection,
    filters: &InvocationRecordsFilters,
    source_scope: InvocationSourceScope,
    snapshot_id: i64,
    column: &'static str,
    count: i64,
) -> Result<Option<f64>> {
    let Some((lower, upper, weight)) = resolve_invocation_network_p95_offsets(count) else {
        return Ok(None);
    };
    let Some(lower_value) = query_invocation_network_sorted_value(
        connection,
        filters,
        source_scope,
        snapshot_id,
        column,
        lower,
    )
    .await?
    else {
        return Ok(None);
    };
    if lower == upper {
        return Ok(Some(lower_value));
    }
    let Some(upper_value) = query_invocation_network_sorted_value(
        connection,
        filters,
        source_scope,
        snapshot_id,
        column,
        upper,
    )
    .await?
    else {
        return Ok(None);
    };
    Ok(Some(lower_value + (upper_value - lower_value) * weight))
}

pub(crate) async fn query_invocation_new_records_count(
    pool: &Pool<Sqlite>,
    filters: &InvocationRecordsFilters,
    source_scope: InvocationSourceScope,
    snapshot_id: i64,
) -> Result<i64> {
    let mut connection = pool.acquire().await?;
    query_invocation_new_records_count_on_connection(
        &mut connection,
        filters,
        source_scope,
        snapshot_id,
    )
    .await
}

async fn query_invocation_new_records_count_on_connection(
    connection: &mut SqliteConnection,
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
        .fetch_one(connection)
        .await?
        .total)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum InvocationSuggestionField {
    Model,
    RequestModel,
    ResponseModel,
    Endpoint,
    FailureKind,
    StickyKey,
    PromptCacheKey,
    RequesterIp,
    ProxyDisplayName,
    UpstreamAccount,
    ServiceTier,
    ReasoningEffort,
}

impl InvocationSuggestionField {
    fn parse(raw: Option<&str>) -> Result<Option<Self>, ApiError> {
        let Some(value) = normalize_query_text(raw) else {
            return Ok(None);
        };
        let normalized = value.to_ascii_lowercase();

        match normalized.as_str() {
            "model" => Ok(Some(Self::Model)),
            "requestmodel" => Ok(Some(Self::RequestModel)),
            "responsemodel" => Ok(Some(Self::ResponseModel)),
            "endpoint" => Ok(Some(Self::Endpoint)),
            "failurekind" => Ok(Some(Self::FailureKind)),
            "stickykey" => Ok(Some(Self::StickyKey)),
            "promptcachekey" => Ok(Some(Self::PromptCacheKey)),
            "requesterip" => Ok(Some(Self::RequesterIp)),
            "proxydisplayname" => Ok(Some(Self::ProxyDisplayName)),
            "upstreamaccount" => Ok(Some(Self::UpstreamAccount)),
            "servicetier" => Ok(Some(Self::ServiceTier)),
            "reasoningeffort" => Ok(Some(Self::ReasoningEffort)),
            _ => Err(ApiError::bad_request(anyhow!(
                "unsupported suggestField: {value}"
            ))),
        }
    }

    fn sql_expr(self) -> String {
        match self {
            Self::Model => "model".to_string(),
            Self::RequestModel => invocation_model_sql_expr(InvocationModelTarget::Request),
            Self::ResponseModel => invocation_model_sql_expr(InvocationModelTarget::Response),
            Self::Endpoint => INVOCATION_ENDPOINT_SQL.to_string(),
            Self::FailureKind => INVOCATION_FAILURE_KIND_SQL.to_string(),
            Self::StickyKey => INVOCATION_STICKY_KEY_SQL.to_string(),
            Self::PromptCacheKey => INVOCATION_PROMPT_CACHE_KEY_SQL.to_string(),
            Self::RequesterIp => INVOCATION_REQUESTER_IP_SQL.to_string(),
            Self::ProxyDisplayName => INVOCATION_PROXY_DISPLAY_SQL.to_string(),
            Self::UpstreamAccount => INVOCATION_UPSTREAM_ACCOUNT_ID_SQL.to_string(),
            Self::ServiceTier => INVOCATION_SERVICE_TIER_SQL.to_string(),
            Self::ReasoningEffort => INVOCATION_REASONING_EFFORT_SQL.to_string(),
        }
    }

    fn clear_field_filter(self, filters: &InvocationRecordsFilters) -> InvocationRecordsFilters {
        let mut next = filters.clone();
        match self {
            Self::Model | Self::RequestModel | Self::ResponseModel => {
                next.model = None;
                next.model_values.clear();
            }
            Self::Endpoint => next.endpoint = None,
            Self::FailureKind => next.failure_kind = None,
            Self::StickyKey => next.sticky_key = None,
            Self::PromptCacheKey => next.prompt_cache_key = None,
            Self::RequesterIp => next.requester_ip = None,
            Self::ProxyDisplayName => next.proxy_display_name = None,
            Self::UpstreamAccount => next.upstream_account_id = None,
            Self::ServiceTier => next.service_tier = None,
            Self::ReasoningEffort => {
                next.reasoning_effort = None;
                next.reasoning_effort_values.clear();
            }
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
    let mut response = InvocationSuggestionsResponse {
        model: empty(),
        request_model: empty(),
        response_model: empty(),
        endpoint: empty(),
        failure_kind: empty(),
        sticky_key: empty(),
        prompt_cache_key: empty(),
        requester_ip: empty(),
        proxy_display_name: empty(),
        upstream_account: empty(),
        service_tier: empty(),
        reasoning_effort: empty(),
    };
    match field {
        InvocationSuggestionField::Model => response.model = bucket,
        InvocationSuggestionField::RequestModel => response.request_model = bucket,
        InvocationSuggestionField::ResponseModel => response.response_model = bucket,
        InvocationSuggestionField::Endpoint => response.endpoint = bucket,
        InvocationSuggestionField::FailureKind => response.failure_kind = bucket,
        InvocationSuggestionField::StickyKey => response.sticky_key = bucket,
        InvocationSuggestionField::PromptCacheKey => response.prompt_cache_key = bucket,
        InvocationSuggestionField::RequesterIp => response.requester_ip = bucket,
        InvocationSuggestionField::ProxyDisplayName => response.proxy_display_name = bucket,
        InvocationSuggestionField::UpstreamAccount => response.upstream_account = bucket,
        InvocationSuggestionField::ServiceTier => response.service_tier = bucket,
        InvocationSuggestionField::ReasoningEffort => response.reasoning_effort = bucket,
    }
    response
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
                    label: None,
                    count: row.count,
                })
            }
        })
        .collect::<Vec<_>>();

    Ok(InvocationSuggestionBucket { items, has_more })
}

pub(crate) async fn query_invocation_upstream_account_suggestion_bucket(
    pool: &Pool<Sqlite>,
    filters: &InvocationRecordsFilters,
    source_scope: InvocationSourceScope,
    snapshot: Option<SnapshotConstraint>,
    match_query: Option<&str>,
    limit: i64,
) -> Result<InvocationSuggestionBucket> {
    #[derive(Debug, FromRow)]
    struct SuggestionRow {
        value: Option<i64>,
        label: Option<String>,
        count: i64,
    }

    let mut query = QueryBuilder::new(
        "SELECT \
            upstream_account_id AS value, \
            COALESCE( \
                MIN(NULLIF(TRIM(COALESCE(upstream_account_name, '')), '')), \
                CAST(upstream_account_id AS TEXT) \
            ) AS label, \
            COUNT(*) AS count \
         FROM ( \
            SELECT \
                ",
    );
    query.push(INVOCATION_UPSTREAM_ACCOUNT_ID_SQL);
    query.push(" AS upstream_account_id, ");
    query.push(INVOCATION_UPSTREAM_ACCOUNT_NAME_SQL);
    query.push(" AS upstream_account_name FROM codex_invocations WHERE 1 = 1");
    apply_invocation_records_filters(&mut query, filters, source_scope, snapshot);
    query.push(") scoped WHERE upstream_account_id IS NOT NULL");
    if let Some(match_query) = match_query {
        let normalized_query = match_query.trim().to_lowercase();
        let like_pattern = format!("%{}%", escape_sql_like(&normalized_query));
        query.push(" AND (LOWER(TRIM(COALESCE(upstream_account_name, ''))) LIKE ");
        query.push_bind(like_pattern.clone()).push(" ESCAPE '\\'");
        query.push(" OR CAST(upstream_account_id AS TEXT) LIKE ");
        query.push_bind(like_pattern).push(" ESCAPE '\\')");
    }
    query.push(
        " GROUP BY upstream_account_id \
          ORDER BY count DESC, label ASC, value ASC \
          LIMIT ",
    );
    query.push_bind(limit.saturating_add(1));

    let rows = query
        .build_query_as::<SuggestionRow>()
        .fetch_all(pool)
        .await?;

    let has_more = rows.len() as i64 > limit;
    let items = rows
        .into_iter()
        .take(limit.max(0) as usize)
        .filter_map(|row| {
            let value = row.value?;
            let name = row
                .label
                .as_deref()
                .map(str::trim)
                .filter(|name| !name.is_empty());
            let label = match name {
                Some(name) if name == value.to_string() => format!("#{value}"),
                Some(name) => format!("{name} (#{value})"),
                None => format!("#{value}"),
            };
            Some(InvocationSuggestionItem {
                value: value.to_string(),
                label: Some(label),
                count: row.count,
            })
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
        && params.models.is_none()
        && params.model_target.is_none()
        && params.model_rerouted.is_none()
        && params.endpoint.is_none()
        && params.invoke_id.is_none()
        && params.attempt_id.is_none()
        && params.request_id.is_none()
        && params.failure_class.is_none()
        && params.failure_kind.is_none()
        && params.prompt_cache_key.is_none()
        && params.sticky_key.is_none()
        && params.upstream_scope.is_none()
        && params.upstream_account_id.is_none()
        && params.proxy_display_name.is_none()
        && params.transport.is_none()
        && params.service_tier.is_none()
        && params.reasoning_effort.is_none()
        && params.reasoning_efforts.is_none()
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
    let mut connection = pool.acquire().await?;
    query_invocation_exception_summary_on_connection(
        &mut connection,
        filters,
        source_scope,
        snapshot_id,
    )
    .await
}

async fn query_invocation_exception_summary_on_connection(
    connection: &mut SqliteConnection,
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
        .fetch_one(connection)
        .await?;
    Ok(InvocationExceptionSummary {
        failure_count: agg.failure_count,
        service_failure_count: agg.service_failure_count,
        client_failure_count: agg.client_failure_count,
        client_abort_count: agg.client_abort_count,
        actionable_failure_count: agg.actionable_failure_count,
    })
}
