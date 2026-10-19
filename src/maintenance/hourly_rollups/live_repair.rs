use super::*;

const LIVE_ROLLUP_LOCK_RETRY_MAX_ATTEMPTS: u32 = 3;
const LIVE_ROLLUP_LOCK_RETRY_DELAY: Duration = Duration::from_millis(50);
const HISTORICAL_ROLLUP_ARCHIVE_SELECTION_BATCH_SIZE: i64 = 64;
const USAGE_BREAKDOWN_REPAIR_SCOPE: &str = "invocation_archive_usage_breakdown";
const USAGE_BREAKDOWN_REPAIR_WORK_BATCH_SIZE: usize = 64;
pub(super) const USAGE_BREAKDOWN_REPAIR_MAX_BOOTSTRAP_PAGES: usize = 4;
// Keep each overlap-closure query bounded; the exclusion set lets large closures converge over
// multiple queries instead of turning this page limit into a correctness cutoff.
const USAGE_BREAKDOWN_REPAIR_MAX_ARCHIVES: usize = 64;
const USAGE_BREAKDOWN_REPAIR_MAX_ELAPSED: Duration = Duration::from_secs(6);
const USAGE_BREAKDOWN_REPAIR_CURSOR_START_RANK: i64 = -1;
// Invocation archives are monthly at most; keep one repair transaction bounded while covering
// every hour in the longest calendar month.
pub(super) const USAGE_BREAKDOWN_REPAIR_MAX_BUCKETS: usize = 31 * 24;
pub(super) const LEGACY_PRUNED_PAYLOAD_MODE_STRUCTURED_ROLLUP_UNKNOWN_REASONING: &str =
    "structured_rollup_unknown_reasoning";
pub(super) const LEGACY_PRUNED_PAYLOAD_MODE_BLOCKED_PAYLOAD_REQUIRED: &str =
    "blocked_payload_required";
