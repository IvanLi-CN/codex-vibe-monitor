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
             COALESCE(proxy_created_at, created_at) AS cursor_created_at, \
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
             created_at AS cursor_created_at, \
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
            " ORDER BY sort_anchor_at DESC, cursor_created_at DESC, prompt_cache_key DESC \
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
             COALESCE(proxy_created_at, created_at) AS cursor_created_at, \
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
             created_at AS cursor_created_at, \
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
        let created_at_cursor_expr = match source_scope {
            InvocationSourceScope::All => "created_at",
            InvocationSourceScope::ProxyOnly => "COALESCE(proxy_created_at, created_at)",
        };
        query
            .push(" AND (sort_anchor_at < ")
            .push_bind(cursor_sort_anchor_at)
            .push(" OR (sort_anchor_at = ")
            .push_bind(cursor_sort_anchor_at)
            .push(" AND (")
            .push(created_at_cursor_expr)
            .push(" < ")
            .push_bind(cursor_created_at)
            .push(" OR (")
            .push(created_at_cursor_expr)
            .push(" = ")
            .push_bind(cursor_created_at)
            .push(" AND prompt_cache_key < ")
            .push_bind(cursor_prompt_cache_key)
            .push("))))");
    }

    query
        .push(" ORDER BY sort_anchor_at DESC, cursor_created_at DESC, prompt_cache_key DESC LIMIT ")
        .push_bind(limit);

    query
        .build_query_as::<PromptCacheConversationAggregateRow>()
        .fetch_all(pool)
        .await
        .map_err(Into::into)
}

fn merge_prompt_cache_lifecycle_aggregate_row(
    aggregates: &mut HashMap<String, PromptCacheConversationAggregateRow>,
    row: PromptCacheConversationAggregateRow,
) {
    match aggregates.entry(row.prompt_cache_key.clone()) {
        std::collections::hash_map::Entry::Occupied(mut entry) => {
            let aggregate = entry.get_mut();
            aggregate.request_count += row.request_count;
            aggregate.total_tokens += row.total_tokens;
            aggregate.total_cost += row.total_cost;
            if row.created_at < aggregate.created_at {
                aggregate.created_at = row.created_at;
            }
            if row.last_activity_at > aggregate.last_activity_at {
                aggregate.last_activity_at = row.last_activity_at;
            }
        }
        std::collections::hash_map::Entry::Vacant(entry) => {
            entry.insert(row);
        }
    }
}

async fn query_prompt_cache_lifecycle_rollup_aggregates_tx(
    tx: &mut SqliteConnection,
    source_scope: InvocationSourceScope,
    selected_keys: &[String],
    snapshot_hour_start_epoch: Option<i64>,
) -> Result<Vec<PromptCacheConversationAggregateRow>> {
    if selected_keys.is_empty() {
        return Ok(Vec::new());
    }

    let mut query = QueryBuilder::<Sqlite>::new(
        "SELECT prompt_cache_key, \
             SUM(request_count) AS request_count, \
             SUM(total_tokens) AS total_tokens, \
             SUM(total_cost) AS total_cost, \
             MIN(first_seen_at) AS created_at, \
             MAX(last_seen_at) AS last_activity_at \
         FROM prompt_cache_rollup_hourly \
         WHERE prompt_cache_key IN (",
    );
    {
        let mut separated = query.separated(", ");
        for key in selected_keys {
            separated.push_bind(key);
        }
    }
    query.push(")");
    if let Some(snapshot_hour_start_epoch) = snapshot_hour_start_epoch {
        query
            .push(" AND bucket_start_epoch < ")
            .push_bind(snapshot_hour_start_epoch);
    }
    if source_scope == InvocationSourceScope::ProxyOnly {
        query.push(" AND source = ").push_bind(SOURCE_PROXY);
    }
    query.push(" GROUP BY prompt_cache_key");

    query
        .build_query_as::<PromptCacheConversationAggregateRow>()
        .fetch_all(&mut *tx)
        .await
        .map_err(Into::into)
}

