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
                    occurred_at: row.occurred_at,
                    status: row.status,
                    failure_class: row.failure_class,
                    route_mode: row.route_mode,
                    model: row.model,
                    total_tokens: row.total_tokens,
                    cost: row.cost,
                    proxy_display_name: row.proxy_display_name,
                    upstream_account_id: row.upstream_account_id,
                    upstream_account_name: row.upstream_account_name,
                    endpoint: row.endpoint,
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
                    is_actionable: row.is_actionable.map(|value| value != 0),
                    response_content_encoding: row.response_content_encoding,
                    requested_service_tier: row.requested_service_tier,
                    service_tier: row.service_tier,
                    billing_service_tier: row.billing_service_tier,
                    t_req_read_ms: row.t_req_read_ms,
                    t_req_parse_ms: row.t_req_parse_ms,
                    t_upstream_connect_ms: row.t_upstream_connect_ms,
                    t_upstream_ttfb_ms: row.t_upstream_ttfb_ms,
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
        SELECT sticky_key, account_id, created_at, updated_at, last_seen_at
        FROM pool_sticky_routes
        WHERE account_id = ?1
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
        .push(" AS route_mode, model, COALESCE(total_tokens, 0) AS total_tokens, cost, source, input_tokens, output_tokens, cache_input_tokens, reasoning_tokens, ")
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
             t_req_read_ms, t_req_parse_ms, t_upstream_connect_ms, t_upstream_ttfb_ms, \
             t_upstream_stream_ms, t_resp_parse_ms, t_persist_ms, t_total_ms, ",
        )
        .push(crate::api::INVOCATION_DOWNSTREAM_STATUS_CODE_SQL)
        .push(" AS downstream_status_code, ")
        .push(crate::api::INVOCATION_DOWNSTREAM_ERROR_MESSAGE_SQL)
        .push(" AS downstream_error_message, ")
        .push(crate::api::INVOCATION_ENDPOINT_SQL)
        .push(" AS endpoint, ")
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
        .push(")) SELECT sticky_key, id, invoke_id, occurred_at, status, failure_class, route_mode, model, total_tokens, cost, source, input_tokens, output_tokens, cache_input_tokens, reasoning_tokens, reasoning_effort, error_message, downstream_status_code, downstream_error_message, failure_kind, is_actionable, proxy_display_name, upstream_account_id, upstream_account_name, response_content_encoding, requested_service_tier, service_tier, billing_service_tier, t_req_read_ms, t_req_parse_ms, t_upstream_connect_ms, t_upstream_ttfb_ms, t_upstream_stream_ms, t_resp_parse_ms, t_persist_ms, t_total_ms, endpoint FROM ranked WHERE row_number <= ")
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

pub(crate) async fn upsert_sticky_route(
    pool: &Pool<Sqlite>,
    sticky_key: &str,
    account_id: i64,
    now_iso: &str,
) -> Result<()> {
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
    .execute(pool)
    .await?;
    Ok(())
}

pub(crate) async fn delete_sticky_route(pool: &Pool<Sqlite>, sticky_key: &str) -> Result<()> {
    sqlx::query("DELETE FROM pool_sticky_routes WHERE sticky_key = ?1")
        .bind(sticky_key)
        .execute(pool)
        .await?;
    Ok(())
}
