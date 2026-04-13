use super::*;

pub(crate) async fn backfill_invocation_archive_expiries(
    pool: &Pool<Sqlite>,
    config: &AppConfig,
) -> Result<usize> {
    let candidates = sqlx::query_as::<_, ArchiveExpiryBackfillCandidate>(
        r#"
        SELECT id, coverage_end_at
        FROM archive_batches
        WHERE dataset = 'codex_invocations'
          AND status = ?1
          AND coverage_end_at IS NOT NULL
          AND archive_expires_at IS NULL
          AND historical_rollups_materialized_at IS NOT NULL
        "#,
    )
    .bind(ARCHIVE_STATUS_COMPLETED)
    .fetch_all(pool)
    .await?;

    let mut updated = 0usize;
    for candidate in candidates {
        let archive_expires_at = shanghai_archive_expiry_from_reference_timestamp(
            &candidate.coverage_end_at,
            config.invocation_archive_ttl_days,
        )?;
        sqlx::query("UPDATE archive_batches SET archive_expires_at = ?1 WHERE id = ?2")
            .bind(archive_expires_at)
            .bind(candidate.id)
            .execute(pool)
            .await?;
        updated += 1;
    }
    Ok(updated)
}

pub(crate) fn classify_raw_compression_alert(
    oldest_uncompressed_age_secs: u64,
    uncompressed_bytes: u64,
) -> RawCompressionAlertLevel {
    const GIB: u64 = 1024 * 1024 * 1024;
    if oldest_uncompressed_age_secs >= 48 * 3600 || uncompressed_bytes >= 20 * GIB {
        RawCompressionAlertLevel::Critical
    } else if oldest_uncompressed_age_secs >= 24 * 3600 || uncompressed_bytes >= 10 * GIB {
        RawCompressionAlertLevel::Warn
    } else {
        RawCompressionAlertLevel::Ok
    }
}

pub(crate) fn invocation_status_counts_toward_terminal_totals(status: Option<&str>) -> bool {
    let normalized_status = status.map(str::trim).unwrap_or_default();
    !normalized_status.eq_ignore_ascii_case("running")
        && !normalized_status.eq_ignore_ascii_case("pending")
}

pub(crate) async fn load_raw_compression_backlog_snapshot(
    pool: &Pool<Sqlite>,
    config: &AppConfig,
) -> Result<RawCompressionBacklogSnapshot> {
    let cutoff = shanghai_local_cutoff_for_age_secs_string(config.proxy_raw_hot_secs);
    let prune_cutoff = shanghai_local_cutoff_string(config.invocation_success_full_days);
    let archive_cutoff = shanghai_local_cutoff_string(config.invocation_max_days);
    let row = sqlx::query_as::<_, RawCompressionBacklogAggRow>(
        r#"
        SELECT
            COUNT(*) AS uncompressed_count,
            COALESCE(SUM(raw_size), 0) AS uncompressed_bytes,
            MIN(occurred_at) AS oldest_occurred_at
        FROM (
            SELECT occurred_at, COALESCE(request_raw_size, 0) AS raw_size
            FROM codex_invocations
            WHERE occurred_at < ?1
              AND occurred_at >= ?2
              AND (
                status != 'success'
                OR detail_level IS NULL
                OR detail_level != ?3
                OR occurred_at >= ?4
              )
              AND request_raw_path IS NOT NULL
              AND request_raw_codec = 'identity'
            UNION ALL
            SELECT occurred_at, COALESCE(response_raw_size, 0) AS raw_size
            FROM codex_invocations
            WHERE occurred_at < ?1
              AND occurred_at >= ?2
              AND (
                status != 'success'
                OR detail_level IS NULL
                OR detail_level != ?3
                OR occurred_at >= ?4
              )
              AND response_raw_path IS NOT NULL
              AND response_raw_codec = 'identity'
        )
        "#,
    )
    .bind(&cutoff)
    .bind(&archive_cutoff)
    .bind(DETAIL_LEVEL_FULL)
    .bind(&prune_cutoff)
    .fetch_one(pool)
    .await?;

    let oldest_uncompressed_age_secs = row
        .oldest_occurred_at
        .as_deref()
        .map(parse_shanghai_local_naive)
        .transpose()?
        .map(|oldest| {
            let now = Utc::now().with_timezone(&Shanghai).naive_local();
            now.signed_duration_since(oldest).num_seconds().max(0) as u64
        })
        .unwrap_or_default();
    let uncompressed_count = row.uncompressed_count.max(0) as u64;
    let uncompressed_bytes = row.uncompressed_bytes.unwrap_or_default().max(0) as u64;
    let alert_level =
        classify_raw_compression_alert(oldest_uncompressed_age_secs, uncompressed_bytes);
    Ok(RawCompressionBacklogSnapshot {
        oldest_uncompressed_age_secs,
        uncompressed_count,
        uncompressed_bytes,
        alert_level,
    })
}

