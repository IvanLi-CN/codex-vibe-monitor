use super::*;

pub(crate) async fn build_account_sticky_keys_response(
    pool: &Pool<Sqlite>,
    account_id: i64,
    selection: AccountStickyKeySelection,
) -> Result<AccountStickyKeysResponse> {
    let range_end = Utc::now();
    let range_start = range_end - ChronoDuration::hours(selection.activity_window_hours());
    let range_start_bound = db_occurred_at_lower_bound(range_start);
    let routes = load_account_sticky_routes(pool, account_id).await?;
    if routes.is_empty() {
        return Ok(AccountStickyKeysResponse {
            range_start: format_utc_iso(range_start),
            range_end: format_utc_iso(range_end),
            selection_mode: selection.selection_mode(),
            selected_limit: selection.selected_limit(),
            selected_activity_hours: selection.selected_activity_hours(),
            implicit_filter: selection.implicit_filter(AccountStickyKeyFilteredCounts::default()),
            conversations: Vec::new(),
        });
    }

    let attached_keys = routes
        .iter()
        .map(|row| row.sticky_key.clone())
        .collect::<Vec<_>>();
    // Keep this endpoint strictly read-only. Proxy/runtime persistence updates
    // `upstream_sticky_key_hourly` inline via `upsert_invocation_hourly_rollups_tx`
    // (plus recompute-on-repair paths), so attached sticky-key totals stay fresh
    // without request-time catch-up.
    let aggregates = query_account_sticky_key_aggregates(pool, account_id, &attached_keys).await?;
    let events =
        query_account_sticky_key_events(pool, account_id, &range_start_bound, &attached_keys)
            .await?;

    let mut aggregate_map = aggregates
        .into_iter()
        .map(|row| (row.sticky_key.clone(), row))
        .collect::<HashMap<_, _>>();
    let mut grouped_events: HashMap<String, Vec<AccountStickyKeyRequestPoint>> = HashMap::new();
    for row in events {
        let status = if row.status.trim().is_empty() {
            "unknown".to_string()
        } else {
            row.status.trim().to_string()
        };
        let request_tokens = row.request_tokens.max(0);
        let points = grouped_events.entry(row.sticky_key.clone()).or_default();
        let cumulative_tokens = points
            .last()
            .map(|point| point.cumulative_tokens)
            .unwrap_or(0)
            + request_tokens;
        points.push(AccountStickyKeyRequestPoint {
            occurred_at: row.occurred_at,
            status: status.clone(),
            is_success: status.eq_ignore_ascii_case("success"),
            request_tokens,
            cumulative_tokens,
        });
    }

    let mut conversations = routes
        .into_iter()
        .map(|route| {
            let aggregate = aggregate_map.remove(&route.sticky_key);
            let last24h_requests = grouped_events.remove(&route.sticky_key).unwrap_or_default();
            AccountStickyKeyConversation {
                sticky_key: route.sticky_key.clone(),
                request_count: aggregate.as_ref().map(|row| row.request_count).unwrap_or(0),
                total_tokens: aggregate.as_ref().map(|row| row.total_tokens).unwrap_or(0),
                total_cost: aggregate.as_ref().map(|row| row.total_cost).unwrap_or(0.0),
                created_at: aggregate
                    .as_ref()
                    .map(|row| row.created_at.clone())
                    .unwrap_or_else(|| route.created_at.clone()),
                last_activity_at: aggregate
                    .as_ref()
                    .map(|row| row.last_activity_at.clone())
                    .unwrap_or_else(|| route.last_seen_at.clone()),
                recent_invocations: Vec::new(),
                last24h_requests,
            }
        })
        .collect::<Vec<_>>();
    conversations.sort_by(|left, right| {
        let left_last_24h = left
            .last24h_requests
            .last()
            .map(|point| point.occurred_at.as_str())
            .unwrap_or("");
        let right_last_24h = right
            .last24h_requests
            .last()
            .map(|point| point.occurred_at.as_str())
            .unwrap_or("");
        right_last_24h
            .cmp(left_last_24h)
            .then_with(|| right.last_activity_at.cmp(&left.last_activity_at))
            .then_with(|| right.created_at.cmp(&left.created_at))
            .then_with(|| left.sticky_key.cmp(&right.sticky_key))
    });

    let mut filtered_counts = AccountStickyKeyFilteredCounts::default();
    if matches!(selection, AccountStickyKeySelection::ActivityWindow(_)) {
        filtered_counts.inactive_count = conversations
            .iter()
            .filter(|conversation| conversation.last24h_requests.is_empty())
            .count() as i64;
        conversations.retain(|conversation| !conversation.last24h_requests.is_empty());
    }

    filtered_counts.capped_count = conversations
        .len()
        .saturating_sub(selection.display_limit().max(0) as usize)
        as i64;
    conversations.truncate(selection.display_limit().max(0) as usize);

    let selected_keys = conversations
        .iter()
        .map(|conversation| conversation.sticky_key.clone())
        .collect::<Vec<_>>();
    let preview_range_start_bound = match selection {
        AccountStickyKeySelection::ActivityWindow(_) => Some(range_start_bound.as_str()),
        AccountStickyKeySelection::Count(_) => None,
    };
    let preview_rows = query_account_sticky_key_recent_invocations(
        pool,
        account_id,
        &selected_keys,
        5,
        preview_range_start_bound,
    )
    .await?;
    let mut grouped_preview_rows: HashMap<
        String,
        Vec<crate::api::PromptCacheConversationInvocationPreviewResponse>,
    > = HashMap::new();
    for row in preview_rows {
        grouped_preview_rows
            .entry(row.sticky_key.clone())
            .or_default()
            .push(
                crate::api::PromptCacheConversationInvocationPreviewResponse {
                    id: row.id,
                    invoke_id: row.invoke_id,
                    prompt_cache_key: Some(row.sticky_key.clone()),
                    occurred_at: row.occurred_at,
                    status: row.status,
                    live_phase: None,
                    failure_class: row.failure_class,
                    route_mode: row.route_mode,
                    model: row.model,
                    request_model: row.request_model,
                    response_model: row.response_model,
                    total_tokens: row.total_tokens,
                    cost: row.cost,
                    proxy_display_name: row.proxy_display_name,
                    upstream_account_id: row.upstream_account_id,
                    upstream_account_name: row.upstream_account_name,
                    upstream_account_plan_type: None,
                    endpoint: row.endpoint,
                    image_intent: row.image_intent,
                    source: row.source,
                    input_tokens: row.input_tokens,
                    output_tokens: row.output_tokens,
                    cache_input_tokens: row.cache_input_tokens,
                    reasoning_tokens: row.reasoning_tokens,
                    reasoning_effort: row.reasoning_effort,
                    error_message: row.error_message,
                    downstream_status_code: row.downstream_status_code,
                    downstream_error_message: row.downstream_error_message,
                    failure_kind: row.failure_kind,
                    blocked_binding: None,
                    is_actionable: row.is_actionable.map(|value| value != 0),
                    response_content_encoding: row.response_content_encoding,
                    request_compression_algorithm: row.request_compression_algorithm,
                    transport: row.transport,
                    requested_service_tier: row.requested_service_tier,
                    service_tier: row.service_tier,
                    billing_service_tier: row.billing_service_tier,
                    compaction_request_kind: row.compaction_request_kind,
                    compaction_response_kind: row.compaction_response_kind,
                    t_req_read_ms: row.t_req_read_ms,
                    t_req_parse_ms: row.t_req_parse_ms,
                    t_upstream_connect_ms: row.t_upstream_connect_ms,
                    t_upstream_ttfb_ms: row.t_upstream_ttfb_ms,
                    first_token_ms: row.first_token_ms,
                    t_upstream_stream_ms: row.t_upstream_stream_ms,
                    t_resp_parse_ms: row.t_resp_parse_ms,
                    t_persist_ms: row.t_persist_ms,
                    t_total_ms: row.t_total_ms,
                },
            );
    }
    for conversation in &mut conversations {
        conversation.recent_invocations = grouped_preview_rows
            .remove(&conversation.sticky_key)
            .unwrap_or_default();
    }

    Ok(AccountStickyKeysResponse {
        range_start: format_utc_iso(range_start),
        range_end: format_utc_iso(range_end),
        selection_mode: selection.selection_mode(),
        selected_limit: selection.selected_limit(),
        selected_activity_hours: selection.selected_activity_hours(),
        implicit_filter: selection.implicit_filter(filtered_counts),
        conversations,
    })
}