const LEGACY_MATERIALIZED_UPSTREAM_ACCOUNT_ARCHIVE_REPLAY_TARGETS: [&str; 3] = [
    HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE,
    HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_STATS_HOURLY,
    HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_STATS_MINUTE,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HourlyRollupRefreshScope {
    Full,
    SkipActiveAccountActivityV2CoverageRepair,
}

pub(crate) async fn sync_hourly_rollups_from_live_tables(pool: &Pool<Sqlite>) -> Result<()> {
    sync_hourly_rollups_from_live_tables_with_scope(pool, None, HourlyRollupRefreshScope::Full)
        .await
}

pub(crate) async fn sync_hourly_rollups_from_live_tables_with_parallel_work_coverage(
    pool: &Pool<Sqlite>,
    invocation_live_days: Option<u64>,
) -> Result<()> {
    sync_hourly_rollups_from_live_tables_with_scope(
        pool,
        invocation_live_days,
        HourlyRollupRefreshScope::Full,
    )
    .await
}

pub(super) async fn sync_hourly_rollups_from_live_tables_with_scope(
    pool: &Pool<Sqlite>,
    invocation_live_days: Option<u64>,
    scope: HourlyRollupRefreshScope,
) -> Result<()> {
    sync_hourly_rollups_from_live_tables_with_scope_and_work(pool, invocation_live_days, scope)
        .await
        .map(|_| ())
}

pub(crate) async fn sync_hourly_rollups_from_live_tables_with_scope_and_work(
    pool: &Pool<Sqlite>,
    invocation_live_days: Option<u64>,
    scope: HourlyRollupRefreshScope,
) -> Result<u64> {
    let mut attempt = 1_u32;
    loop {
        match sync_hourly_rollups_from_live_tables_once(pool, invocation_live_days, scope).await {
            Ok(work_count) => return Ok(work_count),
            Err(err)
                if attempt < LIVE_ROLLUP_LOCK_RETRY_MAX_ATTEMPTS
                    && crate::is_sqlite_lock_error(&err) =>
            {
                warn!(
                    attempt,
                    max_attempts = LIVE_ROLLUP_LOCK_RETRY_MAX_ATTEMPTS,
                    retry_delay_ms = LIVE_ROLLUP_LOCK_RETRY_DELAY.as_millis() as u64,
                    error = %err,
                    "live hourly rollup sync hit sqlite lock; retrying"
                );
                attempt += 1;
                tokio::time::sleep(LIVE_ROLLUP_LOCK_RETRY_DELAY).await;
            }
            Err(err) => {
                return Err(err).with_context(|| {
                    format!(
                        "live hourly rollup sync failed after {attempt}/{} attempt(s)",
                        LIVE_ROLLUP_LOCK_RETRY_MAX_ATTEMPTS
                    )
                });
            }
        }
    }
}

async fn sync_hourly_rollups_from_live_tables_once(
    pool: &Pool<Sqlite>,
    invocation_live_days: Option<u64>,
    scope: HourlyRollupRefreshScope,
) -> Result<u64> {
    let mut work_count = 0_u64;
    if scope == HourlyRollupRefreshScope::Full {
        repair_active_account_activity_v2_coverage(pool).await?;
    }
    loop {
        let updated = replay_live_invocation_hourly_rollups(pool).await?;
        work_count = work_count.saturating_add(updated);
        if updated > 0 {
            crate::record_managed_task_discovered_work(
                &["startup_hourly_rollup_bootstrap"],
                i64::try_from(updated).unwrap_or(i64::MAX),
            );
            crate::record_managed_task_processed_work(
                &["startup_hourly_rollup_bootstrap"],
                i64::try_from(updated).unwrap_or(i64::MAX),
            );
        }
        if updated == 0 {
            break;
        }
    }
    loop {
        let updated = replay_live_forward_proxy_attempt_hourly_rollups(pool).await?;
        work_count = work_count.saturating_add(updated);
        if updated > 0 {
            crate::record_managed_task_discovered_work(
                &["startup_hourly_rollup_bootstrap"],
                i64::try_from(updated).unwrap_or(i64::MAX),
            );
            crate::record_managed_task_processed_work(
                &["startup_hourly_rollup_bootstrap"],
                i64::try_from(updated).unwrap_or(i64::MAX),
            );
        }
        if updated == 0 {
            break;
        }
    }
    loop {
        let updated =
            replay_live_upstream_host_network_minute_rollups_from_invocations(pool).await?;
        work_count = work_count.saturating_add(updated);
        if updated > 0 {
            crate::record_managed_task_discovered_work(
                &["startup_hourly_rollup_bootstrap"],
                i64::try_from(updated).unwrap_or(i64::MAX),
            );
            crate::record_managed_task_processed_work(
                &["startup_hourly_rollup_bootstrap"],
                i64::try_from(updated).unwrap_or(i64::MAX),
            );
        }
        if updated == 0 {
            break;
        }
    }
    loop {
        let updated =
            replay_live_upstream_host_network_minute_rollups_from_pool_attempts(pool).await?;
        work_count = work_count.saturating_add(updated);
        if updated > 0 {
            crate::record_managed_task_discovered_work(
                &["startup_hourly_rollup_bootstrap"],
                i64::try_from(updated).unwrap_or(i64::MAX),
            );
            crate::record_managed_task_processed_work(
                &["startup_hourly_rollup_bootstrap"],
                i64::try_from(updated).unwrap_or(i64::MAX),
            );
        }
        if updated == 0 {
            break;
        }
    }
    let repaired_activity_v2_rows = repair_live_invocation_account_activity_v2_once(pool).await?;
    work_count = work_count.saturating_add(repaired_activity_v2_rows);
    if repaired_activity_v2_rows > 0 {
        crate::record_managed_task_discovered_work(
            &["startup_hourly_rollup_bootstrap"],
            i64::try_from(repaired_activity_v2_rows).unwrap_or(i64::MAX),
        );
        crate::record_managed_task_processed_work(
            &["startup_hourly_rollup_bootstrap"],
            i64::try_from(repaired_activity_v2_rows).unwrap_or(i64::MAX),
        );
    }
    if work_count == 0 {
        crate::record_managed_task_discovered_work(&["startup_hourly_rollup_bootstrap"], 0);
        crate::record_managed_task_processed_work(&["startup_hourly_rollup_bootstrap"], 0);
    }
    wake_account_activity_v2_coverage_repair(pool, repaired_activity_v2_rows).await?;
    if let Some(days) = invocation_live_days {
        maintain_parallel_work_rollups(pool, Some(shanghai_retention_cutoff(days).timestamp()))
            .await?;
    }
    Ok(work_count)
}

pub(crate) async fn wake_account_activity_v2_coverage_repair(
    pool: &Pool<Sqlite>,
    repaired_activity_v2_rows: u64,
) -> Result<()> {
    if repaired_activity_v2_rows > 0 {
        wake_startup_backfill_coverage_repair(pool, "live_account_activity_v2_coverage_updated")
            .await?;
    }
    Ok(())
}

pub(crate) async fn mark_materialized_upstream_account_archive_replayed_tx(
    tx: &mut SqliteConnection,
    file_path: &str,
) -> Result<bool> {
    let manifest_sha = sqlx::query_scalar::<_, Option<String>>(
        "SELECT sha256 FROM archive_batches \
         WHERE dataset = ?1 AND file_path = ?2 \
           AND status IN ('completed', 'materializing') \
         LIMIT 1",
    )
    .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
    .bind(file_path)
    .fetch_optional(&mut *tx)
    .await?
    .flatten()
    .filter(|sha| !sha.trim().is_empty());
    let Some(manifest_sha) = manifest_sha else {
        bail!("archive manifest is missing a SHA before replay marker repair");
    };
    let mut missing_targets = Vec::new();
    let mut has_stale_marker = false;
    for target in LEGACY_MATERIALIZED_UPSTREAM_ACCOUNT_ARCHIVE_REPLAY_TARGETS {
        let marker_sha = sqlx::query_scalar::<_, Option<String>>(
            "SELECT archive_sha256 FROM hourly_rollup_archive_replay \
             WHERE target = ?1 AND dataset = ?2 AND file_path = ?3 LIMIT 1",
        )
        .bind(target)
        .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
        .bind(file_path)
        .fetch_optional(&mut *tx)
        .await?
        .flatten()
        .filter(|sha| !sha.trim().is_empty());
        match marker_sha.as_deref() {
            Some(sha) if sha == manifest_sha => {}
            Some(_) => has_stale_marker = true,
            None => missing_targets.push(target),
        }
    }
    for target in missing_targets {
        mark_hourly_rollup_archive_replayed_tx(
            tx,
            target,
            HOURLY_ROLLUP_DATASET_INVOCATIONS,
            file_path,
        )
        .await?;
    }
    Ok(!has_stale_marker)
}

pub(super) fn can_shortcut_legacy_materialized_upstream_account_targets(
    pending_targets: &[&str],
) -> bool {
    !pending_targets.is_empty()
        && pending_targets.iter().all(|target| {
            LEGACY_MATERIALIZED_UPSTREAM_ACCOUNT_ARCHIVE_REPLAY_TARGETS.contains(target)
        })
}

pub(crate) async fn load_materialized_invocation_archives_missing_upstream_account_markers_tx(
    tx: &mut SqliteConnection,
) -> Result<Vec<String>> {
    Ok(
        load_materialized_invocation_archives_missing_upstream_account_markers_after_id_tx(tx, 0)
            .await?
            .into_iter()
            .map(|row| row.file_path)
            .collect(),
    )
}

#[derive(Debug, sqlx::FromRow)]
struct MaterializedInvocationArchiveMarkerRow {
    id: i64,
    file_path: String,
}

async fn load_materialized_invocation_archives_missing_upstream_account_markers_after_id_tx(
    tx: &mut SqliteConnection,
    after_id: i64,
) -> Result<Vec<MaterializedInvocationArchiveMarkerRow>> {
    sqlx::query_as(
        r#"
        SELECT batches.id, batches.file_path
        FROM archive_batches AS batches
        WHERE batches.dataset = 'codex_invocations'
          AND batches.status = ?1
          AND COALESCE(batches.summary_source_kind, 'unknown') <> 'live_mirror'
          AND batches.historical_rollups_materialized_at IS NOT NULL
          AND batches.sha256 IS NOT NULL
          AND TRIM(batches.sha256) <> ''
          AND (
                NOT EXISTS (
                    SELECT 1
                    FROM hourly_rollup_archive_replay AS replay
                    WHERE replay.target = ?2
                      AND replay.dataset = batches.dataset
                      AND replay.file_path = batches.file_path
                )
                OR NOT EXISTS (
                    SELECT 1
                    FROM hourly_rollup_archive_replay AS replay
                    WHERE replay.target = ?3
                      AND replay.dataset = batches.dataset
                      AND replay.file_path = batches.file_path
                )
                OR NOT EXISTS (
                    SELECT 1
                    FROM hourly_rollup_archive_replay AS replay
                    WHERE replay.target = ?4
                      AND replay.dataset = batches.dataset
                      AND replay.file_path = batches.file_path
                )
          )
          AND batches.id > ?5
        ORDER BY batches.id ASC
        LIMIT ?6
        "#,
    )
    .bind(ARCHIVE_STATUS_COMPLETED)
    .bind(HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE)
    .bind(HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_STATS_HOURLY)
    .bind(HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_STATS_MINUTE)
    .bind(after_id)
    .bind(HISTORICAL_ROLLUP_ARCHIVE_SELECTION_BATCH_SIZE)
    .fetch_all(&mut *tx)
    .await
    .map_err(Into::into)
}

pub(crate) async fn repair_materialized_upstream_account_archive_markers(
    pool: &Pool<Sqlite>,
) -> Result<usize> {
    let mut after_id = 0_i64;
    let mut repaired = 0_usize;
    loop {
        let mut tx = pool.begin().await?;
        let rows =
            load_materialized_invocation_archives_missing_upstream_account_markers_after_id_tx(
                tx.as_mut(),
                after_id,
            )
            .await?;
        let Some(last_id) = rows.last().map(|row| row.id) else {
            break;
        };
        for row in &rows {
            mark_materialized_upstream_account_archive_replayed_tx(tx.as_mut(), &row.file_path)
                .await?;
        }
        tx.commit().await?;
        repaired = repaired.saturating_add(rows.len());
        after_id = last_id;
    }
    Ok(repaired)
}

#[derive(Debug, sqlx::FromRow)]
struct MaterializedInvocationArchiveUsageBreakdownRepairRow {
    id: i64,
    file_path: String,
    sha256: String,
    month_key: String,
    created_at: String,
    stale_rank: i64,
    coverage_start_at: Option<String>,
    coverage_end_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct UsageBreakdownRepairCursor {
    stale_rank: i64,
    month_key: String,
    created_at: String,
    id: i64,
}

impl Default for UsageBreakdownRepairCursor {
    fn default() -> Self {
        Self {
            stale_rank: USAGE_BREAKDOWN_REPAIR_CURSOR_START_RANK,
            month_key: String::new(),
            created_at: String::new(),
            id: 0,
        }
    }
}

async fn load_materialized_invocation_archive_for_usage_breakdown_repair_after_id_tx(
    tx: &mut SqliteConnection,
    cursor: &UsageBreakdownRepairCursor,
) -> Result<Option<MaterializedInvocationArchiveUsageBreakdownRepairRow>> {
    async fn load_ranked_candidate_tx(
        tx: &mut SqliteConnection,
        cursor: &UsageBreakdownRepairCursor,
        stale_rank: i64,
        after_cursor: bool,
    ) -> Result<Option<MaterializedInvocationArchiveUsageBreakdownRepairRow>> {
        let mut query = QueryBuilder::<Sqlite>::new(
            "SELECT batches.id, batches.file_path, batches.sha256, batches.month_key, batches.created_at, ",
        );
        query.push(stale_rank.to_string());
        query.push(
            " AS stale_rank, batches.coverage_start_at, batches.coverage_end_at \
             FROM archive_batches AS batches INDEXED BY idx_archive_batches_usage_breakdown_repair_candidates \
             WHERE batches.dataset = 'codex_invocations' \
               AND batches.status = 'completed' \
               AND batches.historical_rollups_materialized_at IS NOT NULL \
               AND TRIM(batches.sha256) <> '' ",
        );
        query.push(
            " \
               AND COALESCE(batches.summary_source_kind, 'unknown') <> 'live_mirror' \
               AND NOT EXISTS ( \
                    SELECT 1 \
                    FROM hourly_rollup_archive_replay AS unverified \
                    WHERE unverified.dataset = batches.dataset \
                      AND unverified.file_path = batches.file_path \
                      AND ( \
                            unverified.archive_sha256 IS NULL \
                            OR TRIM(unverified.archive_sha256) = '' \
                      ) \
               )",
        );
        query.push(
            " \
               AND NOT EXISTS ( \
                    SELECT 1 \
                    FROM hourly_rollup_archive_repair_deferred AS deferred \
                    WHERE deferred.scope = ",
        );
        query.push_bind(USAGE_BREAKDOWN_REPAIR_SCOPE);
        query.push(" AND deferred.archive_id = batches.id )");
        if stale_rank == 1 {
            query.push(
                " \
               AND EXISTS ( \
                    SELECT 1 \
                    FROM hourly_rollup_archive_replay AS stale \
                    WHERE stale.dataset = batches.dataset \
                      AND stale.file_path = batches.file_path \
                      AND stale.archive_sha256 IS NOT NULL \
                      AND TRIM(stale.archive_sha256) <> '' \
                      AND stale.archive_sha256 <> batches.sha256 \
               )",
            );
        } else {
            query.push(
                " \
               AND NOT EXISTS ( \
                    SELECT 1 \
                    FROM hourly_rollup_archive_replay AS stale \
                    WHERE stale.dataset = batches.dataset \
                      AND stale.file_path = batches.file_path \
                      AND stale.archive_sha256 IS NOT NULL \
                      AND TRIM(stale.archive_sha256) <> '' \
                      AND stale.archive_sha256 <> batches.sha256 \
               ) \
               AND NOT EXISTS ( \
                    SELECT 1 \
                    FROM hourly_rollup_archive_replay AS replay \
                    WHERE replay.target = ",
            );
            query.push_bind(HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE_BREAKDOWN);
            query.push(
                " \
                      AND replay.dataset = batches.dataset \
                      AND replay.file_path = batches.file_path \
                      AND replay.archive_sha256 = batches.sha256 \
               )",
            );
        }
        if after_cursor {
            query.push(
                " \
               AND (batches.month_key, batches.created_at, batches.id) > (",
            );
            query.push_bind(&cursor.month_key);
            query.push(", ");
            query.push_bind(&cursor.created_at);
            query.push(", ");
            query.push_bind(cursor.id);
            query.push(")");
        }
        query.push(
            " ORDER BY batches.month_key ASC, batches.created_at ASC, batches.id ASC LIMIT 1",
        );
        query
            .build_query_as::<MaterializedInvocationArchiveUsageBreakdownRepairRow>()
            .fetch_optional(&mut *tx)
            .await
            .map_err(Into::into)
    }

    match cursor.stale_rank {
        rank if rank <= USAGE_BREAKDOWN_REPAIR_CURSOR_START_RANK => {
            if let Some(row) = load_ranked_candidate_tx(tx, cursor, 0, false).await? {
                return Ok(Some(row));
            }
            load_ranked_candidate_tx(tx, cursor, 1, false).await
        }
        0 => {
            if let Some(row) = load_ranked_candidate_tx(tx, cursor, 0, true).await? {
                return Ok(Some(row));
            }
            load_ranked_candidate_tx(tx, cursor, 1, false).await
        }
        _ => load_ranked_candidate_tx(tx, cursor, 1, true).await,
    }
}

async fn load_usage_breakdown_repair_cursor(
    pool: &Pool<Sqlite>,
) -> Result<UsageBreakdownRepairCursor> {
    Ok(sqlx::query_as::<_, (i64, String, String, i64)>(
        "SELECT cursor_stale_rank, cursor_month_key, cursor_created_at, cursor_id \
         FROM hourly_rollup_archive_repair_progress WHERE scope = ?1",
    )
    .bind(USAGE_BREAKDOWN_REPAIR_SCOPE)
    .fetch_optional(pool)
    .await?
    .map(|(stale_rank, month_key, created_at, id)| {
        if month_key.is_empty() && created_at.is_empty() && id == 0 {
            UsageBreakdownRepairCursor::default()
        } else {
            UsageBreakdownRepairCursor {
                stale_rank,
                month_key,
                created_at,
                id,
            }
        }
    })
    .unwrap_or_default())
}

async fn load_deferred_usage_breakdown_repair_candidate_tx(
    tx: &mut SqliteConnection,
    excluded_archive_ids: &HashSet<i64>,
) -> Result<Option<MaterializedInvocationArchiveUsageBreakdownRepairRow>> {
    let mut query = QueryBuilder::<Sqlite>::new(
        r#"
        SELECT batches.id, batches.file_path, batches.sha256, batches.month_key, batches.created_at,
               deferred.stale_rank, batches.coverage_start_at, batches.coverage_end_at
        FROM hourly_rollup_archive_repair_deferred AS deferred
        INNER JOIN archive_batches AS batches ON batches.id = deferred.archive_id
        WHERE deferred.scope =
        "#,
    );
    query
        .push_bind(USAGE_BREAKDOWN_REPAIR_SCOPE)
        .push(
            " AND batches.dataset = 'codex_invocations' \
             AND batches.status = ",
        )
        .push_bind(ARCHIVE_STATUS_COMPLETED)
        .push(
            " AND COALESCE(batches.summary_source_kind, 'unknown') <> 'live_mirror' \
             AND batches.historical_rollups_materialized_at IS NOT NULL",
        );
    if !excluded_archive_ids.is_empty() {
        query.push(" AND deferred.archive_id NOT IN (");
        let mut ids = query.separated(", ");
        for archive_id in excluded_archive_ids {
            ids.push_bind(archive_id);
        }
        ids.push_unseparated(")");
    }
    query
        .push(" ORDER BY deferred.deferred_at ASC, deferred.archive_id ASC LIMIT 1")
        .build_query_as()
        .fetch_optional(&mut *tx)
        .await
        .map_err(Into::into)
}

async fn delete_obsolete_deferred_usage_breakdown_repair_candidates_tx(
    tx: &mut SqliteConnection,
) -> Result<()> {
    sqlx::query(
        "DELETE FROM hourly_rollup_archive_repair_deferred AS deferred \
         WHERE deferred.scope = ?1 \
           AND NOT EXISTS ( \
                SELECT 1 FROM archive_batches AS batches \
                WHERE batches.id = deferred.archive_id \
                  AND batches.dataset = 'codex_invocations' \
                  AND batches.status = ?2 \
                  AND COALESCE(batches.summary_source_kind, 'unknown') <> 'live_mirror' \
                  AND batches.historical_rollups_materialized_at IS NOT NULL \
           )",
    )
    .bind(USAGE_BREAKDOWN_REPAIR_SCOPE)
    .bind(ARCHIVE_STATUS_COMPLETED)
    .execute(&mut *tx)
    .await?;
    Ok(())
}

async fn delete_deferred_usage_breakdown_repair_candidate_tx(
    tx: &mut SqliteConnection,
    archive_id: i64,
) -> Result<()> {
    sqlx::query(
        "DELETE FROM hourly_rollup_archive_repair_deferred \
         WHERE scope = ?1 AND archive_id = ?2",
    )
    .bind(USAGE_BREAKDOWN_REPAIR_SCOPE)
    .bind(archive_id)
    .execute(&mut *tx)
    .await?;
    Ok(())
}

async fn persist_deferred_usage_breakdown_repair_candidate(
    pool: &Pool<Sqlite>,
    row: &MaterializedInvocationArchiveUsageBreakdownRepairRow,
    cursor: Option<&UsageBreakdownRepairCursor>,
) -> Result<()> {
    let mut tx = pool.begin().await?;
    sqlx::query(
        r#"
        INSERT INTO hourly_rollup_archive_repair_deferred (
            scope, archive_id, stale_rank, month_key, created_at, file_path, deferred_at
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, strftime('%Y-%m-%d %H:%M:%f', 'now'))
        ON CONFLICT(scope, archive_id) DO UPDATE SET
            stale_rank = excluded.stale_rank,
            month_key = excluded.month_key,
            created_at = excluded.created_at,
            file_path = excluded.file_path,
            deferred_at = excluded.deferred_at
        "#,
    )
    .bind(USAGE_BREAKDOWN_REPAIR_SCOPE)
    .bind(row.id)
    .bind(row.stale_rank)
    .bind(&row.month_key)
    .bind(&row.created_at)
    .bind(&row.file_path)
    .execute(&mut *tx)
    .await?;
    if let Some(cursor) = cursor {
        advance_usage_breakdown_repair_cursor_tx(tx.as_mut(), cursor).await?;
    }
    tx.commit().await?;
    Ok(())
}

pub(crate) async fn usage_breakdown_repair_is_pending(pool: &Pool<Sqlite>) -> Result<bool> {
    if load_usage_breakdown_repair_cursor(pool).await? != UsageBreakdownRepairCursor::default() {
        return Ok(true);
    }
    Ok(sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS(SELECT 1 FROM hourly_rollup_archive_repair_deferred WHERE scope = ?1)",
    )
    .bind(USAGE_BREAKDOWN_REPAIR_SCOPE)
    .fetch_one(pool)
    .await?)
}

async fn advance_usage_breakdown_repair_cursor_tx(
    tx: &mut SqliteConnection,
    cursor: &UsageBreakdownRepairCursor,
) -> Result<()> {
    sqlx::query(
        r#"
        INSERT INTO hourly_rollup_archive_repair_progress (
            scope, cursor_id, cursor_stale_rank, cursor_month_key, cursor_created_at, updated_at
        )
        VALUES (?1, ?2, ?3, ?4, ?5, datetime('now'))
        ON CONFLICT(scope) DO UPDATE SET
            cursor_id = excluded.cursor_id,
            cursor_stale_rank = excluded.cursor_stale_rank,
            cursor_month_key = excluded.cursor_month_key,
            cursor_created_at = excluded.cursor_created_at,
            updated_at = excluded.updated_at
        "#,
    )
    .bind(USAGE_BREAKDOWN_REPAIR_SCOPE)
    .bind(cursor.id)
    .bind(cursor.stale_rank)
    .bind(&cursor.month_key)
    .bind(&cursor.created_at)
    .execute(&mut *tx)
    .await?;
    Ok(())
}

pub(super) fn usage_breakdown_repair_bucket_epochs_from_bounds(
    coverage_start_at: &str,
    coverage_end_at: &str,
) -> Result<Option<HashSet<i64>>> {
    let coverage_start_epoch = crate::stats::summary_rollup_bucket_start_epoch(coverage_start_at)?;
    let coverage_end_epoch = crate::stats::summary_rollup_bucket_start_epoch(coverage_end_at)?;
    if coverage_end_epoch < coverage_start_epoch {
        return Ok(Some(HashSet::new()));
    }
    let bucket_count = coverage_end_epoch
        .checked_sub(coverage_start_epoch)
        .and_then(|span| span.checked_div(3_600))
        .and_then(|span| span.checked_add(1))
        .ok_or_else(|| anyhow!("usage breakdown archive coverage span overflowed"))?;
    if bucket_count > USAGE_BREAKDOWN_REPAIR_MAX_BUCKETS as i64 {
        return Ok(None);
    }

    let mut bucket_start_epochs = HashSet::with_capacity(bucket_count as usize);
    let mut current_epoch = coverage_start_epoch;
    while current_epoch <= coverage_end_epoch {
        bucket_start_epochs.insert(current_epoch);
        current_epoch = current_epoch
            .checked_add(3_600)
            .ok_or_else(|| anyhow!("usage breakdown archive bucket epoch overflowed"))?;
    }
    Ok(Some(bucket_start_epochs))
}

pub(super) async fn clear_usage_breakdown_rollup_rows_for_bucket_epochs_tx(
    tx: &mut SqliteConnection,
    bucket_start_epochs: &HashSet<i64>,
) -> Result<()> {
    if bucket_start_epochs.is_empty() {
        return Ok(());
    }

    let mut query = QueryBuilder::<Sqlite>::new(
        "DELETE FROM upstream_account_usage_breakdown_hourly WHERE bucket_start_epoch IN (",
    );
    let mut separated = query.separated(", ");
    for bucket_start_epoch in bucket_start_epochs {
        separated.push_bind(*bucket_start_epoch);
    }
    query.push(")");
    query.build().execute(&mut *tx).await?;
    Ok(())
}

pub(super) async fn clear_invocation_rollup_rows_for_bucket_epochs_tx(
    tx: &mut SqliteConnection,
    bucket_start_epochs: &HashSet<i64>,
) -> Result<()> {
    let mut bucket_start_epochs = bucket_start_epochs.iter().copied().collect::<Vec<_>>();
    bucket_start_epochs.sort_unstable();
    bucket_start_epochs.dedup();

    for table in [
        HOURLY_ROLLUP_TARGET_INVOCATIONS,
        HOURLY_ROLLUP_TARGET_INVOCATION_FAILURES,
        HOURLY_ROLLUP_TARGET_PROXY_PERF,
        HOURLY_ROLLUP_TARGET_PROMPT_CACHE,
        HOURLY_ROLLUP_TARGET_PROMPT_CACHE_UPSTREAM_ACCOUNTS,
        HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE,
        HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE_BREAKDOWN,
        HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_STATS_HOURLY,
        HOURLY_ROLLUP_TARGET_STICKY_KEYS,
    ] {
        delete_hourly_rollup_rows_for_bucket_epochs_tx(tx, table, &bucket_start_epochs).await?;
    }
    delete_rollup_rows_for_bucket_epochs_with_size_tx(
        tx,
        HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_STATS_MINUTE,
        &bucket_start_epochs,
        60,
    )
    .await?;

    let mut query = QueryBuilder::<Sqlite>::new(
        "DELETE FROM hourly_rollup_materialized_buckets WHERE bucket_start_epoch IN (",
    );
    {
        let mut separated = query.separated(", ");
        for bucket_start_epoch in &bucket_start_epochs {
            separated.push_bind(*bucket_start_epoch);
        }
    }
    query.push(")");
    query.build().execute(&mut *tx).await?;

    let retained_live_rows =
        load_live_invocation_hourly_rows_for_bucket_epochs_tx(tx, &bucket_start_epochs).await?;
    upsert_invocation_hourly_rollups_tx(tx, &retained_live_rows, &INVOCATION_HOURLY_ROLLUP_TARGETS)
        .await?;
    rebuild_parallel_work_rollups_for_hours_tx(tx, &bucket_start_epochs).await?;
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub(super) struct InvocationArchiveCoverageRow {
    pub(super) file_path: String,
    pub(super) sha256: Option<String>,
    pub(super) coverage_start_at: String,
    pub(super) coverage_end_at: String,
}

#[derive(sqlx::FromRow)]
struct ForwardProxyArchiveCoverageRow {
    file_path: String,
    sha256: Option<String>,
    coverage_start_at: String,
    coverage_end_at: String,
}

pub(super) async fn load_completed_invocation_archives_overlapping_usage_breakdown_buckets_tx(
    tx: &mut SqliteConnection,
    bucket_start_epochs: &HashSet<i64>,
    excluded_file_paths: &HashSet<String>,
) -> Result<Vec<InvocationArchiveCoverageRow>> {
    if bucket_start_epochs.is_empty() {
        return Ok(Vec::new());
    }

    let min_bucket_start_epoch = bucket_start_epochs
        .iter()
        .copied()
        .min()
        .ok_or_else(|| anyhow!("missing minimum usage breakdown bucket start epoch"))?;
    let max_bucket_start_epoch = bucket_start_epochs
        .iter()
        .copied()
        .max()
        .ok_or_else(|| anyhow!("missing maximum usage breakdown bucket start epoch"))?;
    let overlap_start = chrono::Utc
        .timestamp_opt(min_bucket_start_epoch, 0)
        .single()
        .ok_or_else(|| anyhow!("invalid minimum usage breakdown bucket start epoch"))?;
    let overlap_end = chrono::Utc
        .timestamp_opt(max_bucket_start_epoch + 3_600, 0)
        .single()
        .ok_or_else(|| anyhow!("invalid maximum usage breakdown bucket start epoch"))?;
    let overlap_start =
        crate::stats::format_naive(overlap_start.with_timezone(&Shanghai).naive_local());
    let overlap_end =
        crate::stats::format_naive(overlap_end.with_timezone(&Shanghai).naive_local());

    let mut query = QueryBuilder::<Sqlite>::new(
        "SELECT file_path, sha256, coverage_start_at, coverage_end_at \
         FROM archive_batches WHERE dataset = ",
    );
    query
        .push_bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
        .push(" AND status = ")
        .push_bind(ARCHIVE_STATUS_COMPLETED)
        .push(
            " AND sha256 IS NOT NULL \
               AND TRIM(sha256) <> '' \
               AND COALESCE(summary_source_kind, 'unknown') <> 'live_mirror' \
               AND coverage_start_at IS NOT NULL \
               AND coverage_end_at IS NOT NULL \
               AND coverage_end_at >= ",
        )
        .push_bind(&overlap_start)
        .push(" AND coverage_start_at < ")
        .push_bind(&overlap_end);
    if !excluded_file_paths.is_empty() {
        query.push(" AND file_path NOT IN (");
        {
            let mut separated = query.separated(", ");
            for file_path in excluded_file_paths {
                separated.push_bind(file_path);
            }
        }
        query.push(")");
    }
    query
        .push(" ORDER BY month_key ASC, created_at ASC, id ASC LIMIT ")
        .push_bind(USAGE_BREAKDOWN_REPAIR_MAX_ARCHIVES as i64);
    query
        .build_query_as::<InvocationArchiveCoverageRow>()
        .fetch_all(&mut *tx)
        .await
        .map_err(Into::into)
}

pub(super) async fn load_completed_invocation_archives_overlapping_usage_breakdown_buckets(
    pool: &Pool<Sqlite>,
    bucket_start_epochs: &HashSet<i64>,
    excluded_file_paths: &HashSet<String>,
) -> Result<Vec<InvocationArchiveCoverageRow>> {
    let mut tx = pool.begin().await?;
    let rows = load_completed_invocation_archives_overlapping_usage_breakdown_buckets_tx(
        tx.as_mut(),
        bucket_start_epochs,
        excluded_file_paths,
    )
    .await?;
    tx.commit().await?;
    Ok(rows)
}

fn forward_proxy_archive_bucket_start_epochs_from_bounds(
    coverage_start_at: &str,
    coverage_end_at: &str,
) -> Result<HashSet<i64>> {
    let coverage_start_epoch = align_bucket_epoch(
        parse_utc_naive(coverage_start_at)?.and_utc().timestamp(),
        3_600,
        0,
    );
    let coverage_end_epoch = align_bucket_epoch(
        parse_utc_naive(coverage_end_at)?.and_utc().timestamp(),
        3_600,
        0,
    );
    if coverage_end_epoch < coverage_start_epoch {
        return Ok(HashSet::new());
    }

    let mut bucket_start_epochs = HashSet::new();
    let mut current_epoch = coverage_start_epoch;
    while current_epoch <= coverage_end_epoch {
        bucket_start_epochs.insert(current_epoch);
        current_epoch += 3_600;
    }
    Ok(bucket_start_epochs)
}

async fn load_completed_forward_proxy_archives_overlapping_buckets_tx(
    tx: &mut SqliteConnection,
    bucket_start_epochs: &HashSet<i64>,
    excluded_file_paths: &HashSet<String>,
) -> Result<Vec<ForwardProxyArchiveCoverageRow>> {
    if bucket_start_epochs.is_empty() {
        return Ok(Vec::new());
    }

    let min_bucket_start_epoch = bucket_start_epochs
        .iter()
        .copied()
        .min()
        .ok_or_else(|| anyhow!("missing minimum forward proxy bucket start epoch"))?;
    let max_bucket_start_epoch = bucket_start_epochs
        .iter()
        .copied()
        .max()
        .ok_or_else(|| anyhow!("missing maximum forward proxy bucket start epoch"))?;
    let overlap_start = Utc
        .timestamp_opt(min_bucket_start_epoch, 0)
        .single()
        .ok_or_else(|| anyhow!("invalid minimum forward proxy bucket start epoch"))?
        .naive_utc()
        .format("%Y-%m-%d %H:%M:%S")
        .to_string();
    let overlap_end = Utc
        .timestamp_opt(max_bucket_start_epoch + 3_600, 0)
        .single()
        .ok_or_else(|| anyhow!("invalid maximum forward proxy bucket start epoch"))?
        .naive_utc()
        .format("%Y-%m-%d %H:%M:%S")
        .to_string();

    let mut query = QueryBuilder::<Sqlite>::new(
        "SELECT file_path, sha256, coverage_start_at, coverage_end_at \
         FROM archive_batches WHERE dataset = ",
    );
    query
        .push_bind(HOURLY_ROLLUP_DATASET_FORWARD_PROXY_ATTEMPTS)
        .push(" AND status = ")
        .push_bind(ARCHIVE_STATUS_COMPLETED)
        .push(
            " AND sha256 IS NOT NULL \
               AND TRIM(sha256) <> '' \
               AND coverage_start_at IS NOT NULL \
               AND coverage_end_at IS NOT NULL \
               AND coverage_end_at >= ",
        )
        .push_bind(&overlap_start)
        .push(" AND coverage_start_at < ")
        .push_bind(&overlap_end);
    if !excluded_file_paths.is_empty() {
        query.push(" AND file_path NOT IN (");
        {
            let mut separated = query.separated(", ");
            for file_path in excluded_file_paths {
                separated.push_bind(file_path);
            }
        }
        query.push(")");
    }
    query
        .push(" ORDER BY month_key ASC, created_at ASC, id ASC LIMIT ")
        .push_bind(USAGE_BREAKDOWN_REPAIR_MAX_ARCHIVES as i64);
    query
        .build_query_as::<ForwardProxyArchiveCoverageRow>()
        .fetch_all(&mut *tx)
        .await
        .map_err(Into::into)
}

pub(super) async fn archive_batch_has_completed_manifest_sha_tx(
    tx: &mut SqliteConnection,
    dataset: &str,
    file_path: &str,
) -> Result<bool> {
    Ok(sqlx::query_scalar::<_, i64>(
        r#"
        SELECT 1
        FROM archive_batches
        WHERE dataset = ?1
          AND file_path = ?2
          AND status = 'completed'
          AND sha256 IS NOT NULL
          AND TRIM(sha256) <> ''
        LIMIT 1
        "#,
    )
    .bind(dataset)
    .bind(file_path)
    .fetch_optional(&mut *tx)
    .await?
    .is_some())
}

pub(super) async fn invocation_archive_has_materialized_rollups_tx(
    tx: &mut SqliteConnection,
    file_path: &str,
) -> Result<bool> {
    Ok(sqlx::query_scalar::<_, i64>(
        r#"
        SELECT 1
        FROM archive_batches
        WHERE dataset = ?1
          AND file_path = ?2
          AND status = ?3
          AND historical_rollups_materialized_at IS NOT NULL
        LIMIT 1
        "#,
    )
    .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
    .bind(file_path)
    .bind(ARCHIVE_STATUS_COMPLETED)
    .fetch_optional(&mut *tx)
    .await?
    .is_some())
}

pub(super) async fn reset_invocation_archive_usage_breakdown_backfill_state_tx(
    tx: &mut SqliteConnection,
    file_path: &str,
) -> Result<()> {
    sqlx::query(
        r#"
        DELETE FROM hourly_rollup_archive_replay
        WHERE dataset = ?1
          AND target = ?2
          AND file_path = ?3
        "#,
    )
    .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
    .bind(HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE_BREAKDOWN)
    .bind(file_path)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        r#"
        UPDATE archive_batches
        SET historical_rollups_materialized_at = NULL
        WHERE dataset = ?1
          AND file_path = ?2
        "#,
    )
    .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
    .bind(file_path)
    .execute(&mut *tx)
    .await?;
    delete_hourly_rollup_archive_progress_tx(tx, HOURLY_ROLLUP_DATASET_INVOCATIONS, file_path)
        .await?;
    delete_hourly_rollup_archive_progress_tx(
        tx,
        INVOCATION_USAGE_BREAKDOWN_ARCHIVE_PROGRESS_DATASET,
        file_path,
    )
    .await?;
    Ok(())
}

pub(super) async fn invocation_archive_has_stale_replay_marker_tx(
    tx: &mut SqliteConnection,
    file_path: &str,
) -> Result<bool> {
    Ok(sqlx::query_scalar::<_, i64>(
        r#"
        SELECT 1
        FROM hourly_rollup_archive_replay AS replay
        INNER JOIN archive_batches AS batches
            ON batches.dataset = replay.dataset
           AND batches.file_path = replay.file_path
           AND batches.status = 'completed'
        WHERE replay.dataset = ?1
          AND replay.file_path = ?2
          AND (
              replay.archive_sha256 IS NULL
              OR TRIM(replay.archive_sha256) = ''
              OR replay.archive_sha256 <> batches.sha256
          )
          AND batches.sha256 IS NOT NULL
          AND TRIM(batches.sha256) <> ''
        LIMIT 1
        "#,
    )
    .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
    .bind(file_path)
    .fetch_optional(&mut *tx)
    .await?
    .is_some())
}

pub(super) async fn invocation_archive_has_unverified_replay_marker_tx(
    tx: &mut SqliteConnection,
    file_path: &str,
) -> Result<bool> {
    Ok(sqlx::query_scalar::<_, i64>(
        r#"
        SELECT 1
        FROM hourly_rollup_archive_replay
        WHERE dataset = ?1
          AND file_path = ?2
          AND (
                archive_sha256 IS NULL
                OR TRIM(archive_sha256) = ''
          )
        LIMIT 1
        "#,
    )
    .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
    .bind(file_path)
    .fetch_optional(&mut *tx)
    .await?
    .is_some())
}

pub(super) async fn forward_proxy_archive_has_stale_replay_marker_tx(
    tx: &mut SqliteConnection,
    file_path: &str,
) -> Result<bool> {
    Ok(sqlx::query_scalar::<_, i64>(
        r#"
        SELECT 1
        FROM hourly_rollup_archive_replay AS replay
        INNER JOIN archive_batches AS batches
            ON batches.dataset = replay.dataset
           AND batches.file_path = replay.file_path
           AND batches.status = 'completed'
        WHERE replay.target = ?1
          AND replay.dataset = ?2
          AND replay.file_path = ?3
          AND batches.sha256 IS NOT NULL
          AND TRIM(batches.sha256) <> ''
          AND (
                replay.archive_sha256 IS NULL
                OR TRIM(replay.archive_sha256) = ''
                OR replay.archive_sha256 <> batches.sha256
          )
        LIMIT 1
        "#,
    )
    .bind(HOURLY_ROLLUP_TARGET_FORWARD_PROXY_ATTEMPTS)
    .bind(HOURLY_ROLLUP_DATASET_FORWARD_PROXY_ATTEMPTS)
    .bind(file_path)
    .fetch_optional(&mut *tx)
    .await?
    .is_some())
}

pub(super) async fn pool_upstream_node_health_archive_has_stale_replay_marker_tx(
    tx: &mut SqliteConnection,
    file_path: &str,
) -> Result<bool> {
    Ok(sqlx::query_scalar::<_, i64>(
        r#"
        SELECT 1
        FROM hourly_rollup_archive_replay AS replay
        INNER JOIN archive_batches AS batches
            ON batches.dataset = replay.dataset
           AND batches.file_path = replay.file_path
           AND batches.status = 'completed'
        WHERE replay.target = ?1
          AND replay.dataset = 'pool_upstream_request_attempts'
          AND replay.file_path = ?2
          AND replay.archive_sha256 IS NOT NULL
          AND batches.sha256 IS NOT NULL
          AND TRIM(batches.sha256) <> ''
          AND replay.archive_sha256 <> batches.sha256
        LIMIT 1
        "#,
    )
    .bind(POOL_UPSTREAM_NODE_HEALTH_ARCHIVE_REPLAY_TARGET)
    .bind(file_path)
    .fetch_optional(&mut *tx)
    .await?
    .is_some())
}

pub(super) async fn reset_replaced_pool_upstream_node_health_archive_state_tx(
    tx: &mut SqliteConnection,
    archive_batch_id: i64,
    file_path: &str,
) -> Result<()> {
    delete_pool_upstream_node_health_archive_rows_for_file_tx(tx, file_path).await?;
    delete_pool_upstream_node_health_hourly_archive_rows_for_batch_tx(tx, archive_batch_id).await?;
    delete_hourly_rollup_archive_progress_tx(tx, "pool_upstream_request_attempts", file_path)
        .await?;
    sqlx::query(
        r#"
        DELETE FROM hourly_rollup_archive_replay
        WHERE dataset = 'pool_upstream_request_attempts'
          AND file_path = ?1
          AND target IN (?2, ?3)
        "#,
    )
    .bind(file_path)
    .bind(POOL_UPSTREAM_NODE_HEALTH_ARCHIVE_REPLAY_TARGET)
    .bind(POOL_UPSTREAM_NODE_HEALTH_HOURLY_ARCHIVE_REPLAY_TARGET)
    .execute(&mut *tx)
    .await?;
    Ok(())
}

pub(super) async fn reset_invocation_archive_replay_state_tx(
    tx: &mut SqliteConnection,
    file_paths: &[String],
) -> Result<()> {
    for file_path in file_paths {
        sqlx::query(
            r#"
            DELETE FROM hourly_rollup_archive_replay
            WHERE dataset = ?1 AND file_path = ?2
            "#,
        )
        .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
        .bind(file_path)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            r#"
            DELETE FROM hourly_rollup_archive_progress
            WHERE file_path = ?1
            "#,
        )
        .bind(file_path)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            r#"
            UPDATE archive_batches
            SET historical_rollups_materialized_at = NULL
            WHERE dataset = ?1 AND file_path = ?2
            "#,
        )
        .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
        .bind(file_path)
        .execute(&mut *tx)
        .await?;
    }
    Ok(())
}

