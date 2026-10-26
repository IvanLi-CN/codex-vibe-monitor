use super::*;

#[derive(Debug, Clone, FromRow)]
pub(crate) struct ArchiveBatchPathRow {
    pub(super) file_path: String,
    pub(super) month_key: Option<String>,
    coverage_start_at: Option<String>,
    coverage_end_at: Option<String>,
    pub(super) historical_rollups_materialized_at: Option<String>,
    pub(super) needs_overall: Option<i64>,
    pub(super) needs_failures: Option<i64>,
}

#[derive(Debug, Clone, FromRow)]
struct SummaryArchiveRepairPageRow {
    id: i64,
    file_path: String,
    sha256: String,
    month_key: Option<String>,
    coverage_start_at: Option<String>,
    coverage_end_at: Option<String>,
    historical_rollups_materialized_at: Option<String>,
}

impl SummaryArchiveRepairPageRow {
    fn into_path_row(self) -> ArchiveBatchPathRow {
        ArchiveBatchPathRow {
            file_path: self.file_path,
            month_key: self.month_key,
            coverage_start_at: self.coverage_start_at,
            coverage_end_at: self.coverage_end_at,
            historical_rollups_materialized_at: self.historical_rollups_materialized_at,
            needs_overall: None,
            needs_failures: None,
        }
    }
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
    pub(super) overall: HashSet<(i64, String)>,
    pub(super) failures: HashSet<(i64, String)>,
}

