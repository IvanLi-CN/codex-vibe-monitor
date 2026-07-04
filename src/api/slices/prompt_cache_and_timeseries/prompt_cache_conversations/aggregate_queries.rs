use super::*;

fn append_working_set_freshness_filter<'a>(
    query: &mut QueryBuilder<'a, Sqlite>,
    range_start_bound: &'a str,
    last_activity_column: &str,
    last_terminal_column: &str,
    last_in_flight_column: &str,
) {
    query
        .push(" AND (")
        .push(last_in_flight_column)
        .push(" IS NOT NULL OR ")
        .push(last_terminal_column)
        .push(" >= ")
        .push_bind(range_start_bound)
        .push(" OR (")
        .push(last_in_flight_column)
        .push(" IS NULL AND ")
        .push(last_terminal_column)
        .push(" IS NULL AND ")
        .push(last_activity_column)
        .push(" >= ")
        .push_bind(range_start_bound)
        .push("))");
}

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
    let mut query = QueryBuilder::<Sqlite>::new(match source_scope {
        InvocationSourceScope::All => {
            "SELECT COUNT(*) AS count \
             FROM prompt_cache_working_set_live \
             WHERE source_scope_all = 1"
        }
        InvocationSourceScope::ProxyOnly => {
            "SELECT COUNT(*) AS count \
             FROM prompt_cache_working_set_live \
             WHERE source_scope_proxy_only = 1"
        }
    });
    match source_scope {
        InvocationSourceScope::All => append_working_set_freshness_filter(
            &mut query,
            range_start_bound,
            "last_activity_at",
            "last_terminal_at",
            "last_in_flight_at",
        ),
        InvocationSourceScope::ProxyOnly => append_working_set_freshness_filter(
            &mut query,
            range_start_bound,
            "proxy_last_activity_at",
            "proxy_last_terminal_at",
            "proxy_last_in_flight_at",
        ),
    }

    let (count,) = query.build_query_as::<(i64,)>().fetch_one(pool).await?;
    Ok(count)
}

pub(crate) async fn query_in_progress_prompt_cache_conversation_count(
    pool: &Pool<Sqlite>,
    source_scope: InvocationSourceScope,
    upstream_account_id: Option<i64>,
) -> Result<i64> {
    let mut query = QueryBuilder::<Sqlite>::new(
        "SELECT COUNT(DISTINCT prompt_cache_key) AS count \
         FROM invocation_in_progress_live \
         WHERE prompt_cache_key IS NOT NULL \
           AND prompt_cache_key <> ''",
    );
    if source_scope == InvocationSourceScope::ProxyOnly {
        query.push(" AND source = ").push_bind(SOURCE_PROXY);
    }
    if let Some(upstream_account_id) = upstream_account_id {
        query
            .push(" AND upstream_account_id = ")
            .push_bind(upstream_account_id);
    }

    let (count,) = query.build_query_as::<(i64,)>().fetch_one(pool).await?;
    Ok(count)
}

pub(crate) async fn query_working_prompt_cache_conversation_count_at_snapshot(
    pool: &Pool<Sqlite>,
    range_start_bound: &str,
    _snapshot: &PromptCacheConversationSnapshotFilter,
    source_scope: InvocationSourceScope,
) -> Result<i64> {
    let mut query = QueryBuilder::<Sqlite>::new(match source_scope {
        InvocationSourceScope::All => {
            "SELECT COUNT(*) AS count \
             FROM prompt_cache_working_set_live \
             WHERE source_scope_all = 1"
        }
        InvocationSourceScope::ProxyOnly => {
            "SELECT COUNT(*) AS count \
             FROM prompt_cache_working_set_live \
             WHERE source_scope_proxy_only = 1"
        }
    });
    match source_scope {
        InvocationSourceScope::All => append_working_set_freshness_filter(
            &mut query,
            range_start_bound,
            "last_activity_at",
            "last_terminal_at",
            "last_in_flight_at",
        ),
        InvocationSourceScope::ProxyOnly => append_working_set_freshness_filter(
            &mut query,
            range_start_bound,
            "proxy_last_activity_at",
            "proxy_last_terminal_at",
            "proxy_last_in_flight_at",
        ),
    }

    let (count,) = query.build_query_as::<(i64,)>().fetch_one(pool).await?;
    Ok(count)
}