async fn clear_forward_proxy_rollup_rows_for_bucket_epochs_tx(
    tx: &mut SqliteConnection,
    bucket_start_epochs: &HashSet<i64>,
) -> Result<()> {
    let mut bucket_start_epochs = bucket_start_epochs.iter().copied().collect::<Vec<_>>();
    bucket_start_epochs.sort_unstable();
    bucket_start_epochs.dedup();
    if bucket_start_epochs.is_empty() {
        return Ok(());
    }

    delete_hourly_rollup_rows_for_bucket_epochs_tx(
        tx,
        HOURLY_ROLLUP_TARGET_FORWARD_PROXY_ATTEMPTS,
        &bucket_start_epochs,
    )
    .await?;

    let mut query = QueryBuilder::<Sqlite>::new(
        "DELETE FROM hourly_rollup_materialized_buckets WHERE target = ",
    );
    query
        .push_bind(HOURLY_ROLLUP_TARGET_FORWARD_PROXY_ATTEMPTS)
        .push(" AND bucket_start_epoch IN (");
    {
        let mut separated = query.separated(", ");
        for bucket_start_epoch in &bucket_start_epochs {
            separated.push_bind(*bucket_start_epoch);
        }
    }
    query.push(")");
    query.build().execute(&mut *tx).await?;

    let min_bucket_start_epoch = *bucket_start_epochs
        .first()
        .ok_or_else(|| anyhow!("missing minimum forward proxy bucket start epoch"))?;
    let max_bucket_start_epoch = *bucket_start_epochs
        .last()
        .ok_or_else(|| anyhow!("missing maximum forward proxy bucket start epoch"))?;
    let min_occurred_at = Utc
        .timestamp_opt(min_bucket_start_epoch, 0)
        .single()
        .ok_or_else(|| anyhow!("invalid minimum forward proxy bucket start epoch"))?
        .naive_utc()
        .format("%Y-%m-%d %H:%M:%S")
        .to_string();
    let max_occurred_at = Utc
        .timestamp_opt(max_bucket_start_epoch + 3_600, 0)
        .single()
        .ok_or_else(|| anyhow!("invalid maximum forward proxy bucket start epoch"))?
        .naive_utc()
        .format("%Y-%m-%d %H:%M:%S")
        .to_string();
    let bucket_start_epoch_set = bucket_start_epochs.iter().copied().collect::<HashSet<_>>();
    let retained_live_rows = sqlx::query_as::<_, ForwardProxyAttemptHourlySourceRecord>(
        r#"
        SELECT id, proxy_key, occurred_at, is_success, latency_ms
        FROM forward_proxy_attempts
        WHERE occurred_at >= ?1
          AND occurred_at < ?2
        ORDER BY id ASC
        "#,
    )
    .bind(&min_occurred_at)
    .bind(&max_occurred_at)
    .fetch_all(&mut *tx)
    .await?
    .into_iter()
    .filter(|row| {
        forward_proxy_attempt_bucket_start_epoch(&row.occurred_at)
            .map(|bucket_start_epoch| bucket_start_epoch_set.contains(&bucket_start_epoch))
            .unwrap_or(false)
    })
    .collect::<Vec<_>>();
    upsert_forward_proxy_attempt_hourly_rollups_tx(tx, &retained_live_rows).await?;
    mark_forward_proxy_hourly_rollup_buckets_materialized_tx(tx, &retained_live_rows).await?;
    Ok(())
}