impl ClearedSummaryRollupBuckets {
    pub(super) fn targets_to_clear_for_bucket(
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

pub(crate) async fn load_invocation_summary_repair_seen_ids(
    tx: &mut SqliteConnection,
) -> Result<HashSet<i64>> {
    Ok(sqlx::query_scalar::<_, i64>(
        "SELECT invocation_id FROM hourly_rollup_repair_seen_invocation_ids WHERE dataset = ?1",
    )
    .bind(INVOCATION_SUMMARY_ROLLUP_REPAIR_SEEN_IDS_DATASET)
    .fetch_all(&mut *tx)
    .await?
    .into_iter()
    .collect())
}

pub(crate) async fn persist_invocation_summary_repair_seen_ids(
    tx: &mut SqliteConnection,
    rows: &[InvocationHourlySourceRecord],
) -> Result<()> {
    if rows.is_empty() {
        return Ok(());
    }
    let mut query = QueryBuilder::<Sqlite>::new(
        "INSERT OR IGNORE INTO hourly_rollup_repair_seen_invocation_ids (dataset, invocation_id) ",
    );
    query.push_values(rows, |mut bind, row| {
        bind.push_bind(INVOCATION_SUMMARY_ROLLUP_REPAIR_SEEN_IDS_DATASET)
            .push_bind(row.id);
    });
    query.build().execute(&mut *tx).await?;
    Ok(())
}

pub(crate) async fn retain_unseen_invocation_summary_repair_rows(
    tx: &mut SqliteConnection,
    rows: &mut Vec<InvocationHourlySourceRecord>,
) -> Result<()> {
    if rows.is_empty() {
        return Ok(());
    }
    let mut query = QueryBuilder::<Sqlite>::new(
        "SELECT invocation_id FROM hourly_rollup_repair_seen_invocation_ids WHERE dataset = ",
    );
    query.push_bind(INVOCATION_SUMMARY_ROLLUP_REPAIR_SEEN_IDS_DATASET);
    query.push(" AND invocation_id IN (");
    let mut separated = query.separated(", ");
    for row in rows.iter() {
        separated.push_bind(row.id);
    }
    separated.push_unseparated(")");
    let seen = query
        .build_query_scalar::<i64>()
        .fetch_all(&mut *tx)
        .await?
        .into_iter()
        .collect::<HashSet<_>>();
    rows.retain(|row| !seen.contains(&row.id));
    Ok(())
}

pub(crate) async fn clear_invocation_summary_repair_seen_ids(
    tx: &mut SqliteConnection,
) -> Result<()> {
    sqlx::query("DELETE FROM hourly_rollup_repair_seen_invocation_ids WHERE dataset = ?1")
        .bind(INVOCATION_SUMMARY_ROLLUP_REPAIR_SEEN_IDS_DATASET)
        .execute(&mut *tx)
        .await?;
    Ok(())
}

pub(crate) async fn load_invocation_summary_repair_cleared_buckets(
    tx: &mut SqliteConnection,
) -> Result<ClearedSummaryRollupBuckets> {
    #[derive(Debug, FromRow)]
    struct ClearedBucketRow {
        bucket_start_epoch: i64,
        source: String,
    }

    let rows = sqlx::query_as::<_, ClearedBucketRow>(
        "SELECT bucket_start_epoch, source FROM hourly_rollup_repair_cleared_buckets WHERE dataset = ?1",
    )
    .bind(INVOCATION_SUMMARY_ROLLUP_REPAIR_CLEARED_BUCKETS_DATASET)
    .fetch_all(&mut *tx)
    .await?;
    let mut cleared = ClearedSummaryRollupBuckets::default();
    cleared.overall.extend(
        rows.into_iter()
            .map(|row| (row.bucket_start_epoch, row.source)),
    );
    Ok(cleared)
}

pub(crate) async fn persist_invocation_summary_repair_cleared_buckets(
    tx: &mut SqliteConnection,
    cleared: &ClearedSummaryRollupBuckets,
) -> Result<()> {
    if cleared.overall.is_empty() {
        return Ok(());
    }
    let mut query = QueryBuilder::<Sqlite>::new(
        "INSERT OR IGNORE INTO hourly_rollup_repair_cleared_buckets (dataset, bucket_start_epoch, source) ",
    );
    query.push_values(
        &cleared.overall,
        |mut bind, (bucket_start_epoch, source)| {
            bind.push_bind(INVOCATION_SUMMARY_ROLLUP_REPAIR_CLEARED_BUCKETS_DATASET)
                .push_bind(bucket_start_epoch)
                .push_bind(source);
        },
    );
    query.build().execute(&mut *tx).await?;
    Ok(())
}

pub(crate) async fn clear_invocation_summary_repair_cleared_buckets(
    tx: &mut SqliteConnection,
) -> Result<()> {
    sqlx::query("DELETE FROM hourly_rollup_repair_cleared_buckets WHERE dataset = ?1")
        .bind(INVOCATION_SUMMARY_ROLLUP_REPAIR_CLEARED_BUCKETS_DATASET)
        .execute(&mut *tx)
        .await?;
    Ok(())
}

pub(crate) async fn invocation_summary_repair_backup_exists(
    executor: impl sqlx::Executor<'_, Database = Sqlite>,
) -> Result<bool> {
    Ok(sqlx::query_scalar::<_, i64>(
        r#"
        SELECT COUNT(*) = 2
        FROM sqlite_master
        WHERE type = 'table' AND name IN (?1, ?2)
        "#,
    )
    .bind(INVOCATION_SUMMARY_ROLLUP_REPAIR_BACKUP_OVERALL_TABLE)
    .bind(INVOCATION_SUMMARY_ROLLUP_REPAIR_BACKUP_FAILURES_TABLE)
    .fetch_one(executor)
    .await?
        != 0)
}

pub(crate) async fn create_invocation_summary_repair_backup_tx(
    tx: &mut SqliteConnection,
) -> Result<()> {
    for (table, backup_table) in [
        (
            "invocation_rollup_hourly",
            INVOCATION_SUMMARY_ROLLUP_REPAIR_BACKUP_OVERALL_TABLE,
        ),
        (
            "invocation_failure_rollup_hourly",
            INVOCATION_SUMMARY_ROLLUP_REPAIR_BACKUP_FAILURES_TABLE,
        ),
    ] {
        sqlx::query(&format!("DROP TABLE IF EXISTS {backup_table}"))
            .execute(&mut *tx)
            .await?;
        sqlx::query(&format!(
            "CREATE TABLE {backup_table} AS SELECT * FROM {table} WHERE 0"
        ))
        .execute(&mut *tx)
        .await?;
        sqlx::query(&format!("INSERT INTO {backup_table} SELECT * FROM {table}"))
            .execute(&mut *tx)
            .await?;
    }
    Ok(())
}

pub(crate) async fn clear_invocation_summary_repair_progress_tx(
    tx: &mut SqliteConnection,
) -> Result<()> {
    sqlx::query(
        "DELETE FROM hourly_rollup_live_progress WHERE dataset IN (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
    )
    .bind(INVOCATION_SUMMARY_ROLLUP_REPAIR_ARCHIVE_CURSOR_DATASET)
    .bind(INVOCATION_SUMMARY_ROLLUP_REPAIR_ARCHIVE_ROW_CURSOR_DATASET)
    .bind(INVOCATION_SUMMARY_ROLLUP_REPAIR_STARTED_DATASET)
    .bind(INVOCATION_SUMMARY_ROLLUP_REPAIR_INCOMPLETE_DATASET)
    .bind(INVOCATION_SUMMARY_ROLLUP_REPAIR_MARKER_DATASET)
    .bind(INVOCATION_SUMMARY_ROLLUP_REPAIR_RESTORED_LIVE_CURSOR_DATASET)
    .bind(INVOCATION_SUMMARY_ROLLUP_REPAIR_RESTORED_LIVE_END_DATASET)
    .execute(&mut *tx)
    .await?;
    clear_invocation_summary_repair_seen_ids(tx).await?;
    clear_invocation_summary_repair_cleared_buckets(tx).await?;
    Ok(())
}

pub(crate) async fn restore_invocation_summary_repair_backup(pool: &Pool<Sqlite>) -> Result<()> {
    let mut tx = pool.begin().await?;
    for (table, backup_table) in [
        (
            "invocation_rollup_hourly",
            INVOCATION_SUMMARY_ROLLUP_REPAIR_BACKUP_OVERALL_TABLE,
        ),
        (
            "invocation_failure_rollup_hourly",
            INVOCATION_SUMMARY_ROLLUP_REPAIR_BACKUP_FAILURES_TABLE,
        ),
    ] {
        sqlx::query(&format!("DELETE FROM {table}"))
            .execute(&mut *tx)
            .await?;
        sqlx::query(&format!("INSERT INTO {table} SELECT * FROM {backup_table}"))
            .execute(&mut *tx)
            .await?;
        sqlx::query(&format!("DROP TABLE {backup_table}"))
            .execute(&mut *tx)
            .await?;
    }
    clear_invocation_summary_repair_progress_tx(tx.as_mut()).await?;
    tx.commit().await?;
    Ok(())
}

pub(crate) async fn clear_invocation_summary_rollups_outside_buckets(
    tx: &mut SqliteConnection,
    preserved_bucket_epochs: &HashSet<i64>,
) -> Result<ClearedSummaryRollupBuckets> {
    let mut cleared = ClearedSummaryRollupBuckets::default();
    for table in [
        "invocation_rollup_hourly",
        "invocation_failure_rollup_hourly",
    ] {
        let mut select = QueryBuilder::<Sqlite>::new(format!(
            "SELECT bucket_start_epoch, source FROM {table} WHERE bucket_start_epoch NOT IN ("
        ));
        let mut ids = select.separated(", ");
        for bucket_epoch in preserved_bucket_epochs {
            ids.push_bind(bucket_epoch);
        }
        ids.push_unseparated(")");
        let keys = select
            .build_query_as::<(i64, String)>()
            .fetch_all(&mut *tx)
            .await?;
        for (bucket_start_epoch, source) in keys {
            let key = (bucket_start_epoch, source);
            cleared.overall.insert(key.clone());
            cleared.failures.insert(key);
        }

        let mut delete = QueryBuilder::<Sqlite>::new(format!(
            "DELETE FROM {table} WHERE bucket_start_epoch NOT IN ("
        ));
        let mut ids = delete.separated(", ");
        for bucket_epoch in preserved_bucket_epochs {
            ids.push_bind(bucket_epoch);
        }
        ids.push_unseparated(")");
        delete.build().execute(&mut *tx).await?;
    }
    Ok(cleared)
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

pub(crate) async fn load_live_invocation_hourly_source_rows_after_id(
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
    query.push(" AND sha256 IS NOT NULL AND TRIM(sha256) <> ''");
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
    query.push(" AND sha256 IS NOT NULL AND TRIM(sha256) <> ''");
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

pub(crate) async fn load_completed_invocation_summary_repair_page(
    executor: impl sqlx::Executor<'_, Database = Sqlite>,
    after_archive_id: i64,
    limit: usize,
) -> Result<Vec<(i64, String, ArchiveBatchPathRow)>> {
    let rows = sqlx::query_as::<_, SummaryArchiveRepairPageRow>(
        r#"
        SELECT
            id,
            file_path,
            sha256,
            month_key,
            coverage_start_at,
            coverage_end_at,
            historical_rollups_materialized_at
        FROM archive_batches INDEXED BY idx_archive_batches_summary_rollup_repair_seek
        WHERE dataset = 'codex_invocations'
          AND status = 'completed'
          AND sha256 IS NOT NULL
          AND TRIM(sha256) <> ''
          AND COALESCE(summary_source_kind, 'unknown') <> 'live_mirror'
          AND id > ?1
        ORDER BY id ASC
        LIMIT ?2
        "#,
    )
    .bind(after_archive_id)
    .bind((limit.saturating_add(1)) as i64)
    .fetch_all(executor)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| {
            let id = row.id;
            let sha256 = row.sha256.clone();
            (id, sha256, row.into_path_row())
        })
        .collect())
}

pub(crate) async fn load_invocation_summary_repair_archive_chunk(
    archive_row: &ArchiveBatchPathRow,
    expected_sha256: &str,
    start_after_id: i64,
    limit: i64,
) -> Result<(Vec<InvocationHourlySourceRecord>, i64, bool)> {
    let archive_path = PathBuf::from(archive_row.file_path());
    let archive_ref = archive_sha_log_ref(expected_sha256);
    if !archive_path.exists() {
        bail!(
            "{} (archive_ref={archive_ref})",
            MISSING_INVOCATION_ARCHIVE_REPAIR_PREFIX
        );
    }
    let source_sha_before = sha256_hex_file(&archive_path)?;
    if source_sha_before != expected_sha256 {
        bail!("summary rollup repair archive SHA changed before read (archive_ref={archive_ref})");
    }
    let temp_path = PathBuf::from(format!(
        "{}.{}.sqlite",
        archive_path.display(),
        retention_temp_suffix()
    ));
    if temp_path.exists() {
        remove_temp_sqlite_artifacts(&temp_path);
    }
    let temp_cleanup = TempSqliteCleanup::new(temp_path.clone());
    inflate_gzip_sqlite_file_with_operation_budget(&archive_path, &temp_path).await?;
    let archive_pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect(&sqlite_url_for_path(&temp_path))
        .await
        .context("failed to open archive batch")?;
    let mut rows = load_invocation_hourly_source_rows_after_id(
        &archive_pool,
        start_after_id,
        InvocationSourceScope::All,
        limit.saturating_add(1),
    )
    .await?;
    archive_pool.close().await;
    let source_sha_after = sha256_hex_file(&archive_path)?;
    if source_sha_after != expected_sha256 {
        drop(temp_cleanup);
        bail!("summary rollup repair archive SHA changed after read (archive_ref={archive_ref})");
    }
    let has_more = rows.len() > limit as usize;
    if has_more {
        rows.truncate(limit as usize);
    }
    let next_cursor = rows.last().map(|row| row.id).unwrap_or(start_after_id);
    drop(temp_cleanup);
    Ok((rows, next_cursor, !has_more))
}

pub(crate) async fn validate_invocation_summary_repair_archive(
    archive_row: &ArchiveBatchPathRow,
    expected_sha256: &str,
) -> Result<()> {
    let archive_path = PathBuf::from(archive_row.file_path());
    let archive_ref = archive_sha_log_ref(expected_sha256);
    if !archive_path.exists() {
        bail!(
            "{} (archive_ref={archive_ref})",
            MISSING_INVOCATION_ARCHIVE_REPAIR_PREFIX
        );
    }
    let source_sha_before = sha256_hex_file(&archive_path)?;
    if source_sha_before != expected_sha256 {
        bail!(
            "summary rollup repair archive SHA changed before validation (archive_ref={archive_ref})"
        );
    }
    let temp_path = PathBuf::from(format!(
        "{}.{}.sqlite",
        archive_path.display(),
        retention_temp_suffix()
    ));
    if temp_path.exists() {
        remove_temp_sqlite_artifacts(&temp_path);
    }
    let temp_cleanup = TempSqliteCleanup::new(temp_path.clone());
    inflate_gzip_sqlite_file_with_operation_budget(&archive_path, &temp_path).await?;
    let archive_pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect(&sqlite_url_for_path(&temp_path))
        .await
        .context("failed to open archive batch")?;

    let mut cursor_id = 0_i64;
    loop {
        let rows = load_invocation_hourly_source_rows_after_id(
            &archive_pool,
            cursor_id,
            InvocationSourceScope::All,
            INVOCATION_SUMMARY_ROLLUP_REPAIR_BATCH_SIZE,
        )
        .await?;
        let Some(last_id) = rows.last().map(|row| row.id) else {
            break;
        };
        cursor_id = last_id;
    }
    archive_pool.close().await;
    let source_sha_after = sha256_hex_file(&archive_path)?;
    if source_sha_after != expected_sha256 {
        bail!(
            "summary rollup repair archive SHA changed after validation (archive_ref={archive_ref})"
        );
    }
    drop(temp_cleanup);
    Ok(())
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

pub(crate) async fn load_invocation_archives_missing_effective_rollup_target(
    executor: impl sqlx::Executor<'_, Database = Sqlite>,
    target: &str,
    range: Option<(DateTime<Utc>, DateTime<Utc>)>,
) -> Result<Vec<ArchiveBatchPathRow>> {
    let archive_rows =
        load_invocation_archives_missing_rollup_target(executor, target, range).await?;
    if !account_archive_target_treats_materialized_batch_as_replayed(target) {
        return Ok(archive_rows);
    }
    Ok(archive_rows
        .into_iter()
        .filter(|archive_row| archive_row.historical_rollups_materialized_at.is_none())
        .collect())
}

pub(crate) async fn load_invocation_archives_missing_summary_rollup_markers(
    executor: impl sqlx::Executor<'_, Database = Sqlite>,
) -> Result<Vec<ArchiveBatchPathRow>> {
    sqlx::query_as::<_, ArchiveBatchPathRow>(
        r#"
        SELECT
            batches.file_path,
            batches.month_key,
            batches.coverage_start_at,
            batches.coverage_end_at,
            batches.historical_rollups_materialized_at,
            CASE
                WHEN EXISTS(
                    SELECT 1
                    FROM hourly_rollup_archive_replay AS replay
                    WHERE replay.target = ?2
                      AND replay.dataset = 'codex_invocations'
                      AND replay.file_path = batches.file_path AND replay.archive_sha256 = batches.sha256
                ) THEN 0
                ELSE 1
            END AS needs_overall,
            CASE
                WHEN EXISTS(
                    SELECT 1
                    FROM hourly_rollup_archive_replay AS replay
                    WHERE replay.target = ?3
                      AND replay.dataset = 'codex_invocations'
                      AND replay.file_path = batches.file_path AND replay.archive_sha256 = batches.sha256
                ) THEN 0
                ELSE 1
            END AS needs_failures
        FROM archive_batches AS batches
        WHERE batches.dataset = 'codex_invocations'
          AND batches.status = ?1
          AND COALESCE(batches.summary_source_kind, 'unknown') <> 'live_mirror'
          AND batches.sha256 IS NOT NULL
          AND TRIM(batches.sha256) <> ''
          AND (
            NOT EXISTS(
                SELECT 1
                FROM hourly_rollup_archive_replay AS replay
                WHERE replay.target = ?2
                  AND replay.dataset = 'codex_invocations'
                  AND replay.file_path = batches.file_path AND replay.archive_sha256 = batches.sha256
            )
            OR NOT EXISTS(
                SELECT 1
                FROM hourly_rollup_archive_replay AS replay
                WHERE replay.target = ?3
                  AND replay.dataset = 'codex_invocations'
                  AND replay.file_path = batches.file_path AND replay.archive_sha256 = batches.sha256
            )
          )
        ORDER BY batches.month_key ASC, batches.created_at ASC, batches.id ASC LIMIT 128
        "#,
    )
    .bind(ARCHIVE_STATUS_COMPLETED)
    .bind(HOURLY_ROLLUP_TARGET_INVOCATIONS)
    .bind(HOURLY_ROLLUP_TARGET_INVOCATION_FAILURES)
    .fetch_all(executor)
    .await
    .map_err(Into::into)
}

pub(crate) async fn open_invocation_archive_batch_pool(
    archive_row: &ArchiveBatchPathRow,
    read_surface: &'static str,
) -> Result<Option<(Pool<Sqlite>, TempSqliteCleanup)>> {
    open_archive_batch_pool(archive_row, "codex_invocations", read_surface).await
}

pub(crate) async fn open_pool_upstream_request_attempt_archive_batch_pool(
    archive_row: &ArchiveBatchPathRow,
    read_surface: &'static str,
) -> Result<Option<(Pool<Sqlite>, TempSqliteCleanup)>> {
    open_archive_batch_pool(archive_row, "pool_upstream_request_attempts", read_surface).await
}

struct ArchiveBatchPoolOpenDrain {
    pool: Option<Pool<Sqlite>>,
    cleanup: Option<TempSqliteCleanup>,
    file_phase: Option<crate::maintenance::FilePhasePermit>,
}

impl ArchiveBatchPoolOpenDrain {
    fn new(
        pool: &Pool<Sqlite>,
        cleanup: TempSqliteCleanup,
        file_phase: crate::maintenance::FilePhasePermit,
    ) -> Self {
        Self {
            pool: Some(pool.clone()),
            cleanup: Some(cleanup),
            file_phase: Some(file_phase),
        }
    }

    fn complete(&mut self) {
        self.pool.take();
        self.cleanup.take();
        self.file_phase.take();
    }

    fn disarm(mut self) -> TempSqliteCleanup {
        self.pool.take();
        self.file_phase.take();
        self.cleanup
            .take()
            .expect("archive pool opener drain cleanup")
    }
}

impl Drop for ArchiveBatchPoolOpenDrain {
    fn drop(&mut self) {
        let (Some(pool), Some(cleanup), Some(file_phase)) = (
            self.pool.take(),
            self.cleanup.take(),
            self.file_phase.take(),
        ) else {
            return;
        };
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            drop(runtime.spawn(async move {
                crate::maintenance::with_file_phase_permit(file_phase, async move {
                    pool.close().await;
                    drop(cleanup);
                })
                .await;
            }));
        } else {
            // Runtime teardown cannot provide a native SQLite drain acknowledgement.
            std::mem::forget(pool);
            std::mem::forget(cleanup);
            std::mem::forget(file_phase);
        }
    }
}

async fn open_archive_batch_pool(
    archive_row: &ArchiveBatchPathRow,
    expected_table: &str,
    read_surface: &'static str,
) -> Result<Option<(Pool<Sqlite>, TempSqliteCleanup)>> {
    let file_phase = crate::maintenance::acquire_file_phase_permit(read_surface).await?;
    crate::maintenance::with_file_phase_permit(file_phase.clone(), async move {
        let archive_path = PathBuf::from(&archive_row.file_path);
        let is_materialized_archive = archive_row.historical_rollups_materialized_at.is_some();
        if !archive_path.exists() {
            warn!(
                read_surface,
                historical_rollups_materialized = is_materialized_archive,
                "skipping missing archive while serving read-only historical fallback"
            );
            return Ok(None);
        }

        let temp_path = PathBuf::from(format!(
            "{}.{}.sqlite",
            archive_path.display(),
            retention_temp_suffix()
        ));
        if temp_path.exists() {
            let _ = fs::remove_file(&temp_path);
        }
        let temp_cleanup = TempSqliteCleanup::new(temp_path.clone());
        if let Err(err) =
            inflate_gzip_sqlite_file_with_operation_budget(&archive_path, &temp_path).await
        {
            drop(temp_cleanup);
            if is_unreadable_invocation_summary_archive_error(&err) {
                warn!(
                    read_surface,
                    error = %err,
                    historical_rollups_materialized = is_materialized_archive,
                    "skipping unreadable archive while serving read-only historical fallback"
                );
                return Ok(None);
            }
            return Err(err);
        }
        let archive_pool = match SqlitePoolOptions::new()
            .max_connections(1)
            .connect(&sqlite_url_for_path(&temp_path))
            .await
            .context("failed to open archive batch")
        {
            Ok(pool) => pool,
            Err(err) => {
                drop(temp_cleanup);
                if is_unreadable_invocation_summary_archive_error(&err) {
                    warn!(
                        read_surface,
                        error = %err,
                        historical_rollups_materialized = is_materialized_archive,
                        "skipping unreadable archive while serving read-only historical fallback"
                    );
                    return Ok(None);
                }
                return Err(err);
            }
        };
        let mut pool_drain =
            ArchiveBatchPoolOpenDrain::new(&archive_pool, temp_cleanup, file_phase.clone());
        let has_expected_table = sqlx::query_scalar::<_, i64>(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1)",
        )
        .bind(expected_table)
        .fetch_one(&archive_pool)
        .await?
            != 0;
        if !has_expected_table {
            archive_pool.close().await;
            pool_drain.complete();
            return Ok(None);
        }
        if expected_table == "codex_invocations" {
            ensure_invocation_archive_first_token_compatibility(&archive_pool).await?;
        }

        let temp_cleanup = pool_drain.disarm();
        Ok(Some((archive_pool, temp_cleanup)))
    })
    .await
}

#[derive(Debug, FromRow)]
struct ArchivedPoolRequestCompressionRow {
    invoke_id: String,
    occurred_at: String,
    attempt_index: i64,
    id: i64,
    request_compression_algorithm: Option<String>,
}

#[derive(Debug)]
struct ArchivedPoolRequestCompression {
    attempt_index: i64,
    id: i64,
    algorithm: Option<String>,
}

type ArchivedPoolRequestCompressionMap = HashMap<(String, String), ArchivedPoolRequestCompression>;

fn merge_archived_pool_request_compression(
    compression_by_invocation: &mut ArchivedPoolRequestCompressionMap,
    key: (String, String),
    candidate: ArchivedPoolRequestCompression,
) {
    match compression_by_invocation.entry(key) {
        std::collections::hash_map::Entry::Vacant(entry) => {
            entry.insert(candidate);
        }
        std::collections::hash_map::Entry::Occupied(mut entry)
            if (candidate.attempt_index, candidate.id)
                > (entry.get().attempt_index, entry.get().id) =>
        {
            entry.insert(candidate);
        }
        std::collections::hash_map::Entry::Occupied(_) => {}
    }
}

async fn load_archived_pool_request_compressions_from_executor<'e, E>(
    executor: E,
    range: ExactUtcRange,
) -> Result<ArchivedPoolRequestCompressionMap>
where
    E: sqlx::Executor<'e, Database = Sqlite>,
{
    let rows = sqlx::query_as::<_, ArchivedPoolRequestCompressionRow>(
        r#"
        SELECT
            invoke_id,
            occurred_at,
            attempt_index,
            id,
            NULLIF(TRIM(upstream_request_compression_algorithm), '') AS request_compression_algorithm
        FROM pool_upstream_request_attempts
        WHERE occurred_at >= ?1
          AND occurred_at < ?2
          AND LOWER(TRIM(COALESCE(status, ''))) <> 'budget_exhausted_final'
        ORDER BY invoke_id ASC, occurred_at ASC, attempt_index DESC, id DESC
        "#,
    )
    .bind(db_occurred_at_lower_bound(range.start))
    .bind(db_occurred_at_upper_bound(range.end))
    .fetch_all(executor)
    .await?;

    let mut compression_by_invocation = HashMap::new();
    for row in rows {
        let key = (row.invoke_id, row.occurred_at);
        let candidate = ArchivedPoolRequestCompression {
            attempt_index: row.attempt_index,
            id: row.id,
            algorithm: row.request_compression_algorithm,
        };
        merge_archived_pool_request_compression(&mut compression_by_invocation, key, candidate);
    }
    Ok(compression_by_invocation)
}