pub(crate) async fn log_raw_compression_backlog_if_needed(
    pool: &Pool<Sqlite>,
    config: &AppConfig,
) -> Result<()> {
    let snapshot = load_raw_compression_backlog_snapshot(pool, config).await?;
    match snapshot.alert_level {
        RawCompressionAlertLevel::Ok => {}
        RawCompressionAlertLevel::Warn => {
            warn!(
                oldest_uncompressed_age_secs = snapshot.oldest_uncompressed_age_secs,
                uncompressed_count = snapshot.uncompressed_count,
                uncompressed_bytes = snapshot.uncompressed_bytes,
                alert_level = "warn",
                "raw compression backlog is above warning threshold"
            );
        }
        RawCompressionAlertLevel::Critical => {
            error!(
                oldest_uncompressed_age_secs = snapshot.oldest_uncompressed_age_secs,
                uncompressed_count = snapshot.uncompressed_count,
                uncompressed_bytes = snapshot.uncompressed_bytes,
                alert_level = "critical",
                "raw compression backlog is above critical threshold"
            );
        }
    }
    Ok(())
}

pub(crate) fn archive_file_is_stale_temp(path: &Path) -> bool {
    path.file_name()
        .and_then(|value| value.to_str())
        .map(is_archive_temp_file_name)
        .unwrap_or(false)
}

pub(crate) fn archive_temp_file_is_old_enough(path: &Path) -> bool {
    fs::metadata(path)
        .ok()
        .and_then(|metadata| metadata.modified().ok())
        .and_then(|modified| modified.elapsed().ok())
        .map(|age| age.as_secs() >= DEFAULT_ARCHIVE_TEMP_MIN_AGE_SECS)
        .unwrap_or(false)
}

pub(crate) fn archive_file_size(path: &Path) -> u64 {
    fs::metadata(path)
        .map(|metadata| metadata.len())
        .unwrap_or_default()
}

pub(crate) fn cleanup_stale_archive_temp_files(
    config: &AppConfig,
    dry_run: bool,
) -> Result<ArchiveTempCleanupSummary> {
    let archive_root = resolved_archive_dir(config);
    let mut files = Vec::new();
    collect_archive_file_paths(&archive_root, &mut files)?;
    let mut summary = ArchiveTempCleanupSummary::default();
    for file_path in files {
        if !archive_file_is_stale_temp(&file_path) || !archive_temp_file_is_old_enough(&file_path) {
            continue;
        }
        let file_size = archive_file_size(&file_path);
        if dry_run {
            summary.stale_temp_files_removed += 1;
            summary.stale_temp_bytes_removed += file_size;
            continue;
        }
        match fs::remove_file(&file_path) {
            Ok(_) => {
                summary.stale_temp_files_removed += 1;
                summary.stale_temp_bytes_removed += file_size;
            }
            Err(err) if err.kind() == io::ErrorKind::NotFound => {}
            Err(err) => {
                warn!(
                    file_path = %file_path.display(),
                    error = %err,
                    "failed to remove stale archive temp file"
                );
            }
        }
    }
    Ok(summary)
}

pub(crate) async fn verify_archive_storage(
    pool: &Pool<Sqlite>,
    config: &AppConfig,
) -> Result<ArchiveStorageVerificationSummary> {
    let manifest_rows = sqlx::query_as::<_, ArchiveStorageManifestRow>(
        r#"
        SELECT id, dataset, layout, file_path
        FROM archive_batches
        WHERE status = ?1
        "#,
    )
    .bind(ARCHIVE_STATUS_COMPLETED)
    .fetch_all(pool)
    .await?;
    let mut summary = ArchiveStorageVerificationSummary {
        manifest_rows: manifest_rows.len(),
        ..ArchiveStorageVerificationSummary::default()
    };
    let referenced_paths = manifest_rows
        .iter()
        .map(|row| normalize_path_for_compare(Path::new(&row.file_path)))
        .collect::<HashSet<_>>();
    for row in &manifest_rows {
        if !Path::new(&row.file_path).exists() {
            summary.missing_files += 1;
            warn!(
                archive_batch_id = row.id,
                dataset = row.dataset,
                layout = row.layout,
                file_path = row.file_path,
                "archive manifest points to a missing file"
            );
        }
    }

    let archive_root = resolved_archive_dir(config);
    let mut files = Vec::new();
    collect_archive_file_paths(&archive_root, &mut files)?;
    for file_path in files {
        let normalized = normalize_path_for_compare(&file_path);
        if archive_file_is_stale_temp(&file_path) {
            summary.stale_temp_files += 1;
            summary.stale_temp_bytes += archive_file_size(&file_path);
            continue;
        }
        if !referenced_paths.contains(&normalized) {
            summary.orphan_files += 1;
        }
    }
    Ok(summary)
}