async fn reset_forward_proxy_archive_replay_state_tx(
    tx: &mut SqliteConnection,
    file_paths: &[String],
) -> Result<()> {
    for file_path in file_paths {
        sqlx::query(
            r#"
            DELETE FROM hourly_rollup_archive_replay
            WHERE target = ?1
              AND dataset = ?2
              AND file_path = ?3
            "#,
        )
        .bind(HOURLY_ROLLUP_TARGET_FORWARD_PROXY_ATTEMPTS)
        .bind(HOURLY_ROLLUP_DATASET_FORWARD_PROXY_ATTEMPTS)
        .bind(file_path)
        .execute(&mut *tx)
        .await?;
        delete_hourly_rollup_archive_progress_tx(
            tx,
            HOURLY_ROLLUP_DATASET_FORWARD_PROXY_ATTEMPTS,
            file_path,
        )
        .await?;
        sqlx::query(
            r#"
            UPDATE archive_batches
            SET historical_rollups_materialized_at = NULL
            WHERE dataset = ?1
              AND file_path = ?2
            "#,
        )
        .bind(HOURLY_ROLLUP_DATASET_FORWARD_PROXY_ATTEMPTS)
        .bind(file_path)
        .execute(&mut *tx)
        .await?;
    }
    Ok(())
}

