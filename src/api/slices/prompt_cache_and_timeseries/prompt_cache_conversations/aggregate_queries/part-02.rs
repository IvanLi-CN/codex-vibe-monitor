pub(crate) struct PromptCacheWorkingConversationPageQuery<'a> {
    pub(crate) range_start_bound: &'a str,
    pub(crate) source_scope: InvocationSourceScope,
    pub(crate) blocked_binding_filter: Option<&'a PromptCacheConversationBlockedBindingFilter>,
    pub(crate) cursor: Option<&'a (String, String, String, Option<i64>)>,
    pub(crate) limit: i64,
}

fn append_working_conversation_aggregate_source(
    query: &mut QueryBuilder<Sqlite>,
    source_scope: InvocationSourceScope,
) {
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
}

fn append_working_conversation_aggregate_filters<'a>(
    query: &mut QueryBuilder<'a, Sqlite>,
    range_start_bound: &'a str,
    source_scope: InvocationSourceScope,
    blocked_binding_filter: Option<&PromptCacheConversationBlockedBindingFilter>,
) {
    match source_scope {
        InvocationSourceScope::All => append_working_set_freshness_filter(
            query,
            range_start_bound,
            "last_activity_at",
            "last_terminal_at",
            "last_in_flight_at",
        ),
        InvocationSourceScope::ProxyOnly => append_working_set_freshness_filter(
            query,
            range_start_bound,
            "proxy_last_activity_at",
            "proxy_last_terminal_at",
            "proxy_last_in_flight_at",
        ),
    }
    append_working_set_blocked_binding_filter(
        query,
        source_scope,
        "prompt_cache_key",
        match source_scope {
            InvocationSourceScope::All => "last_terminal_at",
            InvocationSourceScope::ProxyOnly => "proxy_last_terminal_at",
        },
        blocked_binding_filter,
    );
}

fn append_working_conversation_cursor<'a>(
    query: &mut QueryBuilder<'a, Sqlite>,
    source_scope: InvocationSourceScope,
    cursor: Option<&'a (String, String, String, Option<i64>)>,
) {
    let Some((cursor_sort_anchor_at, cursor_created_at, cursor_prompt_cache_key, _)) = cursor
    else {
        return;
    };
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

pub(crate) async fn query_prompt_cache_working_conversation_aggregates_page<'e, E>(
    executor: E,
    query_options: PromptCacheWorkingConversationPageQuery<'_>,
) -> Result<Vec<PromptCacheConversationAggregateRow>>
where
    E: Executor<'e, Database = Sqlite>,
{
    let PromptCacheWorkingConversationPageQuery {
        range_start_bound,
        source_scope,
        blocked_binding_filter,
        cursor,
        limit,
    } = query_options;
    let mut query = QueryBuilder::<Sqlite>::new("SELECT prompt_cache_key, ");
    append_working_conversation_aggregate_source(&mut query, source_scope);
    append_working_conversation_aggregate_filters(
        &mut query,
        range_start_bound,
        source_scope,
        blocked_binding_filter,
    );
    append_working_conversation_cursor(&mut query, source_scope, cursor);

    query
        .push(" ORDER BY sort_anchor_at DESC, cursor_created_at DESC, prompt_cache_key DESC LIMIT ")
        .push_bind(limit);

    query
        .build_query_as::<PromptCacheConversationAggregateRow>()
        .fetch_all(executor)
        .await
        .map_err(Into::into)
}

pub(crate) async fn query_prompt_cache_working_conversation_aggregate_for_key<'e, E>(
    executor: E,
    range_start_bound: &str,
    source_scope: InvocationSourceScope,
    blocked_binding_filter: Option<&PromptCacheConversationBlockedBindingFilter>,
    prompt_cache_key: &str,
) -> Result<Option<PromptCacheConversationAggregateRow>>
where
    E: Executor<'e, Database = Sqlite>,
{
    let mut query = QueryBuilder::<Sqlite>::new("SELECT prompt_cache_key, ");
    append_working_conversation_aggregate_source(&mut query, source_scope);
    append_working_conversation_aggregate_filters(
        &mut query,
        range_start_bound,
        source_scope,
        blocked_binding_filter,
    );
    query
        .push(" AND prompt_cache_key = ")
        .push_bind(prompt_cache_key)
        .push(" LIMIT 1");

    query
        .build_query_as::<PromptCacheConversationAggregateRow>()
        .fetch_optional(executor)
        .await
        .map_err(Into::into)
}

pub(crate) struct PromptCacheWorkingConversationLifecycleQuery<'a> {
    pub(crate) range_start_bound: &'a str,
    pub(crate) source_scope: InvocationSourceScope,
    pub(crate) prompt_cache_key: &'a str,
    pub(crate) blocked_binding_filter: Option<&'a PromptCacheConversationBlockedBindingFilter>,
    pub(crate) snapshot_filter: &'a PromptCacheConversationSnapshotFilter,
    pub(crate) snapshot_hour_start_epoch: i64,
    pub(crate) snapshot_hour_start_bound: &'a str,
}