pub(crate) async fn load_account_sticky_routes(
    pool: &Pool<Sqlite>,
    account_id: i64,
) -> Result<Vec<PoolStickyRouteRow>> {
    sqlx::query_as::<_, PoolStickyRouteRow>(
        r#"
        SELECT
            sticky_key,
            account_id,
            MIN(created_at) AS created_at,
            MAX(updated_at) AS updated_at,
            MAX(last_seen_at) AS last_seen_at
        FROM (
            SELECT sticky_key, account_id, created_at, updated_at, last_seen_at
            FROM pool_sticky_routes
            WHERE account_id = ?1
            UNION ALL
            SELECT sticky_key, account_id, created_at, updated_at, last_seen_at
            FROM pool_sticky_model_routes
            WHERE account_id = ?1
        )
        GROUP BY sticky_key, account_id
        ORDER BY updated_at DESC, last_seen_at DESC, sticky_key ASC
        "#,
    )
    .bind(account_id)
    .fetch_all(pool)
    .await
    .map_err(Into::into)
}

pub(crate) async fn query_account_sticky_key_aggregates(
    pool: &Pool<Sqlite>,
    account_id: i64,
    selected_keys: &[String],
) -> Result<Vec<StickyKeyAggregateRow>> {
    if selected_keys.is_empty() {
        return Ok(Vec::new());
    }
    let mut query = QueryBuilder::<Sqlite>::new(
        "SELECT sticky_key, \
             SUM(request_count) AS request_count, \
             SUM(total_tokens) AS total_tokens, \
             SUM(total_cost) AS total_cost, \
             MIN(first_seen_at) AS created_at, \
             MAX(last_seen_at) AS last_activity_at \
         FROM upstream_sticky_key_hourly \
         WHERE upstream_account_id = ",
    );
    query.push_bind(account_id).push(" AND sticky_key IN (");
    {
        let mut separated = query.separated(", ");
        for key in selected_keys {
            separated.push_bind(key);
        }
    }
    query.push(") GROUP BY sticky_key");

    query
        .build_query_as::<StickyKeyAggregateRow>()
        .fetch_all(pool)
        .await
        .map_err(Into::into)
}

pub(crate) async fn query_account_sticky_key_events(
    pool: &Pool<Sqlite>,
    account_id: i64,
    range_start_bound: &str,
    selected_keys: &[String],
) -> Result<Vec<StickyKeyEventRow>> {
    if selected_keys.is_empty() {
        return Ok(Vec::new());
    }
    const ACCOUNT_EXPR: &str = "CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.upstreamAccountId') AS INTEGER) END";

    let mut query = QueryBuilder::<Sqlite>::new(
        "SELECT occurred_at, COALESCE(status, 'unknown') AS status, COALESCE(total_tokens, 0) AS request_tokens, ",
    );
    query
        .push(crate::api::INVOCATION_STICKY_KEY_SQL)
        .push(" AS sticky_key FROM codex_invocations WHERE occurred_at >= ")
        .push_bind(range_start_bound)
        .push(" AND ")
        .push(ACCOUNT_EXPR)
        .push(" = ")
        .push_bind(account_id)
        .push(" AND ")
        .push(crate::api::INVOCATION_STICKY_KEY_SQL)
        .push(" IN (");
    {
        let mut separated = query.separated(", ");
        for key in selected_keys {
            separated.push_bind(key);
        }
    }
    query.push(") ORDER BY sticky_key ASC, occurred_at ASC, id ASC");

    query
        .build_query_as::<StickyKeyEventRow>()
        .fetch_all(pool)
        .await
        .map_err(Into::into)
}