pub(super) async fn reopen_replaced_materialized_forward_proxy_archive_tx(
    tx: &mut SqliteConnection,
    file_path: &str,
    coverage_start_at: Option<&str>,
    coverage_end_at: Option<&str>,
) -> Result<Option<Vec<String>>> {
    let (Some(coverage_start_at), Some(coverage_end_at)) = (coverage_start_at, coverage_end_at)
    else {
        return Ok(None);
    };
    let mut bucket_start_epochs =
        forward_proxy_archive_bucket_start_epochs_from_bounds(coverage_start_at, coverage_end_at)?;
    let mut reopened_file_paths = vec![file_path.to_string()];
    let mut reopened_file_path_set = HashSet::from([file_path.to_string()]);

    // Rebuild the transitive overlap closure so rows from a retained peer cannot survive the
    // replacement clear and then be added again by a later replay.
    loop {
        let overlapping_archives = load_completed_forward_proxy_archives_overlapping_buckets_tx(
            tx,
            &bucket_start_epochs,
            &reopened_file_path_set,
        )
        .await?;
        let mut expanded = false;
        for overlapping_archive in overlapping_archives {
            if overlapping_archive
                .sha256
                .as_deref()
                .is_none_or(|sha256| sha256.trim().is_empty())
            {
                return Ok(None);
            }
            if !reopened_file_path_set.insert(overlapping_archive.file_path.clone()) {
                continue;
            }
            reopened_file_paths.push(overlapping_archive.file_path);
            bucket_start_epochs.extend(forward_proxy_archive_bucket_start_epochs_from_bounds(
                &overlapping_archive.coverage_start_at,
                &overlapping_archive.coverage_end_at,
            )?);
            expanded = true;
        }
        if !expanded {
            break;
        }
    }

    clear_forward_proxy_rollup_rows_for_bucket_epochs_tx(tx, &bucket_start_epochs).await?;
    reset_forward_proxy_archive_replay_state_tx(tx, &reopened_file_paths).await?;
    Ok(Some(reopened_file_paths))
}

