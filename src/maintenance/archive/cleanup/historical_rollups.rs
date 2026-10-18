use super::*;
use sqlx::FromRow;

pub(crate) fn invocation_status_counts_toward_terminal_totals(status: Option<&str>) -> bool {
    let normalized_status = status.map(str::trim).unwrap_or_default();
    !normalized_status.eq_ignore_ascii_case("running")
        && !normalized_status.eq_ignore_ascii_case("pending")
}

#[derive(Debug, FromRow)]
pub(crate) struct HistoricalRollupPendingArchiveBatchRow {
    pub(super) dataset: String,
    pub(super) month_key: String,
    pub(super) file_path: String,
    pub(super) coverage_start_at: Option<String>,
    pub(super) coverage_end_at: Option<String>,
}

const STARTUP_HISTORICAL_ROLLUP_CANDIDATE_LIMIT: i64 = 32;
const STARTUP_HISTORICAL_ROLLUP_BATCH_LIMIT: usize = 16;

#[derive(Debug)]
pub(crate) struct HistoricalRollupStartupWindowResult {
    pub(crate) summary: HistoricalRollupMaterializationSummary,
    pub(crate) next_cursor_id: i64,
    pub(crate) candidate_count: usize,
    pub(crate) inspected_path_count: usize,
    pub(crate) changed_path_count: usize,
    pub(crate) hit_budget: bool,
    pub(crate) wrapped: bool,
}

#[derive(Debug)]
pub(crate) struct HistoricalRollupStartupPendingHint {
    pub(crate) pending_archive_batches: usize,
    pub(crate) candidate_count: usize,
    pub(crate) inspected_path_count: usize,
}

pub(super) async fn load_historical_rollup_startup_candidates(
    pool: &Pool<Sqlite>,
    cursor_id: i64,
) -> Result<Vec<HistoricalRollupStartupCandidateRow>> {
    sqlx::query_as::<_, HistoricalRollupStartupCandidateRow>(
        r#"
        SELECT
            batches.id,
            batches.dataset,
            batches.file_path,
            batches.sha256,
            batches.row_count,
            batches.summary_source_kind,
            batches.coverage_start_at,
            batches.coverage_end_at
        FROM archive_batches AS batches
        WHERE batches.status = ?4
          AND batches.sha256 IS NOT NULL
          AND TRIM(batches.sha256) <> ''
          AND batches.id > ?5
          AND (
                (batches.dataset = 'codex_invocations'
                    AND COALESCE(batches.summary_source_kind, 'unknown') <> 'live_mirror'
                    AND (
                    batches.historical_rollups_materialized_at IS NULL
                    OR NOT EXISTS (
                        SELECT 1
                        FROM hourly_rollup_archive_replay AS replay
                        WHERE replay.target = ?1
                          AND replay.dataset = batches.dataset
                          AND replay.file_path = batches.file_path
                          AND replay.archive_sha256 = batches.sha256
                    )
                    OR NOT EXISTS (
                        SELECT 1
                        FROM hourly_rollup_archive_replay AS replay
                        WHERE replay.target = ?2
                          AND replay.dataset = batches.dataset
                          AND replay.file_path = batches.file_path
                          AND replay.archive_sha256 = batches.sha256
                    )
                    OR NOT EXISTS (
                        SELECT 1
                        FROM hourly_rollup_archive_replay AS replay
                        WHERE replay.target = ?3
                          AND replay.dataset = batches.dataset
                          AND replay.file_path = batches.file_path
                          AND replay.archive_sha256 = batches.sha256
                    )
                    OR EXISTS (
                        SELECT 1
                        FROM hourly_rollup_archive_replay AS stale
                        WHERE stale.dataset = batches.dataset
                          AND stale.file_path = batches.file_path
                          AND stale.archive_sha256 IS NOT NULL
                          AND TRIM(stale.archive_sha256) <> ''
                          AND stale.archive_sha256 <> batches.sha256
                    )
                ))
                OR (batches.dataset = 'forward_proxy_attempts'
                    AND batches.historical_rollups_materialized_at IS NULL)
          )
        ORDER BY batches.id ASC
        LIMIT ?6
        "#,
    )
    .bind(HOURLY_ROLLUP_TARGET_INVOCATIONS)
    .bind(HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_STATS_HOURLY)
    .bind(HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE_BREAKDOWN)
    .bind(ARCHIVE_STATUS_COMPLETED)
    .bind(cursor_id)
    .bind(STARTUP_HISTORICAL_ROLLUP_CANDIDATE_LIMIT)
    .fetch_all(pool)
    .await
    .context("failed to load historical rollup startup keyset candidates")
}

