const INVOCATION_ATTEMPT_RESPONSE_BODY_SQL: &str = r#"
        SELECT
            inv.id,
            inv.invoke_id,
            inv.payload,
            CASE
                WHEN inv.detail_level != 'structured_only'
                    AND attempts.attempt_index = (
                    SELECT MAX(final_attempt.attempt_index)
                    FROM pool_upstream_request_attempts AS final_attempt
                    WHERE final_attempt.invoke_id = attempts.invoke_id
                      AND final_attempt.occurred_at = attempts.occurred_at
                      AND final_attempt.status != 'budget_exhausted_final'
                ) THEN inv.raw_response
                ELSE ''
            END AS raw_response,
            inv.request_raw_path,
            inv.request_raw_size,
            inv.request_raw_truncated,
            inv.request_raw_truncated_reason,
            CASE
                WHEN inv.detail_level != 'structured_only'
                    AND attempts.response_raw_path IS NOT NULL THEN attempts.response_raw_path
                WHEN inv.detail_level != 'structured_only'
                    AND attempts.attempt_index = (
                    SELECT MAX(final_attempt.attempt_index)
                    FROM pool_upstream_request_attempts AS final_attempt
                    WHERE final_attempt.invoke_id = attempts.invoke_id
                      AND final_attempt.occurred_at = attempts.occurred_at
                      AND final_attempt.status != 'budget_exhausted_final'
                ) THEN inv.response_raw_path
                ELSE NULL
            END AS response_raw_path,
            CASE
                WHEN inv.detail_level != 'structured_only'
                    AND attempts.response_raw_path IS NOT NULL THEN attempts.response_raw_size
                WHEN inv.detail_level != 'structured_only'
                    AND attempts.attempt_index = (
                    SELECT MAX(final_attempt.attempt_index)
                    FROM pool_upstream_request_attempts AS final_attempt
                    WHERE final_attempt.invoke_id = attempts.invoke_id
                      AND final_attempt.occurred_at = attempts.occurred_at
                      AND final_attempt.status != 'budget_exhausted_final'
                ) THEN inv.response_raw_size
                ELSE NULL
            END AS response_raw_size,
            CASE
                WHEN inv.detail_level != 'structured_only'
                    AND attempts.response_raw_path IS NOT NULL THEN attempts.response_raw_truncated
                WHEN inv.detail_level != 'structured_only'
                    AND attempts.attempt_index = (
                    SELECT MAX(final_attempt.attempt_index)
                    FROM pool_upstream_request_attempts AS final_attempt
                    WHERE final_attempt.invoke_id = attempts.invoke_id
                      AND final_attempt.occurred_at = attempts.occurred_at
                      AND final_attempt.status != 'budget_exhausted_final'
                ) THEN inv.response_raw_truncated
                ELSE NULL
            END AS response_raw_truncated,
            CASE
                WHEN inv.detail_level != 'structured_only'
                    AND attempts.response_raw_path IS NOT NULL THEN attempts.response_raw_truncated_reason
                WHEN inv.detail_level != 'structured_only'
                    AND attempts.attempt_index = (
                    SELECT MAX(final_attempt.attempt_index)
                    FROM pool_upstream_request_attempts AS final_attempt
                    WHERE final_attempt.invoke_id = attempts.invoke_id
                      AND final_attempt.occurred_at = attempts.occurred_at
                      AND final_attempt.status != 'budget_exhausted_final'
                ) THEN inv.response_raw_truncated_reason
                ELSE NULL
            END AS response_raw_truncated_reason,
            inv.detail_level,
            inv.detail_prune_reason,
            CASE
                WHEN inv.detail_level != 'structured_only'
                    AND attempts.response_content_encoding IS NOT NULL THEN attempts.response_content_encoding
                WHEN inv.detail_level != 'structured_only'
                    AND attempts.attempt_index = (
                    SELECT MAX(final_attempt.attempt_index)
                    FROM pool_upstream_request_attempts AS final_attempt
                    WHERE final_attempt.invoke_id = attempts.invoke_id
                      AND final_attempt.occurred_at = attempts.occurred_at
                      AND final_attempt.status != 'budget_exhausted_final'
                ) THEN {response_content_encoding}
                ELSE NULL
            END AS response_content_encoding,
            {resolved_failure} AS failure_class,
            attempts.upstream_request_id AS upstream_request_id,
            attempts.attempt_public_id AS attempt_public_id
        FROM codex_invocations AS inv
        JOIN pool_upstream_request_attempts AS attempts
            ON attempts.invoke_id = inv.invoke_id
           AND attempts.occurred_at = inv.occurred_at
        WHERE inv.id = ?1
          AND attempts.attempt_public_id = ?2
        LIMIT 1
        "#;