pub(crate) async fn query_account_sticky_key_recent_invocations(
    pool: &Pool<Sqlite>,
    account_id: i64,
    selected_keys: &[String],
    limit_per_key: i64,
    range_start_bound: Option<&str>,
) -> Result<Vec<AccountStickyKeyInvocationPreviewRow>> {
    if selected_keys.is_empty() || limit_per_key <= 0 {
        return Ok(Vec::new());
    }

    let mut query =
        QueryBuilder::<Sqlite>::new("WITH ranked AS (SELECT id, invoke_id, occurred_at, ");
    query
        .push(crate::api::invocation_display_status_sql())
        .push(" AS status, ")
        .push(crate::api::INVOCATION_RESOLVED_FAILURE_CLASS_SQL)
        .push(" AS failure_class, ")
        .push(crate::api::INVOCATION_ROUTE_MODE_SQL)
        .push(" AS route_mode, model, ")
        .push(crate::api::INVOCATION_REQUEST_MODEL_SQL)
        .push(" AS request_model, ")
        .push(crate::api::INVOCATION_RESPONSE_MODEL_SQL)
        .push(" AS response_model, COALESCE(total_tokens, 0) AS total_tokens, cost, source, input_tokens, output_tokens, cache_input_tokens, reasoning_tokens, ")
        .push(crate::api::INVOCATION_REASONING_EFFORT_SQL)
        .push(" AS reasoning_effort, error_message, ")
        .push(crate::api::INVOCATION_FAILURE_KIND_SQL)
        .push(" AS failure_kind, CASE WHEN ")
        .push(crate::api::INVOCATION_RESOLVED_FAILURE_CLASS_SQL)
        .push(" = 'service_failure' THEN 1 ELSE 0 END AS is_actionable, ")
        .push(crate::api::INVOCATION_PROXY_DISPLAY_SQL)
        .push(" AS proxy_display_name, ")
        .push(crate::api::INVOCATION_UPSTREAM_ACCOUNT_ID_SQL)
        .push(" AS upstream_account_id, ")
        .push(crate::api::INVOCATION_UPSTREAM_ACCOUNT_NAME_SQL)
        .push(" AS upstream_account_name, ")
        .push(crate::api::INVOCATION_RESPONSE_CONTENT_ENCODING_SQL)
        .push(
            " AS response_content_encoding, \
             ",
        )
        .push(
            crate::api::invocation_request_compression_algorithm_with_attempt_fallback_sql(
                "codex_invocations",
            ),
        )
        .push(" AS request_compression_algorithm, ")
        .push(crate::api::INVOCATION_TRANSPORT_SQL)
        .push(
            " AS transport, \
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
        .push(crate::api::INVOCATION_BILLING_SERVICE_TIER_SQL)
        .push(
            " AS billing_service_tier, \
             t_req_read_ms, t_req_parse_ms, t_upstream_connect_ms, t_upstream_ttfb_ms, first_token_ms, \
             t_upstream_stream_ms, t_resp_parse_ms, t_persist_ms, t_total_ms, ",
        )
        .push(crate::api::INVOCATION_DOWNSTREAM_STATUS_CODE_SQL)
        .push(" AS downstream_status_code, ")
        .push(crate::api::INVOCATION_DOWNSTREAM_ERROR_MESSAGE_SQL)
        .push(" AS downstream_error_message, ")
        .push(crate::api::INVOCATION_ENDPOINT_SQL)
        .push(" AS endpoint, ")
        .push(crate::api::INVOCATION_COMPACTION_REQUEST_KIND_SQL)
        .push(" AS compaction_request_kind, ")
        .push(crate::api::INVOCATION_COMPACTION_RESPONSE_KIND_SQL)
        .push(" AS compaction_response_kind, ")
        .push(crate::api::INVOCATION_IMAGE_INTENT_SQL)
        .push(" AS image_intent, ")
        .push(crate::api::INVOCATION_STICKY_KEY_SQL)
        .push(" AS sticky_key, ROW_NUMBER() OVER (PARTITION BY ")
        .push(crate::api::INVOCATION_STICKY_KEY_SQL)
        .push(" ORDER BY occurred_at DESC, id DESC) AS row_number FROM codex_invocations WHERE ")
        .push(crate::api::INVOCATION_UPSTREAM_ACCOUNT_ID_SQL)
        .push(" = ")
        .push_bind(account_id);

    if let Some(range_start_bound) = range_start_bound {
        query
            .push(" AND occurred_at >= ")
            .push_bind(range_start_bound);
    }

    query
        .push(" AND ")
        .push(crate::api::INVOCATION_STICKY_KEY_SQL)
        .push(" IN (");

    {
        let mut separated = query.separated(", ");
        for key in selected_keys {
            separated.push_bind(key);
        }
    }

    query
        .push(")) SELECT sticky_key, id, invoke_id, occurred_at, status, failure_class, route_mode, model, request_model, response_model, total_tokens, cost, source, input_tokens, output_tokens, cache_input_tokens, reasoning_tokens, reasoning_effort, error_message, downstream_status_code, downstream_error_message, failure_kind, is_actionable, proxy_display_name, upstream_account_id, upstream_account_name, response_content_encoding, request_compression_algorithm, transport, requested_service_tier, service_tier, billing_service_tier, t_req_read_ms, t_req_parse_ms, t_upstream_connect_ms, t_upstream_ttfb_ms, first_token_ms, t_upstream_stream_ms, t_resp_parse_ms, t_persist_ms, t_total_ms, endpoint, compaction_request_kind, compaction_response_kind, image_intent FROM ranked WHERE row_number <= ")
        .push_bind(limit_per_key)
        .push(" ORDER BY sticky_key ASC, occurred_at DESC, id DESC");

    query
        .build_query_as::<AccountStickyKeyInvocationPreviewRow>()
        .fetch_all(pool)
        .await
        .map_err(Into::into)
}

pub(crate) async fn load_sticky_route(
    pool: &Pool<Sqlite>,
    sticky_key: &str,
) -> Result<Option<PoolStickyRouteRow>> {
    sqlx::query_as::<_, PoolStickyRouteRow>(
        r#"
        SELECT sticky_key, account_id, created_at, updated_at, last_seen_at
        FROM pool_sticky_routes
        WHERE sticky_key = ?1
        LIMIT 1
        "#,
    )
    .bind(sticky_key)
    .fetch_optional(pool)
    .await
    .map_err(Into::into)
}

pub(crate) const STICKY_AFFINITY_TOKEN_FACTOR: i64 = 1_000_000_000;

pub(crate) fn normalize_sticky_model_key(model: Option<&str>) -> Option<String> {
    let model = model?.trim();
    if model.is_empty() {
        return None;
    }
    let alias = crate::proxy::dated_model_alias_base(model).unwrap_or(model);
    Some(alias.to_ascii_lowercase())
}

pub(crate) fn pack_sticky_affinity_token(epoch: i64, model_generation: i64) -> i64 {
    epoch
        .saturating_mul(STICKY_AFFINITY_TOKEN_FACTOR)
        .saturating_add(model_generation.rem_euclid(STICKY_AFFINITY_TOKEN_FACTOR))
}

pub(crate) fn unpack_sticky_affinity_token(token: i64) -> (i64, i64) {
    (
        token.div_euclid(STICKY_AFFINITY_TOKEN_FACTOR),
        token.rem_euclid(STICKY_AFFINITY_TOKEN_FACTOR),
    )
}

async fn load_sticky_model_route_executor<'e, E>(
    executor: E,
    sticky_key: &str,
    model_key: &str,
) -> Result<Option<PoolStickyRouteRow>>
where
    E: sqlx::Executor<'e, Database = Sqlite>,
{
    sqlx::query_as::<_, PoolStickyRouteRow>(
        r#"
        SELECT sticky_key, account_id, created_at, updated_at, last_seen_at
        FROM pool_sticky_model_routes
        WHERE sticky_key = ?1 AND model_key = ?2
        LIMIT 1
        "#,
    )
    .bind(sticky_key)
    .bind(model_key)
    .fetch_optional(executor)
    .await
    .map_err(Into::into)
}