const PROMPT_CACHE_CONVERSATION_KEY_EXPR: &str = "CASE WHEN json_valid(payload) THEN TRIM(CAST(json_extract(payload, '$.promptCacheKey') AS TEXT)) END";

async fn load_prompt_cache_lifecycle_aggregate_for_key(
    connection: &mut SqliteConnection,
    source_scope: InvocationSourceScope,
    prompt_cache_key: &str,
    snapshot_filter: &PromptCacheConversationSnapshotFilter,
    snapshot_hour_start_epoch: i64,
    snapshot_hour_start_bound: &str,
) -> Result<Option<PromptCacheConversationAggregateRow>> {
    let selected_keys = vec![prompt_cache_key.to_string()];
    Ok(
        query_prompt_cache_conversation_lifecycle_aggregates_on_connection(
            connection,
            source_scope,
            &selected_keys,
            Some(snapshot_filter),
            Some(snapshot_hour_start_epoch),
            Some(snapshot_hour_start_bound),
        )
        .await?
        .remove(prompt_cache_key),
    )
}

async fn query_prompt_cache_working_conversation_lifecycle_tail(
    connection: &mut SqliteConnection,
    source_scope: InvocationSourceScope,
    prompt_cache_key: &str,
    snapshot_filter: &PromptCacheConversationSnapshotFilter,
) -> Result<(Option<String>, Option<String>)> {
    let mut query = QueryBuilder::<Sqlite>::new(
        "SELECT \
             MAX(CASE WHEN LOWER(TRIM(COALESCE(status, ''))) NOT IN ('running', 'pending') THEN occurred_at END) AS last_terminal_at, \
             MAX(CASE WHEN LOWER(TRIM(COALESCE(status, ''))) IN ('running', 'pending') THEN occurred_at END) AS last_in_flight_at \
         FROM codex_invocations \
         WHERE ",
    );
    push_snapshot_invocation_visibility_clause(
        &mut query,
        "occurred_at",
        "id",
        "created_at",
        Some(snapshot_filter),
    );
    query
        .push(" AND ")
        .push(PROMPT_CACHE_CONVERSATION_KEY_EXPR)
        .push(" = ")
        .push_bind(prompt_cache_key);
    if source_scope == InvocationSourceScope::ProxyOnly {
        query.push(" AND source = ").push_bind(SOURCE_PROXY);
    }
    Ok(query
        .build_query_as::<(Option<String>, Option<String>)>()
        .fetch_one(&mut *connection)
        .await?)
}

async fn prompt_cache_lifecycle_matches_blocked_filter(
    connection: &mut SqliteConnection,
    source_scope: InvocationSourceScope,
    prompt_cache_key: &str,
    last_terminal_at: &str,
    blocked_binding_filter: &PromptCacheConversationBlockedBindingFilter,
    snapshot_filter: &PromptCacheConversationSnapshotFilter,
) -> Result<bool> {
    let mut query = QueryBuilder::<Sqlite>::new(
        "SELECT EXISTS(SELECT 1 FROM codex_invocations AS blocked_invocation WHERE ",
    );
    push_snapshot_invocation_visibility_clause(
        &mut query,
        "blocked_invocation.occurred_at",
        "blocked_invocation.id",
        "blocked_invocation.created_at",
        Some(snapshot_filter),
    );
    query
        .push(" AND ")
        .push(PROMPT_CACHE_CONVERSATION_KEY_EXPR.replace("payload", "blocked_invocation.payload"))
        .push(" = ")
        .push_bind(prompt_cache_key)
        .push(" AND blocked_invocation.occurred_at = ")
        .push_bind(last_terminal_at)
        .push(" AND ")
        .push(BLOCKED_BINDING_JSON_EXISTS_SQL.replace("payload", "blocked_invocation.payload"));
    if source_scope == InvocationSourceScope::ProxyOnly {
        query
            .push(" AND blocked_invocation.source = ")
            .push_bind(SOURCE_PROXY);
    }
    if let Some(upstream_account_id) = blocked_binding_filter.upstream_account_id {
        query
            .push(" AND CAST(json_extract(blocked_invocation.payload, '$.blockedBinding.upstreamAccountId') AS INTEGER) = ")
            .push_bind(upstream_account_id);
    }
    if let Some(constraint_source) = blocked_binding_filter.constraint_source {
        let raw = match constraint_source {
            BlockedBindingConstraintSource::UpstreamAccountBinding => "upstreamAccountBinding",
            BlockedBindingConstraintSource::EncryptedSessionOwner => "encryptedSessionOwner",
        };
        query
            .push(" AND CAST(json_extract(blocked_invocation.payload, '$.blockedBinding.constraintSource') AS TEXT) = ")
            .push_bind(raw);
    }
    query.push(")");
    let (matches_filter,) = query
        .build_query_as::<(i64,)>()
        .fetch_one(&mut *connection)
        .await?;
    Ok(matches_filter != 0)
}