async fn fetch_invocation_attempt_response_body_row(
    pool: &Pool<Sqlite>,
    invocation_id: i64,
    attempt_public_id: &str,
) -> Result<Option<InvocationResponseBodyRow>, ApiError> {
    let sql = INVOCATION_ATTEMPT_RESPONSE_BODY_SQL
        .replace(
            "{response_content_encoding}",
            INVOCATION_RESPONSE_CONTENT_ENCODING_SQL,
        )
        .replace(
            "{resolved_failure}",
            &invocation_resolved_failure_class_sql_for("inv"),
        );
    sqlx::query_as::<_, InvocationResponseBodyRow>(&sql)
        .bind(invocation_id)
        .bind(attempt_public_id)
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
    Ok(Json(build_response_body_response(
        &row,
        state.config.database_path.parent(),
    )))
}

pub(crate) async fn fetch_invocation_attempt_response_body(
    State(state): State<Arc<AppState>>,
    axum::extract::Path((id, attempt_public_id)): axum::extract::Path<(i64, String)>,
) -> Result<Json<InvocationResponseBodyResponse>, ApiError> {
    let row =
        fetch_invocation_attempt_response_body_row(&state.pool, id, attempt_public_id.as_str())
            .await?
            .ok_or_else(|| ApiError::bad_request(anyhow!("attempt not found")))?;
    let (has_attempt_capture, has_invocation_capture) = sqlx::query_as::<
        _,
        (Option<i64>, Option<i64>),
    >(
        "SELECT (inv.detail_level != 'structured_only' AND attempts.response_raw_path IS NOT NULL),
                (inv.detail_level != 'structured_only' AND inv.response_raw_path IS NOT NULL)
         FROM pool_upstream_request_attempts AS attempts
         JOIN codex_invocations AS inv
           ON inv.invoke_id = attempts.invoke_id
          AND inv.occurred_at = attempts.occurred_at
         WHERE inv.id = ?1
           AND attempts.attempt_public_id = ?2
         LIMIT 1",
    )
    .bind(id)
    .bind(attempt_public_id.as_str())
    .fetch_optional(&state.pool)
    .await?
    .unwrap_or_default();
    let has_attempt_capture = has_attempt_capture.unwrap_or_default() != 0;
    let has_invocation_capture = has_invocation_capture.unwrap_or_default() != 0;
    let source = Some(if has_attempt_capture {
        "attempt_raw_file"
    } else if has_invocation_capture && row.response_raw_path.is_some() {
        "invocation_raw_file"
    } else {
        "attempt_metrics"
    });
    Ok(Json(build_response_body_response_with_source(
        &row,
        state.config.database_path.parent(),
        source,
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
    fetch_invocation_summary_with_runtime_overlay(state, params, None).await
}

fn invocation_summary_totals_sql() -> String {
    format!(
        "SELECT \
         COUNT(*) AS total_count, \
         COALESCE(SUM(CASE WHEN {resolved_failure} = 'none' AND ({status_norm} IN ('success', 'completed', '{warning_success}') OR ({status_norm} = 'http_200' AND LOWER(TRIM(COALESCE(error_message, ''))) = '')) THEN 1 ELSE 0 END), 0) AS success_count, \
         COALESCE(SUM(CASE WHEN {resolved_failure} IN ('service_failure', 'client_failure', 'client_abort') THEN 1 ELSE 0 END), 0) AS failure_count, \
         COALESCE(SUM(total_tokens), 0) AS total_tokens, \
         COALESCE(SUM(cost), 0.0) AS total_cost, \
         COALESCE(SUM(MAX(COALESCE(input_tokens, 0) - COALESCE(cache_input_tokens, 0), 0)), 0) AS cache_write_tokens, \
         COALESCE(SUM(cache_input_tokens), 0) AS cache_input_tokens, \
         COALESCE(SUM(output_tokens), 0) AS output_tokens, \
         MAX(CASE WHEN total_tokens >= 0 THEN total_tokens END) AS max_total_tokens \
         FROM codex_invocations WHERE 1 = 1",
        status_norm = INVOCATION_STATUS_NORMALIZED_SQL,
        resolved_failure = INVOCATION_RESOLVED_FAILURE_CLASS_SQL,
        warning_success = INVOCATION_STATUS_WARNING_SUCCESS,
    )
}

async fn query_invocation_summary_totals_on_connection(
    connection: &mut SqliteConnection,
    request: &InvocationListRequest,
    source_scope: InvocationSourceScope,
    snapshot_id: i64,
) -> Result<InvocationSummaryAggRow, ApiError> {
    let mut query = QueryBuilder::new(invocation_summary_totals_sql());
    apply_invocation_records_filters(
        &mut query,
        &request.filters,
        source_scope,
        Some(SnapshotConstraint::UpTo(snapshot_id)),
    );
    query
        .build_query_as::<InvocationSummaryAggRow>()
        .fetch_one(&mut *connection)
        .await
        .map_err(ApiError::from)
}

async fn query_invocation_summary_runtime_overlay_delta(
    connection: &mut SqliteConnection,
    request: &InvocationListRequest,
    source_scope: InvocationSourceScope,
    snapshot_id: i64,
    runtime_records: &[ApiInvocation],
) -> Result<(RuntimeSummaryOverlayDelta, i64, i64), ApiError> {
    if request.snapshot_id.is_some() {
        return Ok((RuntimeSummaryOverlayDelta::default(), 0, 0));
    }
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
    let (delta, runtime_new_count, stale_db_runtime_count) = runtime_overlay_total_delta(
        request,
        source_scope,
        runtime_records,
        &db_runtime_keys,
        &db_terminal_keys,
    );
    Ok((
        delta,
        runtime_new_count as i64,
        stale_db_runtime_count as i64,
    ))
}

struct InvocationSummaryQueryResult {
    totals: InvocationSummaryAggRow,
    network: InvocationNetworkSummary,
    exception: InvocationExceptionSummary,
    new_records_count: i64,
    runtime_overlay_delta: RuntimeSummaryOverlayDelta,
    runtime_new_count: i64,
    stale_db_runtime_count: i64,
}

async fn query_invocation_summary_parts(
    connection: &mut SqliteConnection,
    request: &InvocationListRequest,
    source_scope: InvocationSourceScope,
    snapshot_id: i64,
    runtime_records: &[ApiInvocation],
) -> Result<InvocationSummaryQueryResult, ApiError> {
    let totals = query_invocation_summary_totals_on_connection(
        connection,
        request,
        source_scope,
        snapshot_id,
    )
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
    let (runtime_overlay_delta, runtime_new_count, stale_db_runtime_count) =
        query_invocation_summary_runtime_overlay_delta(
            connection,
            request,
            source_scope,
            snapshot_id,
            runtime_records,
        )
        .await?;
    Ok(InvocationSummaryQueryResult {
        totals,
        network,
        exception,
        new_records_count,
        runtime_overlay_delta,
        runtime_new_count,
        stale_db_runtime_count,
    })
}

fn build_invocation_summary_response(
    snapshot_id: i64,
    query: InvocationSummaryQueryResult,
) -> InvocationSummaryResponse {
    let InvocationSummaryQueryResult {
        totals,
        network,
        exception,
        new_records_count,
        runtime_overlay_delta,
        ..
    } = query;
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
    InvocationSummaryResponse {
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
        exception: InvocationExceptionSummary {
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
        },
    }
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

    let query = query_invocation_summary_parts(
        &mut tx,
        &request,
        source_scope,
        snapshot_id,
        &runtime_records,
    )
    .await?;

    if query.runtime_new_count > 0 || query.stale_db_runtime_count > 0 {
        debug!(
            endpoint = "invocation_summary",
            runtime_overlay_row_count = query.runtime_new_count,
            stale_db_runtime_total_count = query.stale_db_runtime_count,
            "adjusted current summary count with memory runtime overlay"
        );
    }
    let response = build_invocation_summary_response(snapshot_id, query);
    tx.commit().await?;
    Ok(Json(response))
}

async fn build_invocation_summary_on_connection(
    request: &InvocationListRequest,
    source_scope: InvocationSourceScope,
    snapshot_id: i64,
    runtime_records: &[ApiInvocation],
    connection: &mut SqliteConnection,
) -> Result<InvocationSummaryResponse, ApiError> {
    let query = query_invocation_summary_parts(
        connection,
        request,
        source_scope,
        snapshot_id,
        runtime_records,
    )
    .await?;
    Ok(build_invocation_summary_response(snapshot_id, query))
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
            "invocation_history_overview",
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
            "invocation_history_overview",
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

async fn query_invocation_suggestion_for_field(
    pool: &Pool<Sqlite>,
    filters: &InvocationRecordsFilters,
    source_scope: InvocationSourceScope,
    snapshot: Option<SnapshotConstraint>,
    field: InvocationSuggestionField,
    match_query: Option<&str>,
) -> Result<InvocationSuggestionBucket, ApiError> {
    let scoped_filters = field.clear_field_filter(filters);
    if field == InvocationSuggestionField::UpstreamAccount {
        query_invocation_upstream_account_suggestion_bucket(
            pool,
            &scoped_filters,
            source_scope,
            snapshot,
            match_query,
            30,
        )
        .await
        .map_err(ApiError::from)
    } else {
        let sql_expr = field.sql_expr();
        query_invocation_suggestion_bucket(
            pool,
            &scoped_filters,
            source_scope,
            snapshot,
            &sql_expr,
            match_query,
            30,
        )
        .await
        .map_err(ApiError::from)
    }
}

async fn query_default_invocation_suggestions(
    pool: &Pool<Sqlite>,
    filters: &InvocationRecordsFilters,
    source_scope: InvocationSourceScope,
    snapshot: Option<SnapshotConstraint>,
) -> Result<InvocationSuggestionsResponse, ApiError> {
    let fields = [
        InvocationSuggestionField::Model,
        InvocationSuggestionField::RequestModel,
        InvocationSuggestionField::ResponseModel,
        InvocationSuggestionField::Endpoint,
        InvocationSuggestionField::FailureKind,
        InvocationSuggestionField::StickyKey,
        InvocationSuggestionField::PromptCacheKey,
        InvocationSuggestionField::RequesterIp,
        InvocationSuggestionField::ProxyDisplayName,
        InvocationSuggestionField::UpstreamAccount,
        InvocationSuggestionField::ServiceTier,
        InvocationSuggestionField::ReasoningEffort,
    ];
    let mut buckets = Vec::with_capacity(fields.len());
    for field in fields {
        buckets.push(
            query_invocation_suggestion_for_field(
                pool,
                filters,
                source_scope,
                snapshot,
                field,
                None,
            )
            .await?,
        );
    }
    let mut buckets = buckets.into_iter();
    Ok(InvocationSuggestionsResponse {
        model: buckets
            .next()
            .unwrap_or_else(empty_invocation_suggestion_bucket),
        request_model: buckets
            .next()
            .unwrap_or_else(empty_invocation_suggestion_bucket),
        response_model: buckets
            .next()
            .unwrap_or_else(empty_invocation_suggestion_bucket),
        endpoint: buckets
            .next()
            .unwrap_or_else(empty_invocation_suggestion_bucket),
        failure_kind: buckets
            .next()
            .unwrap_or_else(empty_invocation_suggestion_bucket),
        sticky_key: buckets
            .next()
            .unwrap_or_else(empty_invocation_suggestion_bucket),
        prompt_cache_key: buckets
            .next()
            .unwrap_or_else(empty_invocation_suggestion_bucket),
        requester_ip: buckets
            .next()
            .unwrap_or_else(empty_invocation_suggestion_bucket),
        proxy_display_name: buckets
            .next()
            .unwrap_or_else(empty_invocation_suggestion_bucket),
        upstream_account: buckets
            .next()
            .unwrap_or_else(empty_invocation_suggestion_bucket),
        service_tier: buckets
            .next()
            .unwrap_or_else(empty_invocation_suggestion_bucket),
        reasoning_effort: buckets
            .next()
            .unwrap_or_else(empty_invocation_suggestion_bucket),
    })
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
        let bucket = query_invocation_suggestion_for_field(
            &state.pool,
            &filters,
            source_scope,
            snapshot,
            field,
            suggest_query.as_deref(),
        )
        .await?;
        return Ok(Json(suggestion_response_for_field(field, bucket)));
    }
    Ok(Json(
        query_default_invocation_suggestions(&state.pool, &filters, source_scope, snapshot).await?,
    ))
}