async fn load_archived_pool_request_compressions(
    pool: &Pool<Sqlite>,
    range: ExactUtcRange,
) -> Result<ArchivedPoolRequestCompressionMap> {
    let archive_rows = load_completed_archive_paths_for_dataset_in_range(
        pool,
        "pool_upstream_request_attempts",
        Some((range.start, range.end)),
    )
    .await?;
    let mut compression_by_invocation = HashMap::new();

    for archive_row in archive_rows {
        let Some((archive_pool, temp_cleanup)) = open_archive_batch_pool(
            &archive_row,
            "pool_upstream_request_attempts",
            "upstream-account-activity-pool-attempts",
        )
        .await?
        else {
            continue;
        };
        if sqlite_table_has_column(
            &archive_pool,
            "pool_upstream_request_attempts",
            "upstream_request_compression_algorithm",
        )
        .await?
        {
            for (key, compression) in
                load_archived_pool_request_compressions_from_executor(&archive_pool, range).await?
            {
                merge_archived_pool_request_compression(
                    &mut compression_by_invocation,
                    key,
                    compression,
                );
            }
        }
        archive_pool.close().await;
        drop(temp_cleanup);
    }

    Ok(compression_by_invocation)
}

async fn ensure_invocation_archive_first_token_compatibility(pool: &Pool<Sqlite>) -> Result<()> {
    if !sqlite_table_has_column(pool, "codex_invocations", "first_token_ms").await? {
        sqlx::query("ALTER TABLE codex_invocations ADD COLUMN first_token_ms REAL")
            .execute(pool)
            .await
            .context("failed to add nullable TTFT compatibility column to archive copy")?;
    }
    Ok(())
}