pub(crate) async fn count_historical_rollup_startup_pending_hint(
    pool: &Pool<Sqlite>,
) -> Result<HistoricalRollupStartupPendingHint> {
    let candidates = load_historical_rollup_startup_candidates(pool, 0).await?;
    let inspected_path_count = candidates.len();
    let pending_archive_batches = candidates
        .iter()
        .filter(|candidate| Path::new(&candidate.file_path).exists())
        .count();

    Ok(HistoricalRollupStartupPendingHint {
        pending_archive_batches,
        candidate_count: candidates.len(),
        inspected_path_count,
    })
}

pub(crate) async fn materialize_historical_rollups_startup_window(
    pool: &Pool<Sqlite>,
    cursor_id: i64,
    max_elapsed: Duration,
) -> Result<HistoricalRollupStartupWindowResult> {
    let mut candidates = load_historical_rollup_startup_candidates(pool, cursor_id).await?;
    let mut wrapped = false;
    if candidates.is_empty() && cursor_id > 0 {
        candidates = load_historical_rollup_startup_candidates(pool, 0).await?;
        wrapped = !candidates.is_empty();
    }
    if candidates.is_empty() {
        return Ok(HistoricalRollupStartupWindowResult {
            summary: HistoricalRollupMaterializationSummary::default(),
            next_cursor_id: 0,
            candidate_count: 0,
            inspected_path_count: 0,
            changed_path_count: 0,
            hit_budget: false,
            wrapped,
        });
    }

    let candidate_batch = candidates
        .iter()
        .take(STARTUP_HISTORICAL_ROLLUP_BATCH_LIMIT)
        .collect::<Vec<_>>();
    // Freeze the set of candidates whose parents were present when locks were collected. A
    // directory recreated after this point must wait for the next bounded pass instead of being
    // replayed without the lock that fenced its original file identity.
    let processable_candidate_ids = candidate_batch
        .iter()
        .filter(|candidate| {
            Path::new(&candidate.file_path)
                .parent()
                .is_some_and(Path::exists)
        })
        .map(|candidate| candidate.id)
        .collect::<Vec<_>>();
    let started_at = Instant::now();
    // The startup-backfill caller holds the P2 SQLite write permit before entering this stage.
    let mut tx = pool.begin().await?;
    let mut next_cursor_id = if wrapped { 0 } else { cursor_id };
    let mut scanned_archive_batches = 0_usize;
    let mut skipped_archive_batches = 0_usize;
    let mut materialized_archive_batches = 0_usize;
    let mut changed_path_count = 0_usize;
    let mut blocked_archive_batches = 0_usize;
    let mut materialized_invocation_batches = 0_usize;
    let mut materialized_forward_proxy_batches = 0_usize;
    let mut inspected_path_count = 0_usize;
    let mut hit_budget = false;

    for candidate in candidate_batch {
        if started_at.elapsed() >= max_elapsed {
            hit_budget = true;
            break;
        }
        if !processable_candidate_ids.contains(&candidate.id) {
            inspected_path_count += 1;
            scanned_archive_batches += 1;
            skipped_archive_batches += 1;
            next_cursor_id = next_cursor_id.max(candidate.id);
            continue;
        }
        let candidate_summary = if candidate.dataset == HOURLY_ROLLUP_DATASET_INVOCATIONS {
            replay_invocation_archive_files_into_hourly_rollups_tx_with_limits(
                tx.as_mut(),
                started_at,
                Some(1),
                Some(max_elapsed),
                0,
                vec![candidate.archive_file()],
            )
            .await?
        } else {
            replay_forward_proxy_archive_files_into_hourly_rollups_tx_with_limits(
                tx.as_mut(),
                started_at,
                Some(1),
                Some(max_elapsed),
                0,
                vec![candidate.archive_file()],
            )
            .await?
        };
        if candidate_summary.scanned_batches == 0 {
            hit_budget |= candidate_summary.hit_budget;
            break;
        }
        inspected_path_count += candidate_summary.scanned_batches as usize;
        scanned_archive_batches += candidate_summary.scanned_batches as usize;
        skipped_archive_batches += candidate_summary.skipped_batches as usize;
        materialized_archive_batches += candidate_summary.materialized_batches as usize;
        changed_path_count += candidate_summary.changed_batches as usize;
        blocked_archive_batches += candidate_summary.blocked_batches as usize;
        if candidate.dataset == HOURLY_ROLLUP_DATASET_INVOCATIONS {
            materialized_invocation_batches += candidate_summary.materialized_batches as usize;
        } else {
            materialized_forward_proxy_batches += candidate_summary.materialized_batches as usize;
        }
        if candidate_summary.hit_budget {
            hit_budget = true;
            if candidate_summary.advance_cursor_after_unstarted_replay {
                next_cursor_id = next_cursor_id.max(candidate.id);
            }
            break;
        }
        next_cursor_id = next_cursor_id.max(candidate.id);
    }
    tx.commit().await?;

    Ok(HistoricalRollupStartupWindowResult {
        summary: HistoricalRollupMaterializationSummary {
            scanned_archive_batches,
            skipped_archive_batches,
            materialized_archive_batches,
            blocked_archive_batches,
            materialized_bucket_count: 0,
            materialized_invocation_batches,
            materialized_forward_proxy_batches,
            last_materialized_bucket_start_epoch: None,
        },
        next_cursor_id,
        candidate_count: candidates.len(),
        inspected_path_count,
        changed_path_count,
        hit_budget,
        wrapped,
    })
}

