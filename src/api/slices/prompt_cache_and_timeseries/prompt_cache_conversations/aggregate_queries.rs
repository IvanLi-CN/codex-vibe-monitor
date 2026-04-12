use super::*;

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

pub(crate) async fn query_working_prompt_cache_conversation_count_at_snapshot(
    pool: &Pool<Sqlite>,
    range_start_bound: &str,
    snapshot: &PromptCacheConversationSnapshotFilter,
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
        .push(" AND ");
    push_snapshot_invocation_visibility_clause(&mut query, "occurred_at", "id", Some(snapshot));
    query
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
    query.push(KEY_EXPR).push(
        " AS prompt_cache_key \
             FROM codex_invocations \
             WHERE ",
    );
    push_snapshot_invocation_visibility_clause(&mut query, "occurred_at", "id", Some(snapshot));
    query
        .push(" AND ")
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
                   CASE \
                       WHEN MAX(last_terminal_at) IS NULL THEN MAX(last_in_flight_at) \
                       WHEN MAX(last_in_flight_at) IS NULL THEN MAX(last_terminal_at) \
                       WHEN MAX(last_terminal_at) >= MAX(last_in_flight_at) THEN MAX(last_terminal_at) \
                       ELSE MAX(last_in_flight_at) \
                   END AS sort_anchor_at \
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

pub(crate) async fn query_prompt_cache_working_conversation_aggregates_page(
    pool: &Pool<Sqlite>,
    range_start_bound: &str,
    snapshot: &PromptCacheConversationSnapshotFilter,
    snapshot_hour_start_epoch: i64,
    snapshot_hour_start_bound: &str,
    source_scope: InvocationSourceScope,
    cursor: Option<&(String, String, String, Option<i64>)>,
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
        .push(" AND ");
    push_snapshot_invocation_visibility_clause(&mut query, "occurred_at", "id", Some(snapshot));
    query
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
    query.push(KEY_EXPR).push(
        " AS prompt_cache_key, MAX(occurred_at) AS last_in_flight_at \
             FROM codex_invocations \
             WHERE ",
    );
    push_snapshot_invocation_visibility_clause(&mut query, "occurred_at", "id", Some(snapshot));
    query
        .push(" AND ")
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
                   CASE \
                       WHEN MAX(last_terminal_at) IS NULL THEN MAX(last_in_flight_at) \
                       WHEN MAX(last_in_flight_at) IS NULL THEN MAX(last_terminal_at) \
                       WHEN MAX(last_terminal_at) >= MAX(last_in_flight_at) THEN MAX(last_terminal_at) \
                       ELSE MAX(last_in_flight_at) \
                   END AS sort_anchor_at \
              FROM working \
              GROUP BY prompt_cache_key\
         ), history_rollup AS (\
            SELECT prompt_cache_key, \
                   SUM(request_count) AS request_count, \
                   SUM(total_tokens) AS total_tokens, \
                   SUM(total_cost) AS total_cost, \
                   MIN(first_seen_at) AS created_at, \
                   MAX(last_seen_at) AS last_activity_at \
              FROM prompt_cache_rollup_hourly \
             WHERE prompt_cache_key IN (SELECT prompt_cache_key FROM collapsed_working) \
               AND bucket_start_epoch < ",
    );
    query.push_bind(snapshot_hour_start_epoch);

    if source_scope == InvocationSourceScope::ProxyOnly {
        query.push(" AND source = ").push_bind(SOURCE_PROXY);
    }

    query.push(
        " GROUP BY prompt_cache_key\
         ), history_live AS (\
            SELECT ",
    );
    query
        .push(KEY_EXPR)
        .push(
            " AS prompt_cache_key, \
                   COUNT(*) AS request_count, \
                   COALESCE(SUM(COALESCE(total_tokens, 0)), 0) AS total_tokens, \
                   COALESCE(SUM(COALESCE(cost, 0.0)), 0.0) AS total_cost, \
                   MIN(occurred_at) AS created_at, \
                   MAX(occurred_at) AS last_activity_at \
              FROM codex_invocations \
             WHERE occurred_at >= ",
        )
        .push_bind(snapshot_hour_start_bound)
        .push(" AND ");
    push_snapshot_invocation_visibility_clause(&mut query, "occurred_at", "id", Some(snapshot));
    query
        .push(" AND ")
        .push(KEY_EXPR)
        .push(" IN (SELECT prompt_cache_key FROM collapsed_working)");

    if source_scope == InvocationSourceScope::ProxyOnly {
        query.push(" AND source = ").push_bind(SOURCE_PROXY);
    }

    query.push(
        " GROUP BY prompt_cache_key\
         ), history_inputs AS (\
            SELECT prompt_cache_key, request_count, total_tokens, total_cost, created_at, last_activity_at \
              FROM history_rollup \
            UNION ALL \
            SELECT prompt_cache_key, request_count, total_tokens, total_cost, created_at, last_activity_at \
              FROM history_live\
         ), history_aggregates AS (\
            SELECT prompt_cache_key, \
                   COALESCE(SUM(request_count), 0) AS request_count, \
                   COALESCE(SUM(total_tokens), 0) AS total_tokens, \
                   COALESCE(SUM(total_cost), 0.0) AS total_cost, \
                   MIN(created_at) AS created_at, \
                   MAX(last_activity_at) AS last_activity_at \
              FROM history_inputs \
             GROUP BY prompt_cache_key\
         ), aggregates AS (\
            SELECT collapsed_working.prompt_cache_key AS prompt_cache_key, \
                   COALESCE(history_aggregates.request_count, 0) AS request_count, \
                   COALESCE(history_aggregates.total_tokens, 0) AS total_tokens, \
                   COALESCE(history_aggregates.total_cost, 0.0) AS total_cost, \
                   COALESCE(history_aggregates.created_at, collapsed_working.sort_anchor_at) AS created_at, \
                   COALESCE(history_aggregates.last_activity_at, collapsed_working.sort_anchor_at) AS last_activity_at, \
                   collapsed_working.sort_anchor_at AS sort_anchor_at, \
                   collapsed_working.last_terminal_at AS last_terminal_at, \
                   collapsed_working.last_in_flight_at AS last_in_flight_at \
              FROM collapsed_working \
              LEFT JOIN history_aggregates ON history_aggregates.prompt_cache_key = collapsed_working.prompt_cache_key",
    );

    query.push(
        " ) \
         SELECT aggregates.prompt_cache_key, aggregates.request_count, aggregates.total_tokens, \
                aggregates.total_cost, aggregates.created_at, aggregates.last_activity_at, \
                aggregates.sort_anchor_at, aggregates.last_terminal_at, \
                aggregates.last_in_flight_at \
           FROM aggregates",
    );

    if let Some((cursor_sort_anchor_at, cursor_created_at, cursor_prompt_cache_key, _)) = cursor {
        query
            .push(" WHERE (aggregates.sort_anchor_at < ")
            .push_bind(cursor_sort_anchor_at)
            .push(" OR (aggregates.sort_anchor_at = ")
            .push_bind(cursor_sort_anchor_at)
            .push(" AND (aggregates.created_at < ")
            .push_bind(cursor_created_at)
            .push(" OR (aggregates.created_at = ")
            .push_bind(cursor_created_at)
            .push(" AND aggregates.prompt_cache_key < ")
            .push_bind(cursor_prompt_cache_key)
            .push("))))");
    }

    query
        .push(
            " ORDER BY aggregates.sort_anchor_at DESC, aggregates.created_at DESC, aggregates.prompt_cache_key DESC \
              LIMIT ",
        )
        .push_bind(limit);

    query
        .build_query_as::<PromptCacheConversationAggregateRow>()
        .fetch_all(pool)
        .await
        .map_err(Into::into)
}