pub(super) struct InvocationArchiveRepairFence {
    pub(super) started_at: Instant,
    pub(super) max_elapsed: Option<Duration>,
    directories: HashSet<String>,
    locks: Vec<RetentionArchiveFileLock>,
}

impl InvocationArchiveRepairFence {
    pub(super) fn new(started_at: Instant, max_elapsed: Option<Duration>) -> Self {
        Self {
            started_at,
            max_elapsed,
            directories: HashSet::new(),
            locks: Vec::new(),
        }
    }

    fn directory_key(path: &Path) -> String {
        retention_archive_parent_identity(path)
            .unwrap_or_else(|| path.parent().unwrap_or(path).to_string_lossy().into_owned())
    }

    pub(super) fn register_held_directory(&mut self, path: &Path) {
        self.directories.insert(Self::directory_key(path));
    }

    pub(super) fn try_fence(&mut self, path: &Path) -> Result<bool> {
        let key = Self::directory_key(path);
        if self.directories.contains(&key) {
            return Ok(true);
        }
        // Publishers use a directory flock, so different files in that directory share one
        // fence. Never wait for a publisher while holding the repair's SQLite transaction.
        match retention_archive_file_try_lock(path) {
            Ok(lock) => {
                self.directories.insert(key);
                self.locks.push(lock);
                Ok(true)
            }
            Err(error) if is_retention_write_deferred(&error) => Ok(false),
            Err(error) => Err(error),
        }
    }
}