pub(crate) async fn load_sticky_model_generation_executor<'e, E>(
    executor: E,
    sticky_key: &str,
    model_key: &str,
) -> Result<i64>
where
    E: sqlx::Executor<'e, Database = Sqlite>,
{
    sqlx::query_scalar::<_, i64>(
        r#"
        SELECT generation
        FROM pool_sticky_model_route_generations
        WHERE sticky_key = ?1 AND model_key = ?2
        LIMIT 1
        "#,
    )
    .bind(sticky_key)
    .bind(model_key)
    .fetch_optional(executor)
    .await
    .map(|generation| generation.unwrap_or_default())
    .map_err(Into::into)
}

pub(crate) async fn load_sticky_route_with_model_generation(
    pool: &Pool<Sqlite>,
    sticky_key: &str,
    requested_model: Option<&str>,
) -> Result<(Option<PoolStickyRouteRow>, i64)> {
    let Some(model_key) = normalize_sticky_model_key(requested_model) else {
        return load_sticky_route_with_generation(pool, sticky_key).await;
    };
    let mut tx = pool.begin().await?;
    let exact = load_sticky_model_route_executor(&mut *tx, sticky_key, &model_key).await?;
    let fallback = if exact.is_none() {
        sqlx::query_as::<_, PoolStickyRouteRow>(
            r#"
            SELECT sticky_key, account_id, created_at, updated_at, last_seen_at
            FROM pool_sticky_routes
            WHERE sticky_key = ?1
            LIMIT 1
            "#,
        )
        .bind(sticky_key)
        .fetch_optional(&mut *tx)
        .await?
    } else {
        None
    };
    let epoch = load_sticky_affinity_generation_executor(&mut *tx, sticky_key).await?;
    let generation =
        load_sticky_model_generation_executor(&mut *tx, sticky_key, &model_key).await?;
    tx.commit().await?;
    Ok((
        exact.or(fallback),
        pack_sticky_affinity_token(epoch, generation),
    ))
}

pub(crate) async fn upsert_sticky_model_route_executor<'e, E>(
    executor: E,
    sticky_key: &str,
    model_key: &str,
    account_id: i64,
    now_iso: &str,
) -> Result<()>
where
    E: sqlx::Executor<'e, Database = Sqlite>,
{
    sqlx::query(
        r#"
        INSERT INTO pool_sticky_model_routes (
            sticky_key, model_key, account_id, created_at, updated_at, last_seen_at
        ) VALUES (?1, ?2, ?3, ?4, ?4, ?4)
        ON CONFLICT(sticky_key, model_key) DO UPDATE SET
            account_id = excluded.account_id,
            updated_at = excluded.updated_at,
            last_seen_at = excluded.last_seen_at
        "#,
    )
    .bind(sticky_key)
    .bind(model_key)
    .bind(account_id)
    .bind(now_iso)
    .execute(executor)
    .await?;
    Ok(())
}

