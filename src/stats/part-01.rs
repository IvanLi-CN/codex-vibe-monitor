pub(crate) async fn query_stats_row(
    pool: &Pool<Sqlite>,
    filter: StatsFilter,
    source_scope: InvocationSourceScope,
) -> Result<StatsRow> {
    let mut query = QueryBuilder::<Sqlite>::new("SELECT ");
    query
        .push(stats_success_failure_select_sql())
        .push(" FROM ");
    let recent_limit = match &filter {
        StatsFilter::RecentLimit(limit) => Some(*limit),
        _ => None,
    };
    let is_recent = recent_limit.is_some();
    if is_recent {
        query.push("(SELECT * FROM codex_invocations");
    } else {
        query.push("codex_invocations");
    }

    let mut has_where = false;
    match filter {
        StatsFilter::All => {}
        StatsFilter::Since(start) => {
            query
                .push(" WHERE occurred_at >= ")
                .push_bind(db_occurred_at_lower_bound(start));
            has_where = true;
        }
        StatsFilter::Range(start, end) => {
            query
                .push(" WHERE occurred_at >= ")
                .push_bind(db_occurred_at_lower_bound(start))
                .push(" AND occurred_at < ")
                .push_bind(db_occurred_at_upper_bound(end));
            has_where = true;
        }
        StatsFilter::RecentLimit(limit) => {
            let _ = limit;
        }
    }
    if source_scope == InvocationSourceScope::ProxyOnly {
        query
            .push(if has_where {
                " AND source = "
            } else {
                " WHERE source = "
            })
            .push_bind(SOURCE_PROXY);
    }
    if is_recent {
        query
            .push(" ORDER BY occurred_at DESC LIMIT ")
            .push_bind(recent_limit.expect("recent filter has a limit"))
            .push(") AS recent");
    }
    query
        .build_query_as::<StatsRow>()
        .fetch_one(pool)
        .await
        .map_err(Into::into)
}

/// Aggregate the live invocation table through a previously admitted durable id fence.
///
/// Summary projection hydration uses this instead of an unbounded `StatsFilter::All` read so
/// concurrent inserts after the bounded admission cannot expand the scan or change the snapshot
/// being published.
pub(crate) async fn query_stats_row_through_id(
    pool: &Pool<Sqlite>,
    source_scope: InvocationSourceScope,
    upper_bound_id: i64,
) -> Result<StatsRow> {
    let mut query = QueryBuilder::<Sqlite>::new("SELECT ");
    query
        .push(stats_success_failure_select_sql())
        .push(" FROM codex_invocations WHERE id <= ")
        .push_bind(upper_bound_id);
    if source_scope == InvocationSourceScope::ProxyOnly {
        query.push(" AND source = ").push_bind(SOURCE_PROXY);
    }
    query
        .build_query_as::<StatsRow>()
        .fetch_one(pool)
        .await
        .map_err(Into::into)
}

pub(crate) async fn query_upstream_account_stats_row(
    pool: &Pool<Sqlite>,
    filter: StatsFilter,
    source_scope: InvocationSourceScope,
    upstream_account_id: i64,
) -> Result<StatsRow> {
    let account_filter = "CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.upstreamAccountId') AS INTEGER) END";
    let mut query = QueryBuilder::<Sqlite>::new("SELECT ");
    query.push(stats_success_failure_select_sql());
    match filter {
        StatsFilter::All => {
            query.push(" FROM codex_invocations WHERE ");
            query
                .push(account_filter)
                .push(" = ")
                .push_bind(upstream_account_id);
            if source_scope == InvocationSourceScope::ProxyOnly {
                query.push(" AND source = ").push_bind(SOURCE_PROXY);
            }
        }
        StatsFilter::Since(start) => {
            query.push(" FROM codex_invocations WHERE ");
            query
                .push(account_filter)
                .push(" = ")
                .push_bind(upstream_account_id);
            query
                .push(" AND occurred_at >= ")
                .push_bind(db_occurred_at_lower_bound(start));
            if source_scope == InvocationSourceScope::ProxyOnly {
                query.push(" AND source = ").push_bind(SOURCE_PROXY);
            }
        }
        StatsFilter::Range(start, end) => {
            query.push(" FROM codex_invocations WHERE ");
            query
                .push(account_filter)
                .push(" = ")
                .push_bind(upstream_account_id);
            query
                .push(" AND occurred_at >= ")
                .push_bind(db_occurred_at_lower_bound(start))
                .push(" AND occurred_at < ")
                .push_bind(db_occurred_at_upper_bound(end));
            if source_scope == InvocationSourceScope::ProxyOnly {
                query.push(" AND source = ").push_bind(SOURCE_PROXY);
            }
        }
        StatsFilter::RecentLimit(limit) => {
            query.push(" FROM (SELECT * FROM codex_invocations WHERE ");
            query
                .push(account_filter)
                .push(" = ")
                .push_bind(upstream_account_id);
            if source_scope == InvocationSourceScope::ProxyOnly {
                query.push(" AND source = ").push_bind(SOURCE_PROXY);
            }
            query
                .push(" ORDER BY occurred_at DESC, id DESC LIMIT ")
                .push_bind(limit)
                .push(") AS recent");
        }
    }
    query
        .build_query_as::<StatsRow>()
        .fetch_one(pool)
        .await
        .map_err(Into::into)
}

