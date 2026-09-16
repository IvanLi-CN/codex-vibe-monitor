pub(crate) fn bucket_spec_from_seconds(bucket_seconds: i64) -> Option<&'static str> {
    match bucket_seconds {
        60 => Some("1m"),
        300 => Some("5m"),
        900 => Some("15m"),
        1800 => Some("30m"),
        3600 => Some("1h"),
        21_600 => Some("6h"),
        43_200 => Some("12h"),
        86_400 => Some("1d"),
        _ => None,
    }
}

pub(crate) fn available_timeseries_bucket_specs(subhour_supported: bool) -> Vec<String> {
    if subhour_supported {
        vec!["1m", "5m", "15m", "30m", "1h", "6h", "12h", "1d"]
            .into_iter()
            .map(str::to_string)
            .collect()
    } else {
        vec!["1h", "6h", "12h", "1d"]
            .into_iter()
            .map(str::to_string)
            .collect()
    }
}

pub(crate) fn default_bucket_seconds(range: ChronoDuration) -> i64 {
    let seconds = range.num_seconds();
    if seconds <= 3_600 {
        60
    } else if seconds <= 172_800 {
        1_800
    } else if seconds <= 2_592_000 {
        3_600
    } else {
        86_400
    }
}

pub(crate) fn align_bucket_epoch(epoch: i64, bucket_seconds: i64, offset_seconds: i64) -> i64 {
    ((epoch + offset_seconds) / bucket_seconds) * bucket_seconds - offset_seconds
}

pub(crate) fn parse_summary_window(
    query: &SummaryQuery,
    default_limit: i64,
) -> Result<SummaryWindow> {
    match query.window.as_deref() {
        Some("current") => {
            let limit = query.limit.unwrap_or(default_limit).clamp(1, default_limit);
            Ok(SummaryWindow::Current(limit))
        }
        Some("all") => Ok(SummaryWindow::All),
        Some("previous7d") => Ok(SummaryWindow::PreviousFullDays(7)),
        Some(raw @ ("today" | "yesterday" | "thisWeek" | "thisMonth")) => {
            Ok(SummaryWindow::Calendar(raw.to_string()))
        }
        Some(raw) => Ok(SummaryWindow::Duration(parse_duration_spec(raw)?)),
        None => Ok(SummaryWindow::Duration(ChronoDuration::days(1))),
    }
}