#[cfg(test)]
mod ttft_archive_compatibility_tests {
    use super::*;

    #[tokio::test]
    async fn legacy_archive_rows_expose_null_ttft_after_compatibility_upgrade() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("open sqlite");
        sqlx::query("CREATE TABLE codex_invocations (id INTEGER PRIMARY KEY)")
            .execute(&pool)
            .await
            .expect("create legacy invocation table");
        sqlx::query("INSERT INTO codex_invocations (id) VALUES (1)")
            .execute(&pool)
            .await
            .expect("insert legacy row");

        ensure_invocation_archive_first_token_compatibility(&pool)
            .await
            .expect("upgrade temporary archive copy");

        let first_token_ms =
            sqlx::query_scalar::<_, Option<f64>>("SELECT first_token_ms FROM codex_invocations")
                .fetch_one(&pool)
                .await
                .expect("read nullable TTFT");
        assert_eq!(first_token_ms, None);
    }

    #[tokio::test]
    async fn archived_pool_request_compression_uses_the_final_attempt() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("open sqlite");
        sqlx::query(
            "CREATE TABLE pool_upstream_request_attempts (id INTEGER PRIMARY KEY, invoke_id TEXT NOT NULL, occurred_at TEXT NOT NULL, attempt_index INTEGER NOT NULL, status TEXT, upstream_request_compression_algorithm TEXT)",
        )
        .execute(&pool)
        .await
        .expect("create archived attempts table");

        let range = ExactUtcRange {
            start: Utc::now() - ChronoDuration::minutes(1),
            end: Utc::now() + ChronoDuration::minutes(1),
        };
        let occurred_at = db_occurred_at_lower_bound(Utc::now());
        let prior_occurred_at =
            db_occurred_at_lower_bound(Utc::now() - ChronoDuration::seconds(30));
        sqlx::query(
            "INSERT INTO pool_upstream_request_attempts (id, invoke_id, occurred_at, attempt_index, status, upstream_request_compression_algorithm) VALUES (1, 'retry', ?1, 1, 'success', 'br'), (2, 'retry', ?1, 2, 'success', 'zstd'), (3, 'retry', ?2, 1, 'success', 'gzip'), (4, 'final-unknown', ?1, 1, 'success', 'gzip'), (5, 'final-unknown', ?1, 2, 'http_failure', NULL), (6, 'retry', ?1, 3, 'budget_exhausted_final', NULL)",
        )
        .bind(&occurred_at)
        .bind(&prior_occurred_at)
        .execute(&pool)
        .await
        .expect("insert archived attempts");

        let mut compression_by_invocation =
            load_archived_pool_request_compressions_from_executor(&pool, range)
                .await
                .expect("read archived request compression");

        assert_eq!(
            compression_by_invocation
                .get(&("retry".to_string(), occurred_at.clone()))
                .map(|compression| compression.algorithm.as_deref()),
            Some(Some("zstd"))
        );
        assert_eq!(
            compression_by_invocation
                .get(&("retry".to_string(), prior_occurred_at))
                .map(|compression| compression.algorithm.as_deref()),
            Some(Some("gzip"))
        );
        assert_eq!(
            compression_by_invocation
                .get(&("final-unknown".to_string(), occurred_at.clone()))
                .map(|compression| compression.algorithm.as_deref()),
            Some(None)
        );

        merge_archived_pool_request_compression(
            &mut compression_by_invocation,
            ("retry".to_string(), occurred_at.clone()),
            ArchivedPoolRequestCompression {
                attempt_index: 3,
                id: 6,
                algorithm: Some("deflate".to_string()),
            },
        );
        assert_eq!(
            compression_by_invocation
                .get(&("retry".to_string(), occurred_at))
                .map(|compression| compression.algorithm.as_deref()),
            Some(Some("deflate"))
        );
    }
}