pub(crate) async fn bump_sticky_model_generation_executor<'e, E>(
    executor: E,
    sticky_key: &str,
    model_key: &str,
    now_iso: &str,
) -> Result<i64>
where
    E: sqlx::Executor<'e, Database = Sqlite>,
{
    sqlx::query_scalar::<_, i64>(
        r#"
        INSERT INTO pool_sticky_model_route_generations (
            sticky_key,
            model_key,
            generation,
            last_clear_cause_attempt_public_id,
            last_clear_cause_http_status,
            updated_at
        ) VALUES (?1, ?2, 1, NULL, NULL, ?3)
        ON CONFLICT(sticky_key, model_key) DO UPDATE SET
            generation = pool_sticky_model_route_generations.generation + 1,
            last_clear_cause_attempt_public_id = NULL,
            last_clear_cause_http_status = NULL,
            updated_at = excluded.updated_at
        RETURNING generation
        "#,
    )
    .bind(sticky_key)
    .bind(model_key)
    .bind(now_iso)
    .fetch_one(executor)
    .await
    .map_err(Into::into)
}

pub(crate) async fn bump_sticky_model_generation_with_clear_cause_executor<'e, E>(
    executor: E,
    sticky_key: &str,
    model_key: &str,
    cause_attempt_public_id: Option<&str>,
    cause_http_status: Option<i64>,
    now_iso: &str,
) -> Result<i64>
where
    E: sqlx::Executor<'e, Database = Sqlite>,
{
    sqlx::query_scalar::<_, i64>(
        r#"
        INSERT INTO pool_sticky_model_route_generations (
            sticky_key,
            model_key,
            generation,
            last_clear_cause_attempt_public_id,
            last_clear_cause_http_status,
            updated_at
        ) VALUES (?1, ?2, 1, ?3, ?4, ?5)
        ON CONFLICT(sticky_key, model_key) DO UPDATE SET
            generation = pool_sticky_model_route_generations.generation + 1,
            last_clear_cause_attempt_public_id = excluded.last_clear_cause_attempt_public_id,
            last_clear_cause_http_status = excluded.last_clear_cause_http_status,
            updated_at = excluded.updated_at
        RETURNING generation
        "#,
    )
    .bind(sticky_key)
    .bind(model_key)
    .bind(cause_attempt_public_id)
    .bind(cause_http_status)
    .bind(now_iso)
    .fetch_one(executor)
    .await
    .map_err(Into::into)
}

pub(crate) async fn upsert_sticky_route_for_model_if_current(
    pool: &Pool<Sqlite>,
    sticky_key: &str,
    requested_model: Option<&str>,
    account_id: i64,
    expected_token: Option<i64>,
    now_iso: &str,
) -> Result<bool> {
    let Some(model_key) = normalize_sticky_model_key(requested_model) else {
        return upsert_sticky_route_and_bump_generation_if_changed(
            pool, sticky_key, account_id, now_iso,
        )
        .await;
    };
    let mut conn = pool.acquire().await?;
    sqlx::query("BEGIN IMMEDIATE")
        .execute(conn.as_mut())
        .await?;
    let outcome: Result<bool> = async {
        if let Some(expected_token) = expected_token {
            let (expected_epoch, expected_generation) = unpack_sticky_affinity_token(expected_token);
            let epoch = load_sticky_affinity_generation_executor(conn.as_mut(), sticky_key).await?;
            let generation =
                load_sticky_model_generation_executor(conn.as_mut(), sticky_key, &model_key).await?;
            if epoch != expected_epoch || generation != expected_generation {
                return Ok(false);
            }
        }
        let previous = sqlx::query_scalar::<_, i64>(
            "SELECT account_id FROM pool_sticky_model_routes WHERE sticky_key = ?1 AND model_key = ?2 LIMIT 1",
        )
        .bind(sticky_key)
        .bind(&model_key)
        .fetch_optional(conn.as_mut())
        .await?;
        let changed = previous != Some(account_id);
        upsert_sticky_model_route_executor(conn.as_mut(), sticky_key, &model_key, account_id, now_iso)
            .await?;
        if changed {
            bump_sticky_model_generation_executor(conn.as_mut(), sticky_key, &model_key, now_iso)
                .await?;
        }
        Ok(changed)
    }
    .await;
    match outcome {
        Ok(changed) => {
            sqlx::query("COMMIT").execute(conn.as_mut()).await?;
            Ok(changed)
        }
        Err(error) => {
            let _ = sqlx::query("ROLLBACK").execute(conn.as_mut()).await;
            Err(error)
        }
    }
}

pub(crate) async fn load_sticky_route_with_generation(
    pool: &Pool<Sqlite>,
    sticky_key: &str,
) -> Result<(Option<PoolStickyRouteRow>, i64)> {
    let mut tx = pool.begin().await?;
    let route = sqlx::query_as::<_, PoolStickyRouteRow>(
        r#"
        SELECT sticky_key, account_id, created_at, updated_at, last_seen_at
        FROM pool_sticky_routes
        WHERE sticky_key = ?1
        LIMIT 1
        "#,
    )
    .bind(sticky_key)
    .fetch_optional(&mut *tx)
    .await?;
    let generation = sqlx::query_scalar::<_, i64>(
        r#"
        SELECT generation
        FROM pool_sticky_route_generations
        WHERE sticky_key = ?1
        LIMIT 1
        "#,
    )
    .bind(sticky_key)
    .fetch_optional(&mut *tx)
    .await?
    .unwrap_or_default();
    tx.commit().await?;
    Ok((route, generation))
}

pub(crate) async fn load_sticky_affinity_generation_executor<'e, E>(
    executor: E,
    sticky_key: &str,
) -> Result<i64>
where
    E: sqlx::Executor<'e, Database = Sqlite>,
{
    sqlx::query_scalar::<_, i64>(
        r#"
        SELECT generation
        FROM pool_sticky_route_generations
        WHERE sticky_key = ?1
        LIMIT 1
        "#,
    )
    .bind(sticky_key)
    .fetch_optional(executor)
    .await
    .map(|generation| generation.unwrap_or_default())
    .map_err(Into::into)
}

