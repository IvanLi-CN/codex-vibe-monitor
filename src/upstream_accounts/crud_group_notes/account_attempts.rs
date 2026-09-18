pub(crate) async fn list_upstream_account_attempts(
    State(state): State<Arc<AppState>>,
    AxumPath(account_id): AxumPath<i64>,
    Query(params): Query<ListUpstreamAccountAttemptsQuery>,
) -> Result<Json<UpstreamAccountAttemptListResponse>, (StatusCode, String)> {
    let response =
        load_upstream_account_attempt_page_from_query(state.as_ref(), account_id, &params)
            .await
            .map_err(internal_error_tuple)?;
    Ok(Json(response))
}

pub(crate) async fn load_upstream_account_attempt_page_from_query(
    state: &AppState,
    account_id: i64,
    params: &ListUpstreamAccountAttemptsQuery,
) -> Result<UpstreamAccountAttemptListResponse> {
    let page = normalize_upstream_account_list_page(params.page);
    let page_size = normalize_upstream_account_list_page_size(params.page_size);
    let filters = normalize_upstream_account_attempt_filters(params);
    load_upstream_account_attempt_page(state, account_id, page, page_size, &filters).await
}

pub(crate) async fn locate_upstream_account_attempt(
    State(state): State<Arc<AppState>>,
    AxumPath(account_id): AxumPath<i64>,
    Query(params): Query<LocateUpstreamAccountAttemptQuery>,
) -> Result<Json<UpstreamAccountAttemptListResponse>, (StatusCode, String)> {
    let page_size = normalize_upstream_account_list_page_size(params.page_size);
    let cutoff = shanghai_local_cutoff_string(ACCOUNT_ATTEMPT_RETENTION_DAYS);
    let target_occurred_at = sqlx::query_scalar::<_, String>(
        r#"
        SELECT occurred_at
        FROM pool_upstream_request_attempts
        WHERE attempt_public_id = ?1
          AND upstream_account_id = ?2
          AND occurred_at >= ?3
        "#,
    )
    .bind(&params.attempt_id)
    .bind(account_id)
    .bind(&cutoff)
    .fetch_optional(&state.pool)
    .await
    .map_err(internal_error_tuple)?
    .ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            "upstream account attempt was not found".to_string(),
        )
    })?;
    let newer_count = sqlx::query_scalar::<_, i64>(
        r#"
        SELECT COUNT(*)
        FROM pool_upstream_request_attempts
        WHERE upstream_account_id = ?1
          AND occurred_at >= ?2
          AND (
                occurred_at > ?3
                OR (
                    occurred_at = ?3
                    AND id > (
                        SELECT id
                        FROM pool_upstream_request_attempts
                        WHERE attempt_public_id = ?4
                    )
                )
              )
        "#,
    )
    .bind(account_id)
    .bind(&cutoff)
    .bind(&target_occurred_at)
    .bind(&params.attempt_id)
    .fetch_one(&state.pool)
    .await
    .map_err(internal_error_tuple)?
    .max(0) as usize;
    let page = newer_count / page_size + 1;
    let response = load_upstream_account_attempt_page(
        state.as_ref(),
        account_id,
        page,
        page_size,
        &UpstreamAccountAttemptFilters::default(),
    )
    .await
    .map_err(internal_error_tuple)?;
    Ok(Json(response))
}
fn normalize_upstream_account_attempt_filters(
    params: &ListUpstreamAccountAttemptsQuery,
) -> UpstreamAccountAttemptFilters {
    let attempt_type = params
        .attempt_type
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_ascii_lowercase)
        .filter(|value| {
            matches!(
                value.as_str(),
                ACCOUNT_ATTEMPT_TYPE_NORMAL
                    | ACCOUNT_ATTEMPT_TYPE_REMOTE_V2
                    | ACCOUNT_ATTEMPT_TYPE_IMAGE
                    | ACCOUNT_ATTEMPT_TYPE_COMPACT
            )
        });
    let model = params
        .model
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);
    let sticky_key = params
        .sticky_key
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);
    UpstreamAccountAttemptFilters {
        attempt_type,
        model,
        sticky_key,
    }
}