async fn query_prompt_cache_lifecycle_snapshot_rollup_aggregates_tx(
    tx: &mut SqliteConnection,
    source_scope: InvocationSourceScope,
    selected_keys: &[String],
    rollup_live_cursor: i64,
    snapshot_hour_start_epoch: i64,
    snapshot_hour_start_bound: &str,
) -> Result<Vec<PromptCacheConversationAggregateRow>> {
    if selected_keys.is_empty() {
        return Ok(Vec::new());
    }

    const KEY_EXPR: &str = "CASE WHEN json_valid(payload) THEN TRIM(CAST(json_extract(payload, '$.promptCacheKey') AS TEXT)) END";
    const EXACT_BUCKET_EXPR: &str = "((CASE WHEN instr(occurred_at, 'T') > 0 THEN CAST(strftime('%s', occurred_at) AS INTEGER) ELSE CAST(strftime('%s', occurred_at || '+08:00') AS INTEGER) END) / 3600) * 3600";
    const DELTA_COUNT_EXPR: &str =
        "CASE WHEN r.request_count > COALESCE(e.request_count, 0) THEN r.request_count - COALESCE(e.request_count, 0) ELSE 0 END";
    const DELTA_TOKENS_EXPR: &str =
        "CASE WHEN r.request_count > COALESCE(e.request_count, 0) THEN MAX(r.total_tokens - COALESCE(e.total_tokens, 0), 0) ELSE 0 END";
    const DELTA_COST_EXPR: &str =
        "CASE WHEN r.request_count > COALESCE(e.request_count, 0) THEN MAX(r.total_cost - COALESCE(e.total_cost, 0.0), 0.0) ELSE 0.0 END";

    let mut query = QueryBuilder::<Sqlite>::new("WITH exact_live AS (SELECT source, ");
    query
        .push(KEY_EXPR)
        .push(" AS prompt_cache_key, ")
        .push(EXACT_BUCKET_EXPR)
        .push(
            " AS bucket_start_epoch, \
             COUNT(*) AS request_count, \
             COALESCE(SUM(COALESCE(total_tokens, 0)), 0) AS total_tokens, \
             COALESCE(SUM(COALESCE(cost, 0.0)), 0.0) AS total_cost \
         FROM codex_invocations \
         WHERE id <= ",
        )
        .push_bind(rollup_live_cursor)
        .push(" AND occurred_at < ")
        .push_bind(snapshot_hour_start_bound)
        .push(" AND ")
        .push(KEY_EXPR)
        .push(" IN (");
    {
        let mut separated = query.separated(", ");
        for key in selected_keys {
            separated.push_bind(key);
        }
    }
    query.push(")");
    if source_scope == InvocationSourceScope::ProxyOnly {
        query.push(" AND source = ").push_bind(SOURCE_PROXY);
    }
    query.push(" GROUP BY source, prompt_cache_key, bucket_start_epoch) SELECT r.prompt_cache_key, SUM(");
    query
        .push(DELTA_COUNT_EXPR)
        .push(") AS request_count, SUM(")
        .push(DELTA_TOKENS_EXPR)
        .push(") AS total_tokens, SUM(")
        .push(DELTA_COST_EXPR)
        .push(") AS total_cost, MIN(CASE WHEN ")
        .push(DELTA_COUNT_EXPR)
        .push(" > 0 THEN r.first_seen_at END) AS created_at, MAX(CASE WHEN ")
        .push(DELTA_COUNT_EXPR)
        .push(
            " > 0 THEN r.last_seen_at END) AS last_activity_at \
         FROM prompt_cache_rollup_hourly r \
         LEFT JOIN exact_live e \
           ON e.source = r.source \
          AND e.prompt_cache_key = r.prompt_cache_key \
          AND e.bucket_start_epoch = r.bucket_start_epoch \
         WHERE r.bucket_start_epoch < ",
        )
        .push_bind(snapshot_hour_start_epoch)
        .push(" AND r.prompt_cache_key IN (");
    {
        let mut separated = query.separated(", ");
        for key in selected_keys {
            separated.push_bind(key);
        }
    }
    query.push(")");
    if source_scope == InvocationSourceScope::ProxyOnly {
        query.push(" AND r.source = ").push_bind(SOURCE_PROXY);
    }
    query
        .push(" GROUP BY r.prompt_cache_key HAVING SUM(")
        .push(DELTA_COUNT_EXPR)
        .push(") > 0");

    query
        .build_query_as::<PromptCacheConversationAggregateRow>()
        .fetch_all(&mut *tx)
        .await
        .map_err(Into::into)
}