pub(crate) async fn query_upstream_account_invocation_preview_rows_from_executor<'e, E>(
    executor: E,
    range: ExactUtcRange,
    source_scope: InvocationSourceScope,
    has_cost_breakdown_columns: bool,
    has_first_token_column: bool,
) -> Result<Vec<crate::api::UpstreamAccountInvocationPreviewRow>>
where
    E: sqlx::Executor<'e, Database = Sqlite>,
{
    let mut query = QueryBuilder::<Sqlite>::new("SELECT id, invoke_id, ");
    let conversation_created_at_sql = crate::api::invocation_history_conversation_created_at_sql(
        crate::api::INVOCATION_PROMPT_CACHE_KEY_SQL,
        source_scope,
    );
    let cost_breakdown_columns = if has_cost_breakdown_columns {
        "cost_input, cost_cache_write, cost_cache_read, cost_output, cost_reasoning"
    } else {
        "NULL AS cost_input, NULL AS cost_cache_write, NULL AS cost_cache_read, NULL AS cost_output, NULL AS cost_reasoning"
    };
    let first_token_ms = if has_first_token_column {
        "first_token_ms"
    } else {
        "NULL AS first_token_ms"
    };
    query
        .push(crate::api::INVOCATION_PROMPT_CACHE_KEY_SQL)
        .push(" AS prompt_cache_key, occurred_at, ")
        .push(conversation_created_at_sql.as_str())
        .push(" AS conversation_created_at, ")
        .push(crate::api::invocation_display_status_sql())
        .push(" AS status, ")
        .push("NULL AS live_phase, ")
        .push(crate::api::INVOCATION_RESOLVED_FAILURE_CLASS_SQL)
        .push(" AS failure_class, ")
        .push(crate::api::INVOCATION_ROUTE_MODE_SQL)
        .push(" AS route_mode, model, ")
        .push(crate::api::INVOCATION_REQUEST_MODEL_SQL)
        .push(" AS request_model, ")
        .push(crate::api::INVOCATION_RESPONSE_MODEL_SQL)
        .push(" AS response_model, COALESCE(total_tokens, 0) AS total_tokens, cost, ")
        .push(cost_breakdown_columns)
        .push(", source, input_tokens, output_tokens, cache_input_tokens, reasoning_tokens, ")
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
        .push("NULL AS upstream_account_name, NULL AS upstream_account_plan_type, ")
        .push(crate::api::INVOCATION_RESPONSE_CONTENT_ENCODING_SQL)
        .push(" AS response_content_encoding, ")
        .push(crate::api::INVOCATION_REQUEST_COMPRESSION_ALGORITHM_SQL)
        .push(" AS request_compression_algorithm, ")
        .push(crate::api::INVOCATION_TRANSPORT_SQL)
        .push(" AS transport, ")
        .push(crate::api::INVOCATION_COMPACTION_REQUEST_KIND_SQL)
        .push(" AS compaction_request_kind, ")
        .push(crate::api::INVOCATION_COMPACTION_RESPONSE_KIND_SQL)
        .push(" AS compaction_response_kind, ")
        .push(crate::api::INVOCATION_IMAGE_INTENT_SQL)
        .push(
            " AS image_intent, \
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
        .push(" AS billing_service_tier, t_req_read_ms, t_req_parse_ms, t_upstream_connect_ms, t_upstream_ttfb_ms, ")
        .push(first_token_ms)
        .push(", t_upstream_stream_ms, t_resp_parse_ms, t_persist_ms, t_total_ms, ")
        .push(crate::api::INVOCATION_DOWNSTREAM_STATUS_CODE_SQL)
        .push(" AS downstream_status_code, ")
        .push(crate::api::INVOCATION_DOWNSTREAM_ERROR_MESSAGE_SQL)
        .push(" AS downstream_error_message, ")
        .push(crate::api::INVOCATION_ENDPOINT_SQL)
        .push(
            " AS endpoint \
             FROM codex_invocations \
             WHERE occurred_at >= ",
        )
        .push_bind(db_occurred_at_lower_bound(range.start))
        .push(" AND occurred_at < ")
        .push_bind(db_occurred_at_upper_bound(range.end));
    if source_scope == InvocationSourceScope::ProxyOnly {
        query.push(" AND source = ").push_bind(SOURCE_PROXY);
    }
    query.push(" ORDER BY occurred_at DESC, id DESC");

    query
        .build_query_as::<crate::api::UpstreamAccountInvocationPreviewRow>()
        .fetch_all(executor)
        .await
        .map_err(Into::into)
}

pub(crate) async fn query_completed_invocation_archive_preview_rows(
    pool: &Pool<Sqlite>,
    source_scope: InvocationSourceScope,
    range: ExactUtcRange,
    exclude_invocation_ids: Option<&HashSet<i64>>,
) -> Result<Vec<crate::api::UpstreamAccountInvocationPreviewRow>> {
    let archive_rows =
        load_completed_invocation_archive_paths_in_range(pool, Some((range.start, range.end)))
            .await?;
    let archived_pool_request_compressions =
        load_archived_pool_request_compressions(pool, range).await?;
    let mut previews = Vec::new();

    for archive_row in archive_rows {
        let Some((archive_pool, temp_cleanup)) =
            open_invocation_archive_batch_pool(&archive_row, "upstream-account-activity").await?
        else {
            continue;
        };
        let has_cost_breakdown_columns =
            sqlite_table_has_column(&archive_pool, "codex_invocations", "cost_input").await?;
        let has_first_token_column =
            sqlite_table_has_column(&archive_pool, "codex_invocations", "first_token_ms").await?;
        let mut rows = query_upstream_account_invocation_preview_rows_from_executor(
            &archive_pool,
            range,
            source_scope,
            has_cost_breakdown_columns,
            has_first_token_column,
        )
        .await?;
        for row in &mut rows {
            if let Some(compression) = archived_pool_request_compressions
                .get(&(row.invoke_id.clone(), row.occurred_at.clone()))
            {
                row.request_compression_algorithm = compression.algorithm.clone();
            }
        }
        let rows = rows
            .into_iter()
            .filter(|row| {
                !exclude_invocation_ids.is_some_and(|excluded_ids| excluded_ids.contains(&row.id))
            })
            .collect::<Vec<_>>();
        previews.extend(rows);
        archive_pool.close().await;
        drop(temp_cleanup);
    }

    Ok(previews)
}

#[derive(Debug, Clone, FromRow)]
pub(crate) struct MaterializedBucketRow {
    bucket_start_epoch: i64,
    source: String,
}

#[derive(Debug, Clone, FromRow)]
pub(crate) struct ReplayedInvocationArchiveRow {
    pub(super) file_path: String,
    pub(super) month_key: String,
    pub(super) coverage_start_at: Option<String>,
    pub(super) coverage_end_at: Option<String>,
}

pub(crate) async fn load_materialized_rollup_bucket_sources(
    pool: &Pool<Sqlite>,
    target: &str,
    bucket_sources: &HashSet<(i64, String)>,
) -> Result<HashSet<(i64, String)>> {
    if bucket_sources.is_empty() {
        return Ok(HashSet::new());
    }

    let min_bucket_epoch = bucket_sources
        .iter()
        .map(|(bucket_start_epoch, _)| *bucket_start_epoch)
        .min()
        .ok_or_else(|| anyhow!("missing minimum materialized bucket epoch"))?;
    let max_bucket_epoch = bucket_sources
        .iter()
        .map(|(bucket_start_epoch, _)| *bucket_start_epoch)
        .max()
        .ok_or_else(|| anyhow!("missing maximum materialized bucket epoch"))?;

    let rows = sqlx::query_as::<_, MaterializedBucketRow>(
        r#"
        SELECT bucket_start_epoch, source
        FROM hourly_rollup_materialized_buckets
        WHERE target = ?1
          AND bucket_start_epoch >= ?2
          AND bucket_start_epoch <= ?3
        "#,
    )
    .bind(target)
    .bind(min_bucket_epoch)
    .bind(max_bucket_epoch)
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|row| (row.bucket_start_epoch, row.source))
        .filter(|key| bucket_sources.contains(key))
        .collect())
}