fn push_upstream_account_attempt_scope<'a>(
    query: &mut QueryBuilder<'a, Sqlite>,
    account_id: i64,
    cutoff: &'a str,
) {
    query
        .push(" WHERE attempts.upstream_account_id = ")
        .push_bind(account_id)
        .push(" AND attempts.occurred_at >= ")
        .push_bind(cutoff);
}

fn push_upstream_account_attempt_filters(
    query: &mut QueryBuilder<'_, Sqlite>,
    filters: &UpstreamAccountAttemptFilters,
    include_sticky_key: bool,
) {
    if let Some(attempt_type) = filters.attempt_type.as_deref() {
        query.push(" AND ");
        push_upstream_account_attempt_type_condition(query, attempt_type);
    }

    if let Some(model) = filters.model.as_deref() {
        let normalized_model = model.trim().to_ascii_lowercase();
        query.push(" AND (LOWER(TRIM(COALESCE(");
        query.push(ACCOUNT_ATTEMPT_MODEL_SQL);
        query.push(", ''))) = ");
        query.push_bind(normalized_model.clone());
        query.push(" OR LOWER(TRIM(COALESCE(");
        query.push(ACCOUNT_ATTEMPT_REQUEST_MODEL_SQL);
        query.push(", ''))) = ");
        query.push_bind(normalized_model.clone());
        query.push(" OR LOWER(TRIM(COALESCE(");
        query.push(ACCOUNT_ATTEMPT_RESPONSE_MODEL_SQL);
        query.push(", ''))) = ");
        query.push_bind(normalized_model);
        query.push(" OR LOWER(TRIM(COALESCE(inv.model, ''))) = ");
        query.push_bind(model.trim().to_ascii_lowercase());
        query.push(")");
    }

    if include_sticky_key && let Some(sticky_key) = filters.sticky_key.as_deref() {
        if sticky_key == ACCOUNT_ATTEMPT_STICKY_KEY_UNBOUND {
            query.push(" AND (attempts.sticky_key IS NULL OR TRIM(attempts.sticky_key) = '')");
        } else {
            query
                .push(" AND LOWER(TRIM(COALESCE(attempts.sticky_key, ''))) = ")
                .push_bind(sticky_key.trim().to_ascii_lowercase());
        }
    }
}

fn push_upstream_account_attempt_type_condition(
    query: &mut QueryBuilder<'_, Sqlite>,
    attempt_type: &str,
) {
    match attempt_type {
        ACCOUNT_ATTEMPT_TYPE_IMAGE => push_upstream_account_attempt_image_condition(query),
        ACCOUNT_ATTEMPT_TYPE_REMOTE_V2 => push_upstream_account_attempt_remote_v2_condition(query),
        ACCOUNT_ATTEMPT_TYPE_COMPACT => push_upstream_account_attempt_compact_condition(query),
        ACCOUNT_ATTEMPT_TYPE_NORMAL => {
            query.push("(").push("NOT ");
            push_upstream_account_attempt_image_condition(query);
            query.push(" AND NOT ");
            push_upstream_account_attempt_remote_v2_condition(query);
            query.push(" AND NOT ");
            push_upstream_account_attempt_compact_condition(query);
            query.push(")");
        }
        _ => {
            query.push("1 = 1");
        }
    }
}

fn push_upstream_account_attempt_image_condition(query: &mut QueryBuilder<'_, Sqlite>) {
    query
        .push("(attempts.endpoint LIKE ")
        .push_bind(ACCOUNT_ATTEMPT_IMAGE_ENDPOINT_PREFIX)
        .push(" OR ")
        .push("COALESCE(")
        .push(ACCOUNT_ATTEMPT_IMAGE_INTENT_SQL)
        .push(", '') IN ('yes', 'direct_image'))");
}