pub(crate) async fn query_prompt_cache_working_conversation_lifecycle_aggregate_for_key(
    connection: &mut SqliteConnection,
    query_options: PromptCacheWorkingConversationLifecycleQuery<'_>,
) -> Result<Option<PromptCacheConversationAggregateRow>> {
    let PromptCacheWorkingConversationLifecycleQuery {
        range_start_bound,
        source_scope,
        prompt_cache_key,
        blocked_binding_filter,
        snapshot_filter,
        snapshot_hour_start_epoch,
        snapshot_hour_start_bound,
    } = query_options;
    // P2 can lag a terminal P1 write, so combine durable rollups with the exact tail.
    let Some(mut aggregate) = load_prompt_cache_lifecycle_aggregate_for_key(
        connection,
        source_scope,
        prompt_cache_key,
        snapshot_filter,
        snapshot_hour_start_epoch,
        snapshot_hour_start_bound,
    )
    .await?
    else {
        return Ok(None);
    };
    let (last_terminal_at, last_in_flight_at) =
        query_prompt_cache_working_conversation_lifecycle_tail(
            connection,
            source_scope,
            prompt_cache_key,
            snapshot_filter,
        )
        .await?;
    let is_fresh = last_in_flight_at.is_some()
        || last_terminal_at
            .as_deref()
            .is_some_and(|value| value >= range_start_bound)
        || (last_in_flight_at.is_none()
            && last_terminal_at.is_none()
            && aggregate.last_activity_at.as_str() >= range_start_bound);
    if !is_fresh {
        return Ok(None);
    }
    aggregate.cursor_created_at = Some(aggregate.created_at.clone());
    aggregate.sort_anchor_at = Some(
        resolve_working_conversation_sort_anchor(
            last_terminal_at.as_deref(),
            last_in_flight_at.as_deref(),
            aggregate.created_at.as_str(),
        )
        .to_string(),
    );
    aggregate.last_terminal_at = last_terminal_at;
    aggregate.last_in_flight_at = last_in_flight_at;
    if let Some(blocked_binding_filter) = blocked_binding_filter
        && blocked_binding_filter.is_active()
    {
        let Some(last_terminal_at) = aggregate.last_terminal_at.as_deref() else {
            return Ok(None);
        };
        if !prompt_cache_lifecycle_matches_blocked_filter(
            connection,
            source_scope,
            prompt_cache_key,
            last_terminal_at,
            blocked_binding_filter,
            snapshot_filter,
        )
        .await?
        {
            return Ok(None);
        }
    }
    Ok(Some(aggregate))
}

pub(crate) fn merge_prompt_cache_lifecycle_aggregate_row(
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

pub(crate) async fn query_prompt_cache_lifecycle_rollup_aggregates_tx(
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

pub(crate) async fn query_prompt_cache_lifecycle_snapshot_rollup_aggregates_tx(
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
    const DELTA_COUNT_EXPR: &str = "CASE WHEN r.request_count > COALESCE(e.request_count, 0) THEN r.request_count - COALESCE(e.request_count, 0) ELSE 0 END";
    const DELTA_TOKENS_EXPR: &str = "CASE WHEN r.request_count > COALESCE(e.request_count, 0) THEN MAX(r.total_tokens - COALESCE(e.total_tokens, 0), 0) ELSE 0 END";
    const DELTA_COST_EXPR: &str = "CASE WHEN r.request_count > COALESCE(e.request_count, 0) THEN MAX(r.total_cost - COALESCE(e.total_cost, 0.0), 0.0) ELSE 0.0 END";

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
    query.push(
        " GROUP BY source, prompt_cache_key, bucket_start_epoch) SELECT r.prompt_cache_key, SUM(",
    );
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

pub(crate) async fn query_prompt_cache_lifecycle_exact_tail_aggregates_tx(
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
    query.push(KEY_EXPR).push(
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
    let aggregates = query_prompt_cache_conversation_lifecycle_aggregates_on_connection(
        tx.as_mut(),
        source_scope,
        selected_keys,
        snapshot_filter,
        snapshot_hour_start_epoch,
        snapshot_hour_start_bound,
    )
    .await?;
    tx.commit().await?;
    Ok(aggregates)
}

pub(crate) async fn query_prompt_cache_conversation_lifecycle_aggregates_on_connection(
    connection: &mut SqliteConnection,
    source_scope: InvocationSourceScope,
    selected_keys: &[String],
    snapshot_filter: Option<&PromptCacheConversationSnapshotFilter>,
    snapshot_hour_start_epoch: Option<i64>,
    snapshot_hour_start_bound: Option<&str>,
) -> Result<HashMap<String, PromptCacheConversationAggregateRow>> {
    if selected_keys.is_empty() {
        return Ok(HashMap::new());
    }

    let rollup_live_cursor = load_invocation_summary_rollup_live_cursor_tx(connection).await?;
    let mut aggregates = HashMap::<String, PromptCacheConversationAggregateRow>::new();
    if snapshot_filter.is_none() {
        for row in query_prompt_cache_lifecycle_rollup_aggregates_tx(
            connection,
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
            connection,
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
        connection,
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
    Ok(aggregates)
}