pub(super) async fn reopen_replaced_materialized_invocation_archive_tx(
    tx: &mut SqliteConnection,
    file_path: &str,
    expected_sha256: &str,
    coverage_start_at: Option<&str>,
    coverage_end_at: Option<&str>,
    fence: &mut InvocationArchiveRepairFence,
) -> Result<InvocationArchiveReopenResult> {
    if !fence.try_fence(Path::new(file_path))? {
        return Ok(InvocationArchiveReopenResult::BudgetExhausted);
    }
    let started_at = fence.started_at;
    let max_elapsed = fence.max_elapsed;
    match invocation_archive_file_is_readable_with_budget(
        Path::new(file_path),
        expected_sha256,
        started_at,
        max_elapsed,
    )
    .await
    {
        InvocationArchiveReadability::Readable => {}
        InvocationArchiveReadability::Rejected => {
            return Ok(InvocationArchiveReopenResult::Rejected);
        }
        InvocationArchiveReadability::BudgetExhausted => {
            return Ok(InvocationArchiveReopenResult::BudgetExhausted);
        }
    }
    let (Some(coverage_start_at), Some(coverage_end_at)) = (coverage_start_at, coverage_end_at)
    else {
        return Ok(InvocationArchiveReopenResult::Rejected);
    };
    let Some(mut bucket_start_epochs) =
        usage_breakdown_repair_bucket_epochs_from_bounds(coverage_start_at, coverage_end_at)?
    else {
        return Ok(InvocationArchiveReopenResult::Rejected);
    };
    let mut reopened_file_paths = Vec::new();
    let mut reopened_file_path_set = HashSet::new();

    // Resetting an overlapping archive requires clearing its entire coverage before it can be
    // replayed. Keep expanding the overlap set until every reopened archive is represented.
    loop {
        let overlapping_archives =
            load_completed_invocation_archives_overlapping_usage_breakdown_buckets_tx(
                tx,
                &bucket_start_epochs,
                &reopened_file_path_set,
            )
            .await?;
        let mut expanded = false;
        for overlapping_archive in overlapping_archives {
            let Some(_) = overlapping_archive
                .sha256
                .as_deref()
                .filter(|sha256| !sha256.trim().is_empty())
            else {
                // An unverifiable overlap cannot be replayed after its rows are cleared.
                // Leave the complete closure quarantined rather than partially rebuilding it.
                return Ok(InvocationArchiveReopenResult::Rejected);
            };
            if !reopened_file_path_set.insert(overlapping_archive.file_path.clone()) {
                continue;
            }
            if !fence.try_fence(Path::new(&overlapping_archive.file_path))? {
                return Ok(InvocationArchiveReopenResult::BudgetExhausted);
            }
            if historical_rollup_elapsed_budget_reached(started_at, max_elapsed) {
                return Ok(InvocationArchiveReopenResult::BudgetExhausted);
            }
            match invocation_archive_file_is_readable_with_budget(
                Path::new(&overlapping_archive.file_path),
                overlapping_archive
                    .sha256
                    .as_deref()
                    .expect("overlap archive SHA is checked before replay"),
                started_at,
                max_elapsed,
            )
            .await
            {
                InvocationArchiveReadability::Readable => {}
                InvocationArchiveReadability::Rejected => {
                    return Ok(InvocationArchiveReopenResult::Rejected);
                }
                InvocationArchiveReadability::BudgetExhausted => {
                    return Ok(InvocationArchiveReopenResult::BudgetExhausted);
                }
            }
            let Some(overlap_bucket_start_epochs) =
                usage_breakdown_repair_bucket_epochs_from_bounds(
                    &overlapping_archive.coverage_start_at,
                    &overlapping_archive.coverage_end_at,
                )?
            else {
                return Ok(InvocationArchiveReopenResult::Rejected);
            };
            reopened_file_paths.push(overlapping_archive.file_path);
            bucket_start_epochs.extend(overlap_bucket_start_epochs);
            if bucket_start_epochs.len() > USAGE_BREAKDOWN_REPAIR_MAX_BUCKETS {
                return Ok(InvocationArchiveReopenResult::Rejected);
            }
            expanded = true;
        }
        if !expanded {
            break;
        }
    }
    if !reopened_file_path_set.contains(file_path) {
        return Ok(InvocationArchiveReopenResult::Rejected);
    }
    clear_invocation_rollup_rows_for_bucket_epochs_tx(tx, &bucket_start_epochs).await?;
    reset_invocation_archive_replay_state_tx(tx, &reopened_file_paths).await?;
    Ok(InvocationArchiveReopenResult::Reopened(reopened_file_paths))
}

pub(super) async fn invocation_archive_is_missing_summary_projection_proof_tx(
    tx: &mut SqliteConnection,
    file_path: &str,
) -> Result<bool> {
    for target in SUMMARY_PROJECTION_ARCHIVE_REPLAY_TARGETS {
        if !hourly_rollup_archive_replayed_tx(
            tx,
            target,
            HOURLY_ROLLUP_DATASET_INVOCATIONS,
            file_path,
        )
        .await?
        {
            return Ok(true);
        }
    }
    Ok(false)
}

async fn reopen_materialized_invocation_archive_usage_breakdown_backfill_tx(
    tx: &mut SqliteConnection,
    file_path: &str,
    expected_sha256: &str,
    coverage_start_at: Option<&str>,
    coverage_end_at: Option<&str>,
    fence: &mut InvocationArchiveRepairFence,
) -> Result<InvocationArchiveReopenResult> {
    if !fence.try_fence(Path::new(file_path))? {
        return Ok(InvocationArchiveReopenResult::BudgetExhausted);
    }
    let started_at = fence.started_at;
    let max_elapsed = fence.max_elapsed;
    match invocation_archive_file_is_readable_with_budget(
        Path::new(file_path),
        expected_sha256,
        started_at,
        max_elapsed,
    )
    .await
    {
        InvocationArchiveReadability::Readable => {}
        InvocationArchiveReadability::Rejected => {
            return Ok(InvocationArchiveReopenResult::Rejected);
        }
        InvocationArchiveReadability::BudgetExhausted => {
            return Ok(InvocationArchiveReopenResult::BudgetExhausted);
        }
    }
    let mut reopened_file_paths = vec![file_path.to_string()];
    if let (Some(coverage_start_at), Some(coverage_end_at)) = (coverage_start_at, coverage_end_at) {
        let Some(mut bucket_start_epochs) =
            usage_breakdown_repair_bucket_epochs_from_bounds(coverage_start_at, coverage_end_at)?
        else {
            return Ok(InvocationArchiveReopenResult::Rejected);
        };
        let mut reopened_file_path_set = HashSet::from([file_path.to_string()]);

        loop {
            let overlapping_archives =
                load_completed_invocation_archives_overlapping_usage_breakdown_buckets_tx(
                    tx,
                    &bucket_start_epochs,
                    &reopened_file_path_set,
                )
                .await?;
            let mut expanded = false;
            for overlapping_archive in overlapping_archives {
                if overlapping_archive
                    .sha256
                    .as_deref()
                    .is_none_or(|sha256| sha256.trim().is_empty())
                {
                    return Ok(InvocationArchiveReopenResult::Rejected);
                }
                if !reopened_file_path_set.insert(overlapping_archive.file_path.clone()) {
                    continue;
                }
                if !fence.try_fence(Path::new(&overlapping_archive.file_path))? {
                    return Ok(InvocationArchiveReopenResult::BudgetExhausted);
                }
                if historical_rollup_elapsed_budget_reached(started_at, max_elapsed) {
                    return Ok(InvocationArchiveReopenResult::BudgetExhausted);
                }
                match invocation_archive_file_is_readable_with_budget(
                    Path::new(&overlapping_archive.file_path),
                    overlapping_archive
                        .sha256
                        .as_deref()
                        .expect("overlap archive SHA is checked before replay"),
                    started_at,
                    max_elapsed,
                )
                .await
                {
                    InvocationArchiveReadability::Readable => {}
                    InvocationArchiveReadability::Rejected => {
                        return Ok(InvocationArchiveReopenResult::Rejected);
                    }
                    InvocationArchiveReadability::BudgetExhausted => {
                        return Ok(InvocationArchiveReopenResult::BudgetExhausted);
                    }
                }
                let Some(overlap_bucket_start_epochs) =
                    usage_breakdown_repair_bucket_epochs_from_bounds(
                        &overlapping_archive.coverage_start_at,
                        &overlapping_archive.coverage_end_at,
                    )?
                else {
                    return Ok(InvocationArchiveReopenResult::Rejected);
                };
                reopened_file_paths.push(overlapping_archive.file_path);
                bucket_start_epochs.extend(overlap_bucket_start_epochs);
                if bucket_start_epochs.len() > USAGE_BREAKDOWN_REPAIR_MAX_BUCKETS {
                    return Ok(InvocationArchiveReopenResult::Rejected);
                }
                expanded = true;
            }
            if !expanded {
                break;
            }
        }
        clear_usage_breakdown_rollup_rows_for_bucket_epochs_tx(tx, &bucket_start_epochs).await?;
        let mut bucket_start_epochs = bucket_start_epochs.into_iter().collect::<Vec<_>>();
        bucket_start_epochs.sort_unstable();
        let retained_live_rows =
            load_live_invocation_hourly_rows_for_bucket_epochs_tx(tx, &bucket_start_epochs).await?;
        upsert_invocation_hourly_rollups_tx(
            tx,
            &retained_live_rows,
            &[HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE_BREAKDOWN],
        )
        .await?;
    }
    for reopened_file_path in &reopened_file_paths {
        reset_invocation_archive_usage_breakdown_backfill_state_tx(tx, reopened_file_path).await?;
    }
    Ok(InvocationArchiveReopenResult::Reopened(reopened_file_paths))
}