pub(crate) fn shanghai_month_keys_for_bucket_starts(
    bucket_start_epochs: impl IntoIterator<Item = i64>,
) -> HashSet<String> {
    bucket_start_epochs
        .into_iter()
        .filter_map(|bucket_start_epoch| {
            Utc.timestamp_opt(bucket_start_epoch, 0)
                .single()
                .map(|dt| dt.with_timezone(&Shanghai).format("%Y-%m").to_string())
        })
        .collect()
}

pub(crate) fn shanghai_month_key_for_bucket_start(bucket_start_epoch: i64) -> Option<String> {
    Utc.timestamp_opt(bucket_start_epoch, 0)
        .single()
        .map(|dt| dt.with_timezone(&Shanghai).format("%Y-%m").to_string())
}

pub(crate) fn shanghai_month_bucket_start_epochs(month_key: &str) -> Result<HashSet<i64>> {
    let month_start = NaiveDate::parse_from_str(&format!("{month_key}-01"), "%Y-%m-%d")
        .with_context(|| format!("invalid archive month key: {month_key}"))?;
    let next_month_start = if month_start.month() == 12 {
        NaiveDate::from_ymd_opt(month_start.year() + 1, 1, 1)
    } else {
        NaiveDate::from_ymd_opt(month_start.year(), month_start.month() + 1, 1)
    }
    .ok_or_else(|| anyhow!("failed to resolve next month start for archive month {month_key}"))?;
    let month_start_local = month_start.and_hms_opt(0, 0, 0).ok_or_else(|| {
        anyhow!("failed to resolve local month start for archive month {month_key}")
    })?;
    let next_month_start_local = next_month_start.and_hms_opt(0, 0, 0).ok_or_else(|| {
        anyhow!("failed to resolve next local month start for archive month {month_key}")
    })?;
    let start_epoch = Shanghai
        .from_local_datetime(&month_start_local)
        .single()
        .ok_or_else(|| anyhow!("failed to localize archive month start for {month_key}"))?
        .with_timezone(&Utc)
        .timestamp();
    let end_epoch_exclusive = Shanghai
        .from_local_datetime(&next_month_start_local)
        .single()
        .ok_or_else(|| anyhow!("failed to localize next archive month start for {month_key}"))?
        .with_timezone(&Utc)
        .timestamp();

    let mut bucket_start_epochs = HashSet::new();
    let mut current_epoch = align_bucket_epoch(start_epoch, 3_600, 0);
    while current_epoch < end_epoch_exclusive {
        bucket_start_epochs.insert(current_epoch);
        current_epoch += 3_600;
    }
    Ok(bucket_start_epochs)
}