pub(crate) async fn count_historical_rollup_archive_batches(
    pool: &Pool<Sqlite>,
    pending_only: bool,
) -> Result<i64> {
    let mut query =
        QueryBuilder::<Sqlite>::new("SELECT COUNT(*) FROM archive_batches WHERE status = ");
    query.push_bind(ARCHIVE_STATUS_COMPLETED);
    query.push(" AND dataset IN (");
    {
        let mut separated = query.separated(", ");
        for dataset in HISTORICAL_ROLLUP_ARCHIVE_DATASETS {
            separated.push_bind(dataset);
        }
    }
    query.push(")");
    if pending_only {
        query.push(" AND historical_rollups_materialized_at IS NULL");
    }
    Ok(query.build_query_scalar::<i64>().fetch_one(pool).await?)
}

pub(crate) fn historical_rollup_materialized_bucket_targets() -> [&'static str; 7] {
    [
        HOURLY_ROLLUP_TARGET_INVOCATIONS,
        HOURLY_ROLLUP_TARGET_INVOCATION_FAILURES,
        HOURLY_ROLLUP_TARGET_PROXY_PERF,
        HOURLY_ROLLUP_TARGET_PROMPT_CACHE,
        HOURLY_ROLLUP_TARGET_PROMPT_CACHE_UPSTREAM_ACCOUNTS,
        HOURLY_ROLLUP_TARGET_STICKY_KEYS,
        HOURLY_ROLLUP_TARGET_FORWARD_PROXY_ATTEMPTS,
    ]
}