fn push_upstream_account_attempt_remote_v2_condition(query: &mut QueryBuilder<'_, Sqlite>) {
    query
        .push("(attempts.endpoint = ")
        .push_bind(ACCOUNT_ATTEMPT_RESPONSES_ENDPOINT)
        .push(" AND (")
        .push("(")
        .push("COALESCE(")
        .push(ACCOUNT_ATTEMPT_COMPACTION_REQUEST_KIND_SQL)
        .push(", '') = 'remote_v2' AND LOWER(TRIM(COALESCE(inv.status, attempts.status, ''))) IN ('running', 'pending')) OR COALESCE(")
        .push(ACCOUNT_ATTEMPT_COMPACTION_RESPONSE_KIND_SQL)
        .push(", '') = 'remote_v2'))");
}

fn push_upstream_account_attempt_compact_condition(query: &mut QueryBuilder<'_, Sqlite>) {
    query
        .push("(attempts.endpoint = ")
        .push_bind(ACCOUNT_ATTEMPT_COMPACT_ENDPOINT)
        .push(" OR ")
        .push("COALESCE(")
        .push(ACCOUNT_ATTEMPT_COMPACTION_REQUEST_KIND_SQL)
        .push(", '') = 'compact' OR COALESCE(")
        .push(ACCOUNT_ATTEMPT_COMPACTION_RESPONSE_KIND_SQL)
        .push(", '') = 'compact')");
}

async fn load_upstream_account_attempt_total(
    pool: &Pool<Sqlite>,
    account_id: i64,
    cutoff: &str,
    filters: &UpstreamAccountAttemptFilters,
) -> Result<usize> {
    let mut query = QueryBuilder::<Sqlite>::new(
        r#"
        SELECT COUNT(*)
        FROM pool_upstream_request_attempts AS attempts
        LEFT JOIN codex_invocations AS inv
            ON inv.invoke_id = attempts.invoke_id
           AND inv.occurred_at = attempts.occurred_at
        "#,
    );
    push_upstream_account_attempt_scope(&mut query, account_id, cutoff);
    push_upstream_account_attempt_filters(&mut query, filters, true);
    let total = query
        .build_query_scalar::<i64>()
        .fetch_one(pool)
        .await?
        .max(0) as usize;
    Ok(total)
}

async fn load_upstream_account_attempt_sticky_key_options(
    pool: &Pool<Sqlite>,
    account_id: i64,
    cutoff: &str,
    filters: &UpstreamAccountAttemptFilters,
) -> Result<Vec<UpstreamAccountAttemptStickyKeyOption>> {
    let mut query = QueryBuilder::<Sqlite>::new(
        r#"
        SELECT
            CASE
                WHEN attempts.sticky_key IS NULL OR TRIM(attempts.sticky_key) = '' THEN
        "#,
    );
    query.push_bind(ACCOUNT_ATTEMPT_STICKY_KEY_UNBOUND).push(
        r#"
                ELSE TRIM(attempts.sticky_key)
            END AS value,
            MAX(attempts.created_at) AS latest_created_at
        FROM pool_upstream_request_attempts AS attempts
        LEFT JOIN codex_invocations AS inv
            ON inv.invoke_id = attempts.invoke_id
           AND inv.occurred_at = attempts.occurred_at
        "#,
    );
    push_upstream_account_attempt_scope(&mut query, account_id, cutoff);
    push_upstream_account_attempt_filters(&mut query, filters, false);
    query.push(
        r#"
        GROUP BY value
        ORDER BY MAX(attempts.created_at) DESC, value ASC
        "#,
    );
    let rows = query
        .build_query_as::<UpstreamAccountAttemptStickyKeyOption>()
        .fetch_all(pool)
        .await?;
    Ok(rows)
}