pub(crate) async fn load_sticky_affinity_generation(
    pool: &Pool<Sqlite>,
    sticky_key: &str,
) -> Result<i64> {
    load_sticky_affinity_generation_executor(pool, sticky_key).await
}

pub(crate) async fn bump_sticky_affinity_generation_executor<'e, E>(
    executor: E,
    sticky_key: &str,
    now_iso: &str,
) -> Result<i64>
where
    E: sqlx::Executor<'e, Database = Sqlite>,
{
    sqlx::query_scalar::<_, i64>(
        r#"
        INSERT INTO pool_sticky_route_generations (
            sticky_key,
            generation,
            last_clear_cause_attempt_public_id,
            last_clear_cause_http_status,
            updated_at
        ) VALUES (?1, 1, NULL, NULL, ?2)
        ON CONFLICT(sticky_key) DO UPDATE SET
            generation = pool_sticky_route_generations.generation + 1,
            last_clear_cause_attempt_public_id = NULL,
            last_clear_cause_http_status = NULL,
            updated_at = excluded.updated_at
        RETURNING generation
        "#,
    )
    .bind(sticky_key)
    .bind(now_iso)
    .fetch_one(executor)
    .await
    .map_err(Into::into)
}

pub(crate) async fn bump_sticky_affinity_generation(
    pool: &Pool<Sqlite>,
    sticky_key: &str,
    now_iso: &str,
) -> Result<i64> {
    bump_sticky_affinity_generation_executor(pool, sticky_key, now_iso).await
}

pub(crate) async fn upsert_sticky_route_executor<'e, E>(
    executor: E,
    sticky_key: &str,
    account_id: i64,
    now_iso: &str,
) -> Result<()>
where
    E: sqlx::Executor<'e, Database = Sqlite>,
{
    sqlx::query(
        r#"
        INSERT INTO pool_sticky_routes (
            sticky_key, account_id, created_at, updated_at, last_seen_at
        ) VALUES (?1, ?2, ?3, ?3, ?3)
        ON CONFLICT(sticky_key) DO UPDATE SET
            account_id = excluded.account_id,
            updated_at = excluded.updated_at,
            last_seen_at = excluded.last_seen_at
        "#,
    )
    .bind(sticky_key)
    .bind(account_id)
    .bind(now_iso)
    .execute(executor)
    .await?;
    Ok(())
}

pub(crate) async fn upsert_sticky_route(
    pool: &Pool<Sqlite>,
    sticky_key: &str,
    account_id: i64,
    now_iso: &str,
) -> Result<()> {
    upsert_sticky_route_executor(pool, sticky_key, account_id, now_iso).await
}

pub(crate) async fn upsert_sticky_route_and_bump_generation_if_changed(
    pool: &Pool<Sqlite>,
    sticky_key: &str,
    account_id: i64,
    now_iso: &str,
) -> Result<bool> {
    let mut conn = pool.acquire().await?;
    sqlx::query("BEGIN IMMEDIATE")
        .execute(conn.as_mut())
        .await?;
    let outcome: Result<bool> = async {
        let previous_account_id = sqlx::query_scalar::<_, i64>(
            "SELECT account_id FROM pool_sticky_routes WHERE sticky_key = ?1 LIMIT 1",
        )
        .bind(sticky_key)
        .fetch_optional(conn.as_mut())
        .await?;
        let target_changed = previous_account_id != Some(account_id);
        upsert_sticky_route_executor(conn.as_mut(), sticky_key, account_id, now_iso).await?;
        if target_changed {
            bump_sticky_affinity_generation_executor(conn.as_mut(), sticky_key, now_iso).await?;
        }
        Ok(target_changed)
    }
    .await;
    match outcome {
        Ok(target_changed) => {
            sqlx::query("COMMIT").execute(conn.as_mut()).await?;
            Ok(target_changed)
        }
        Err(error) => {
            let _ = sqlx::query("ROLLBACK").execute(conn.as_mut()).await;
            Err(error)
        }
    }
}

pub(crate) async fn delete_sticky_route_executor<'e, E>(executor: E, sticky_key: &str) -> Result<()>
where
    E: sqlx::Executor<'e, Database = Sqlite>,
{
    sqlx::query("DELETE FROM pool_sticky_routes WHERE sticky_key = ?1")
        .bind(sticky_key)
        .execute(executor)
        .await?;
    Ok(())
}

pub(crate) async fn delete_sticky_model_routes_executor<'e, E>(
    executor: E,
    sticky_key: &str,
) -> Result<()>
where
    E: sqlx::Executor<'e, Database = Sqlite>,
{
    sqlx::query("DELETE FROM pool_sticky_model_routes WHERE sticky_key = ?1")
        .bind(sticky_key)
        .execute(executor)
        .await?;
    Ok(())
}

pub(crate) async fn delete_sticky_route(pool: &Pool<Sqlite>, sticky_key: &str) -> Result<()> {
    delete_sticky_route_executor(pool, sticky_key).await
}

pub(crate) async fn delete_sticky_routes_for_account_executor(
    conn: &mut SqliteConnection,
    account_id: i64,
    now_iso: &str,
) -> Result<()> {
    let sticky_keys = sqlx::query_scalar::<_, String>(
        r#"
        SELECT DISTINCT sticky_key
        FROM (
            SELECT sticky_key FROM pool_sticky_routes WHERE account_id = ?1
            UNION ALL
            SELECT sticky_key FROM pool_sticky_model_routes WHERE account_id = ?1
        )
        "#,
    )
    .bind(account_id)
    .fetch_all(&mut *conn)
    .await?;
    for sticky_key in sticky_keys {
        bump_sticky_affinity_generation_executor(&mut *conn, &sticky_key, now_iso).await?;
    }
    sqlx::query("DELETE FROM pool_sticky_routes WHERE account_id = ?1")
        .bind(account_id)
        .execute(&mut *conn)
        .await?;
    sqlx::query("DELETE FROM pool_sticky_model_routes WHERE account_id = ?1")
        .bind(account_id)
        .execute(&mut *conn)
        .await?;
    Ok(())
}