pub(crate) async fn load_latest_materialized_legacy_invocation_rollup_bucket_epoch(
    pool: &Pool<Sqlite>,
    config: &AppConfig,
) -> Result<Option<i64>> {
    let invocation_archive_cutoff = shanghai_local_cutoff_string(config.invocation_max_days);
    let latest_coverage_end_at: Option<String> = sqlx::query_scalar(
        r#"
        SELECT MAX(coverage_end_at)
        FROM archive_batches
        WHERE dataset = 'codex_invocations'
          AND status = ?1
          AND COALESCE(summary_source_kind, 'unknown') <> 'live_mirror'
          AND historical_rollups_materialized_at IS NOT NULL
          AND coverage_end_at IS NOT NULL
          AND coverage_end_at < ?2
        "#,
    )
    .bind(ARCHIVE_STATUS_COMPLETED)
    .bind(invocation_archive_cutoff)
    .fetch_one(pool)
    .await?;

    Ok(latest_coverage_end_at.and_then(|coverage_end_at| {
        parse_shanghai_local_naive(&coverage_end_at)
            .ok()
            .and_then(|naive| {
                let bucket_start_epoch =
                    align_bucket_epoch(local_naive_to_utc(naive, Shanghai).timestamp(), 3_600, 0);
                Utc.timestamp_opt(bucket_start_epoch, 0)
                    .single()
                    .map(|_| bucket_start_epoch)
            })
    }))
}

pub(crate) async fn count_materialized_historical_rollup_buckets(
    pool: &Pool<Sqlite>,
) -> Result<i64> {
    let mut query = QueryBuilder::<Sqlite>::new(
        "SELECT COUNT(*) FROM hourly_rollup_materialized_buckets WHERE target IN (",
    );
    {
        let mut separated = query.separated(", ");
        for target in historical_rollup_materialized_bucket_targets() {
            separated.push_bind(target);
        }
    }
    query.push(")");
    Ok(query.build_query_scalar::<i64>().fetch_one(pool).await?)
}

pub(crate) async fn load_historical_rollup_backfill_snapshot(
    pool: &Pool<Sqlite>,
    config: &AppConfig,
) -> Result<HistoricalRollupBackfillSnapshot> {
    let mut query = QueryBuilder::<Sqlite>::new(
        "SELECT dataset, month_key, file_path, coverage_start_at, coverage_end_at \
         FROM archive_batches WHERE status = ",
    );
    query.push_bind(ARCHIVE_STATUS_COMPLETED);
    query.push(" AND historical_rollups_materialized_at IS NULL AND dataset IN (");
    {
        let mut separated = query.separated(", ");
        for dataset in HISTORICAL_ROLLUP_ARCHIVE_DATASETS {
            separated.push_bind(dataset);
        }
    }
    query.push(") ORDER BY month_key ASC, id ASC");
    // This is an exact diagnostics snapshot. Bounded materialization paths must not call it on
    // normal runs because their replay selectors already own the bounded page and cursor.
    let pending_rows = query
        .build_query_as::<HistoricalRollupPendingArchiveBatchRow>()
        .fetch_all(pool)
        .await?;
    let pending_buckets = pending_rows
        .iter()
        .filter(|row| Path::new(&row.file_path).exists())
        .map(estimate_historical_rollup_pending_bucket_count)
        .sum::<u64>();
    let legacy_archive_pending = pending_rows
        .iter()
        .filter(|row| Path::new(&row.file_path).exists())
        .count() as u64;
    let legacy_invocation_pending = pending_rows
        .iter()
        .filter(|row| Path::new(&row.file_path).exists())
        .any(|row| row.dataset == HOURLY_ROLLUP_DATASET_INVOCATIONS);
    let pending_usage_breakdown_batches = load_invocation_archive_files_missing_rollup_target(
        pool,
        HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE_BREAKDOWN,
    )
    .await?
    .len() as u64;
    let last_materialized_hour =
        load_latest_materialized_legacy_invocation_rollup_bucket_epoch(pool, config)
            .await?
            .and_then(|epoch| Utc.timestamp_opt(epoch, 0).single())
            .map(format_utc_iso);
    let has_invocation_rollup_gap =
        legacy_invocation_pending || pending_usage_breakdown_batches > 0;
    let alert_level = if legacy_archive_pending == 0 && pending_usage_breakdown_batches == 0 {
        HistoricalRollupBackfillAlertLevel::None
    } else if has_invocation_rollup_gap {
        HistoricalRollupBackfillAlertLevel::Critical
    } else {
        HistoricalRollupBackfillAlertLevel::Warn
    };

    Ok(HistoricalRollupBackfillSnapshot {
        pending_buckets,
        legacy_archive_pending,
        pending_usage_breakdown_batches,
        last_materialized_hour,
        alert_level,
    })
}