pub(crate) fn archive_bucket_start_epochs_from_bounds(
    month_key: Option<&str>,
    coverage_start_at: Option<&str>,
    coverage_end_at: Option<&str>,
) -> Result<HashSet<i64>> {
    if let (Some(coverage_start_at), Some(coverage_end_at)) = (coverage_start_at, coverage_end_at) {
        let coverage_start_epoch = summary_rollup_bucket_start_epoch(coverage_start_at)?;
        let coverage_end_epoch = summary_rollup_bucket_start_epoch(coverage_end_at)?;
        if coverage_end_epoch < coverage_start_epoch {
            return Ok(HashSet::new());
        }

        let mut bucket_start_epochs = HashSet::new();
        let mut current_epoch = coverage_start_epoch;
        while current_epoch <= coverage_end_epoch {
            bucket_start_epochs.insert(current_epoch);
            current_epoch += 3_600;
        }
        return Ok(bucket_start_epochs);
    }

    month_key
        .map(shanghai_month_bucket_start_epochs)
        .transpose()
        .map(|maybe_buckets| maybe_buckets.unwrap_or_default())
}

pub(crate) fn archive_bucket_start_epochs_for_row(
    archive_row: &ArchiveBatchPathRow,
) -> Result<HashSet<i64>> {
    archive_bucket_start_epochs_from_bounds(
        archive_row.month_key.as_deref(),
        archive_row.coverage_start_at.as_deref(),
        archive_row.coverage_end_at.as_deref(),
    )
}