#[derive(Debug, Clone, FromRow)]
pub(crate) struct ArchiveBatchPathRow {
    file_path: String,
    month_key: Option<String>,
    coverage_start_at: Option<String>,
    coverage_end_at: Option<String>,
    historical_rollups_materialized_at: Option<String>,
    needs_overall: Option<i64>,
    needs_failures: Option<i64>,
}

impl ArchiveBatchPathRow {
    pub(crate) fn from_file_path(file_path: impl Into<String>) -> Self {
        Self {
            file_path: file_path.into(),
            month_key: None,
            coverage_start_at: None,
            coverage_end_at: None,
            historical_rollups_materialized_at: None,
            needs_overall: None,
            needs_failures: None,
        }
    }

    pub(crate) fn with_coverage(
        file_path: impl Into<String>,
        coverage_start_at: Option<String>,
        coverage_end_at: Option<String>,
    ) -> Self {
        Self::with_coverage_and_historical_rollups(
            file_path,
            coverage_start_at,
            coverage_end_at,
            None,
        )
    }

    pub(crate) fn with_coverage_and_historical_rollups(
        file_path: impl Into<String>,
        coverage_start_at: Option<String>,
        coverage_end_at: Option<String>,
        historical_rollups_materialized_at: Option<String>,
    ) -> Self {
        Self {
            file_path: file_path.into(),
            month_key: None,
            coverage_start_at,
            coverage_end_at,
            historical_rollups_materialized_at,
            needs_overall: None,
            needs_failures: None,
        }
    }

    pub(crate) fn file_path(&self) -> &str {
        &self.file_path
    }

    pub(crate) fn has_materialized_historical_rollups(&self) -> bool {
        self.historical_rollups_materialized_at.is_some()
    }

    pub(crate) fn coverage_start_at(&self) -> Option<&str> {
        self.coverage_start_at.as_deref()
    }

    pub(crate) fn coverage_end_at(&self) -> Option<&str> {
        self.coverage_end_at.as_deref()
    }
}

#[derive(Debug, Clone, FromRow)]
pub(crate) struct ArchivedInvocationFailureRow {
    pub(crate) id: i64,
    pub(crate) occurred_at: String,
    pub(crate) source: String,
    pub(crate) status: Option<String>,
    pub(crate) error_message: Option<String>,
    pub(crate) failure_kind: Option<String>,
    pub(crate) failure_class: Option<String>,
    pub(crate) is_actionable: Option<i64>,
}

#[derive(Debug, Default)]
pub(crate) struct ClearedSummaryRollupBuckets {
    overall: HashSet<(i64, String)>,
    failures: HashSet<(i64, String)>,
}

impl ClearedSummaryRollupBuckets {
    fn targets_to_clear_for_bucket(
        &mut self,
        key: &(i64, String),
        requested_targets: &[&str],
    ) -> Vec<&'static str> {
        let mut targets = Vec::new();
        if requested_targets.contains(&HOURLY_ROLLUP_TARGET_INVOCATIONS)
            && self.overall.insert(key.clone())
        {
            targets.push(HOURLY_ROLLUP_TARGET_INVOCATIONS);
        }
        if requested_targets.contains(&HOURLY_ROLLUP_TARGET_INVOCATION_FAILURES)
            && self.failures.insert(key.clone())
        {
            targets.push(HOURLY_ROLLUP_TARGET_INVOCATION_FAILURES);
        }
        targets
    }
}

pub(crate) const INVOCATION_SUMMARY_ROLLUP_REPAIR_MARKER_DATASET: &str =
    "codex_invocations_summary_rollup_v2";
pub(crate) const INVOCATION_SUMMARY_ROLLUP_REPAIR_MARKER_DONE: i64 = 1;
pub(crate) const INVOCATION_SUMMARY_ROLLUP_TARGETS: [&str; 2] = [
    HOURLY_ROLLUP_TARGET_INVOCATIONS,
    HOURLY_ROLLUP_TARGET_INVOCATION_FAILURES,
];

pub(crate) async fn load_invocation_hourly_source_rows_after_id(
    pool: &Pool<Sqlite>,
    start_after_id: i64,
    source_scope: InvocationSourceScope,
    limit: i64,
) -> Result<Vec<InvocationHourlySourceRecord>> {
    let archive_columns = load_archive_table_columns(pool, "codex_invocations").await?;
    let query_sql = build_legacy_compatible_invocation_archive_query(&archive_columns);
    match source_scope {
        InvocationSourceScope::All => sqlx::query_as::<_, InvocationHourlySourceRecord>(&query_sql)
            .bind(start_after_id)
            .bind(limit)
            .fetch_all(pool)
            .await
            .map_err(Into::into),
        InvocationSourceScope::ProxyOnly => {
            let proxy_query_sql =
                query_sql.replacen("WHERE id > ?1", "WHERE id > ?1 AND source = ?3", 1);
            sqlx::query_as::<_, InvocationHourlySourceRecord>(&proxy_query_sql)
                .bind(start_after_id)
                .bind(limit)
                .bind(SOURCE_PROXY)
                .fetch_all(pool)
                .await
                .map_err(Into::into)
        }
    }
}