pub(crate) async fn materialize_usage_breakdown_historical_rollups_bounded_from_skip(
    pool: &Pool<Sqlite>,
    config: &AppConfig,
    max_archive_batches: Option<u64>,
    max_elapsed: Option<Duration>,
    skip_pending_archives: usize,
) -> Result<HistoricalRollupMaterializationSummary> {
    let started_at = Instant::now();
    // The replay selector applies the caller's page cursor while fetching max+skip rows; do not
    // load the current backlog just to wrap this offset modulo its changing size.

    let Some(admission) = super::super::retention::acquire_retention_write_admission(
        "historical_rollup_usage_breakdown",
    )
    .await
    else {
        return Err(super::super::retention::retention_write_deferred(
            "historical_rollup_usage_breakdown",
        ));
    };
    let mut tx = pool.begin().await?;
    let invocation_summary =
        replay_invocation_usage_breakdown_archives_into_hourly_rollups_tx_with_limits(
            tx.as_mut(),
            started_at,
            max_archive_batches,
            max_elapsed,
            skip_pending_archives,
        )
        .await?;
    tx.commit().await?;
    drop(admission);

    Ok(HistoricalRollupMaterializationSummary {
        scanned_archive_batches: invocation_summary.scanned_batches as usize,
        skipped_archive_batches: invocation_summary.skipped_batches as usize,
        materialized_archive_batches: invocation_summary.materialized_batches as usize,
        blocked_archive_batches: invocation_summary.blocked_batches as usize,
        materialized_bucket_count: count_materialized_historical_rollup_buckets(pool).await?
            as usize,
        materialized_invocation_batches: invocation_summary.materialized_batches as usize,
        materialized_forward_proxy_batches: 0,
        last_materialized_bucket_start_epoch:
            load_latest_materialized_legacy_invocation_rollup_bucket_epoch(pool, config).await?,
    })
}

pub(crate) async fn materialize_historical_rollups(
    pool: &Pool<Sqlite>,
    config: &AppConfig,
    dry_run: bool,
) -> Result<HistoricalRollupMaterializationSummary> {
    materialize_historical_rollups_bounded(pool, config, dry_run, None, None).await
}

pub(crate) async fn materialize_historical_rollups_bounded(
    pool: &Pool<Sqlite>,
    config: &AppConfig,
    dry_run: bool,
    max_archive_batches: Option<u64>,
    max_elapsed: Option<Duration>,
) -> Result<HistoricalRollupMaterializationSummary> {
    materialize_historical_rollups_bounded_from_skip(
        pool,
        config,
        dry_run,
        max_archive_batches,
        max_elapsed,
        0,
    )
    .await
}