pub(crate) async fn query_existing_working_prompt_cache_conversation_keys(
    pool: &Pool<Sqlite>,
    range_start_bound: &str,
    source_scope: InvocationSourceScope,
    prompt_cache_keys: &HashSet<String>,
) -> Result<HashSet<String>> {
    if prompt_cache_keys.is_empty() {
        return Ok(HashSet::new());
    }

    #[derive(Debug, FromRow)]
    struct PromptCacheKeyRow {
        prompt_cache_key: String,
    }

    let mut query = QueryBuilder::<Sqlite>::new(match source_scope {
        InvocationSourceScope::All => {
            "SELECT prompt_cache_key \
             FROM prompt_cache_working_set_live \
             WHERE source_scope_all = 1"
        }
        InvocationSourceScope::ProxyOnly => {
            "SELECT prompt_cache_key \
             FROM prompt_cache_working_set_live \
             WHERE source_scope_proxy_only = 1"
        }
    });
    match source_scope {
        InvocationSourceScope::All => append_working_set_freshness_filter(
            &mut query,
            range_start_bound,
            "last_activity_at",
            "last_terminal_at",
            "last_in_flight_at",
        ),
        InvocationSourceScope::ProxyOnly => append_working_set_freshness_filter(
            &mut query,
            range_start_bound,
            "proxy_last_activity_at",
            "proxy_last_terminal_at",
            "proxy_last_in_flight_at",
        ),
    }
    query.push(" AND prompt_cache_key IN (");
    {
        let mut separated = query.separated(", ");
        for key in prompt_cache_keys {
            separated.push_bind(key);
        }
    }
    query.push(")");

    Ok(query
        .build_query_as::<PromptCacheKeyRow>()
        .fetch_all(pool)
        .await?
        .into_iter()
        .map(|row| row.prompt_cache_key)
        .collect())
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
    let mut query = QueryBuilder::<Sqlite>::new(
        "SELECT \
            prompt_cache_key, \
            ",
    );
    if source_scope == InvocationSourceScope::ProxyOnly {
        query.push(
            "proxy_request_count AS request_count, \
             proxy_total_tokens AS total_tokens, \
             proxy_total_cost AS total_cost, \
             COALESCE(proxy_created_at, created_at) AS created_at, \
             COALESCE(proxy_last_activity_at, last_activity_at) AS last_activity_at, \
             proxy_sort_anchor_at AS sort_anchor_at, \
             proxy_last_terminal_at AS last_terminal_at, \
             proxy_last_in_flight_at AS last_in_flight_at \
         FROM prompt_cache_working_set_live \
         WHERE source_scope_proxy_only = 1 \
           AND proxy_sort_anchor_at IS NOT NULL",
        );
    } else {
        query.push(
            "request_count, \
             total_tokens, \
             total_cost, \
             created_at, \
             last_activity_at, \
             sort_anchor_at, \
             last_terminal_at, \
             last_in_flight_at \
         FROM prompt_cache_working_set_live \
         WHERE source_scope_all = 1",
        );
    }
    match source_scope {
        InvocationSourceScope::All => append_working_set_freshness_filter(
            &mut query,
            range_start_bound,
            "last_activity_at",
            "last_terminal_at",
            "last_in_flight_at",
        ),
        InvocationSourceScope::ProxyOnly => append_working_set_freshness_filter(
            &mut query,
            range_start_bound,
            "proxy_last_activity_at",
            "proxy_last_terminal_at",
            "proxy_last_in_flight_at",
        ),
    }
    query
        .push(
            " ORDER BY sort_anchor_at DESC, created_at DESC, prompt_cache_key DESC \
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
    _snapshot: &PromptCacheConversationSnapshotFilter,
    _snapshot_hour_start_epoch: i64,
    _snapshot_hour_start_bound: &str,
    source_scope: InvocationSourceScope,
    cursor: Option<&(String, String, String, Option<i64>)>,
    limit: i64,
) -> Result<Vec<PromptCacheConversationAggregateRow>> {
    let mut query = QueryBuilder::<Sqlite>::new("SELECT prompt_cache_key, ");
    if source_scope == InvocationSourceScope::ProxyOnly {
        query.push(
            "proxy_request_count AS request_count, \
             proxy_total_tokens AS total_tokens, \
             proxy_total_cost AS total_cost, \
             COALESCE(proxy_created_at, created_at) AS created_at, \
             COALESCE(proxy_last_activity_at, last_activity_at) AS last_activity_at, \
             proxy_sort_anchor_at AS sort_anchor_at, \
             proxy_last_terminal_at AS last_terminal_at, \
             proxy_last_in_flight_at AS last_in_flight_at \
         FROM prompt_cache_working_set_live \
         WHERE source_scope_proxy_only = 1 \
           AND proxy_sort_anchor_at IS NOT NULL",
        );
    } else {
        query.push(
            "request_count, \
             total_tokens, \
             total_cost, \
             created_at, \
             last_activity_at, \
             sort_anchor_at, \
             last_terminal_at, \
             last_in_flight_at \
         FROM prompt_cache_working_set_live \
         WHERE source_scope_all = 1",
        );
    }
    match source_scope {
        InvocationSourceScope::All => append_working_set_freshness_filter(
            &mut query,
            range_start_bound,
            "last_activity_at",
            "last_terminal_at",
            "last_in_flight_at",
        ),
        InvocationSourceScope::ProxyOnly => append_working_set_freshness_filter(
            &mut query,
            range_start_bound,
            "proxy_last_activity_at",
            "proxy_last_terminal_at",
            "proxy_last_in_flight_at",
        ),
    }

    if let Some((cursor_sort_anchor_at, cursor_created_at, cursor_prompt_cache_key, _)) = cursor {
        query
            .push(" AND (sort_anchor_at < ")
            .push_bind(cursor_sort_anchor_at)
            .push(" OR (sort_anchor_at = ")
            .push_bind(cursor_sort_anchor_at)
            .push(" AND (created_at < ")
            .push_bind(cursor_created_at)
            .push(" OR (created_at = ")
            .push_bind(cursor_created_at)
            .push(" AND prompt_cache_key < ")
            .push_bind(cursor_prompt_cache_key)
            .push("))))");
    }

    query
        .push(
            " ORDER BY sort_anchor_at DESC, created_at DESC, prompt_cache_key DESC \
              LIMIT ",
        )
        .push_bind(limit);

    query
        .build_query_as::<PromptCacheConversationAggregateRow>()
        .fetch_all(pool)
        .await
        .map_err(Into::into)
}