pub(crate) async fn query_stats_row(
    pool: &Pool<Sqlite>,
    filter: StatsFilter,
    source_scope: InvocationSourceScope,
) -> Result<StatsRow> {
    match (filter, source_scope) {
        (StatsFilter::All, InvocationSourceScope::ProxyOnly) => {
            let query = format!(
                "SELECT {} FROM codex_invocations WHERE source = ?1",
                stats_success_failure_select_sql()
            );
            sqlx::query_as::<_, StatsRow>(&query)
                .bind(SOURCE_PROXY)
                .fetch_one(pool)
                .await
                .map_err(Into::into)
        }
        (StatsFilter::All, InvocationSourceScope::All) => {
            let query = format!(
                "SELECT {} FROM codex_invocations",
                stats_success_failure_select_sql()
            );
            sqlx::query_as::<_, StatsRow>(&query)
                .fetch_one(pool)
                .await
                .map_err(Into::into)
        }
        (StatsFilter::Since(start), InvocationSourceScope::ProxyOnly) => {
            let query = format!(
                "SELECT {} FROM codex_invocations WHERE source = ?1 AND occurred_at >= ?2",
                stats_success_failure_select_sql()
            );
            sqlx::query_as::<_, StatsRow>(&query)
                .bind(SOURCE_PROXY)
                .bind(db_occurred_at_lower_bound(start))
                .fetch_one(pool)
                .await
                .map_err(Into::into)
        }
        (StatsFilter::Since(start), InvocationSourceScope::All) => {
            let query = format!(
                "SELECT {} FROM codex_invocations WHERE occurred_at >= ?1",
                stats_success_failure_select_sql()
            );
            sqlx::query_as::<_, StatsRow>(&query)
                .bind(db_occurred_at_lower_bound(start))
                .fetch_one(pool)
                .await
                .map_err(Into::into)
        }
        (StatsFilter::Range(start, end), InvocationSourceScope::ProxyOnly) => {
            let query = format!(
                "SELECT {} FROM codex_invocations WHERE source = ?1 AND occurred_at >= ?2 AND occurred_at < ?3",
                stats_success_failure_select_sql()
            );
            sqlx::query_as::<_, StatsRow>(&query)
                .bind(SOURCE_PROXY)
                .bind(db_occurred_at_lower_bound(start))
                .bind(db_occurred_at_upper_bound(end))
                .fetch_one(pool)
                .await
                .map_err(Into::into)
        }
        (StatsFilter::Range(start, end), InvocationSourceScope::All) => {
            let query = format!(
                "SELECT {} FROM codex_invocations WHERE occurred_at >= ?1 AND occurred_at < ?2",
                stats_success_failure_select_sql()
            );
            sqlx::query_as::<_, StatsRow>(&query)
                .bind(db_occurred_at_lower_bound(start))
                .bind(db_occurred_at_upper_bound(end))
                .fetch_one(pool)
                .await
                .map_err(Into::into)
        }
        (StatsFilter::RecentLimit(limit), InvocationSourceScope::ProxyOnly) => {
            let query = format!(
                "WITH recent AS ( \
                    SELECT * \
                    FROM codex_invocations \
                    WHERE source = ?1 \
                    ORDER BY occurred_at DESC \
                    LIMIT ?2 \
                ) \
                SELECT {} FROM recent",
                stats_success_failure_select_sql()
            );
            sqlx::query_as::<_, StatsRow>(&query)
                .bind(SOURCE_PROXY)
                .bind(limit)
                .fetch_one(pool)
                .await
                .map_err(Into::into)
        }
        (StatsFilter::RecentLimit(limit), InvocationSourceScope::All) => {
            let query = format!(
                "WITH recent AS ( \
                    SELECT * \
                    FROM codex_invocations \
                    ORDER BY occurred_at DESC \
                    LIMIT ?1 \
                ) \
                SELECT {} FROM recent",
                stats_success_failure_select_sql()
            );
            sqlx::query_as::<_, StatsRow>(&query)
                .bind(limit)
                .fetch_one(pool)
                .await
                .map_err(Into::into)
        }
    }
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

pub(crate) async fn load_completed_invocation_archive_paths(
    executor: impl sqlx::Executor<'_, Database = Sqlite>,
) -> Result<Vec<ArchiveBatchPathRow>> {
    load_completed_archive_paths_for_dataset(executor, HOURLY_ROLLUP_DATASET_INVOCATIONS).await
}

pub(crate) async fn load_completed_archive_paths_for_dataset(
    executor: impl sqlx::Executor<'_, Database = Sqlite>,
    dataset: &str,
) -> Result<Vec<ArchiveBatchPathRow>> {
    let mut query = QueryBuilder::<Sqlite>::new(
        r#"
        SELECT
            file_path,
            month_key,
            coverage_start_at,
            coverage_end_at,
            historical_rollups_materialized_at,
            NULL AS needs_overall,
            NULL AS needs_failures
        FROM archive_batches
        WHERE dataset =
        "#,
    );
    query.push_bind(dataset).push(" AND status = ");
    query.push_bind(ARCHIVE_STATUS_COMPLETED);
    if dataset == HOURLY_ROLLUP_DATASET_INVOCATIONS {
        query.push(" AND COALESCE(summary_source_kind, 'unknown') <> 'live_mirror'");
    }
    query.push(" ORDER BY month_key ASC, created_at ASC, id ASC");
    query
        .build_query_as::<ArchiveBatchPathRow>()
        .fetch_all(executor)
        .await
        .map_err(Into::into)
}

pub(crate) async fn load_invocation_archives_missing_rollup_target(
    executor: impl sqlx::Executor<'_, Database = Sqlite>,
    target: &str,
    range: Option<(DateTime<Utc>, DateTime<Utc>)>,
) -> Result<Vec<ArchiveBatchPathRow>> {
    load_invocation_archives_missing_rollup_target_with_limit(executor, target, range, None).await
}

pub(crate) async fn load_invocation_archives_missing_rollup_target_bounded(
    executor: impl sqlx::Executor<'_, Database = Sqlite>,
    target: &str,
    range: Option<(DateTime<Utc>, DateTime<Utc>)>,
    limit: usize,
) -> Result<Vec<ArchiveBatchPathRow>> {
    load_invocation_archives_missing_rollup_target_with_limit(executor, target, range, Some(limit))
        .await
}

async fn load_invocation_archives_missing_rollup_target_with_limit(
    executor: impl sqlx::Executor<'_, Database = Sqlite>,
    target: &str,
    range: Option<(DateTime<Utc>, DateTime<Utc>)>,
    limit: Option<usize>,
) -> Result<Vec<ArchiveBatchPathRow>> {
    let mut query = QueryBuilder::<Sqlite>::new(
        r#"
        SELECT
            batches.file_path,
            batches.month_key,
            batches.coverage_start_at,
            batches.coverage_end_at,
            batches.historical_rollups_materialized_at,
            NULL AS needs_overall,
            NULL AS needs_failures
        FROM archive_batches AS batches
        WHERE batches.dataset = 'codex_invocations'
          AND batches.status =
        "#,
    );
    query.push_bind(ARCHIVE_STATUS_COMPLETED);
    query.push(
        r#"
          AND COALESCE(batches.summary_source_kind, 'unknown') <> 'live_mirror'
          AND NOT EXISTS(
            SELECT 1
            FROM hourly_rollup_archive_replay AS replay
            WHERE replay.target =
        "#,
    );
    query.push_bind(target);
    query.push(
        r#"
              AND replay.dataset = 'codex_invocations'
              AND replay.file_path = batches.file_path
              AND batches.sha256 IS NOT NULL
              AND TRIM(batches.sha256) <> ''
              AND replay.archive_sha256 = batches.sha256
          )
        "#,
    );

    if let Some((start, end)) = range {
        let start_bound = db_occurred_at_upper_bound(start);
        let end_bound = db_occurred_at_lower_bound(end);
        query.push(
            r#"
            AND (
                batches.coverage_start_at IS NULL
                OR batches.coverage_end_at IS NULL
                OR (
                    batches.coverage_end_at >=
            "#,
        );
        query.push_bind(start_bound).push(
            r#"
                    AND batches.coverage_start_at <
            "#,
        );
        query.push_bind(end_bound).push(
            r#"
                )
            )
            "#,
        );
    }

    query.push(" ORDER BY batches.month_key ASC, batches.created_at ASC, batches.id ASC");
    if let Some(limit) = limit {
        query
            .push(" LIMIT ")
            .push_bind((limit.saturating_add(1)) as i64);
    }
    query
        .build_query_as::<ArchiveBatchPathRow>()
        .fetch_all(executor)
        .await
        .map_err(Into::into)
}

pub(crate) fn account_archive_target_treats_materialized_batch_as_replayed(target: &str) -> bool {
    matches!(
        target,
        HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE
            | HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_STATS_HOURLY
            | HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_STATS_MINUTE
    )
}

pub(crate) async fn load_completed_invocation_archive_paths_in_range(
    executor: impl sqlx::Executor<'_, Database = Sqlite>,
    range: Option<(DateTime<Utc>, DateTime<Utc>)>,
) -> Result<Vec<ArchiveBatchPathRow>> {
    load_completed_archive_paths_for_dataset_in_range_with_limit(
        executor,
        HOURLY_ROLLUP_DATASET_INVOCATIONS,
        range,
        None,
    )
    .await
}

pub(crate) async fn load_completed_invocation_archive_paths_in_range_bounded(
    executor: impl sqlx::Executor<'_, Database = Sqlite>,
    range: Option<(DateTime<Utc>, DateTime<Utc>)>,
    limit: usize,
) -> Result<Vec<ArchiveBatchPathRow>> {
    load_completed_archive_paths_for_dataset_in_range_with_limit(
        executor,
        HOURLY_ROLLUP_DATASET_INVOCATIONS,
        range,
        Some(limit),
    )
    .await
}

pub(crate) async fn load_completed_archive_paths_for_dataset_in_range(
    executor: impl sqlx::Executor<'_, Database = Sqlite>,
    dataset: &str,
    range: Option<(DateTime<Utc>, DateTime<Utc>)>,
) -> Result<Vec<ArchiveBatchPathRow>> {
    load_completed_archive_paths_for_dataset_in_range_with_limit(executor, dataset, range, None)
        .await
}

async fn load_completed_archive_paths_for_dataset_in_range_with_limit(
    executor: impl sqlx::Executor<'_, Database = Sqlite>,
    dataset: &str,
    range: Option<(DateTime<Utc>, DateTime<Utc>)>,
    limit: Option<usize>,
) -> Result<Vec<ArchiveBatchPathRow>> {
    let mut query = QueryBuilder::<Sqlite>::new(
        r#"
        SELECT
            file_path,
            month_key,
            coverage_start_at,
            coverage_end_at,
            historical_rollups_materialized_at,
            NULL AS needs_overall,
            NULL AS needs_failures
        FROM archive_batches
        WHERE dataset =
        "#,
    );
    query.push_bind(dataset).push(
        r#"
          AND status =
        "#,
    );
    query.push_bind(ARCHIVE_STATUS_COMPLETED);
    if dataset == HOURLY_ROLLUP_DATASET_INVOCATIONS {
        // Detail mirrors preserve payload observability while the canonical invocation remains
        // live. They are not Summary sources and must not consume archive admission capacity.
        query.push(" AND COALESCE(summary_source_kind, 'unknown') <> 'live_mirror'");
    }

    if let Some((start, end)) = range {
        let start_bound = db_occurred_at_upper_bound(start);
        let end_bound = db_occurred_at_lower_bound(end);
        query.push(
            r#"
            AND (
                coverage_start_at IS NULL
                OR coverage_end_at IS NULL
                OR (
                    coverage_end_at >=
            "#,
        );
        query.push_bind(start_bound).push(
            r#"
                    AND coverage_start_at <
            "#,
        );
        query.push_bind(end_bound).push(
            r#"
                )
            )
            "#,
        );
    }

    query.push(" ORDER BY month_key ASC, created_at ASC, id ASC");
    if let Some(limit) = limit {
        query
            .push(" LIMIT ")
            .push_bind((limit.saturating_add(1)) as i64);
    }
    query
        .build_query_as::<ArchiveBatchPathRow>()
        .fetch_all(executor)
        .await
        .map_err(Into::into)
}

pub(crate) async fn load_completed_invocation_archives_in_range(
    executor: impl sqlx::Executor<'_, Database = Sqlite>,
    range: Option<(DateTime<Utc>, DateTime<Utc>)>,
) -> Result<Vec<(String, Option<String>, Option<String>, Option<String>)>> {
    Ok(
        load_completed_invocation_archive_paths_in_range(executor, range)
            .await?
            .into_iter()
            .map(|row| {
                (
                    row.file_path,
                    row.coverage_start_at,
                    row.coverage_end_at,
                    row.historical_rollups_materialized_at,
                )
            })
            .collect(),
    )
}