#[derive(Debug, FromRow)]
struct ArchiveBatchCleanupCandidate {
    id: i64,
    dataset: String,
    file_path: String,
    historical_rollups_materialized_at: Option<String>,
    coverage_end_at: Option<String>,
}

pub(crate) async fn cleanup_expired_archive_batches(
    pool: &Pool<Sqlite>,
    config: &AppConfig,
    dry_run: bool,
) -> Result<usize> {
    if !dry_run {
        backfill_invocation_archive_expiries(pool, config).await?;
    }
    let cutoff = format_naive(Utc::now().with_timezone(&Shanghai).naive_local());
    let invocation_archive_cutoff = shanghai_local_cutoff_string(config.invocation_max_days);
    let candidates = sqlx::query_as::<_, ArchiveBatchCleanupCandidate>(
        r#"
        SELECT id, dataset, file_path, historical_rollups_materialized_at, coverage_end_at
        FROM archive_batches
        WHERE status = ?1
          AND archive_expires_at IS NOT NULL
          AND archive_expires_at < ?2
        ORDER BY archive_expires_at ASC, id ASC
        "#,
    )
    .bind(ARCHIVE_STATUS_COMPLETED)
    .bind(&cutoff)
    .fetch_all(pool)
    .await?;

    let mut eligible_candidates = Vec::new();
    for candidate in candidates {
        if HISTORICAL_ROLLUP_ARCHIVE_DATASETS.contains(&candidate.dataset.as_str())
            && candidate.historical_rollups_materialized_at.is_none()
        {
            continue;
        }
        if candidate.dataset == HOURLY_ROLLUP_DATASET_INVOCATIONS {
            if candidate
                .coverage_end_at
                .as_deref()
                .map(|coverage_end_at| coverage_end_at >= invocation_archive_cutoff.as_str())
                .unwrap_or(true)
            {
                continue;
            }
        }
        eligible_candidates.push(candidate);
    }

    if dry_run {
        for candidate in &eligible_candidates {
            info!(
                dataset = candidate.dataset,
                file_path = candidate.file_path,
                "retention dry-run planned archive batch cleanup"
            );
        }
        return Ok(eligible_candidates.len());
    }

    let mut deleted = 0usize;
    for candidate in eligible_candidates {
        match fs::remove_file(&candidate.file_path) {
            Ok(_) => {}
            Err(err) if err.kind() == io::ErrorKind::NotFound => {}
            Err(err) => {
                warn!(
                    dataset = candidate.dataset,
                    file_path = candidate.file_path,
                    error = %err,
                    "failed to remove expired archive batch file; keeping manifest"
                );
                continue;
            }
        }

        let mut tx = pool.begin().await?;
        sqlx::query("DELETE FROM archive_batch_upstream_activity WHERE archive_batch_id = ?1")
            .bind(candidate.id)
            .execute(tx.as_mut())
            .await?;
        sqlx::query("DELETE FROM archive_batches WHERE id = ?1")
            .bind(candidate.id)
            .execute(tx.as_mut())
            .await?;
        sqlx::query(
            "DELETE FROM hourly_rollup_archive_replay WHERE dataset = ?1 AND file_path = ?2",
        )
        .bind(&candidate.dataset)
        .bind(&candidate.file_path)
        .execute(tx.as_mut())
        .await?;
        tx.commit().await?;
        deleted += 1;
    }

    Ok(deleted)
}

#[derive(Debug, FromRow)]
struct HistoricalRollupPendingArchiveBatchRow {
    dataset: String,
    month_key: String,
    file_path: String,
    coverage_start_at: Option<String>,
    coverage_end_at: Option<String>,
}

#[derive(Debug, FromRow)]
struct LegacyArchivePruneCandidateRow {
    id: i64,
    dataset: String,
    file_path: String,
    historical_rollups_materialized_at: Option<String>,
    coverage_end_at: Option<String>,
}