/// Applies a manual conversation-level target. The fallback and every materialized
/// model route move together, while the conversation epoch fences in-flight writes.
pub(crate) async fn overwrite_sticky_routes_for_manual_binding_executor(
    conn: &mut SqliteConnection,
    sticky_key: &str,
    account_id: i64,
    now_iso: &str,
) -> Result<()> {
    let fallback_account_id = sqlx::query_scalar::<_, i64>(
        "SELECT account_id FROM pool_sticky_routes WHERE sticky_key = ?1 LIMIT 1",
    )
    .bind(sticky_key)
    .fetch_optional(&mut *conn)
    .await?;
    let model_routes = sqlx::query_as::<_, (String, i64)>(
        "SELECT model_key, account_id FROM pool_sticky_model_routes WHERE sticky_key = ?1",
    )
    .bind(sticky_key)
    .fetch_all(&mut *conn)
    .await?;

    let fallback_changed = fallback_account_id != Some(account_id);
    let model_routes_changed = model_routes
        .iter()
        .any(|(_, previous_account_id)| *previous_account_id != account_id);
    if !fallback_changed && !model_routes_changed {
        return Ok(());
    }

    bump_sticky_affinity_generation_executor(&mut *conn, sticky_key, now_iso).await?;
    if fallback_changed {
        upsert_sticky_route_executor(&mut *conn, sticky_key, account_id, now_iso).await?;
    }
    for (model_key, previous_account_id) in model_routes {
        if previous_account_id != account_id {
            upsert_sticky_model_route_executor(
                &mut *conn, sticky_key, &model_key, account_id, now_iso,
            )
            .await?;
            bump_sticky_model_generation_executor(&mut *conn, sticky_key, &model_key, now_iso)
                .await?;
        }
    }
    Ok(())
}

pub(crate) async fn overwrite_sticky_routes_for_manual_binding(
    pool: &Pool<Sqlite>,
    sticky_key: &str,
    account_id: i64,
    now_iso: &str,
) -> Result<()> {
    let mut conn = pool.acquire().await?;
    sqlx::query("BEGIN IMMEDIATE")
        .execute(conn.as_mut())
        .await?;
    let outcome = overwrite_sticky_routes_for_manual_binding_executor(
        conn.as_mut(),
        sticky_key,
        account_id,
        now_iso,
    )
    .await;
    match outcome {
        Ok(()) => {
            sqlx::query("COMMIT").execute(conn.as_mut()).await?;
            Ok(())
        }
        Err(error) => {
            let _ = sqlx::query("ROLLBACK").execute(conn.as_mut()).await;
            Err(error)
        }
    }
}

pub(crate) async fn delete_sticky_route_if_matches(
    pool: &Pool<Sqlite>,
    sticky_key: &str,
    account_id: i64,
    expected_generation: Option<i64>,
    now_iso: &str,
) -> Result<bool> {
    delete_sticky_route_if_matches_with_cause(
        pool,
        sticky_key,
        account_id,
        expected_generation,
        None,
        None,
        None,
        None,
        now_iso,
    )
    .await
}

