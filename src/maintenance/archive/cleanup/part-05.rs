#[derive(Debug, FromRow)]
pub(crate) struct LegacyArchivePruneCandidateRow {
    id: i64,
    dataset: String,
    file_path: String,
    sha256: String,
    cleanup_state: String,
    historical_rollups_materialized_at: Option<String>,
    coverage_end_at: Option<String>,
}

pub(crate) fn estimate_historical_rollup_pending_bucket_count(
    row: &HistoricalRollupPendingArchiveBatchRow,
) -> u64 {
    if let (Some(start), Some(end)) = (&row.coverage_start_at, &row.coverage_end_at)
        && let (Ok(start_local), Ok(end_local)) = (
            parse_shanghai_local_naive(start),
            parse_shanghai_local_naive(end),
        )
    {
        let start_utc = local_naive_to_utc(start_local, Shanghai);
        let end_utc = local_naive_to_utc(end_local, Shanghai);
        let secs = (end_utc.timestamp() - start_utc.timestamp()).max(0);
        return ((secs + 3_599) / 3_600).max(1) as u64;
    }

    let Ok(start_date) = NaiveDate::parse_from_str(&format!("{}-01", row.month_key), "%Y-%m-%d")
    else {
        return 0;
    };
    let (next_year, next_month) = if start_date.month() == 12 {
        (start_date.year() + 1, 1)
    } else {
        (start_date.year(), start_date.month() + 1)
    };
    let Some(next_month_date) = NaiveDate::from_ymd_opt(next_year, next_month, 1) else {
        return 0;
    };
    let Some(start_naive) = start_date.and_hms_opt(0, 0, 0) else {
        return 0;
    };
    let Some(end_naive) = next_month_date.and_hms_opt(0, 0, 0) else {
        return 0;
    };
    let start_utc = local_naive_to_utc(start_naive, Shanghai);
    let end_utc = local_naive_to_utc(end_naive, Shanghai);
    ((end_utc.timestamp() - start_utc.timestamp()).max(0) / 3_600) as u64
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
    let pending_archive_files = load_invocation_archive_files_missing_rollup_target(
        pool,
        HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE_BREAKDOWN,
    )
    .await?;
    let pending_usage_breakdown_batches = pending_archive_files.len();
    let bounded_skip = if pending_usage_breakdown_batches == 0 {
        0
    } else {
        skip_pending_archives % pending_usage_breakdown_batches
    };

    let mut tx = pool.begin().await?;
    let invocation_summary =
        replay_invocation_usage_breakdown_archives_into_hourly_rollups_tx_with_limits(
            tx.as_mut(),
            started_at,
            max_archive_batches,
            max_elapsed,
            bounded_skip,
        )
        .await?;
    tx.commit().await?;

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
    let pending_snapshot = load_historical_rollup_backfill_snapshot(pool, config).await?;
    let bounded_skip = if pending_snapshot.legacy_archive_pending == 0 {
        0
    } else {
        skip_pending_archives % pending_snapshot.legacy_archive_pending as usize
    };
    if dry_run {
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

    let mut tx = pool.begin().await?;
    let invocation_summary = replay_invocation_archives_into_hourly_rollups_tx_with_limits(
        tx.as_mut(),
        started_at,
        max_archive_batches,
        max_elapsed,
        bounded_skip,
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

pub(crate) async fn prune_legacy_archive_batches(
    pool: &Pool<Sqlite>,
    config: &AppConfig,
    dry_run: bool,
) -> Result<LegacyArchivePruneSummary> {
    let candidates = load_legacy_archive_prune_candidates(pool).await?;
    let context = load_legacy_archive_prune_context(pool, config).await?;
    let mut summary = LegacyArchivePruneSummary {
        scanned_archive_batches: candidates.len(),
        ..LegacyArchivePruneSummary::default()
    };
    for candidate in candidates {
        match prune_legacy_archive_candidate(pool, &candidate, &context, dry_run).await? {
            LegacyArchivePruneOutcome::Deleted => summary.deleted_archive_batches += 1,
            LegacyArchivePruneOutcome::SkippedUnmaterialized => {
                summary.skipped_unmaterialized_batches += 1;
            }
            LegacyArchivePruneOutcome::SkippedRetained => summary.skipped_retained_batches += 1,
            LegacyArchivePruneOutcome::Noop => {}
        }
    }
    Ok(summary)
}

async fn load_legacy_archive_prune_candidates(
    pool: &Pool<Sqlite>,
) -> Result<Vec<LegacyArchivePruneCandidateRow>> {
    let mut query = QueryBuilder::<Sqlite>::new(
        "SELECT id, dataset, file_path, sha256, cleanup_state, historical_rollups_materialized_at, coverage_end_at \
         FROM archive_batches WHERE status = ",
    );
    query.push_bind(ARCHIVE_STATUS_COMPLETED);
    query.push(" AND dataset IN (");
    {
        let mut separated = query.separated(", ");
        for dataset in HISTORICAL_ROLLUP_ARCHIVE_DATASETS {
            separated.push_bind(dataset);
        }
    }
    query.push(") AND COALESCE(layout, ");
    query.push_bind(ARCHIVE_LAYOUT_LEGACY_MONTH);
    query.push(") = ");
    query.push_bind(ARCHIVE_LAYOUT_LEGACY_MONTH);
    query.push(" ORDER BY month_key ASC, id ASC");
    let candidates = query
        .build_query_as::<LegacyArchivePruneCandidateRow>()
        .fetch_all(pool)
        .await?;
    Ok(candidates)
}

struct LegacyArchivePruneContext {
    pending_account_count: i64,
    invocation_archive_cutoff: String,
    long_term_stats_ready: bool,
    long_term_stats_archive_files: HashSet<(String, String)>,
    long_term_stats_attempt_archive_files: HashSet<(String, String)>,
}

async fn load_legacy_archive_prune_context(
    pool: &Pool<Sqlite>,
    config: &AppConfig,
) -> Result<LegacyArchivePruneContext> {
    let pending_account_count = count_upstream_accounts_missing_last_activity(pool).await?;
    let invocation_archive_cutoff = shanghai_local_cutoff_string(config.invocation_max_days);
    let long_term_stats_status =
        sqlx::query_scalar::<_, String>("SELECT status FROM long_term_stats_state WHERE id = 1")
            .fetch_optional(pool)
            .await?;
    let long_term_stats_ready = long_term_stats_status
        .as_deref()
        .is_some_and(|status| matches!(status, "ready" | "empty"));
    let long_term_stats_archive_files = if long_term_stats_ready {
        load_long_term_stats_archive_files(pool, "codex_invocations").await?
    } else {
        HashSet::new()
    };
    let long_term_stats_attempt_archive_files = if long_term_stats_ready {
        load_long_term_stats_archive_files(pool, "pool_upstream_request_attempts").await?
    } else {
        HashSet::new()
    };
    Ok(LegacyArchivePruneContext {
        pending_account_count: pending_account_count.min(i64::MAX as u64) as i64,
        invocation_archive_cutoff,
        long_term_stats_ready,
        long_term_stats_archive_files,
        long_term_stats_attempt_archive_files,
    })
}

enum LegacyArchivePruneOutcome {
    Deleted,
    SkippedUnmaterialized,
    SkippedRetained,
    Noop,
}

fn legacy_archive_prune_gate(
    candidate: &LegacyArchivePruneCandidateRow,
    context: &LegacyArchivePruneContext,
    file_missing: bool,
) -> Option<LegacyArchivePruneOutcome> {
    if candidate.dataset == HOURLY_ROLLUP_DATASET_INVOCATIONS && context.pending_account_count > 0 {
        return Some(LegacyArchivePruneOutcome::SkippedUnmaterialized);
    }
    if matches!(
        candidate.dataset.as_str(),
        HOURLY_ROLLUP_DATASET_INVOCATIONS | "pool_upstream_request_attempts"
    ) && file_missing
    {
        return Some(LegacyArchivePruneOutcome::SkippedUnmaterialized);
    }
    let identity = (candidate.file_path.clone(), candidate.sha256.clone());
    if candidate.dataset == HOURLY_ROLLUP_DATASET_INVOCATIONS
        && (!context.long_term_stats_ready
            || !context.long_term_stats_archive_files.contains(&identity))
    {
        return Some(LegacyArchivePruneOutcome::SkippedUnmaterialized);
    }
    if candidate.dataset == "pool_upstream_request_attempts"
        && !context
            .long_term_stats_attempt_archive_files
            .contains(&identity)
    {
        return Some(LegacyArchivePruneOutcome::SkippedUnmaterialized);
    }
    if !file_missing && candidate.historical_rollups_materialized_at.is_none() {
        return Some(LegacyArchivePruneOutcome::SkippedUnmaterialized);
    }
    if candidate.dataset == HOURLY_ROLLUP_DATASET_INVOCATIONS
        && candidate
            .coverage_end_at
            .as_deref()
            .map(|coverage_end_at| coverage_end_at >= context.invocation_archive_cutoff.as_str())
            .unwrap_or(true)
    {
        return Some(LegacyArchivePruneOutcome::SkippedRetained);
    }
    None
}

async fn prune_legacy_archive_candidate(
    pool: &Pool<Sqlite>,
    candidate: &LegacyArchivePruneCandidateRow,
    context: &LegacyArchivePruneContext,
    dry_run: bool,
) -> Result<LegacyArchivePruneOutcome> {
    if candidate.cleanup_state == ARCHIVE_CLEANUP_STATE_DELETE_PENDING {
        if dry_run {
            return Ok(LegacyArchivePruneOutcome::Deleted);
        }
        return Ok(
            if finalize_archive_batch_file_deletion(
                pool,
                candidate.id,
                &candidate.dataset,
                &candidate.file_path,
                &candidate.sha256,
            )
            .await?
            {
                LegacyArchivePruneOutcome::Deleted
            } else {
                LegacyArchivePruneOutcome::SkippedUnmaterialized
            },
        );
    }
    let file_missing = archive_file_is_confirmed_missing(&candidate.file_path);
    if let Some(outcome) = legacy_archive_prune_gate(candidate, context, file_missing) {
        return Ok(outcome);
    }
    if dry_run {
        info!(
            dataset = candidate.dataset,
            file_path = candidate.file_path,
            "maintenance dry-run planned legacy archive prune"
        );
        return Ok(LegacyArchivePruneOutcome::Deleted);
    }
    if file_missing {
        return stage_and_finalize_legacy_archive_candidate(pool, candidate, None).await;
    }
    let integrity_source_safe_start =
        match legacy_archive_integrity_source_safe_start(pool, candidate).await {
            Ok(source_safe_start) => source_safe_start,
            Err(error) => {
                warn!(
                    dataset = candidate.dataset,
                    file_path = candidate.file_path,
                    error = %error,
                    "could not prove long-term source boundary; retaining legacy archive batch"
                );
                return Ok(LegacyArchivePruneOutcome::SkippedUnmaterialized);
            }
        };
    stage_and_finalize_legacy_archive_candidate(pool, candidate, integrity_source_safe_start).await
}

async fn legacy_archive_integrity_source_safe_start(
    pool: &Pool<Sqlite>,
    candidate: &LegacyArchivePruneCandidateRow,
) -> Result<Option<NaiveDate>> {
    if matches!(
        candidate.dataset.as_str(),
        HOURLY_ROLLUP_DATASET_INVOCATIONS | "pool_upstream_request_attempts"
    ) {
        crate::long_term_stats::long_term_integrity_source_safe_start_for_archive_cleanup(
            pool,
            &candidate.dataset,
            &candidate.file_path,
            candidate.coverage_end_at.as_deref(),
        )
        .await
    } else {
        Ok(None)
    }
}

async fn stage_and_finalize_legacy_archive_candidate(
    pool: &Pool<Sqlite>,
    candidate: &LegacyArchivePruneCandidateRow,
    source_safe_start: Option<NaiveDate>,
) -> Result<LegacyArchivePruneOutcome> {
    let staged = stage_archive_batch_deletion(
        pool,
        candidate.id,
        &candidate.dataset,
        &candidate.file_path,
        &candidate.sha256,
        source_safe_start,
    )
    .await?;
    if !staged {
        return Ok(LegacyArchivePruneOutcome::Noop);
    }
    let deleted = finalize_archive_batch_file_deletion(
        pool,
        candidate.id,
        &candidate.dataset,
        &candidate.file_path,
        &candidate.sha256,
    )
    .await?;
    Ok(if deleted {
        LegacyArchivePruneOutcome::Deleted
    } else {
        LegacyArchivePruneOutcome::Noop
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

pub(crate) async fn prune_archive_batches(
    pool: &Pool<Sqlite>,
    config: &AppConfig,
    dry_run: bool,
) -> Result<ArchiveBatchPruneSummary> {
    let expired_archive_batches_deleted =
        cleanup_expired_archive_batches(pool, config, dry_run).await?;
    let legacy_summary = prune_legacy_archive_batches(pool, config, dry_run).await?;
    Ok(ArchiveBatchPruneSummary {
        expired_archive_batches_deleted,
        legacy_archive_batches_deleted: legacy_summary.deleted_archive_batches,
    })
}

pub(crate) async fn compact_old_quota_snapshots(
    pool: &Pool<Sqlite>,
    config: &AppConfig,
    dry_run: bool,
) -> Result<(usize, usize)> {
    let cutoff = shanghai_utc_cutoff_string(config.quota_snapshot_full_days);
    let spec = archive_table_spec("codex_quota_snapshots");

    if dry_run {
        return dry_run_quota_snapshot_compaction(pool, &spec, &cutoff).await;
    }

    let mut rows_archived = 0usize;
    let mut archive_batches = 0usize;

    loop {
        let candidate_limit =
            super::super::retention::retention_candidate_limit(config, "quota_compaction");
        let candidates = sqlx::query_as::<_, TimestampedArchiveCandidate>(
            r#"
            WITH ranked AS (
                SELECT
                    id,
                    captured_at AS timestamp_value,
                    ROW_NUMBER() OVER (
                        PARTITION BY strftime('%Y-%m-%d', datetime(captured_at, '+8 hours'))
                        ORDER BY captured_at DESC, id DESC
                    ) AS row_num
                FROM codex_quota_snapshots
                WHERE captured_at < ?1
            )
            SELECT id, timestamp_value
            FROM ranked
            WHERE row_num > 1
            ORDER BY timestamp_value ASC, id ASC
            LIMIT ?2
            "#,
        )
        .bind(&cutoff)
        .bind(candidate_limit as i64)
        .fetch_all(pool)
        .await?;

        if candidates.is_empty() {
            break;
        }

        let candidate_remaining_hint = usize::from(candidates.len() >= candidate_limit);
        let mut by_month: BTreeMap<String, Vec<TimestampedArchiveCandidate>> = BTreeMap::new();
        for candidate in candidates {
            let month_key = shanghai_month_key_from_utc_naive(&candidate.timestamp_value)?;
            by_month.entry(month_key).or_default().push(candidate);
        }

        let (archived, batches) = archive_quota_snapshot_month_groups(
            pool,
            config,
            spec,
            by_month,
            candidate_remaining_hint,
        )
        .await?;
        rows_archived += archived;
        archive_batches += batches;
    }

    Ok((rows_archived, archive_batches))
}

async fn archive_quota_snapshot_month_groups(
    pool: &Pool<Sqlite>,
    config: &AppConfig,
    spec: ArchiveTableSpec,
    by_month: BTreeMap<String, Vec<TimestampedArchiveCandidate>>,
    candidate_remaining_hint: usize,
) -> Result<(usize, usize)> {
    let mut rows_archived = 0usize;
    let mut archive_batches = 0usize;
    for (month_key, group) in by_month {
        let group = super::super::retention::take_retention_micro_batch(group, |_| 256);
        let prepare_started = Instant::now();
        let ids = group
            .iter()
            .map(|candidate| candidate.id)
            .collect::<Vec<_>>();
        let Some(mut archive_outcome) =
            super::super::retention::retention_prepared_batch_or_deferred(
                archive_rows_into_month_batch(pool, config, spec, &month_key, &ids).await,
            )?
        else {
            return Ok((rows_archived, archive_batches));
        };
        set_archive_batch_coverage_from_utc_rows(
            &mut archive_outcome,
            group
                .iter()
                .map(|candidate| candidate.timestamp_value.as_str()),
        )?;
        let prepare_elapsed = prepare_started.elapsed();
        let Some(admission) =
            super::super::retention::acquire_retention_write_admission("quota_compaction").await
        else {
            return Ok((rows_archived, archive_batches));
        };
        let execute_started = Instant::now();
        let mut tx = pool.begin().await?;
        upsert_archive_batch_manifest(tx.as_mut(), &archive_outcome).await?;
        delete_rows_by_ids(tx.as_mut(), spec.dataset, &ids).await?;
        let commit_started = Instant::now();
        tx.commit().await?;
        super::super::retention::retention_record_commit!(
            "quota_compaction",
            admission.admission_mode(),
            group.len(),
            group.len().saturating_mul(256),
            prepare_elapsed,
            admission.lock_wait(),
            commit_started.duration_since(execute_started),
            commit_started.elapsed(),
            admission.p1_waiter_count(),
            candidate_remaining_hint,
        );
        drop(admission);
        rows_archived += group.len();
        archive_batches += 1;
    }
    Ok((rows_archived, archive_batches))
}

async fn dry_run_quota_snapshot_compaction(
    pool: &Pool<Sqlite>,
    spec: &ArchiveTableSpec,
    cutoff: &str,
) -> Result<(usize, usize)> {
    let batch_counts = sqlx::query_as::<_, DryRunBatchCount>(
        r#"
        WITH ranked AS (
            SELECT captured_at,
                   ROW_NUMBER() OVER (
                       PARTITION BY strftime('%Y-%m-%d', datetime(captured_at, '+8 hours'))
                       ORDER BY captured_at DESC, id DESC
                   ) AS row_num
            FROM codex_quota_snapshots
            WHERE captured_at < ?1
        )
        SELECT strftime('%Y-%m', datetime(captured_at, '+8 hours')) AS month_key,
               COUNT(*) AS row_count
        FROM ranked
        WHERE row_num > 1
        GROUP BY 1
        ORDER BY 1
        "#,
    )
    .bind(cutoff)
    .fetch_all(pool)
    .await?;
    for batch in &batch_counts {
        info!(
            dataset = spec.dataset,
            month_key = %batch.month_key,
            rows = batch.row_count,
            "retention dry-run planned quota compaction batch"
        );
    }
    Ok((
        batch_counts
            .iter()
            .map(|batch| batch.row_count as usize)
            .sum(),
        batch_counts.len(),
    ))
}