pub(crate) fn replayed_archive_bucket_start_epochs(
    archive_row: &ReplayedInvocationArchiveRow,
) -> Result<HashSet<i64>> {
    archive_bucket_start_epochs_from_bounds(
        Some(archive_row.month_key.as_str()),
        archive_row.coverage_start_at.as_deref(),
        archive_row.coverage_end_at.as_deref(),
    )
}

pub(crate) async fn load_replayed_invocation_archives_for_month_keys(
    executor: impl sqlx::Executor<'_, Database = Sqlite>,
    target: &str,
    month_keys: &HashSet<String>,
) -> Result<Vec<ReplayedInvocationArchiveRow>> {
    if month_keys.is_empty() {
        return Ok(Vec::new());
    }

    let mut month_keys = month_keys.iter().cloned().collect::<Vec<_>>();
    month_keys.sort();

    let mut query = QueryBuilder::<Sqlite>::new(
        r#"
        SELECT batches.file_path, batches.month_key
             , batches.coverage_start_at, batches.coverage_end_at
        FROM archive_batches AS batches
        WHERE batches.dataset = 'codex_invocations'
          AND batches.status =
        "#,
    );
    query.push_bind(ARCHIVE_STATUS_COMPLETED);
    query.push(
        r#"
         AND COALESCE(batches.summary_source_kind, 'unknown') <> 'live_mirror'
         AND EXISTS(
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
         )
         AND batches.month_key IN (
        "#,
    );
    {
        let mut separated = query.separated(", ");
        for month_key in month_keys {
            separated.push_bind(month_key);
        }
    }
    query.push(") ORDER BY batches.month_key ASC, batches.created_at ASC, batches.id ASC");

    query
        .build_query_as::<ReplayedInvocationArchiveRow>()
        .fetch_all(executor)
        .await
        .map_err(Into::into)
}

pub(crate) fn materialized_archive_path_row(
    file_path: String,
    coverage_start_at: Option<String>,
    coverage_end_at: Option<String>,
) -> ArchiveBatchPathRow {
    ArchiveBatchPathRow {
        file_path,
        month_key: None,
        coverage_start_at,
        coverage_end_at,
        historical_rollups_materialized_at: Some("materialized".to_string()),
        needs_overall: None,
        needs_failures: None,
    }
}