#[expect(
    clippy::too_many_arguments,
    reason = "Sticky clear fencing carries the route identity, generation, and causal attempt metadata."
)]
pub(crate) async fn delete_sticky_route_if_matches_with_cause(
    pool: &Pool<Sqlite>,
    sticky_key: &str,
    account_id: i64,
    expected_generation: Option<i64>,
    cause_attempt_id: Option<i64>,
    cause_http_status: Option<i64>,
    cause_reason_code: Option<&str>,
    prompt_cache_key: Option<&str>,
    now_iso: &str,
) -> Result<bool> {
    let mut conn = pool.acquire().await?;
    sqlx::query("BEGIN IMMEDIATE")
        .execute(conn.as_mut())
        .await?;
    let outcome: Result<bool> = async {
        let request_model = if let Some(cause_attempt_id) = cause_attempt_id {
            sqlx::query_scalar::<_, Option<String>>(
                "SELECT request_model FROM pool_upstream_request_attempts WHERE id = ?1",
            )
            .bind(cause_attempt_id)
            .fetch_optional(conn.as_mut())
            .await?
            .flatten()
        } else {
            None
        };
        let model_key = normalize_sticky_model_key(request_model.as_deref());
        let current_epoch =
            load_sticky_affinity_generation_executor(conn.as_mut(), sticky_key).await?;
        if let Some(expected_generation) = expected_generation {
            let is_current = if let Some(model_key) = model_key.as_deref() {
                let (expected_epoch, expected_model_generation) =
                    unpack_sticky_affinity_token(expected_generation);
                let current_model_generation =
                    load_sticky_model_generation_executor(conn.as_mut(), sticky_key, model_key)
                        .await?;
                current_epoch == expected_epoch && current_model_generation == expected_model_generation
            } else {
                current_epoch == expected_generation
            };
            if !is_current {
                return Ok(false);
            }
        }
        let attempt_context = if let Some(cause_attempt_id) = cause_attempt_id {
            let attempt = sqlx::query_as::<_, (
                Option<String>,
                Option<String>,
                Option<String>,
                Option<String>,
                Option<String>,
            )>(
                "SELECT started_at, occurred_at, attempt_public_id, invoke_id, routing_source FROM pool_upstream_request_attempts WHERE id = ?1",
            )
            .bind(cause_attempt_id)
            .fetch_optional(conn.as_mut())
            .await?
            .unwrap_or((None, None, None, None, None));
            let Some(occurred_at) = attempt.0.clone().or(attempt.1.clone()) else {
                return Ok(false);
            };
            let Some(occurred_at_utc) = parse_to_utc_datetime(&occurred_at) else {
                return Ok(false);
            };
            Some((occurred_at_utc, attempt.2, attempt.3, attempt.4))
        } else {
            None
        };
        let exact_model_route = if let Some(model_key) = model_key.as_deref() {
            sqlx::query_scalar::<_, Option<String>>(
                "SELECT updated_at FROM pool_sticky_model_routes WHERE sticky_key = ?1 AND model_key = ?2 AND account_id = ?3 LIMIT 1",
            )
            .bind(sticky_key)
            .bind(model_key)
            .bind(account_id)
            .fetch_optional(conn.as_mut())
            .await?
            .flatten()
        } else {
            None
        };
        let route_updated_at = if exact_model_route.is_some() {
            exact_model_route.clone()
        } else {
            sqlx::query_scalar::<_, Option<String>>(
                "SELECT updated_at FROM pool_sticky_routes WHERE sticky_key = ?1 AND account_id = ?2 LIMIT 1",
            )
            .bind(sticky_key)
            .bind(account_id)
            .fetch_optional(conn.as_mut())
            .await?
            .flatten()
        };
        let Some(route_updated_at) = route_updated_at else {
            return Ok(false);
        };
        if let Some((cause_occurred_at_utc, _, _, _)) = attempt_context.as_ref() {
            let Some(route_updated_at) = parse_to_utc_datetime(&route_updated_at) else {
                return Ok(false);
            };
            if route_updated_at > *cause_occurred_at_utc {
                return Ok(false);
            }
        }
        let deleted = if exact_model_route.is_some() {
            sqlx::query(
                "DELETE FROM pool_sticky_model_routes WHERE sticky_key = ?1 AND model_key = ?2 AND account_id = ?3",
            )
            .bind(sticky_key)
            .bind(model_key.as_deref().expect("exact route requires a model key"))
            .bind(account_id)
            .execute(conn.as_mut())
            .await?
            .rows_affected()
                > 0
        } else {
            sqlx::query(
                "DELETE FROM pool_sticky_routes WHERE sticky_key = ?1 AND account_id = ?2",
            )
            .bind(sticky_key)
            .bind(account_id)
            .execute(conn.as_mut())
            .await?
            .rows_affected()
                > 0
        };
        if deleted {
            let cause_attempt_public_id = attempt_context
                .as_ref()
                .and_then(|(_, attempt_public_id, _, _)| attempt_public_id.as_deref());
            if let Some(model_key) = model_key.as_deref().filter(|_| exact_model_route.is_some()) {
                bump_sticky_model_generation_with_clear_cause_executor(
                    conn.as_mut(),
                    sticky_key,
                    model_key,
                    cause_attempt_public_id,
                    cause_http_status,
                    now_iso,
                )
                .await?;
            } else {
                bump_sticky_affinity_generation_executor(conn.as_mut(), sticky_key, now_iso).await?;
                sqlx::query(
                    r#"
                    UPDATE pool_sticky_route_generations
                    SET last_clear_cause_attempt_public_id = ?2,
                        last_clear_cause_http_status = ?3,
                        updated_at = ?4
                    WHERE sticky_key = ?1
                    "#,
                )
                .bind(sticky_key)
                .bind(cause_attempt_public_id)
                .bind(cause_http_status)
                .bind(now_iso)
                .execute(conn.as_mut())
                .await?;
            }
            let account_name = sqlx::query_scalar::<_, Option<String>>(
                "SELECT display_name FROM pool_upstream_accounts WHERE id = ?1",
            )
            .bind(account_id)
            .fetch_optional(conn.as_mut())
            .await?
            .flatten();
            let (trigger_attempt_id, invoke_id, routing_source) = attempt_context
                .as_ref()
                .map(|(_, attempt_public_id, invoke_id, routing_source)| {
                    (
                        attempt_public_id.clone(),
                        invoke_id.clone(),
                        routing_source.clone(),
                    )
                })
                .unwrap_or((None, None, None));
            if prompt_cache_key == Some(sticky_key) {
                crate::api::append_runtime_sticky_target_cleared_event_executor(
                    conn.as_mut(),
                    sticky_key,
                    account_id,
                    account_name,
                    now_iso,
                    invoke_id,
                    crate::api::PromptCacheConversationOperationRoutingContext {
                        reason_code: cause_reason_code
                            .unwrap_or("automaticStickyClear")
                            .to_string(),
                        routing_source,
                        routing_selection_audit: None,
                        http_status: cause_http_status.and_then(|value| u16::try_from(value).ok()),
                        trigger_attempt_id,
                        causing_attempt_id: None,
                        causing_http_status: None,
                    },
                    crate::api::PromptCacheConversationOperationRoutingScope {
                        kind: if exact_model_route.is_some() {
                            "model".to_string()
                        } else {
                            "all".to_string()
                        },
                        model_key: exact_model_route
                            .as_ref()
                            .and_then(|_| model_key.clone()),
                        request_model: exact_model_route.as_ref().and_then(|_| {
                            request_model.filter(|request_model| {
                                model_key.as_deref() != Some(request_model.as_str())
                            })
                        }),
                    },
                )
                .await?;
            }
        }
        Ok(deleted)
    }
    .await;
    match outcome {
        Ok(deleted) => {
            sqlx::query("COMMIT").execute(conn.as_mut()).await?;
            Ok(deleted)
        }
        Err(error) => {
            let _ = sqlx::query("ROLLBACK").execute(conn.as_mut()).await;
            Err(error)
        }
    }
}