async fn query_prompt_cache_lifecycle_exact_tail_aggregates_tx(
    tx: &mut SqliteConnection,
    source_scope: InvocationSourceScope,
    selected_keys: &[String],
    rollup_live_cursor: i64,
    snapshot_filter: Option<&PromptCacheConversationSnapshotFilter>,
    snapshot_hour_start_bound: Option<&str>,
) -> Result<Vec<PromptCacheConversationAggregateRow>> {
    if selected_keys.is_empty() {
        return Ok(Vec::new());
    }

    const KEY_EXPR: &str = "CASE WHEN json_valid(payload) THEN TRIM(CAST(json_extract(payload, '$.promptCacheKey') AS TEXT)) END";
    let mut query = QueryBuilder::<Sqlite>::new("SELECT ");
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
         WHERE ",
        );
    if let Some(snapshot_filter) = snapshot_filter {
        query.push("id <= ").push_bind(
            snapshot_filter
                .snapshot_boundary_row_id_ceiling
                .unwrap_or_default(),
        );
        if let Some(created_at_upper_bound) = snapshot_filter.snapshot_created_at_upper_bound() {
            query
                .push(" AND julianday(created_at) <= julianday(")
                .push_bind(created_at_upper_bound)
                .push(")");
        }
        if let Some(snapshot_hour_start_bound) = snapshot_hour_start_bound {
            query
                .push(" AND (occurred_at >= ")
                .push_bind(snapshot_hour_start_bound)
                .push(" OR id > ")
                .push_bind(rollup_live_cursor)
                .push(")");
        }
        query
            .push(" AND occurred_at < ")
            .push_bind(snapshot_filter.snapshot_upper_bound());
    } else {
        query.push("id > ").push_bind(rollup_live_cursor);
    }
    query.push(" AND ").push(KEY_EXPR).push(" IN (");
    {
        let mut separated = query.separated(", ");
        for key in selected_keys {
            separated.push_bind(key);
        }
    }
    query.push(")");
    if source_scope == InvocationSourceScope::ProxyOnly {
        query.push(" AND source = ").push_bind(SOURCE_PROXY);
    }
    query.push(" GROUP BY prompt_cache_key");

    query
        .build_query_as::<PromptCacheConversationAggregateRow>()
        .fetch_all(&mut *tx)
        .await
        .map_err(Into::into)
}

pub(crate) async fn query_prompt_cache_conversation_lifecycle_aggregates(
    pool: &Pool<Sqlite>,
    source_scope: InvocationSourceScope,
    selected_keys: &[String],
    snapshot_filter: Option<&PromptCacheConversationSnapshotFilter>,
    snapshot_hour_start_epoch: Option<i64>,
    snapshot_hour_start_bound: Option<&str>,
) -> Result<HashMap<String, PromptCacheConversationAggregateRow>> {
    if selected_keys.is_empty() {
        return Ok(HashMap::new());
    }

    let mut tx = pool.begin().await?;
    let rollup_live_cursor = load_invocation_summary_rollup_live_cursor_tx(tx.as_mut()).await?;
    let mut aggregates = HashMap::<String, PromptCacheConversationAggregateRow>::new();
    if snapshot_filter.is_none() {
        for row in query_prompt_cache_lifecycle_rollup_aggregates_tx(
            tx.as_mut(),
            source_scope,
            selected_keys,
            snapshot_hour_start_epoch,
        )
        .await?
        {
            merge_prompt_cache_lifecycle_aggregate_row(&mut aggregates, row);
        }
    } else if let (Some(snapshot_hour_start_epoch), Some(snapshot_hour_start_bound)) =
        (snapshot_hour_start_epoch, snapshot_hour_start_bound)
    {
        for row in query_prompt_cache_lifecycle_snapshot_rollup_aggregates_tx(
            tx.as_mut(),
            source_scope,
            selected_keys,
            rollup_live_cursor,
            snapshot_hour_start_epoch,
            snapshot_hour_start_bound,
        )
        .await?
        {
            merge_prompt_cache_lifecycle_aggregate_row(&mut aggregates, row);
        }
    }
    for row in query_prompt_cache_lifecycle_exact_tail_aggregates_tx(
        tx.as_mut(),
        source_scope,
        selected_keys,
        rollup_live_cursor,
        snapshot_filter,
        // Snapshot rollup rows are already reduced by exact rows at or before the
        // snapshot boundary, so the exact tail must scan the full snapshot range.
        if snapshot_filter.is_some() {
            None
        } else {
            snapshot_hour_start_bound
        },
    )
    .await?
    {
        merge_prompt_cache_lifecycle_aggregate_row(&mut aggregates, row);
    }
    tx.commit().await?;
    Ok(aggregates)
}