fn estimate_historical_rollup_pending_bucket_count(
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

pub(crate) async fn count_materialized_historical_rollup_buckets(pool: &Pool<Sqlite>) -> Result<i64> {
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
    let last_materialized_hour =
        load_latest_materialized_legacy_invocation_rollup_bucket_epoch(pool, config)
            .await?
            .and_then(|epoch| Utc.timestamp_opt(epoch, 0).single())
            .map(format_utc_iso);
    let alert_level = if legacy_archive_pending == 0 {
        HistoricalRollupBackfillAlertLevel::None
    } else if legacy_invocation_pending {
        HistoricalRollupBackfillAlertLevel::Critical
    } else {
        HistoricalRollupBackfillAlertLevel::Warn
    };

    Ok(HistoricalRollupBackfillSnapshot {
        pending_buckets,
        legacy_archive_pending,
        last_materialized_hour,
        alert_level,
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
    let started_at = Instant::now();
    let scanned_archive_batches = count_historical_rollup_archive_batches(pool, false).await?;
    let pending_snapshot = load_historical_rollup_backfill_snapshot(pool, config).await?;
    if dry_run {
        return Ok(HistoricalRollupMaterializationSummary {
            scanned_archive_batches: scanned_archive_batches as usize,
            materialized_archive_batches: pending_snapshot.legacy_archive_pending as usize,
            materialized_bucket_count: pending_snapshot.pending_buckets as usize,
            materialized_invocation_batches: 0,
            materialized_forward_proxy_batches: 0,
            last_materialized_bucket_start_epoch:
                load_latest_materialized_legacy_invocation_rollup_bucket_epoch(pool, config).await?,
        });
    }

    let mut tx = pool.begin().await?;
    let materialized_invocation_batches = replay_invocation_archives_into_hourly_rollups_tx_with_limits(
        tx.as_mut(),
        max_archive_batches,
        max_elapsed,
    )
    .await?;
    let remaining_budget =
        historical_rollup_materialization_remaining_budget(started_at, max_elapsed);
    let materialized_forward_proxy_batches =
        replay_forward_proxy_archives_into_hourly_rollups_tx_with_limits(
            tx.as_mut(),
            max_archive_batches
                .map(|limit| limit.saturating_sub(materialized_invocation_batches)),
            remaining_budget,
        )
        .await?;
    loop {
        let updated = replay_live_invocation_hourly_rollups_tx(tx.as_mut()).await?;
        if updated == 0 {
            break;
        }
    }
    loop {
        let updated = replay_live_forward_proxy_attempt_hourly_rollups_tx(tx.as_mut()).await?;
        if updated == 0 {
            break;
        }
    }
    tx.commit().await?;

    Ok(HistoricalRollupMaterializationSummary {
        scanned_archive_batches: scanned_archive_batches as usize,
        materialized_archive_batches: (materialized_invocation_batches
            + materialized_forward_proxy_batches) as usize,
        materialized_bucket_count: count_materialized_historical_rollup_buckets(pool).await?
            as usize,
        materialized_invocation_batches: materialized_invocation_batches as usize,
        materialized_forward_proxy_batches: materialized_forward_proxy_batches as usize,
        last_materialized_bucket_start_epoch:
            load_latest_materialized_legacy_invocation_rollup_bucket_epoch(pool, config).await?,
    })
}

pub(crate) async fn prune_legacy_archive_batches(
    pool: &Pool<Sqlite>,
    config: &AppConfig,
    dry_run: bool,
) -> Result<LegacyArchivePruneSummary> {
    let mut query = QueryBuilder::<Sqlite>::new(
        "SELECT id, dataset, file_path, historical_rollups_materialized_at, coverage_end_at \
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

    let pending_account_count = count_upstream_accounts_missing_last_activity(pool).await?;
    let invocation_archive_cutoff = shanghai_local_cutoff_string(config.invocation_max_days);
    let mut summary = LegacyArchivePruneSummary {
        scanned_archive_batches: candidates.len(),
        ..LegacyArchivePruneSummary::default()
    };

    for candidate in candidates {
        let file_missing = !Path::new(&candidate.file_path).exists();

        if candidate.dataset == HOURLY_ROLLUP_DATASET_INVOCATIONS && pending_account_count > 0 {
            summary.skipped_unmaterialized_batches += 1;
            continue;
        }

        if !file_missing && candidate.historical_rollups_materialized_at.is_none() {
            summary.skipped_unmaterialized_batches += 1;
            continue;
        }

        if candidate.dataset == HOURLY_ROLLUP_DATASET_INVOCATIONS
            && candidate
                .coverage_end_at
                .as_deref()
                .map(|coverage_end_at| coverage_end_at >= invocation_archive_cutoff.as_str())
                .unwrap_or(true)
        {
            summary.skipped_retained_batches += 1;
            continue;
        }

        if dry_run {
            info!(
                dataset = candidate.dataset,
                file_path = candidate.file_path,
                "maintenance dry-run planned legacy archive prune"
            );
            summary.deleted_archive_batches += 1;
            continue;
        }

        match fs::remove_file(&candidate.file_path) {
            Ok(_) => {}
            Err(err) if err.kind() == io::ErrorKind::NotFound => {}
            Err(err) => {
                warn!(
                    dataset = candidate.dataset,
                    file_path = candidate.file_path,
                    error = %err,
                    "failed to remove legacy archive batch file; keeping metadata"
                );
                summary.skipped_unmaterialized_batches += 1;
                continue;
            }
        }

        let mut tx = pool.begin().await?;
        sqlx::query("DELETE FROM archive_batch_upstream_activity WHERE archive_batch_id = ?1")
            .bind(candidate.id)
            .execute(tx.as_mut())
            .await?;
        sqlx::query(
            "DELETE FROM hourly_rollup_archive_replay WHERE dataset = ?1 AND file_path = ?2",
        )
        .bind(&candidate.dataset)
        .bind(&candidate.file_path)
        .execute(tx.as_mut())
        .await?;
        sqlx::query("DELETE FROM archive_batches WHERE id = ?1")
            .bind(candidate.id)
            .execute(tx.as_mut())
            .await?;
        tx.commit().await?;
        summary.deleted_archive_batches += 1;
    }

    Ok(summary)
}

fn historical_rollup_materialization_remaining_budget(
    started_at: Instant,
    max_elapsed: Option<Duration>,
) -> Option<Duration> {
    max_elapsed.map(|limit| limit.saturating_sub(started_at.elapsed()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{thread, time::Duration};

    #[test]
    fn historical_rollup_materialization_remaining_budget_clamps_to_zero_when_elapsed() {
        let started_at = Instant::now();
        thread::sleep(Duration::from_millis(10));

        let remaining = historical_rollup_materialization_remaining_budget(
            started_at,
            Some(Duration::from_millis(1)),
        );

        assert_eq!(remaining, Some(Duration::ZERO));
    }

    #[test]
    fn historical_rollup_materialization_remaining_budget_preserves_unbounded_mode() {
        assert_eq!(
            historical_rollup_materialization_remaining_budget(Instant::now(), None),
            None
        );
    }
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
        let batch_counts = sqlx::query_as::<_, DryRunBatchCount>(
            r#"
            WITH ranked AS (
                SELECT
                    captured_at,
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
        .bind(&cutoff)
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
        return Ok((
            batch_counts
                .iter()
                .map(|batch| batch.row_count as usize)
                .sum(),
            batch_counts.len(),
        ));
    }

    let mut rows_archived = 0usize;
    let mut archive_batches = 0usize;

    loop {
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
        .bind(config.retention_batch_rows as i64)
        .fetch_all(pool)
        .await?;

        if candidates.is_empty() {
            break;
        }

        let mut by_month: BTreeMap<String, Vec<TimestampedArchiveCandidate>> = BTreeMap::new();
        for candidate in candidates {
            let month_key = shanghai_month_key_from_utc_naive(&candidate.timestamp_value)?;
            by_month.entry(month_key).or_default().push(candidate);
        }

        for (month_key, group) in by_month {
            rows_archived += group.len();
            archive_batches += 1;
            let ids = group
                .iter()
                .map(|candidate| candidate.id)
                .collect::<Vec<_>>();
            let mut archive_outcome =
                archive_rows_into_month_batch(pool, config, spec, &month_key, &ids).await?;
            set_archive_batch_coverage_from_utc_rows(
                &mut archive_outcome,
                group
                    .iter()
                    .map(|candidate| candidate.timestamp_value.as_str()),
            )?;
            let mut tx = pool.begin().await?;
            upsert_archive_batch_manifest(tx.as_mut(), &archive_outcome).await?;
            delete_rows_by_ids(tx.as_mut(), spec.dataset, &ids).await?;
            tx.commit().await?;
        }
    }

    Ok((rows_archived, archive_batches))
}
