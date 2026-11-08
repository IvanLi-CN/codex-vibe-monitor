use super::*;
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
    pub(crate) fn parse(raw: Option<&str>) -> Result<Option<Self>, ApiError> {
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

    pub(crate) fn sql_expr(self) -> String {
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

    pub(crate) fn clear_field_filter(
        self,
        filters: &InvocationRecordsFilters,
    ) -> InvocationRecordsFilters {
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

pub(crate) async fn query_invocation_exception_summary_on_connection(
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

pub(crate) async fn list_invocations(
    State(state): State<Arc<AppState>>,
    Query(params): Query<ListQuery>,
) -> Result<Json<ListResponse>, ApiError> {
    let runtime_overlay = load_invocation_anchor_runtime_records(&params)?;
    list_invocations_with_runtime_overlay(state, params, runtime_overlay).await
}

pub(crate) async fn list_invocations_with_runtime_overlay(
    state: Arc<AppState>,
    params: ListQuery,
    runtime_overlay_override: Option<Vec<ApiInvocation>>,
) -> Result<Json<ListResponse>, ApiError> {
    let request = build_resolved_invocation_list_request(
        &state.pool,
        &params,
        state.config.list_limit_max as i64,
    )
    .await?;
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

pub(crate) async fn list_invocation_page_with_runtime_overlay_on_connection(
    request: &InvocationListRequest,
    source_scope: InvocationSourceScope,
    snapshot_id: i64,
    runtime_overlay_records: &[ApiInvocation],
    pricing_catalog: &PricingCatalog,
    connection: &mut SqliteConnection,
) -> Result<ListResponse, ApiError> {
    let db_terminal_keys = if runtime_overlay_records.is_empty() {
        HashSet::new()
    } else {
        query_terminal_db_keys_for_runtime_records_on_connection(
            connection,
            runtime_overlay_records,
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
    let total = count_query
        .build_query_as::<CountRow>()
        .fetch_one(&mut *connection)
        .await?
        .total;
    let db_runtime_keys = if runtime_overlay_records.is_empty() {
        HashSet::new()
    } else {
        query_current_runtime_db_keys_on_connection(
            connection,
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
        .fetch_all(&mut *connection)
        .await?
        .into_iter()
        .map(|row| row.id)
        .collect::<Vec<_>>();

    if page_ids.is_empty() {
        let (mut records, total) = overlay_runtime_records_for_current_page(
            request,
            source_scope,
            runtime_overlay_records.to_vec(),
            &db_runtime_keys,
            &db_terminal_keys,
            true,
            Vec::new(),
            total,
            "invocation_history_overview",
        );
        apply_invocation_cost_audits(&mut records, pricing_catalog);
        return Ok(ListResponse {
            snapshot_id,
            total,
            page: request.page,
            page_size: request.page_size,
            records,
        });
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
        .fetch_all(&mut *connection)
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
        request,
        source_scope,
        runtime_overlay_records.to_vec(),
        &db_runtime_keys,
        &db_terminal_keys,
        true,
        records,
        total,
        "invocation_history_overview",
    );
    apply_invocation_cost_audits(&mut records, pricing_catalog);

    Ok(ListResponse {
        snapshot_id,
        total,
        page: request.page,
        page_size: request.page_size,
        records,
    })
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LocateInvocationResponse {
    pub(crate) anchor_id: String,
    pub(crate) snapshot_id: i64,
    pub(crate) invoke_id: String,
    pub(crate) attempt_id: Option<String>,
    pub(crate) total: i64,
    pub(crate) page: i64,
    pub(crate) page_size: i64,
    pub(crate) records: Vec<ApiInvocation>,
    pub(crate) target_index: usize,
    pub(crate) target_absolute_index: i64,
}

#[derive(Debug, FromRow)]
pub(crate) struct InvocationLocateRow {
    pub(crate) id: i64,
    pub(crate) occurred_at: String,
}

#[derive(Debug, FromRow)]
pub(crate) struct InvocationLocateAttemptRow {
    pub(crate) invoke_id: String,
    pub(crate) upstream_account_id: Option<i64>,
}

pub(crate) async fn locate_invocation_page(
    state: Arc<AppState>,
    params: &LocateInvocationQuery,
) -> Result<Option<LocateInvocationResponse>, ApiError> {
    let request_id = normalize_query_text(params.invoke_id.as_deref())
        .or_else(|| normalize_query_text(params.request_id.as_deref()));
    let attempt_id = normalize_query_text(params.attempt_id.as_deref());
    if request_id.is_none() && attempt_id.is_none() {
        return Err(ApiError::bad_request(anyhow!(
            "invokeId or attemptId is required"
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
                invoke_id: request_id.clone(),
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
                "invokeId": normalize_query_text(params.invoke_id.as_deref())
                    .or_else(|| normalize_query_text(params.request_id.as_deref())),
                "attemptId": normalize_query_text(params.attempt_id.as_deref()),
            })),
        )
            .into_response()),
    }
}

pub(crate) async fn fetch_invocation_summary(
    State(state): State<Arc<AppState>>,
    Query(params): Query<ListQuery>,
) -> Result<Json<InvocationSummaryResponse>, ApiError> {
    fetch_invocation_summary_with_runtime_overlay(state, params, None).await
}

pub(crate) async fn fetch_invocation_summary_with_runtime_overlay(
    state: Arc<AppState>,
    params: ListQuery,
    runtime_overlay_override: Option<Vec<ApiInvocation>>,
) -> Result<Json<InvocationSummaryResponse>, ApiError> {
    let request = build_resolved_invocation_list_request(
        &state.pool,
        &params,
        state.config.list_limit_max as i64,
    )
    .await?;
    let source_scope = resolve_default_source_scope(&state.pool).await?;
    let runtime_records =
        runtime_overlay_override.unwrap_or_else(|| runtime_overlay_snapshot(state.as_ref()));
    let mut tx = state.pool.begin().await?;
    let snapshot_id = request
        .snapshot_id
        .unwrap_or(resolve_invocation_snapshot_id_on_connection(&mut tx, source_scope).await?);

    let totals_sql = format!(
        "SELECT \
         COUNT(*) AS total_count, \
         COALESCE(SUM(CASE WHEN {resolved_failure} = 'none' AND ({status_norm} IN ('success', 'succeeded', 'completed', 'ok', '{warning_success}') OR ({status_norm} = 'http_200' AND LOWER(TRIM(COALESCE(error_message, ''))) = '')) THEN 1 ELSE 0 END), 0) AS success_count, \
         COALESCE(SUM(CASE WHEN {resolved_failure} IN ('service_failure', 'client_failure', 'client_abort') THEN 1 ELSE 0 END), 0) AS failure_count, \
         COALESCE(SUM(total_tokens), 0) AS total_tokens, \
         COALESCE(SUM(cost), 0.0) AS total_cost, \
         COALESCE(SUM({valid_cache_write_tokens_sql}), 0) AS cache_write_tokens, \
         COALESCE(SUM({valid_cache_input_tokens_sql}), 0) AS cache_input_tokens, \
         COALESCE(SUM(output_tokens), 0) AS output_tokens, \
         MAX(CASE WHEN total_tokens >= 0 THEN total_tokens END) AS max_total_tokens \
         FROM codex_invocations WHERE 1 = 1",
        status_norm = INVOCATION_STATUS_NORMALIZED_SQL,
        resolved_failure = INVOCATION_RESOLVED_FAILURE_CLASS_SQL,
        warning_success = INVOCATION_STATUS_WARNING_SUCCESS,
        valid_cache_write_tokens_sql = INVOCATION_VALID_CACHE_WRITE_TOKENS_SQL,
        valid_cache_input_tokens_sql = INVOCATION_VALID_CACHE_INPUT_TOKENS_SQL,
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
        .fetch_one(&mut *tx)
        .await?;

    let network = query_invocation_network_summary_on_connection(
        &mut tx,
        &request.filters,
        source_scope,
        snapshot_id,
    )
    .await?;

    let exception = query_invocation_exception_summary_on_connection(
        &mut tx,
        &request.filters,
        source_scope,
        snapshot_id,
    )
    .await?;

    let new_records_count = query_invocation_new_records_count_on_connection(
        &mut tx,
        &request.filters,
        source_scope,
        snapshot_id,
    )
    .await?;

    let runtime_overlay_delta = if request.snapshot_id.is_none() {
        let db_runtime_keys = query_current_runtime_db_keys_on_connection(
            &mut tx,
            &request.filters,
            source_scope,
            Some(SnapshotConstraint::UpTo(snapshot_id)),
        )
        .await?;
        let db_terminal_keys = query_terminal_db_keys_for_runtime_records_on_connection(
            &mut tx,
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
        )?;
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
    let merged = merge_invocation_summary_totals(&totals, &exception, &runtime_overlay_delta)?;
    let total_count = merged.total_count;
    let success_count = merged.success_count;
    let failure_count = merged.failure_count;
    let total_tokens = merged.total_tokens;
    let total_cost = merged.total_cost;
    let cache_write_tokens = merged.cache_write_tokens;
    let cache_input_tokens = merged.cache_input_tokens;
    let output_tokens = merged.output_tokens;
    let max_tokens_per_request = match (
        totals.max_total_tokens,
        runtime_overlay_delta.max_total_tokens,
    ) {
        (Some(left), Some(right)) => Some(left.max(right)),
        (Some(left), None) => Some(left),
        (None, Some(right)) => Some(right),
        (None, None) => None,
    };
    let max_total_ms = match (network.max_total_ms, runtime_overlay_delta.max_total_ms) {
        (Some(left), Some(right)) => Some(left.max(right)),
        (Some(left), None) => Some(left),
        (None, Some(right)) => Some(right),
        (None, None) => None,
    };
    let avg_tokens_per_request = if total_count <= 0 {
        0.0
    } else {
        total_tokens as f64 / total_count as f64
    };
    let exception_summary = merged.exception;

    let response = InvocationSummaryResponse {
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
            max_tokens_per_request,
        },
        network: InvocationNetworkSummary {
            max_total_ms,
            ..network
        },
        exception: exception_summary,
    };
    tx.commit().await?;
    Ok(Json(response))
}

pub(crate) async fn build_invocation_summary_on_connection(
    request: &InvocationListRequest,
    source_scope: InvocationSourceScope,
    snapshot_id: i64,
    runtime_records: &[ApiInvocation],
    connection: &mut SqliteConnection,
) -> Result<InvocationSummaryResponse, ApiError> {
    let totals_sql = format!(
        "SELECT \
         COUNT(*) AS total_count, \
         COALESCE(SUM(CASE WHEN {resolved_failure} = 'none' AND ({status_norm} IN ('success', 'succeeded', 'completed', 'ok', '{warning_success}') OR ({status_norm} = 'http_200' AND LOWER(TRIM(COALESCE(error_message, ''))) = '')) THEN 1 ELSE 0 END), 0) AS success_count, \
         COALESCE(SUM(CASE WHEN {resolved_failure} IN ('service_failure', 'client_failure', 'client_abort') THEN 1 ELSE 0 END), 0) AS failure_count, \
         COALESCE(SUM(total_tokens), 0) AS total_tokens, \
         COALESCE(SUM(cost), 0.0) AS total_cost, \
         COALESCE(SUM({valid_cache_write_tokens_sql}), 0) AS cache_write_tokens, \
         COALESCE(SUM({valid_cache_input_tokens_sql}), 0) AS cache_input_tokens, \
         COALESCE(SUM(output_tokens), 0) AS output_tokens, \
         MAX(CASE WHEN total_tokens >= 0 THEN total_tokens END) AS max_total_tokens \
         FROM codex_invocations WHERE 1 = 1",
        status_norm = INVOCATION_STATUS_NORMALIZED_SQL,
        resolved_failure = INVOCATION_RESOLVED_FAILURE_CLASS_SQL,
        warning_success = INVOCATION_STATUS_WARNING_SUCCESS,
        valid_cache_write_tokens_sql = INVOCATION_VALID_CACHE_WRITE_TOKENS_SQL,
        valid_cache_input_tokens_sql = INVOCATION_VALID_CACHE_INPUT_TOKENS_SQL,
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
        .fetch_one(&mut *connection)
        .await?;
    let network = query_invocation_network_summary_on_connection(
        connection,
        &request.filters,
        source_scope,
        snapshot_id,
    )
    .await?;
    let exception = query_invocation_exception_summary_on_connection(
        connection,
        &request.filters,
        source_scope,
        snapshot_id,
    )
    .await?;
    let new_records_count = query_invocation_new_records_count_on_connection(
        connection,
        &request.filters,
        source_scope,
        snapshot_id,
    )
    .await?;

    let runtime_overlay_delta = if request.snapshot_id.is_none() {
        let db_runtime_keys = query_current_runtime_db_keys_on_connection(
            connection,
            &request.filters,
            source_scope,
            Some(SnapshotConstraint::UpTo(snapshot_id)),
        )
        .await?;
        let db_terminal_keys = query_terminal_db_keys_for_runtime_records_on_connection(
            connection,
            runtime_records,
            Some(SnapshotConstraint::UpTo(snapshot_id)),
        )
        .await?;
        runtime_overlay_total_delta(
            request,
            source_scope,
            runtime_records,
            &db_runtime_keys,
            &db_terminal_keys,
        )?
        .0
    } else {
        RuntimeSummaryOverlayDelta::default()
    };
    let merged = merge_invocation_summary_totals(&totals, &exception, &runtime_overlay_delta)?;
    let total_count = merged.total_count;
    let success_count = merged.success_count;
    let failure_count = merged.failure_count;
    let total_tokens = merged.total_tokens;
    let total_cost = merged.total_cost;
    let cache_write_tokens = merged.cache_write_tokens;
    let cache_input_tokens = merged.cache_input_tokens;
    let output_tokens = merged.output_tokens;
    let max_tokens_per_request = match (
        totals.max_total_tokens,
        runtime_overlay_delta.max_total_tokens,
    ) {
        (Some(left), Some(right)) => Some(left.max(right)),
        (Some(left), None) => Some(left),
        (None, Some(right)) => Some(right),
        (None, None) => None,
    };
    let max_total_ms = match (network.max_total_ms, runtime_overlay_delta.max_total_ms) {
        (Some(left), Some(right)) => Some(left.max(right)),
        (Some(left), None) => Some(left),
        (None, Some(right)) => Some(right),
        (None, None) => None,
    };
    let avg_tokens_per_request = if total_count <= 0 {
        0.0
    } else {
        total_tokens as f64 / total_count as f64
    };
    let exception = merged.exception;
    Ok(InvocationSummaryResponse {
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
            max_tokens_per_request,
        },
        network: InvocationNetworkSummary {
            max_total_ms,
            ..network
        },
        exception,
    })
}

pub(crate) async fn fetch_invocation_history_overview_with_runtime_overlay(
    state: Arc<AppState>,
    params: ListQuery,
    runtime_overlay_records: Vec<ApiInvocation>,
    max_records: usize,
) -> Result<InvocationHistoryOverviewResponse, ApiError> {
    let request = build_resolved_invocation_list_request(
        &state.pool,
        &params,
        state.config.list_limit_max as i64,
    )
    .await?;
    let source_scope = resolve_default_source_scope(&state.pool).await?;
    let pricing_catalog = state.pricing_catalog.read().await.clone();
    let mut tx = state.pool.begin().await?;
    let snapshot_id = request
        .snapshot_id
        .unwrap_or(resolve_invocation_snapshot_id_on_connection(&mut tx, source_scope).await?);
    let summary = build_invocation_summary_on_connection(
        &request,
        source_scope,
        snapshot_id,
        &runtime_overlay_records,
        &mut tx,
    )
    .await?;
    let chart_total = summary.total_count;
    let mut records = Vec::new();
    let mut page = 1_i64;
    while records.len() < max_records {
        let mut page_request = request.clone();
        page_request.page = page;
        let response = list_invocation_page_with_runtime_overlay_on_connection(
            &page_request,
            source_scope,
            snapshot_id,
            &runtime_overlay_records,
            &pricing_catalog,
            &mut tx,
        )
        .await?;
        let received_count = response.records.len();
        if received_count == 0 {
            break;
        }
        let remaining = max_records - records.len();
        records.extend(response.records.into_iter().take(remaining));
        if records.len() >= chart_total as usize || received_count < page_request.page_size as usize
        {
            break;
        }
        page += 1;
    }
    let mut chart_range_start = records
        .iter()
        .map(|record| record.occurred_at.clone())
        .min();
    let mut chart_range_end = records
        .iter()
        .map(|record| record.occurred_at.clone())
        .max();
    if chart_total as usize > records.len() {
        let overview_page_size = request.page_size.max(1);
        let oldest_page = (chart_total + overview_page_size - 1) / overview_page_size;
        let mut oldest_request = request;
        oldest_request.page = oldest_page.max(1);
        let oldest_response = list_invocation_page_with_runtime_overlay_on_connection(
            &oldest_request,
            source_scope,
            snapshot_id,
            &runtime_overlay_records,
            &pricing_catalog,
            &mut tx,
        )
        .await?;
        for record in oldest_response.records {
            chart_range_start = Some(
                chart_range_start.map_or(record.occurred_at.clone(), |current| {
                    current.min(record.occurred_at.clone())
                }),
            );
            chart_range_end = Some(
                chart_range_end.map_or(record.occurred_at.clone(), |current| {
                    current.max(record.occurred_at.clone())
                }),
            );
        }
    }
    tx.commit().await?;
    Ok(InvocationHistoryOverviewResponse {
        summary,
        chart_is_sampled: chart_total as usize > records.len(),
        records,
        chart_total,
        chart_range_start,
        chart_range_end,
    })
}

pub(crate) async fn fetch_invocation_new_records_count(
    State(state): State<Arc<AppState>>,
    Query(params): Query<ListQuery>,
) -> Result<Json<InvocationNewRecordsCountResponse>, ApiError> {
    let request = build_resolved_invocation_list_request(
        &state.pool,
        &params,
        state.config.list_limit_max as i64,
    )
    .await?;
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
    let request = build_resolved_invocation_list_request(
        &state.pool,
        &params,
        state.config.list_limit_max as i64,
    )
    .await?;
    let filters = request.filters;
    let source_scope = resolve_default_source_scope(&state.pool).await?;
    let snapshot = request.snapshot_id.map(SnapshotConstraint::UpTo);
    let suggest_field = InvocationSuggestionField::parse(params.suggest_field.as_deref())?;
    let suggest_query = normalize_query_text(params.suggest_query.as_deref());

    if let Some(field) = suggest_field {
        let scoped_filters = field.clear_field_filter(&filters);
        let bucket = match field {
            InvocationSuggestionField::UpstreamAccount => {
                query_invocation_upstream_account_suggestion_bucket(
                    &state.pool,
                    &scoped_filters,
                    source_scope,
                    snapshot,
                    suggest_query.as_deref(),
                    SUGGESTION_LIMIT,
                )
                .await?
            }
            _ => {
                let sql_expr = field.sql_expr();
                query_invocation_suggestion_bucket(
                    &state.pool,
                    &scoped_filters,
                    source_scope,
                    snapshot,
                    &sql_expr,
                    suggest_query.as_deref(),
                    SUGGESTION_LIMIT,
                )
                .await?
            }
        };
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
    let request_model = query_invocation_suggestion_bucket(
        &state.pool,
        &InvocationRecordsFilters {
            model: None,
            model_values: Vec::new(),
            ..filters.clone()
        },
        source_scope,
        snapshot,
        &invocation_model_sql_expr(InvocationModelTarget::Request),
        None,
        SUGGESTION_LIMIT,
    )
    .await?;
    let response_model = query_invocation_suggestion_bucket(
        &state.pool,
        &InvocationRecordsFilters {
            model: None,
            model_values: Vec::new(),
            ..filters.clone()
        },
        source_scope,
        snapshot,
        &invocation_model_sql_expr(InvocationModelTarget::Response),
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
    let sticky_key = query_invocation_suggestion_bucket(
        &state.pool,
        &InvocationRecordsFilters {
            sticky_key: None,
            ..filters.clone()
        },
        source_scope,
        snapshot,
        INVOCATION_STICKY_KEY_SQL,
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
    let proxy_display_name = query_invocation_suggestion_bucket(
        &state.pool,
        &InvocationRecordsFilters {
            proxy_display_name: None,
            ..filters.clone()
        },
        source_scope,
        snapshot,
        INVOCATION_PROXY_DISPLAY_SQL,
        None,
        SUGGESTION_LIMIT,
    )
    .await?;
    let upstream_account = query_invocation_upstream_account_suggestion_bucket(
        &state.pool,
        &InvocationRecordsFilters {
            upstream_account_id: None,
            ..filters.clone()
        },
        source_scope,
        snapshot,
        None,
        SUGGESTION_LIMIT,
    )
    .await?;
    let service_tier = query_invocation_suggestion_bucket(
        &state.pool,
        &InvocationRecordsFilters {
            service_tier: None,
            ..filters.clone()
        },
        source_scope,
        snapshot,
        INVOCATION_SERVICE_TIER_SQL,
        None,
        SUGGESTION_LIMIT,
    )
    .await?;
    let reasoning_effort = query_invocation_suggestion_bucket(
        &state.pool,
        &InvocationRecordsFilters {
            reasoning_effort: None,
            ..filters.clone()
        },
        source_scope,
        snapshot,
        INVOCATION_REASONING_EFFORT_SQL,
        None,
        SUGGESTION_LIMIT,
    )
    .await?;

    Ok(Json(InvocationSuggestionsResponse {
        model,
        request_model,
        response_model,
        endpoint,
        failure_kind,
        sticky_key,
        prompt_cache_key,
        requester_ip,
        proxy_display_name,
        upstream_account,
        service_tier,
        reasoning_effort,
    }))
}

pub(crate) async fn fetch_stats(
    State(state): State<Arc<AppState>>,
) -> Result<Json<StatsResponse>, ApiError> {
    fetch_summary(
        State(state),
        Query(SummaryQuery {
            window: Some("all".to_string()),
            limit: None,
            time_zone: None,
            upstream_account_id: None,
        }),
    )
    .await
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