async fn load_live_invocation_hourly_source_rows_after_id(
    executor: impl sqlx::Executor<'_, Database = Sqlite>,
    start_after_id: i64,
    source_scope: InvocationSourceScope,
    limit: i64,
) -> Result<Vec<InvocationHourlySourceRecord>> {
    let mut query = QueryBuilder::<Sqlite>::new(
        r#"
        SELECT
            id,
            occurred_at,
            source,
            status,
            detail_level,
            model,
            input_tokens,
            output_tokens,
            cache_input_tokens,
            reasoning_tokens,
            total_tokens,
            cost,
            cost_input,
            cost_cache_write,
            cost_cache_read,
            cost_output,
            cost_reasoning,
            error_message,
            failure_kind,
            failure_class,
            is_actionable,
            payload,
            t_total_ms,
            t_req_read_ms,
            t_req_parse_ms,
            t_upstream_connect_ms,
            t_upstream_ttfb_ms,
            first_token_ms,
            t_upstream_stream_ms,
            t_resp_parse_ms,
            t_persist_ms
        FROM codex_invocations
        WHERE id >
        "#,
    );
    query.push_bind(start_after_id);
    if source_scope == InvocationSourceScope::ProxyOnly {
        query.push(" AND source = ").push_bind(SOURCE_PROXY);
    }
    query.push(" ORDER BY id ASC LIMIT ").push_bind(limit);
    query
        .build_query_as::<InvocationHourlySourceRecord>()
        .fetch_all(executor)
        .await
        .map_err(Into::into)
}

pub(crate) async fn load_live_invocation_ids_after_id(
    executor: impl sqlx::Executor<'_, Database = Sqlite>,
    source_scope: InvocationSourceScope,
    start_after_id: i64,
) -> Result<HashSet<i64>> {
    if start_after_id < 0 {
        return Ok(HashSet::new());
    }

    let mut query = QueryBuilder::<Sqlite>::new("SELECT id FROM codex_invocations WHERE id > ");
    query.push_bind(start_after_id);
    if source_scope == InvocationSourceScope::ProxyOnly {
        query.push(" AND source = ").push_bind(SOURCE_PROXY);
    }

    #[derive(Debug, FromRow)]
    struct IdRow {
        id: i64,
    }

    Ok(query
        .build_query_as::<IdRow>()
        .fetch_all(executor)
        .await?
        .into_iter()
        .map(|row| row.id)
        .collect())
}

pub(crate) async fn load_live_invocation_ids_after_id_bounded(
    executor: impl sqlx::Executor<'_, Database = Sqlite>,
    source_scope: InvocationSourceScope,
    start_after_id: i64,
    limit: usize,
) -> Result<HashSet<i64>> {
    Ok(load_live_invocation_ids_after_id_bounded_snapshot(
        executor,
        source_scope,
        start_after_id,
        limit,
    )
    .await?
    .ids)
}

/// A bounded live-tail admission together with the highest durable id it observed. Consumers
/// fence follow-up aggregates at `upper_bound_id`, so concurrent inserts cannot turn a proven
/// bounded tail into an unbounded aggregate.
#[derive(Debug, Clone)]
pub(crate) struct BoundedLiveInvocationIds {
    pub(crate) ids: HashSet<i64>,
    pub(crate) upper_bound_id: i64,
}

pub(crate) async fn load_live_invocation_ids_after_id_bounded_snapshot(
    executor: impl sqlx::Executor<'_, Database = Sqlite>,
    source_scope: InvocationSourceScope,
    start_after_id: i64,
    limit: usize,
) -> Result<BoundedLiveInvocationIds> {
    if start_after_id < 0 {
        return Ok(BoundedLiveInvocationIds {
            ids: HashSet::new(),
            upper_bound_id: start_after_id,
        });
    }
    let mut query = QueryBuilder::<Sqlite>::new("SELECT id FROM codex_invocations WHERE id > ");
    query.push_bind(start_after_id);
    if source_scope == InvocationSourceScope::ProxyOnly {
        query.push(" AND source = ").push_bind(SOURCE_PROXY);
    }
    #[derive(Debug, FromRow)]
    struct IdRow {
        id: i64,
    }
    let rows = query
        .push(" ORDER BY id ASC")
        .push(" LIMIT ")
        .push_bind((limit.saturating_add(1)) as i64)
        .build_query_as::<IdRow>()
        .fetch_all(executor)
        .await?;
    if rows.len() > limit {
        return Err(anyhow!("summary live-tail id budget exceeded ({})", limit));
    }
    let upper_bound_id = rows.last().map(|row| row.id).unwrap_or(start_after_id);
    Ok(BoundedLiveInvocationIds {
        ids: rows.into_iter().map(|row| row.id).collect(),
        upper_bound_id,
    })
}