pub(crate) async fn repair_materialized_invocation_archive_usage_breakdown_backfill_state(
    pool: &Pool<Sqlite>,
) -> Result<usize> {
    repair_materialized_invocation_archive_usage_breakdown_backfill_state_with_budget(
        pool,
        Some(USAGE_BREAKDOWN_REPAIR_MAX_ELAPSED),
        false,
    )
    .await
}

#[cfg(test)]
pub(crate) async fn repair_materialized_invocation_archive_usage_breakdown_backfill_state_with_elapsed_budget_for_test(
    pool: &Pool<Sqlite>,
    max_elapsed: Duration,
) -> Result<usize> {
    repair_materialized_invocation_archive_usage_breakdown_backfill_state_with_budget(
        pool,
        Some(max_elapsed),
        true,
    )
    .await
}

async fn repair_materialized_invocation_archive_usage_breakdown_backfill_state_with_budget(
    pool: &Pool<Sqlite>,
    max_elapsed: Option<Duration>,
    allow_one_candidate_after_budget: bool,
) -> Result<usize> {
    let mut cursor = load_usage_breakdown_repair_cursor(pool).await?;
    let mut touched_batches = 0usize;
    let started_at = Instant::now();
    let mut budget_overrun_candidate_started = false;
    let mut skipped_deferred_archive_ids = HashSet::new();

    // One candidate per transaction keeps closure expansion, rollup rebuilding, and marker
    // resets below the recovery write budget. The durable cursor lets skipped/quarantined rows
    // yield to later recoverable rows; a separately durable deferred queue keeps budget-exhausted
    // candidates retryable without blocking the rest of the ordered set.
    for _ in 0..USAGE_BREAKDOWN_REPAIR_WORK_BATCH_SIZE {
        if historical_rollup_elapsed_budget_reached(started_at, max_elapsed)
            && (!allow_one_candidate_after_budget || budget_overrun_candidate_started)
        {
            break;
        }
        let (
            row,
            deferred_candidate,
            archive_proof_valid,
            has_stale_replay_marker,
            breakdown_replayed,
        ) = {
            let mut tx = pool.begin().await?;
            let (row, deferred_candidate) = if let Some(row) =
                load_materialized_invocation_archive_for_usage_breakdown_repair_after_id_tx(
                    tx.as_mut(),
                    &cursor,
                )
                .await?
            {
                (row, false)
            } else {
                delete_obsolete_deferred_usage_breakdown_repair_candidates_tx(tx.as_mut()).await?;
                if let Some(row) = load_deferred_usage_breakdown_repair_candidate_tx(
                    tx.as_mut(),
                    &skipped_deferred_archive_ids,
                )
                .await?
                {
                    (row, true)
                } else {
                    if cursor != UsageBreakdownRepairCursor::default() {
                        advance_usage_breakdown_repair_cursor_tx(
                            tx.as_mut(),
                            &UsageBreakdownRepairCursor::default(),
                        )
                        .await?;
                    }
                    // Commit even when the cursor is already at its origin: the cleanup above
                    // may have removed deferred rows that became ineligible for repair.
                    tx.commit().await?;
                    break;
                }
            };
            let archive_proof_valid = archive_batch_has_completed_manifest_sha_tx(
                tx.as_mut(),
                HOURLY_ROLLUP_DATASET_INVOCATIONS,
                &row.file_path,
            )
            .await?
                && !invocation_archive_has_unverified_replay_marker_tx(tx.as_mut(), &row.file_path)
                    .await?;
            let has_stale_replay_marker = if archive_proof_valid {
                invocation_archive_has_stale_replay_marker_tx(tx.as_mut(), &row.file_path).await?
            } else {
                false
            };
            let breakdown_replayed = if archive_proof_valid {
                hourly_rollup_archive_replayed_tx(
                    tx.as_mut(),
                    HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE_BREAKDOWN,
                    HOURLY_ROLLUP_DATASET_INVOCATIONS,
                    &row.file_path,
                )
                .await?
            } else {
                false
            };
            tx.commit().await?;
            (
                row,
                deferred_candidate,
                archive_proof_valid,
                has_stale_replay_marker,
                breakdown_replayed,
            )
        };
        let next_cursor = UsageBreakdownRepairCursor {
            stale_rank: row.stale_rank,
            month_key: row.month_key.clone(),
            created_at: row.created_at.clone(),
            id: row.id,
        };
        if !archive_proof_valid && deferred_candidate {
            // An already deferred row remains retryable until its manifest and replay proof are
            // available again. Skip it for this pass so a recoverable deferred tail can proceed,
            // but never delete the only durable retry record while proof is still absent.
            skipped_deferred_archive_ids.insert(row.id);
            budget_overrun_candidate_started = true;
            continue;
        }

        let reopen_outcome =
            if archive_proof_valid && (has_stale_replay_marker || !breakdown_replayed) {
                // The archive fence and all archive reads live outside the database transaction. The
                // helper revalidates the manifest and overlap closure in a short write transaction
                // before clearing or resetting any durable state.
                let mut archive_fence = InvocationArchiveRepairFence::new(started_at, None);
                Some(if has_stale_replay_marker {
                    reopen_replaced_materialized_invocation_archive_without_long_tx(
                        pool,
                        &row.file_path,
                        &row.sha256,
                        row.coverage_start_at.as_deref(),
                        row.coverage_end_at.as_deref(),
                        &mut archive_fence,
                    )
                    .await?
                } else {
                    reopen_materialized_invocation_archive_usage_breakdown_without_long_tx(
                        pool,
                        &row.file_path,
                        &row.sha256,
                        row.coverage_start_at.as_deref(),
                        row.coverage_end_at.as_deref(),
                        &mut archive_fence,
                    )
                    .await?
                })
            } else {
                None
            };

        if matches!(
            reopen_outcome.as_ref(),
            Some(InvocationArchiveReopenResult::Rejected)
        ) {
            persist_deferred_usage_breakdown_repair_candidate(
                pool,
                &row,
                (!deferred_candidate).then_some(&next_cursor),
            )
            .await?;
            if !deferred_candidate {
                cursor = next_cursor;
            } else {
                skipped_deferred_archive_ids.insert(row.id);
            }
            budget_overrun_candidate_started = true;
            continue;
        }

        if matches!(
            reopen_outcome.as_ref(),
            Some(InvocationArchiveReopenResult::BudgetExhausted)
        ) {
            persist_deferred_usage_breakdown_repair_candidate(
                pool,
                &row,
                (!deferred_candidate).then_some(&next_cursor),
            )
            .await?;
            break;
        }

        if let Some(InvocationArchiveReopenResult::Reopened(reopened)) = reopen_outcome {
            touched_batches = touched_batches.saturating_add(reopened.len());
        }
        let mut tx = pool.begin().await?;
        if deferred_candidate {
            delete_deferred_usage_breakdown_repair_candidate_tx(tx.as_mut(), row.id).await?;
        } else {
            advance_usage_breakdown_repair_cursor_tx(tx.as_mut(), &next_cursor).await?;
            cursor = next_cursor;
        }
        budget_overrun_candidate_started = true;
        tx.commit().await?;
    }

    Ok(touched_batches)
}