pub(crate) async fn materialize_historical_rollups_bounded_from_skip(
    pool: &Pool<Sqlite>,
    config: &AppConfig,
    dry_run: bool,
    max_archive_batches: Option<u64>,
    max_elapsed: Option<Duration>,
    skip_pending_archives: usize,
) -> Result<HistoricalRollupMaterializationSummary> {
    let started_at = Instant::now();
    if dry_run {
        let pending_snapshot = load_historical_rollup_backfill_snapshot(pool, config).await?;
        return Ok(HistoricalRollupMaterializationSummary {
            scanned_archive_batches: pending_snapshot.legacy_archive_pending as usize,
            skipped_archive_batches: 0,
            materialized_archive_batches: pending_snapshot.legacy_archive_pending as usize,
            blocked_archive_batches: 0,
            materialized_bucket_count: pending_snapshot.pending_buckets as usize,
            materialized_invocation_batches: 0,
            materialized_forward_proxy_batches: 0,
            last_materialized_bucket_start_epoch:
                load_latest_materialized_legacy_invocation_rollup_bucket_epoch(pool, config).await?,
        });
    }

    // Normal replay is already bounded by the selector's max+skip page; an exact backlog
    // snapshot here would turn each bounded pass back into a full pending-set read.
    let Some(admission) = super::super::retention::acquire_retention_write_admission(
        "historical_rollup_materialization",
    )
    .await
    else {
        return Err(super::super::retention::retention_write_deferred(
            "historical_rollup_materialization",
        ));
    };
    let mut tx = pool.begin().await?;
    let invocation_summary = replay_invocation_archives_into_hourly_rollups_tx_with_limits(
        tx.as_mut(),
        started_at,
        max_archive_batches,
        max_elapsed,
        skip_pending_archives,
    )
    .await?;
    let remaining_budget =
        historical_rollup_materialization_remaining_budget(started_at, max_elapsed);
    let forward_proxy_summary = replay_forward_proxy_archives_into_hourly_rollups_tx_with_limits(
        tx.as_mut(),
        started_at,
        max_archive_batches
            .map(|limit| limit.saturating_sub(invocation_summary.budget_consumed_batches)),
        remaining_budget,
        invocation_summary.remaining_skip_batches,
    )
    .await?;
    loop {
        if historical_rollup_materialization_budget_exhausted(started_at, max_elapsed) {
            break;
        }
        let updated = replay_live_invocation_hourly_rollups_tx(tx.as_mut()).await?;
        if updated == 0 {
            break;
        }
    }
    loop {
        if historical_rollup_materialization_budget_exhausted(started_at, max_elapsed) {
            break;
        }
        let updated = replay_live_forward_proxy_attempt_hourly_rollups_tx(tx.as_mut()).await?;
        if updated == 0 {
            break;
        }
    }
    tx.commit().await?;
    drop(admission);

    crate::record_managed_task_processed_work(
        &["materialize_historical_rollups"],
        i64::try_from(
            invocation_summary.materialized_batches + forward_proxy_summary.materialized_batches,
        )
        .unwrap_or(i64::MAX),
    );

    Ok(HistoricalRollupMaterializationSummary {
        scanned_archive_batches: (invocation_summary.scanned_batches
            + forward_proxy_summary.scanned_batches) as usize,
        skipped_archive_batches: (invocation_summary.skipped_batches
            + forward_proxy_summary.skipped_batches) as usize,
        materialized_archive_batches: (invocation_summary.materialized_batches
            + forward_proxy_summary.materialized_batches)
            as usize,
        blocked_archive_batches: invocation_summary.blocked_batches as usize,
        materialized_bucket_count: count_materialized_historical_rollup_buckets(pool).await?
            as usize,
        materialized_invocation_batches: invocation_summary.materialized_batches as usize,
        materialized_forward_proxy_batches: forward_proxy_summary.materialized_batches as usize,
        last_materialized_bucket_start_epoch:
            load_latest_materialized_legacy_invocation_rollup_bucket_epoch(pool, config).await?,
    })
}

pub(crate) fn historical_rollup_materialization_remaining_budget(
    started_at: Instant,
    max_elapsed: Option<Duration>,
) -> Option<Duration> {
    max_elapsed.map(|limit| limit.saturating_sub(started_at.elapsed()))
}

pub(crate) fn historical_rollup_materialization_budget_exhausted(
    started_at: Instant,
    max_elapsed: Option<Duration>,
) -> bool {
    matches!(
        historical_rollup_materialization_remaining_budget(started_at, max_elapsed),
        Some(remaining) if remaining.is_zero()
    )
}
