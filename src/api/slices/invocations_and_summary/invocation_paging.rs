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
        let pricing_catalog = state.pricing_catalog.read().await.clone();
        return Ok(Json(
            list_legacy_invocations_with_runtime_overlay(
                &state.pool,
                &request,
                source_scope,
                runtime_overlay_records,
                &pricing_catalog,
            )
            .await?,
        ));
    }

    let snapshot_id = request
        .snapshot_id
        .unwrap_or(resolve_invocation_snapshot_id(&state.pool, source_scope).await?);
    let pricing_catalog = state.pricing_catalog.read().await.clone();
    let mut tx = state.pool.begin().await?;
    let response = list_invocation_page_with_runtime_overlay_on_connection(
        &request,
        source_scope,
        snapshot_id,
        &runtime_overlay_records,
        &pricing_catalog,
        "invocation_records",
        &mut tx,
    )
    .await?;
    Ok(Json(response))
}

#[derive(Debug, FromRow)]
struct InvocationCountRow {
    total: i64,
}

#[derive(Debug, FromRow)]
struct InvocationPageIdRow {
    id: i64,
}

async fn list_legacy_invocations_with_runtime_overlay(
    pool: &Pool<Sqlite>,
    request: &InvocationListRequest,
    source_scope: InvocationSourceScope,
    runtime_overlay_records: Vec<ApiInvocation>,
    pricing_catalog: &PricingCatalog,
) -> Result<ListResponse, ApiError> {
    let db_terminal_keys = if runtime_overlay_records.is_empty() {
        HashSet::new()
    } else {
        query_terminal_db_keys_for_runtime_records(pool, &runtime_overlay_records, None).await?
    };
    let mut query = build_invocation_select_query();
    apply_invocation_records_filters(&mut query, &request.filters, source_scope, None);
    append_invocation_order_clause(&mut query, request.sort_by, request.sort_order);
    query.push(" LIMIT ").push_bind(request.page_size);
    let mut records = query
        .build_query_as::<ApiInvocation>()
        .fetch_all(pool)
        .await?;
    for record in &mut records {
        hydrate_api_invocation_blocked_binding(record);
    }
    let total = records.len() as i64;
    let (mut records, total) =
        overlay_runtime_records_for_current_page(InvocationRuntimeOverlayInput {
            request,
            source_scope,
            runtime_records: runtime_overlay_records,
            db_runtime_keys: &HashSet::new(),
            db_terminal_keys: &db_terminal_keys,
            db_records_are_prefix: false,
            records,
            total,
            endpoint: "invocation_records_legacy",
        });
    apply_invocation_cost_audits(&mut records, pricing_catalog);
    Ok(ListResponse {
        snapshot_id: 0,
        total,
        page: 1,
        page_size: request.page_size,
        records,
    })
}

async fn list_invocation_page_with_runtime_overlay_on_connection(
    request: &InvocationListRequest,
    source_scope: InvocationSourceScope,
    snapshot_id: i64,
    runtime_overlay_records: &[ApiInvocation],
    pricing_catalog: &PricingCatalog,
    endpoint: &'static str,
    connection: &mut SqliteConnection,
) -> Result<ListResponse, ApiError> {
    let page_state = load_invocation_page_query_state(
        request,
        source_scope,
        snapshot_id,
        runtime_overlay_records,
        connection,
    )
    .await?;
    if page_state.page_ids.is_empty() {
        let mut records = Vec::new();
        let (mut records, total) =
            overlay_runtime_records_for_current_page(InvocationRuntimeOverlayInput {
                request,
                source_scope,
                runtime_records: runtime_overlay_records.to_vec(),
                db_runtime_keys: &page_state.db_runtime_keys,
                db_terminal_keys: &page_state.db_terminal_keys,
                db_records_are_prefix: true,
                records: std::mem::take(&mut records),
                total: page_state.total,
                endpoint,
            });
        apply_invocation_cost_audits(&mut records, pricing_catalog);
        return Ok(ListResponse {
            snapshot_id,
            total,
            page: request.page,
            page_size: request.page_size,
            records,
        });
    }

    let records = query_invocation_page_records(
        request,
        source_scope,
        snapshot_id,
        &page_state.page_ids,
        connection,
    )
    .await?;
    let (mut records, total) =
        overlay_runtime_records_for_current_page(InvocationRuntimeOverlayInput {
            request,
            source_scope,
            runtime_records: runtime_overlay_records.to_vec(),
            db_runtime_keys: &page_state.db_runtime_keys,
            db_terminal_keys: &page_state.db_terminal_keys,
            db_records_are_prefix: true,
            records,
            total: page_state.total,
            endpoint,
        });
    apply_invocation_cost_audits(&mut records, pricing_catalog);
    Ok(ListResponse {
        snapshot_id,
        total,
        page: request.page,
        page_size: request.page_size,
        records,
    })
}

struct InvocationPageQueryState {
    total: i64,
    db_runtime_keys: HashSet<(String, String)>,
    db_terminal_keys: HashSet<(String, String)>,
    page_ids: Vec<i64>,
}

async fn load_invocation_page_query_state(
    request: &InvocationListRequest,
    source_scope: InvocationSourceScope,
    snapshot_id: i64,
    runtime_overlay_records: &[ApiInvocation],
    connection: &mut SqliteConnection,
) -> Result<InvocationPageQueryState, ApiError> {
    let snapshot = Some(SnapshotConstraint::UpTo(snapshot_id));
    let db_terminal_keys = if runtime_overlay_records.is_empty() {
        HashSet::new()
    } else {
        query_terminal_db_keys_for_runtime_records_on_connection(
            connection,
            runtime_overlay_records,
            snapshot,
        )
        .await?
    };
    let mut count_query =
        QueryBuilder::new("SELECT COUNT(*) AS total FROM codex_invocations WHERE 1 = 1");
    apply_invocation_records_filters(&mut count_query, &request.filters, source_scope, snapshot);
    let total = count_query
        .build_query_as::<InvocationCountRow>()
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
            snapshot,
        )
        .await?
    };
    let offset = (request.page - 1).saturating_mul(request.page_size);
    let mut page_id_query = QueryBuilder::new("SELECT id FROM codex_invocations WHERE 1 = 1");
    apply_invocation_records_filters(&mut page_id_query, &request.filters, source_scope, snapshot);
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
        .build_query_as::<InvocationPageIdRow>()
        .fetch_all(&mut *connection)
        .await?
        .into_iter()
        .map(|row| row.id)
        .collect();
    Ok(InvocationPageQueryState {
        total,
        db_runtime_keys,
        db_terminal_keys,
        page_ids,
    })
}

async fn query_invocation_page_records(
    request: &InvocationListRequest,
    source_scope: InvocationSourceScope,
    snapshot_id: i64,
    page_ids: &[i64],
    connection: &mut SqliteConnection,
) -> Result<Vec<ApiInvocation>, ApiError> {
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
        for &id in page_ids {
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
        .iter()
        .copied()
        .enumerate()
        .map(|(index, id)| (id, index))
        .collect::<HashMap<_, _>>();
    records.sort_by_key(|record| {
        page_positions
            .get(&record.id)
            .copied()
            .unwrap_or(usize::MAX)
    });
    Ok(records)
}
