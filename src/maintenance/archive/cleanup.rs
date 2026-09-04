use super::*;
use futures_util::{StreamExt, stream};
use sqlx::FromRow;
use tracing::warn;

pub(crate) async fn backfill_invocation_archive_expiries(
    pool: &Pool<Sqlite>,
    config: &AppConfig,
) -> Result<usize> {
    let candidate_limit =
        super::super::retention::retention_candidate_limit(config, "archive_expiry_backfill");
    let mut candidates = sqlx::query_as::<_, ArchiveExpiryBackfillCandidate>(
        r#"
        SELECT id, coverage_end_at
        FROM archive_batches
        WHERE dataset = 'codex_invocations'
          AND status = ?1
          AND coverage_end_at IS NOT NULL
          AND archive_expires_at IS NULL
          AND historical_rollups_materialized_at IS NOT NULL
        ORDER BY id ASC
        LIMIT ?2
        "#,
    )
    .bind(ARCHIVE_STATUS_COMPLETED)
    .bind(candidate_limit.saturating_add(1) as i64)
    .fetch_all(pool)
    .await?;

    let candidate_remaining_hint = usize::from(candidates.len() > candidate_limit);
    candidates.truncate(candidate_limit);
    let candidate_count = candidates.len();
    let mut updated = 0usize;
    for candidate in candidates {
        let archive_expires_at = shanghai_archive_expiry_from_reference_timestamp(
            &candidate.coverage_end_at,
            config.invocation_archive_ttl_days,
        )?;
        let Some(admission) =
            super::super::retention::acquire_retention_write_admission("archive_expiry_backfill")
                .await
        else {
            break;
        };
        let execute_started = Instant::now();
        sqlx::query("UPDATE archive_batches SET archive_expires_at = ?1 WHERE id = ?2")
            .bind(archive_expires_at)
            .bind(candidate.id)
            .execute(pool)
            .await?;
        super::super::retention::retention_record_commit!(
            "archive_expiry_backfill",
            admission.admission_mode(),
            1,
            128,
            Duration::ZERO,
            admission.lock_wait(),
            execute_started.elapsed(),
            Duration::ZERO,
            admission.p1_waiter_count(),
            candidate_remaining_hint.max(usize::from(updated.saturating_add(1) < candidate_count)),
        );
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
pub(crate) struct ArchiveBatchCleanupCandidate {
    id: i64,
    dataset: String,
    file_path: String,
    sha256: String,
    cleanup_state: String,
    historical_rollups_materialized_at: Option<String>,
    coverage_end_at: Option<String>,
}

fn archive_file_is_confirmed_missing(file_path: &str) -> bool {
    matches!(
        fs::metadata(file_path),
        Err(error) if error.kind() == io::ErrorKind::NotFound
    )
}

async fn stage_archive_batch_deletion(
    pool: &Pool<Sqlite>,
    archive_batch_id: i64,
    dataset: &str,
    file_path: &str,
    expected_sha256: &str,
    source_safe_start: Option<NaiveDate>,
) -> Result<bool> {
    let Some(admission) =
        super::super::retention::acquire_retention_write_admission("archive_cleanup_stage").await
    else {
        return Ok(false);
    };
    let execute_started = Instant::now();
    let mut tx = pool.begin().await?;
    let staged = sqlx::query(
        r#"
        UPDATE archive_batches
        SET cleanup_state = ?1,
            cleanup_source_safe_start_date = ?2
        WHERE id = ?3
          AND dataset = ?4
          AND file_path = ?5
          AND sha256 = ?6
          AND status = ?7
          AND cleanup_state = ?8
        "#,
    )
    .bind(ARCHIVE_CLEANUP_STATE_DELETE_PENDING)
    .bind(source_safe_start.map(|date| date.to_string()))
    .bind(archive_batch_id)
    .bind(dataset)
    .bind(file_path)
    .bind(expected_sha256)
    .bind(ARCHIVE_STATUS_COMPLETED)
    .bind(ARCHIVE_CLEANUP_STATE_ACTIVE)
    .execute(tx.as_mut())
    .await?
    .rows_affected()
        != 0;
    if !staged {
        tx.rollback().await?;
        return Ok(false);
    }
    let commit_started = Instant::now();
    tx.commit().await?;
    super::super::retention::retention_record_commit!(
        "archive_cleanup_stage",
        admission.admission_mode(),
        1,
        512,
        Duration::ZERO,
        admission.lock_wait(),
        commit_started.duration_since(execute_started),
        commit_started.elapsed(),
        admission.p1_waiter_count(),
        0,
    );
    Ok(true)
}

async fn delete_archive_batch_metadata_tx(
    tx: &mut SqliteConnection,
    archive_batch_id: i64,
    dataset: &str,
    file_path: &str,
    expected_sha256: &str,
) -> Result<bool> {
    let still_pending = sqlx::query_scalar::<_, i64>(
        r#"
        SELECT EXISTS(
            SELECT 1
            FROM archive_batches
            WHERE id = ?1
              AND dataset = ?2
              AND file_path = ?3
              AND sha256 = ?4
              AND status = ?5
              AND cleanup_state = ?6
        )
        "#,
    )
    .bind(archive_batch_id)
    .bind(dataset)
    .bind(file_path)
    .bind(expected_sha256)
    .bind(ARCHIVE_STATUS_COMPLETED)
    .bind(ARCHIVE_CLEANUP_STATE_DELETE_PENDING)
    .fetch_one(&mut *tx)
    .await?
        != 0;
    if !still_pending {
        return Ok(false);
    }
    sqlx::query("DELETE FROM archive_batch_upstream_activity WHERE archive_batch_id = ?1")
        .bind(archive_batch_id)
        .execute(&mut *tx)
        .await?;
    delete_pool_upstream_node_health_archive_rows_for_file_tx(&mut *tx, file_path).await?;
    sqlx::query("DELETE FROM hourly_rollup_archive_replay WHERE dataset = ?1 AND file_path = ?2")
        .bind(dataset)
        .bind(file_path)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM hourly_rollup_archive_progress WHERE dataset = ?1 AND file_path = ?2")
        .bind(dataset)
        .bind(file_path)
        .execute(&mut *tx)
        .await?;
    let deleted = sqlx::query(
        r#"
        DELETE FROM archive_batches
        WHERE id = ?1
          AND dataset = ?2
          AND file_path = ?3
          AND sha256 = ?4
          AND status = ?5
          AND cleanup_state = ?6
        "#,
    )
    .bind(archive_batch_id)
    .bind(dataset)
    .bind(file_path)
    .bind(expected_sha256)
    .bind(ARCHIVE_STATUS_COMPLETED)
    .bind(ARCHIVE_CLEANUP_STATE_DELETE_PENDING)
    .execute(&mut *tx)
    .await?
    .rows_affected()
        != 0;
    Ok(deleted)
}

async fn finalize_archive_batch_file_deletion(
    pool: &Pool<Sqlite>,
    archive_batch_id: i64,
    dataset: &str,
    file_path: &str,
    expected_sha256: &str,
) -> Result<bool> {
    finalize_archive_batch_file_deletion_with_remove(
        pool,
        archive_batch_id,
        dataset,
        file_path,
        expected_sha256,
        |path| fs::remove_file(path),
    )
    .await
}

async fn finalize_archive_batch_file_deletion_with_remove<F>(
    pool: &Pool<Sqlite>,
    archive_batch_id: i64,
    dataset: &str,
    file_path: &str,
    expected_sha256: &str,
    remove_file: F,
) -> Result<bool>
where
    F: FnOnce(&str) -> io::Result<()>,
{
    // Take the SQLite writer lock before touching the file. Legacy writers reactivate a pending
    // manifest and rename its file under the same lock, so they either win before this check or
    // wait until this identity has been fully finalized.
    let Some(admission) =
        super::super::retention::acquire_retention_write_admission("archive_cleanup_finalize")
            .await
    else {
        return Ok(false);
    };
    let execute_started = Instant::now();
    let mut tx = pool.begin_with("BEGIN IMMEDIATE").await?;
    let staged_source_safe_start = sqlx::query_scalar::<_, Option<String>>(
        r#"
        SELECT cleanup_source_safe_start_date
        FROM archive_batches
        WHERE id = ?1
          AND dataset = ?2
          AND file_path = ?3
          AND sha256 = ?4
          AND status = ?5
          AND cleanup_state = ?6
        LIMIT 1
        "#,
    )
    .bind(archive_batch_id)
    .bind(dataset)
    .bind(file_path)
    .bind(expected_sha256)
    .bind(ARCHIVE_STATUS_COMPLETED)
    .bind(ARCHIVE_CLEANUP_STATE_DELETE_PENDING)
    .fetch_optional(tx.as_mut())
    .await?;
    let Some(staged_source_safe_start) = staged_source_safe_start else {
        tx.rollback().await?;
        return Ok(false);
    };
    let source_safe_start = match staged_source_safe_start {
        Some(value) => match NaiveDate::parse_from_str(&value, "%Y-%m-%d") {
            Ok(value) => Some(value),
            Err(error) => {
                warn!(
                    dataset,
                    file_path,
                    cleanup_source_safe_start_date = value,
                    error = %error,
                    "archive cleanup source boundary is invalid; retaining pending metadata"
                );
                tx.rollback().await?;
                return Ok(false);
            }
        },
        None => None,
    };

    if Path::new(file_path).exists() {
        let file_sha256 = match sha256_hex_file(Path::new(file_path)) {
            Ok(value) => value,
            Err(error) => {
                warn!(
                    dataset,
                    file_path,
                    error = %error,
                    "archive file identity could not be verified; retaining pending metadata"
                );
                tx.rollback().await?;
                return Ok(false);
            }
        };
        if file_sha256 != expected_sha256 {
            warn!(
                dataset,
                file_path,
                expected_sha256,
                file_sha256,
                "archive file identity changed after deletion was staged; retaining reactivated manifest"
            );
            tx.rollback().await?;
            return Ok(false);
        }
    }

    match remove_file(file_path) {
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => {
            warn!(
                dataset,
                file_path,
                error = %error,
                "archive file deletion is pending; retaining metadata for a later retry"
            );
            tx.rollback().await?;
            return Ok(false);
        }
    }
    if let Some(source_safe_start) = source_safe_start {
        crate::long_term_stats::advance_long_term_integrity_source_start_tx(
            tx.as_mut(),
            archive_batch_id,
            source_safe_start,
        )
        .await?;
    }
    if !delete_archive_batch_metadata_tx(
        tx.as_mut(),
        archive_batch_id,
        dataset,
        file_path,
        expected_sha256,
    )
    .await?
    {
        tx.rollback().await?;
        return Ok(false);
    }
    let commit_started = Instant::now();
    tx.commit().await?;
    super::super::retention::retention_record_commit!(
        "archive_cleanup_finalize",
        admission.admission_mode(),
        1,
        1024,
        Duration::ZERO,
        admission.lock_wait(),
        commit_started.duration_since(execute_started),
        commit_started.elapsed(),
        admission.p1_waiter_count(),
        0,
    );
    Ok(true)
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
    let owner_facing_node_health_window_cutoff = shanghai_local_cutoff_string(7);
    let candidates = sqlx::query_as::<_, ArchiveBatchCleanupCandidate>(
        r#"
        SELECT id, dataset, file_path, sha256, cleanup_state, historical_rollups_materialized_at, coverage_end_at
        FROM archive_batches
        WHERE status = ?1
          AND archive_expires_at IS NOT NULL
          AND archive_expires_at < ?2
        ORDER BY archive_expires_at ASC, id ASC
        LIMIT ?3
        "#,
    )
    .bind(ARCHIVE_STATUS_COMPLETED)
    .bind(&cutoff)
    .bind(super::super::retention::retention_candidate_limit(
        config,
        "archive_cleanup",
    ) as i64)
    .fetch_all(pool)
    .await?;
    let materialized_pool_upstream_cache_files = sqlx::query_scalar::<_, String>(
        r#"
        SELECT file_path
        FROM hourly_rollup_archive_replay
        WHERE target = ?1
          AND dataset = 'pool_upstream_request_attempts'
        "#,
    )
    .bind(POOL_UPSTREAM_NODE_HEALTH_ARCHIVE_REPLAY_TARGET)
    .fetch_all(pool)
    .await?
    .into_iter()
    .collect::<HashSet<_>>();
    let materialized_pool_upstream_hourly_files = sqlx::query_scalar::<_, String>(
        r#"
        SELECT file_path
        FROM hourly_rollup_archive_replay
        WHERE target = ?1
          AND dataset = 'pool_upstream_request_attempts'
        "#,
    )
    .bind(POOL_UPSTREAM_NODE_HEALTH_HOURLY_ARCHIVE_REPLAY_TARGET)
    .fetch_all(pool)
    .await?
    .into_iter()
    .collect::<HashSet<_>>();
    let long_term_stats_status =
        sqlx::query_scalar::<_, String>("SELECT status FROM long_term_stats_state WHERE id = 1")
            .fetch_optional(pool)
            .await?;
    let long_term_stats_archive_files = if long_term_stats_status
        .as_deref()
        .is_some_and(|status| matches!(status, "ready" | "empty"))
    {
        sqlx::query_as::<_, (String, String)>(
            r#"
            SELECT replay.file_path, replay.archive_sha256
            FROM hourly_rollup_archive_replay replay
            INNER JOIN archive_batches batches
              ON batches.dataset = 'codex_invocations'
             AND batches.file_path = replay.file_path
             AND batches.sha256 = replay.archive_sha256
            WHERE replay.target = ?1
              AND replay.dataset = 'codex_invocations'
            "#,
        )
        .bind(LONG_TERM_STATS_ARCHIVE_REPLAY_TARGET)
        .fetch_all(pool)
        .await?
        .into_iter()
        .collect::<HashSet<_>>()
    } else {
        HashSet::new()
    };
    let long_term_stats_attempt_archive_files = if long_term_stats_status
        .as_deref()
        .is_some_and(|status| matches!(status, "ready" | "empty"))
    {
        sqlx::query_as::<_, (String, String)>(
            r#"
            SELECT replay.file_path, replay.archive_sha256
            FROM hourly_rollup_archive_replay replay
            INNER JOIN archive_batches batches
              ON batches.dataset = 'pool_upstream_request_attempts'
             AND batches.file_path = replay.file_path
             AND batches.sha256 = replay.archive_sha256
            WHERE replay.target = ?1
              AND replay.dataset = 'pool_upstream_request_attempts'
            "#,
        )
        .bind(LONG_TERM_STATS_ARCHIVE_REPLAY_TARGET)
        .fetch_all(pool)
        .await?
        .into_iter()
        .collect::<HashSet<_>>()
    } else {
        HashSet::new()
    };

    let mut eligible_candidates = Vec::new();
    for candidate in candidates {
        if candidate.cleanup_state == ARCHIVE_CLEANUP_STATE_DELETE_PENDING {
            if candidate.dataset == HOURLY_ROLLUP_DATASET_INVOCATIONS
                && !summary_archive_snapshot_has_proof(pool, candidate.id, &candidate.sha256)
                    .await?
            {
                // A pending deletion from an older process is still subject to the durable
                // Summary Snapshot gate.  Without a matching proof, retaining the manifest is
                // safer than deleting the only authoritative source.
                continue;
            }
            eligible_candidates.push(candidate);
            continue;
        }
        if HISTORICAL_ROLLUP_ARCHIVE_DATASETS.contains(&candidate.dataset.as_str())
            && candidate.historical_rollups_materialized_at.is_none()
        {
            continue;
        }
        if candidate.dataset == "pool_upstream_request_attempts"
            && (candidate.historical_rollups_materialized_at.is_none()
                || !materialized_pool_upstream_cache_files.contains(&candidate.file_path)
                || !materialized_pool_upstream_hourly_files.contains(&candidate.file_path))
        {
            continue;
        }
        if candidate.dataset == "pool_upstream_request_attempts"
            && !long_term_stats_attempt_archive_files
                .contains(&(candidate.file_path.clone(), candidate.sha256.clone()))
        {
            continue;
        }
        // Only an already-staged deletion has evidence that the source archive was readable
        // when cleanup began. A missing invocation or attempt archive is source loss, even if
        // older replay markers exist, so retain its manifest for long-term reconciliation.
        if matches!(
            candidate.dataset.as_str(),
            HOURLY_ROLLUP_DATASET_INVOCATIONS | "pool_upstream_request_attempts"
        ) && archive_file_is_confirmed_missing(&candidate.file_path)
        {
            continue;
        }
        if candidate.dataset == HOURLY_ROLLUP_DATASET_INVOCATIONS
            && (!long_term_stats_status
                .as_deref()
                .is_some_and(|status| matches!(status, "ready" | "empty"))
                || !long_term_stats_archive_files
                    .contains(&(candidate.file_path.clone(), candidate.sha256.clone())))
        {
            continue;
        }
        if candidate.dataset == HOURLY_ROLLUP_DATASET_INVOCATIONS
            && !summary_archive_snapshot_has_proof(pool, candidate.id, &candidate.sha256).await?
        {
            // Source cleanup is allowed only after a normalized Snapshot page and its manifest
            // identity have committed. Legacy archives are picked up by the background
            // Snapshot backfill instead of being retired here.
            continue;
        }
        if candidate.dataset == "pool_upstream_request_attempts"
            && candidate
                .coverage_end_at
                .as_deref()
                .map(|coverage_end_at| {
                    coverage_end_at >= owner_facing_node_health_window_cutoff.as_str()
                })
                .unwrap_or(true)
        {
            continue;
        }
        if candidate.dataset == HOURLY_ROLLUP_DATASET_INVOCATIONS
            && candidate
                .coverage_end_at
                .as_deref()
                .map(|coverage_end_at| coverage_end_at >= invocation_archive_cutoff.as_str())
                .unwrap_or(true)
        {
            continue;
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
        if candidate.cleanup_state == ARCHIVE_CLEANUP_STATE_DELETE_PENDING {
            if finalize_archive_batch_file_deletion(
                pool,
                candidate.id,
                &candidate.dataset,
                &candidate.file_path,
                &candidate.sha256,
            )
            .await?
            {
                deleted += 1;
            }
            continue;
        }
        let file_missing = match fs::metadata(&candidate.file_path) {
            Ok(_) => false,
            Err(error) if error.kind() == io::ErrorKind::NotFound => true,
            Err(error) => {
                warn!(
                    dataset = candidate.dataset,
                    file_path = candidate.file_path,
                    error = %error,
                    "could not inspect expired archive file; retaining metadata for a later retry"
                );
                continue;
            }
        };
        if file_missing {
            // Eligibility and replay gates above already prove this expired manifest can retire.
            // Do not derive a source boundary from metadata when its source file is gone.
            if stage_archive_batch_deletion(
                pool,
                candidate.id,
                &candidate.dataset,
                &candidate.file_path,
                &candidate.sha256,
                None,
            )
            .await?
                && finalize_archive_batch_file_deletion(
                    pool,
                    candidate.id,
                    &candidate.dataset,
                    &candidate.file_path,
                    &candidate.sha256,
                )
                .await?
            {
                deleted += 1;
            }
            continue;
        }
        let integrity_source_safe_start = if matches!(
            candidate.dataset.as_str(),
            HOURLY_ROLLUP_DATASET_INVOCATIONS | "pool_upstream_request_attempts"
        ) {
            match crate::long_term_stats::long_term_integrity_source_safe_start_for_archive_cleanup(
                pool,
                &candidate.dataset,
                &candidate.file_path,
                candidate.coverage_end_at.as_deref(),
            )
            .await
            {
                Ok(source_safe_start) => source_safe_start,
                Err(error) => {
                    warn!(
                        dataset = candidate.dataset,
                        file_path = candidate.file_path,
                        error = %error,
                        "could not prove long-term source boundary; retaining expired archive batch"
                    );
                    continue;
                }
            }
        } else {
            None
        };
        if stage_archive_batch_deletion(
            pool,
            candidate.id,
            &candidate.dataset,
            &candidate.file_path,
            &candidate.sha256,
            integrity_source_safe_start,
        )
        .await?
            && finalize_archive_batch_file_deletion(
                pool,
                candidate.id,
                &candidate.dataset,
                &candidate.file_path,
                &candidate.sha256,
            )
            .await?
        {
            deleted += 1;
        }
    }

    Ok(deleted)
}

#[derive(Debug, FromRow)]
pub(crate) struct HistoricalRollupPendingArchiveBatchRow {
    dataset: String,
    month_key: String,
    file_path: String,
    coverage_start_at: Option<String>,
    coverage_end_at: Option<String>,
}

const STARTUP_HISTORICAL_ROLLUP_CANDIDATE_LIMIT: i64 = 32;
const STARTUP_HISTORICAL_ROLLUP_BATCH_LIMIT: usize = 16;
const LEGACY_DETAIL_MIRROR_RECOVERY_CANDIDATE_LIMIT: i64 = 128;
const LEGACY_DETAIL_MIRROR_IDENTITY_PAGE_SIZE: i64 = 400;
const SUMMARY_STARTUP_LEGACY_DETAIL_MIRROR_CANDIDATE_LIMIT: i64 = 512;
const SUMMARY_STARTUP_LEGACY_DETAIL_MIRROR_PROOF_CONCURRENCY: usize = 4;

#[derive(Debug, Clone, FromRow)]
struct HistoricalRollupStartupCandidateRow {
    id: i64,
    dataset: String,
    file_path: String,
    sha256: String,
    row_count: i64,
    summary_source_kind: String,
    coverage_start_at: Option<String>,
    coverage_end_at: Option<String>,
}

impl HistoricalRollupStartupCandidateRow {
    fn archive_file(&self) -> ArchiveBatchFileRow {
        ArchiveBatchFileRow {
            id: self.id,
            file_path: self.file_path.clone(),
            coverage_start_at: self.coverage_start_at.clone(),
            coverage_end_at: self.coverage_end_at.clone(),
        }
    }
}

const SUMMARY_ARCHIVE_SNAPSHOT_BACKFILL_SCOPE: &str = "summary-global";
const SUMMARY_ARCHIVE_SNAPSHOT_BACKFILL_CANDIDATE_LIMIT: i64 = 8;
const SUMMARY_ARCHIVE_SNAPSHOT_BACKFILL_RETRY_SECS: i64 = 3_600;
const SUMMARY_ARCHIVE_SNAPSHOT_BACKFILL_CURSOR_VERSION: i64 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SummaryArchiveSnapshotBackfillDisposition {
    Complete,
    InProgress,
    Deferred,
    TransientFailure,
    Unrecoverable,
}

impl SummaryArchiveSnapshotBackfillDisposition {
    fn from_storage(disposition: &str, failure_kind: &str) -> Self {
        match disposition {
            "complete" => Self::Complete,
            "in_progress" => Self::InProgress,
            "deferred" => Self::Deferred,
            "unavailable"
                if matches!(
                    failure_kind,
                    "verification_failed"
                        | "manifest_sha_mismatch"
                        | "invalid_timestamp"
                        | "row_count_mismatch"
                ) =>
            {
                Self::Unrecoverable
            }
            "unavailable" => Self::TransientFailure,
            _ => Self::TransientFailure,
        }
    }

    fn next_probe_at(self, retry_attempt: i64) -> String {
        match self {
            Self::Complete | Self::Unrecoverable => {
                format_utc_iso(Utc::now() + ChronoDuration::days(365))
            }
            Self::InProgress => format_utc_iso(Utc::now()),
            Self::Deferred | Self::TransientFailure => {
                let exponent = u32::try_from(retry_attempt.clamp(0, 5)).unwrap_or(5);
                let delay_secs = SUMMARY_ARCHIVE_SNAPSHOT_BACKFILL_RETRY_SECS
                    .saturating_mul(1_i64 << exponent)
                    .min(24 * 3_600);
                format_utc_iso(Utc::now() + ChronoDuration::seconds(delay_secs))
            }
        }
    }
}

#[derive(Debug, Clone)]
struct SummaryArchiveSnapshotBackfillProgress {
    page_index: u32,
    next_occurred_at: Option<String>,
    next_row_id: i64,
    cursor_version: i64,
    retry_attempt: i64,
}

#[derive(Debug, Default)]
pub(crate) struct SummaryArchiveSnapshotBackfillWindowResult {
    pub(crate) next_cursor_id: i64,
    pub(crate) candidate_count: usize,
    pub(crate) scanned_archive_batches: usize,
    pub(crate) materialized_archive_batches: usize,
    pub(crate) unavailable_archive_batches: usize,
    pub(crate) hit_budget: bool,
    pub(crate) wrapped: bool,
}

async fn load_summary_archive_snapshot_backfill_checkpoint(
    pool: &Pool<Sqlite>,
) -> Result<(i64, i64)> {
    Ok(sqlx::query_as::<_, (i64, i64)>(
        "SELECT next_archive_batch_id, manifest_high_watermark_id \
         FROM summary_archive_snapshot_backfill_checkpoint WHERE scope = ?1",
    )
    .bind(SUMMARY_ARCHIVE_SNAPSHOT_BACKFILL_SCOPE)
    .fetch_optional(pool)
    .await?
    .unwrap_or((0, 0)))
}

async fn store_summary_archive_snapshot_backfill_checkpoint(
    pool: &Pool<Sqlite>,
    next_archive_batch_id: i64,
    manifest_high_watermark_id: i64,
    completed: bool,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO summary_archive_snapshot_backfill_checkpoint \
         (scope, next_archive_batch_id, manifest_high_watermark_id, completed, updated_at) \
         VALUES (?1, ?2, ?3, ?4, datetime('now')) \
         ON CONFLICT(scope) DO UPDATE SET \
           next_archive_batch_id = excluded.next_archive_batch_id, \
           manifest_high_watermark_id = excluded.manifest_high_watermark_id, \
           completed = excluded.completed, updated_at = excluded.updated_at",
    )
    .bind(SUMMARY_ARCHIVE_SNAPSHOT_BACKFILL_SCOPE)
    .bind(next_archive_batch_id.max(0))
    .bind(manifest_high_watermark_id.max(0))
    .bind(i64::from(completed))
    .execute(pool)
    .await?;
    Ok(())
}

async fn load_summary_archive_snapshot_backfill_candidates(
    pool: &Pool<Sqlite>,
    cursor_id: i64,
    limit: i64,
) -> Result<Vec<HistoricalRollupStartupCandidateRow>> {
    sqlx::query_as::<_, HistoricalRollupStartupCandidateRow>(
        r#"
        SELECT
            batches.id,
            batches.dataset,
            batches.file_path,
            batches.sha256,
            batches.row_count,
            COALESCE(batches.summary_source_kind, 'unknown') AS summary_source_kind,
            batches.coverage_start_at,
            batches.coverage_end_at
        FROM archive_batches AS batches
        WHERE batches.dataset = 'codex_invocations'
          AND batches.status = ?1
          AND COALESCE(batches.summary_source_kind, 'unknown') <> 'live_mirror'
          AND batches.id > ?2
          AND NOT EXISTS (
                SELECT 1
                FROM summary_archive_snapshot_backfill_outcome AS outcome
                WHERE outcome.archive_batch_id = batches.id
                  AND outcome.manifest_sha256 = batches.sha256
                  AND julianday(outcome.next_probe_at) > julianday('now')
          )
        ORDER BY CASE
            WHEN batches.coverage_end_at IS NOT NULL
             AND batches.coverage_end_at >= datetime('now', '-30 days')
             AND (batches.coverage_start_at IS NULL OR batches.coverage_start_at <= datetime('now')) THEN 0
            ELSE 1
        END, batches.id ASC
        LIMIT ?3
        "#,
    )
    .bind(ARCHIVE_STATUS_COMPLETED)
    .bind(cursor_id.max(0))
    .bind(limit.max(1))
    .fetch_all(pool)
    .await
    .context("failed to load Summary Snapshot V2 backfill candidates")
}

async fn load_summary_archive_snapshot_backfill_progress(
    pool: &Pool<Sqlite>,
    candidate: &HistoricalRollupStartupCandidateRow,
) -> Result<SummaryArchiveSnapshotBackfillProgress> {
    let progress = sqlx::query_as::<_, (i64, Option<String>, i64, i64, i64)>(
        "SELECT next_page_index, next_occurred_at, next_row_id, cursor_version, retry_attempt
         FROM summary_archive_snapshot_backfill_outcome
         WHERE archive_batch_id = ?1 AND manifest_sha256 = ?2",
    )
    .bind(candidate.id)
    .bind(&candidate.sha256)
    .fetch_optional(pool)
    .await
    .context("load Summary Snapshot V2 backfill page progress")?;
    let Some((page_index, occurred_at, row_id, cursor_version, retry_attempt)) = progress else {
        return Ok(SummaryArchiveSnapshotBackfillProgress {
            page_index: 0,
            next_occurred_at: None,
            next_row_id: 0,
            cursor_version: SUMMARY_ARCHIVE_SNAPSHOT_BACKFILL_CURSOR_VERSION,
            retry_attempt: 0,
        });
    };
    Ok(SummaryArchiveSnapshotBackfillProgress {
        page_index: u32::try_from(page_index.max(0)).unwrap_or(u32::MAX),
        next_occurred_at: occurred_at,
        next_row_id: row_id.max(0),
        cursor_version,
        retry_attempt: retry_attempt.max(0),
    })
}

async fn record_summary_archive_snapshot_backfill_outcome(
    pool: &Pool<Sqlite>,
    candidate: &HistoricalRollupStartupCandidateRow,
    disposition: &str,
    failure_kind: &str,
    next_page_index: u32,
    next_row_id: i64,
) -> Result<()> {
    let mut tx = pool.begin().await?;
    record_summary_archive_snapshot_backfill_outcome_tx(
        tx.as_mut(),
        candidate,
        disposition,
        failure_kind,
        &SummaryArchiveSnapshotBackfillProgress {
            page_index: next_page_index,
            next_occurred_at: None,
            next_row_id,
            cursor_version: SUMMARY_ARCHIVE_SNAPSHOT_BACKFILL_CURSOR_VERSION,
            retry_attempt: 0,
        },
    )
    .await?;
    tx.commit().await?;
    Ok(())
}

async fn record_summary_archive_snapshot_backfill_outcome_preserving_progress(
    pool: &Pool<Sqlite>,
    candidate: &HistoricalRollupStartupCandidateRow,
    disposition: &str,
    failure_kind: &str,
    progress: SummaryArchiveSnapshotBackfillProgress,
) -> Result<()> {
    let mut tx = pool.begin().await?;
    record_summary_archive_snapshot_backfill_outcome_tx(
        tx.as_mut(),
        candidate,
        disposition,
        failure_kind,
        &SummaryArchiveSnapshotBackfillProgress {
            cursor_version: progress
                .cursor_version
                .max(SUMMARY_ARCHIVE_SNAPSHOT_BACKFILL_CURSOR_VERSION),
            retry_attempt: progress.retry_attempt.saturating_add(1),
            ..progress
        },
    )
    .await?;
    tx.commit().await?;
    Ok(())
}

fn classify_summary_archive_snapshot_backfill_error(error: &anyhow::Error) -> &'static str {
    let message = error.to_string().to_ascii_lowercase();
    if message.contains("busy") || message.contains("locked") {
        "deferred:sqlite_busy_or_locked"
    } else if message.contains("budget") || message.contains("deadline") {
        "deferred:budget"
    } else if message.contains("invalid")
        || message.contains("mismatch")
        || message.contains("proof")
        || message.contains("semantic")
        || message.contains("page")
    {
        "unavailable:verification_failed"
    } else {
        "unavailable:source_read_failed"
    }
}

async fn record_summary_archive_snapshot_backfill_outcome_tx(
    connection: &mut SqliteConnection,
    candidate: &HistoricalRollupStartupCandidateRow,
    disposition: &str,
    failure_kind: &str,
    progress: &SummaryArchiveSnapshotBackfillProgress,
) -> Result<()> {
    let disposition_kind =
        SummaryArchiveSnapshotBackfillDisposition::from_storage(disposition, failure_kind);
    let next_probe_at = disposition_kind.next_probe_at(progress.retry_attempt);
    sqlx::query(
        "INSERT INTO summary_archive_snapshot_backfill_outcome \
         (archive_batch_id, manifest_sha256, disposition, failure_kind, next_probe_at, \
          next_page_index, next_occurred_at, next_row_id, cursor_version, retry_attempt, updated_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, datetime('now')) \
         ON CONFLICT(archive_batch_id, manifest_sha256) DO UPDATE SET \
           disposition = excluded.disposition, failure_kind = excluded.failure_kind, \
           next_probe_at = excluded.next_probe_at, next_page_index = excluded.next_page_index, \
           next_occurred_at = excluded.next_occurred_at, next_row_id = excluded.next_row_id, \
           cursor_version = excluded.cursor_version, retry_attempt = excluded.retry_attempt, \
           updated_at = excluded.updated_at",
    )
    .bind(candidate.id)
    .bind(&candidate.sha256)
    .bind(disposition)
    .bind(failure_kind)
    .bind(next_probe_at)
    .bind(i64::from(progress.page_index))
    .bind(progress.next_occurred_at.as_deref())
    .bind(progress.next_row_id.max(0))
    .bind(progress.cursor_version.max(1))
    .bind(progress.retry_attempt.max(0))
    .execute(&mut *connection)
    .await
    .context("failed to persist Summary Snapshot V2 backfill outcome")?;
    Ok(())
}

fn summary_archive_snapshot_backfill_budget_exhausted(
    started_at: Instant,
    max_elapsed: Duration,
) -> bool {
    started_at.elapsed() >= max_elapsed
}

fn sha256_hex_file_with_budget(
    path: &Path,
    started_at: Instant,
    max_elapsed: Duration,
) -> Result<Option<String>> {
    let mut file = fs::File::open(path)
        .with_context(|| format!("failed to open file for sha256 {}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buf = [0_u8; 8192];
    loop {
        if summary_archive_snapshot_backfill_budget_exhausted(started_at, max_elapsed) {
            return Ok(None);
        }
        let read = file.read(&mut buf)?;
        if read == 0 {
            break;
        }
        hasher.update(&buf[..read]);
    }
    Ok(Some(format!("{:x}", hasher.finalize())))
}

async fn backfill_summary_archive_snapshot_v2_candidate(
    pool: &Pool<Sqlite>,
    candidate: &HistoricalRollupStartupCandidateRow,
    started_at: Instant,
    max_elapsed: Duration,
) -> Result<&'static str> {
    // A valid V2 page is already an exact authority and does not require reopening the raw
    // archive. Marking it complete here also upgrades V2 pages written by an older process into
    // the durable backfill outcome index.
    if summary_archive_snapshot_has_proof(pool, candidate.id, &candidate.sha256).await? {
        return Ok("complete");
    }
    if candidate.row_count <= 0 {
        return Ok("unavailable:empty_archive");
    }
    let archive_path = Path::new(&candidate.file_path);
    if !archive_path.exists() {
        return Ok("unavailable:missing_source");
    }
    let Some(actual_sha256) = sha256_hex_file_with_budget(archive_path, started_at, max_elapsed)?
    else {
        return Ok("deferred:budget");
    };
    if actual_sha256 != candidate.sha256 {
        return Ok("unavailable:manifest_sha_mismatch");
    }
    let archive_row = crate::stats::ArchiveBatchPathRow::with_coverage(
        candidate.file_path.clone(),
        candidate.coverage_start_at.clone(),
        candidate.coverage_end_at.clone(),
    );
    let archive_source =
        crate::stats::open_invocation_archive_batch_pool(&archive_row, "summary-snapshot-backfill")
            .await;
    let Some((archive_pool, temp_cleanup)) = (match archive_source {
        Ok(source) => source,
        Err(_) => return Ok("unavailable:source_open_failed"),
    }) else {
        return Ok("unavailable:source_open_failed");
    };

    let result = async {
        let archive_columns =
            load_archive_table_columns(&archive_pool, "codex_invocations").await?;
        let query_sql = build_invocation_archive_rows_time_chunk_query(&archive_columns);
        let mut progress = load_summary_archive_snapshot_backfill_progress(pool, candidate).await?;
        if progress.cursor_version < SUMMARY_ARCHIVE_SNAPSHOT_BACKFILL_CURSOR_VERSION
            && (progress.next_row_id > 0 || progress.page_index > 0)
        {
            // A pre-V2 checkpoint only proves ID order. It cannot be combined with the
            // chronological V2 proof, so discard its unproven pages and restart this manifest.
            let mut tx = pool.begin().await?;
            sqlx::query(
                "DELETE FROM summary_archive_snapshot
                 WHERE archive_batch_id = ?1 AND manifest_sha256 = ?2",
            )
            .bind(candidate.id)
            .bind(&candidate.sha256)
            .execute(tx.as_mut())
            .await?;
            record_summary_archive_snapshot_backfill_outcome_tx(
                tx.as_mut(),
                candidate,
                "in_progress",
                "legacy_cursor_reset",
                &SummaryArchiveSnapshotBackfillProgress {
                    page_index: 0,
                    next_occurred_at: None,
                    next_row_id: 0,
                    cursor_version: SUMMARY_ARCHIVE_SNAPSHOT_BACKFILL_CURSOR_VERSION,
                    retry_attempt: 0,
                },
            )
            .await?;
            tx.commit().await?;
            progress.page_index = 0;
            progress.next_occurred_at = None;
            progress.next_row_id = 0;
            progress.cursor_version = SUMMARY_ARCHIVE_SNAPSHOT_BACKFILL_CURSOR_VERSION;
            progress.retry_attempt = 0;
        }
        let mut total_rows = sqlx::query_scalar::<_, Option<i64>>(
            "SELECT SUM(row_count) FROM summary_archive_snapshot
             WHERE archive_batch_id = ?1 AND manifest_sha256 = ?2 AND page_index < ?3",
        )
        .bind(candidate.id)
        .bind(&candidate.sha256)
        .bind(i64::from(progress.page_index))
        .fetch_one(pool)
        .await?
        .unwrap_or_default();
        loop {
            if summary_archive_snapshot_backfill_budget_exhausted(started_at, max_elapsed) {
                return Ok("deferred:budget");
            }
            let (rows, has_more) = load_invocation_archive_rows_time_chunk(
                &archive_pool,
                &query_sql,
                progress.next_occurred_at.as_deref(),
                progress.next_row_id,
            )
            .await?;
            if rows.is_empty() {
                break;
            }
            let coverage_start = if progress.page_index == 0 {
                candidate
                    .coverage_start_at
                    .clone()
                    .or_else(|| rows.first().map(|row| row.occurred_at.clone()))
            } else {
                rows.first().map(|row| row.occurred_at.clone())
            };
            let coverage_end = if !has_more {
                candidate
                    .coverage_end_at
                    .clone()
                    .or_else(|| rows.last().map(|row| row.occurred_at.clone()))
            } else {
                rows.last().map(|row| row.occurred_at.clone())
            };
            let (Some(coverage_start), Some(coverage_end)) = (coverage_start, coverage_end) else {
                bail!("Summary Snapshot source page is missing coverage");
            };
            let invoke_ids_by_row_id = if archive_columns.contains("invoke_id") {
                let mut query = QueryBuilder::<Sqlite>::new(
                    "SELECT id, invoke_id FROM codex_invocations WHERE id IN (",
                );
                let mut separated = query.separated(", ");
                for row in &rows {
                    separated.push_bind(row.id);
                }
                separated.push_unseparated(")");
                query
                    .build_query_as::<(i64, String)>()
                    .fetch_all(&archive_pool)
                    .await?
                    .into_iter()
                    .collect::<HashMap<_, _>>()
            } else {
                HashMap::new()
            };
            let payload = super::super::retention::encode_summary_archive_snapshot_v2_payload(
                &rows,
                &invoke_ids_by_row_id,
            )?;
            let page = SummaryArchiveSnapshotPage {
                archive_batch_id: candidate.id,
                manifest_sha256: candidate.sha256.clone(),
                page_index: progress.page_index,
                coverage_start,
                coverage_end,
                row_count: u32::try_from(rows.len())
                    .context("Summary Snapshot V2 page row count overflow")?,
                payload,
            };
            let mut tx = pool.begin().await?;
            store_summary_archive_snapshot_page_v2_tx(tx.as_mut(), &page).await?;
            record_summary_archive_snapshot_backfill_outcome_tx(
                tx.as_mut(),
                candidate,
                "in_progress",
                "",
                &SummaryArchiveSnapshotBackfillProgress {
                    page_index: progress.page_index.saturating_add(1),
                    next_occurred_at: rows.last().map(|row| row.occurred_at.clone()),
                    next_row_id: rows
                        .last()
                        .map(|row| row.id)
                        .unwrap_or(progress.next_row_id),
                    cursor_version: SUMMARY_ARCHIVE_SNAPSHOT_BACKFILL_CURSOR_VERSION,
                    retry_attempt: 0,
                },
            )
            .await?;
            tx.commit().await?;
            total_rows = total_rows.saturating_add(i64::try_from(rows.len())?);
            progress.next_row_id = rows
                .last()
                .map(|row| row.id)
                .ok_or_else(|| anyhow!("Summary Snapshot page is missing row id"))?;
            progress.next_occurred_at = rows.last().map(|row| row.occurred_at.clone());
            progress.page_index = progress.page_index.saturating_add(1);
            if !has_more {
                break;
            }
        }
        if total_rows != candidate.row_count {
            bail!(
                "Summary Snapshot V2 row count mismatch: expected {}, got {}",
                candidate.row_count,
                total_rows
            );
        }
        if !summary_archive_snapshot_has_proof(pool, candidate.id, &candidate.sha256).await? {
            bail!("Summary Snapshot V2 proof validation failed after backfill");
        }
        Ok("complete")
    }
    .await;
    archive_pool.close().await;
    drop(temp_cleanup);
    result?;
    Ok("complete")
}

pub(crate) async fn backfill_summary_archive_snapshots_v2_window(
    pool: &Pool<Sqlite>,
    max_elapsed: Duration,
) -> Result<SummaryArchiveSnapshotBackfillWindowResult> {
    let started_at = Instant::now();
    let (mut cursor_id, mut high_watermark_id) =
        load_summary_archive_snapshot_backfill_checkpoint(pool).await?;
    let observed_high_watermark = sqlx::query_scalar::<_, i64>(
        "SELECT COALESCE(MAX(id), 0) FROM archive_batches \
         WHERE dataset = 'codex_invocations' AND status = ?1",
    )
    .bind(ARCHIVE_STATUS_COMPLETED)
    .fetch_one(pool)
    .await?;
    high_watermark_id = high_watermark_id.max(observed_high_watermark);
    let mut candidates = load_summary_archive_snapshot_backfill_candidates(
        pool,
        cursor_id,
        SUMMARY_ARCHIVE_SNAPSHOT_BACKFILL_CANDIDATE_LIMIT,
    )
    .await?;
    let mut wrapped = false;
    if candidates.is_empty() && cursor_id > 0 {
        candidates = load_summary_archive_snapshot_backfill_candidates(
            pool,
            0,
            SUMMARY_ARCHIVE_SNAPSHOT_BACKFILL_CANDIDATE_LIMIT,
        )
        .await?;
        wrapped = !candidates.is_empty();
        if wrapped {
            cursor_id = 0;
        }
    }
    let candidate_count = candidates.len();
    let mut result = SummaryArchiveSnapshotBackfillWindowResult {
        next_cursor_id: cursor_id,
        candidate_count,
        wrapped,
        ..SummaryArchiveSnapshotBackfillWindowResult::default()
    };
    for candidate in candidates {
        if started_at.elapsed() >= max_elapsed {
            result.hit_budget = true;
            break;
        }
        result.scanned_archive_batches += 1;
        let outcome = match backfill_summary_archive_snapshot_v2_candidate(
            pool,
            &candidate,
            started_at,
            max_elapsed,
        )
        .await
        {
            Ok(outcome) => outcome,
            Err(error) => {
                let outcome = classify_summary_archive_snapshot_backfill_error(&error);
                if outcome.starts_with("deferred:") {
                    outcome
                } else {
                    // A corrupt or semantically invalid page is scoped to this manifest. Keep
                    // the cursor moving so an independent candidate can still be recovered, but
                    // preserve the last verified page cursor for retryable source failures.
                    outcome
                }
            }
        };
        if let Some((disposition, failure_kind)) = outcome.split_once(':') {
            if disposition == "deferred" {
                result.hit_budget = true;
                break;
            }
            let progress =
                load_summary_archive_snapshot_backfill_progress(pool, &candidate).await?;
            record_summary_archive_snapshot_backfill_outcome_preserving_progress(
                pool,
                &candidate,
                disposition,
                failure_kind,
                progress,
            )
            .await?;
            result.unavailable_archive_batches += 1;
        } else {
            record_summary_archive_snapshot_backfill_outcome(
                pool, &candidate, "complete", "", 0, 0,
            )
            .await?;
            result.materialized_archive_batches += 1;
        }
        result.next_cursor_id = candidate.id;
        store_summary_archive_snapshot_backfill_checkpoint(
            pool,
            result.next_cursor_id,
            high_watermark_id,
            result.next_cursor_id >= high_watermark_id && !result.hit_budget,
        )
        .await?;
    }
    if candidate_count == 0 {
        store_summary_archive_snapshot_backfill_checkpoint(pool, 0, high_watermark_id, true)
            .await?;
    }
    Ok(result)
}

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

#[derive(Debug)]
pub(crate) struct LegacyDetailMirrorRecoveryWindowResult {
    pub(crate) next_cursor_id: i64,
    pub(crate) candidate_count: usize,
    pub(crate) inspected_path_count: usize,
    pub(crate) changed_path_count: usize,
    pub(crate) hit_budget: bool,
    pub(crate) wrapped: bool,
}

#[derive(Debug)]
pub(crate) struct SummaryStartupLegacyDetailMirrorRecoveryWindowResult {
    pub(crate) next_cursor_id: i64,
    pub(crate) candidate_count: usize,
    pub(crate) inspected_path_count: usize,
    pub(crate) changed_path_count: usize,
    pub(crate) unavailable_path_count: usize,
    pub(crate) hit_budget: bool,
    pub(crate) completed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LegacyDetailMirrorProof {
    Proven,
    NotMirror,
    BudgetExhausted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SummaryStartupLegacyDetailMirrorProof {
    Proven,
    NotMirror,
    BudgetExhausted,
    Unavailable,
}

async fn load_historical_rollup_startup_candidates(
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

async fn legacy_invocation_archive_is_live_detail_mirror(
    pool: &Pool<Sqlite>,
    candidate: &HistoricalRollupStartupCandidateRow,
    started_at: Instant,
    max_elapsed: Duration,
) -> Result<LegacyDetailMirrorProof> {
    if candidate.row_count < 0 {
        return Ok(LegacyDetailMirrorProof::NotMirror);
    }
    let archive_path = Path::new(&candidate.file_path);
    let Some(sha256_before_open) =
        legacy_detail_mirror_sha256_with_budget(archive_path, started_at, max_elapsed)?
    else {
        return Ok(LegacyDetailMirrorProof::BudgetExhausted);
    };
    if sha256_before_open != candidate.sha256 {
        return Ok(LegacyDetailMirrorProof::NotMirror);
    }
    if started_at.elapsed() >= max_elapsed {
        return Ok(LegacyDetailMirrorProof::BudgetExhausted);
    }

    let temp_path = PathBuf::from(format!(
        "{}.{}.sqlite",
        archive_path.display(),
        retention_temp_suffix()
    ));
    let temp_cleanup = TempSqliteCleanup(temp_path.clone());
    if !inflate_gzip_sqlite_file_with_budget(
        archive_path,
        &temp_path,
        started_at,
        Some(max_elapsed),
    )? {
        drop(temp_cleanup);
        return Ok(LegacyDetailMirrorProof::BudgetExhausted);
    }
    if started_at.elapsed() >= max_elapsed {
        drop(temp_cleanup);
        return Ok(LegacyDetailMirrorProof::BudgetExhausted);
    };
    let archive_pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect(&sqlite_url_for_path(&temp_path))
        .await
        .with_context(|| {
            format!(
                "failed to open legacy detail archive {}",
                archive_path.display()
            )
        })?;

    let mut after_id = i64::MIN;
    let mut matched_rows = 0_i64;
    let proof_result: Result<LegacyDetailMirrorProof> = async {
        loop {
            if started_at.elapsed() >= max_elapsed {
                return Ok(LegacyDetailMirrorProof::BudgetExhausted);
            }
            let archive_rows = sqlx::query_as::<_, (i64, String)>(
                "SELECT id, invoke_id FROM codex_invocations WHERE id > ?1 ORDER BY id ASC LIMIT ?2",
            )
            .bind(after_id)
            .bind(LEGACY_DETAIL_MIRROR_IDENTITY_PAGE_SIZE)
            .fetch_all(&archive_pool)
            .await
            .with_context(|| {
                format!(
                    "failed to read legacy detail archive identities for archive batch {}",
                    candidate.id
                )
            })?;
            if started_at.elapsed() >= max_elapsed {
                return Ok(LegacyDetailMirrorProof::BudgetExhausted);
            }
            let Some(last_id) = archive_rows.last().map(|(id, _)| *id) else {
                break;
            };
            after_id = last_id;

            let mut live_query = QueryBuilder::<Sqlite>::new(
                "SELECT id, invoke_id FROM codex_invocations WHERE id IN (",
            );
            {
                let mut ids = live_query.separated(", ");
                for (id, _) in &archive_rows {
                    ids.push_bind(id);
                }
            }
            live_query.push(")");
            let live_rows = live_query
                .build_query_as::<(i64, String)>()
                .fetch_all(pool)
                .await
                .context("failed to read live identities for legacy detail mirror recovery")?;
            if started_at.elapsed() >= max_elapsed {
                return Ok(LegacyDetailMirrorProof::BudgetExhausted);
            }
            if live_rows.len() != archive_rows.len() {
                return Ok(LegacyDetailMirrorProof::NotMirror);
            }
            let live_invoke_ids = live_rows.into_iter().collect::<HashMap<_, _>>();
            if archive_rows
                .iter()
                .any(|(id, invoke_id)| live_invoke_ids.get(id) != Some(invoke_id))
            {
                return Ok(LegacyDetailMirrorProof::NotMirror);
            }
            matched_rows += archive_rows.len() as i64;
        }
        Ok(LegacyDetailMirrorProof::Proven)
    }
    .await;
    archive_pool.close().await;
    let proof = proof_result?;
    if proof != LegacyDetailMirrorProof::Proven {
        return Ok(proof);
    }
    if matched_rows != candidate.row_count {
        return Ok(LegacyDetailMirrorProof::NotMirror);
    }
    let Some(sha256_after_read) =
        legacy_detail_mirror_sha256_with_budget(archive_path, started_at, max_elapsed)?
    else {
        return Ok(LegacyDetailMirrorProof::BudgetExhausted);
    };
    Ok(if sha256_after_read == candidate.sha256 {
        LegacyDetailMirrorProof::Proven
    } else {
        LegacyDetailMirrorProof::NotMirror
    })
}

fn legacy_detail_mirror_sha256_with_budget(
    path: &Path,
    started_at: Instant,
    max_elapsed: Duration,
) -> Result<Option<String>> {
    if started_at.elapsed() >= max_elapsed {
        return Ok(None);
    }
    let mut file = fs::File::open(path).with_context(|| {
        format!(
            "failed to open legacy detail archive for sha256 {}",
            path.display()
        )
    })?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 8192];
    loop {
        let read = file.read(&mut buffer).with_context(|| {
            format!(
                "failed to read legacy detail archive for sha256 {}",
                path.display()
            )
        })?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
        if started_at.elapsed() >= max_elapsed {
            return Ok(None);
        }
    }
    Ok(Some(format!("{:x}", hasher.finalize())))
}

async fn load_legacy_detail_mirror_recovery_candidates(
    pool: &Pool<Sqlite>,
    cursor_id: i64,
    high_watermark_id: Option<i64>,
    candidate_limit: i64,
) -> Result<Vec<HistoricalRollupStartupCandidateRow>> {
    sqlx::query_as::<_, HistoricalRollupStartupCandidateRow>(
        r#"
        SELECT
            id,
            dataset,
            file_path,
            sha256,
            row_count,
            summary_source_kind,
            coverage_start_at,
            coverage_end_at
        FROM archive_batches
        WHERE dataset = 'codex_invocations'
          AND status = ?1
          AND COALESCE(summary_source_kind, 'unknown') = ?2
          AND id > ?3
          AND (?4 IS NULL OR id <= ?4)
        ORDER BY id ASC
        LIMIT ?5
        "#,
    )
    .bind(ARCHIVE_STATUS_COMPLETED)
    .bind(SUMMARY_ARCHIVE_SOURCE_KIND_UNKNOWN)
    .bind(cursor_id)
    .bind(high_watermark_id)
    .bind(candidate_limit)
    .fetch_all(pool)
    .await
    .context("failed to load legacy detail mirror recovery candidates")
}

pub(crate) async fn summary_startup_legacy_detail_mirror_high_watermark(
    pool: &Pool<Sqlite>,
) -> Result<Option<i64>> {
    sqlx::query_scalar::<_, Option<i64>>(
        "SELECT MAX(id) FROM archive_batches \
         WHERE dataset = ?1 AND status = ?2 AND COALESCE(summary_source_kind, 'unknown') = ?3",
    )
    .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
    .bind(ARCHIVE_STATUS_COMPLETED)
    .bind(SUMMARY_ARCHIVE_SOURCE_KIND_UNKNOWN)
    .fetch_one(pool)
    .await
    .context("failed to load summary startup legacy detail mirror high-watermark")
}

async fn update_summary_startup_proven_legacy_detail_mirrors(
    pool: &Pool<Sqlite>,
    proven_mirrors: Vec<HistoricalRollupStartupCandidateRow>,
) -> Result<usize> {
    if proven_mirrors.is_empty() {
        return Ok(0);
    }

    let mut changed_path_count = 0_usize;
    let mut tx = pool.begin().await?;
    for candidate in proven_mirrors {
        changed_path_count += sqlx::query(
            "UPDATE archive_batches SET summary_source_kind = ?1 \
             WHERE id = ?2 AND status = ?3 \
               AND COALESCE(summary_source_kind, 'unknown') = ?4 AND sha256 = ?5",
        )
        .bind(SUMMARY_ARCHIVE_SOURCE_KIND_LIVE_MIRROR)
        .bind(candidate.id)
        .bind(ARCHIVE_STATUS_COMPLETED)
        .bind(SUMMARY_ARCHIVE_SOURCE_KIND_UNKNOWN)
        .bind(&candidate.sha256)
        .execute(tx.as_mut())
        .await?
        .rows_affected() as usize;
    }
    tx.commit().await?;
    Ok(changed_path_count)
}

pub(crate) async fn reconcile_legacy_detail_mirrors_startup_window(
    pool: &Pool<Sqlite>,
    cursor_id: i64,
    max_elapsed: Duration,
) -> Result<LegacyDetailMirrorRecoveryWindowResult> {
    let mut candidates = load_legacy_detail_mirror_recovery_candidates(
        pool,
        cursor_id,
        None,
        LEGACY_DETAIL_MIRROR_RECOVERY_CANDIDATE_LIMIT,
    )
    .await?;
    let mut wrapped = false;
    if candidates.is_empty() && cursor_id > 0 {
        candidates = load_legacy_detail_mirror_recovery_candidates(
            pool,
            0,
            None,
            LEGACY_DETAIL_MIRROR_RECOVERY_CANDIDATE_LIMIT,
        )
        .await?;
        wrapped = !candidates.is_empty();
    }
    if candidates.is_empty() {
        return Ok(LegacyDetailMirrorRecoveryWindowResult {
            next_cursor_id: 0,
            candidate_count: 0,
            inspected_path_count: 0,
            changed_path_count: 0,
            hit_budget: false,
            wrapped,
        });
    }

    let started_at = Instant::now();
    let mut next_cursor_id = cursor_id;
    let mut inspected_path_count = 0_usize;
    let mut hit_budget = false;
    let mut proven_mirrors = Vec::new();
    for candidate in candidates.iter() {
        if started_at.elapsed() >= max_elapsed {
            hit_budget = true;
            break;
        }
        match legacy_invocation_archive_is_live_detail_mirror(
            pool,
            candidate,
            started_at,
            max_elapsed,
        )
        .await?
        {
            LegacyDetailMirrorProof::Proven => proven_mirrors.push(candidate),
            LegacyDetailMirrorProof::NotMirror => {}
            LegacyDetailMirrorProof::BudgetExhausted => {
                hit_budget = true;
                break;
            }
        }
        inspected_path_count += 1;
        next_cursor_id = candidate.id;
    }

    let mut changed_path_count = 0_usize;
    if !proven_mirrors.is_empty() {
        let mut tx = pool.begin().await?;
        for candidate in proven_mirrors {
            changed_path_count += sqlx::query(
                "UPDATE archive_batches SET summary_source_kind = ?1 \
                 WHERE id = ?2 AND status = ?3 AND summary_source_kind = ?4 AND sha256 = ?5",
            )
            .bind(SUMMARY_ARCHIVE_SOURCE_KIND_LIVE_MIRROR)
            .bind(candidate.id)
            .bind(ARCHIVE_STATUS_COMPLETED)
            .bind(SUMMARY_ARCHIVE_SOURCE_KIND_UNKNOWN)
            .bind(&candidate.sha256)
            .execute(tx.as_mut())
            .await?
            .rows_affected() as usize;
        }
        tx.commit().await?;
    }

    Ok(LegacyDetailMirrorRecoveryWindowResult {
        next_cursor_id,
        candidate_count: candidates.len(),
        inspected_path_count,
        changed_path_count,
        hit_budget,
        wrapped,
    })
}

/// Reconciles one high-throughput, finite portion of the cold Summary startup source snapshot.
///
/// A failed identity read stays `unknown`: it remains conservative source evidence, but must not
/// stop later, independently provable mirrors from leaving Summary archive admission. A
/// budget-exhausted proof does not advance its cursor, so the next bounded window retries that
/// same record instead of silently skipping it.
pub(crate) async fn reconcile_legacy_detail_mirrors_for_summary_startup_window(
    pool: &Pool<Sqlite>,
    cursor_id: i64,
    high_watermark_id: i64,
    max_elapsed: Duration,
) -> Result<SummaryStartupLegacyDetailMirrorRecoveryWindowResult> {
    let candidates = load_legacy_detail_mirror_recovery_candidates(
        pool,
        cursor_id,
        Some(high_watermark_id),
        SUMMARY_STARTUP_LEGACY_DETAIL_MIRROR_CANDIDATE_LIMIT,
    )
    .await?;
    if candidates.is_empty() {
        return Ok(SummaryStartupLegacyDetailMirrorRecoveryWindowResult {
            next_cursor_id: cursor_id,
            candidate_count: 0,
            inspected_path_count: 0,
            changed_path_count: 0,
            unavailable_path_count: 0,
            hit_budget: false,
            completed: true,
        });
    }

    let started_at = Instant::now();
    let candidate_count = candidates.len();
    let mut proof_results = stream::iter(candidates.into_iter().enumerate().map(
        |(index, candidate)| async move {
            let proof = match legacy_invocation_archive_is_live_detail_mirror(
                pool,
                &candidate,
                started_at,
                max_elapsed,
            )
            .await
            {
                Ok(LegacyDetailMirrorProof::Proven) => {
                    SummaryStartupLegacyDetailMirrorProof::Proven
                }
                Ok(LegacyDetailMirrorProof::NotMirror) => {
                    SummaryStartupLegacyDetailMirrorProof::NotMirror
                }
                Ok(LegacyDetailMirrorProof::BudgetExhausted) => {
                    SummaryStartupLegacyDetailMirrorProof::BudgetExhausted
                }
                // Missing, unreadable, corrupt, or concurrently replaced files remain
                // fail-closed unknown sources. The next exact Summary build will account for
                // their coverage rather than accepting a guessed mirror classification.
                Err(_) => SummaryStartupLegacyDetailMirrorProof::Unavailable,
            };
            (index, candidate, proof)
        },
    ))
    .buffer_unordered(SUMMARY_STARTUP_LEGACY_DETAIL_MIRROR_PROOF_CONCURRENCY)
    .collect::<Vec<_>>()
    .await;
    proof_results.sort_unstable_by_key(|(index, _, _)| *index);

    let mut next_cursor_id = cursor_id;
    let mut inspected_path_count = 0_usize;
    let mut unavailable_path_count = 0_usize;
    let mut hit_budget = false;
    let mut proven_mirrors = Vec::new();
    for (_, candidate, proof) in proof_results {
        let candidate_id = candidate.id;
        match proof {
            SummaryStartupLegacyDetailMirrorProof::Proven => proven_mirrors.push(candidate),
            SummaryStartupLegacyDetailMirrorProof::NotMirror => {}
            SummaryStartupLegacyDetailMirrorProof::Unavailable => unavailable_path_count += 1,
            SummaryStartupLegacyDetailMirrorProof::BudgetExhausted => {
                hit_budget = true;
                break;
            }
        }
        inspected_path_count += 1;
        next_cursor_id = candidate_id;
    }

    let changed_path_count =
        update_summary_startup_proven_legacy_detail_mirrors(pool, proven_mirrors).await?;
    Ok(SummaryStartupLegacyDetailMirrorRecoveryWindowResult {
        next_cursor_id,
        candidate_count,
        inspected_path_count,
        changed_path_count,
        unavailable_path_count,
        hit_budget,
        completed: !hit_budget
            && candidate_count < SUMMARY_STARTUP_LEGACY_DETAIL_MIRROR_CANDIDATE_LIMIT as usize,
    })
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

    let started_at = Instant::now();
    let mut tx = pool.begin().await?;
    let mut next_cursor_id = cursor_id;
    let mut scanned_archive_batches = 0_usize;
    let mut skipped_archive_batches = 0_usize;
    let mut materialized_archive_batches = 0_usize;
    let mut changed_path_count = 0_usize;
    let mut blocked_archive_batches = 0_usize;
    let mut materialized_invocation_batches = 0_usize;
    let mut materialized_forward_proxy_batches = 0_usize;
    let mut inspected_path_count = 0_usize;
    let mut hit_budget = false;

    for candidate in candidates
        .iter()
        .take(STARTUP_HISTORICAL_ROLLUP_BATCH_LIMIT)
    {
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
                next_cursor_id = candidate.id;
            }
            break;
        }
        next_cursor_id = candidate.id;
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

    let pending_account_count = count_upstream_accounts_missing_last_activity(pool).await?;
    let invocation_archive_cutoff = shanghai_local_cutoff_string(config.invocation_max_days);
    let long_term_stats_status =
        sqlx::query_scalar::<_, String>("SELECT status FROM long_term_stats_state WHERE id = 1")
            .fetch_optional(pool)
            .await?;
    let long_term_stats_archive_files = if long_term_stats_status
        .as_deref()
        .is_some_and(|status| matches!(status, "ready" | "empty"))
    {
        sqlx::query_as::<_, (String, String)>(
            r#"
            SELECT replay.file_path, replay.archive_sha256
            FROM hourly_rollup_archive_replay replay
            INNER JOIN archive_batches batches
              ON batches.dataset = 'codex_invocations'
             AND batches.file_path = replay.file_path
             AND batches.sha256 = replay.archive_sha256
            WHERE replay.target = ?1
              AND replay.dataset = 'codex_invocations'
            "#,
        )
        .bind(LONG_TERM_STATS_ARCHIVE_REPLAY_TARGET)
        .fetch_all(pool)
        .await?
        .into_iter()
        .collect::<HashSet<_>>()
    } else {
        HashSet::new()
    };
    let long_term_stats_attempt_archive_files = if long_term_stats_status
        .as_deref()
        .is_some_and(|status| matches!(status, "ready" | "empty"))
    {
        sqlx::query_as::<_, (String, String)>(
            r#"
            SELECT replay.file_path, replay.archive_sha256
            FROM hourly_rollup_archive_replay replay
            INNER JOIN archive_batches batches
              ON batches.dataset = 'pool_upstream_request_attempts'
             AND batches.file_path = replay.file_path
             AND batches.sha256 = replay.archive_sha256
            WHERE replay.target = ?1
              AND replay.dataset = 'pool_upstream_request_attempts'
            "#,
        )
        .bind(LONG_TERM_STATS_ARCHIVE_REPLAY_TARGET)
        .fetch_all(pool)
        .await?
        .into_iter()
        .collect::<HashSet<_>>()
    } else {
        HashSet::new()
    };
    let mut summary = LegacyArchivePruneSummary {
        scanned_archive_batches: candidates.len(),
        ..LegacyArchivePruneSummary::default()
    };

    for candidate in candidates {
        if candidate.cleanup_state == ARCHIVE_CLEANUP_STATE_DELETE_PENDING {
            let deleted = if dry_run {
                true
            } else {
                finalize_archive_batch_file_deletion(
                    pool,
                    candidate.id,
                    &candidate.dataset,
                    &candidate.file_path,
                    &candidate.sha256,
                )
                .await?
            };
            if deleted {
                summary.deleted_archive_batches += 1;
            } else {
                summary.skipped_unmaterialized_batches += 1;
            }
            continue;
        }
        let file_missing = archive_file_is_confirmed_missing(&candidate.file_path);

        if candidate.dataset == HOURLY_ROLLUP_DATASET_INVOCATIONS && pending_account_count > 0 {
            summary.skipped_unmaterialized_batches += 1;
            continue;
        }

        if matches!(
            candidate.dataset.as_str(),
            HOURLY_ROLLUP_DATASET_INVOCATIONS | "pool_upstream_request_attempts"
        ) && file_missing
        {
            // A legacy manifest without a staged delete may be a lost invocation or attempt
            // source, not a completed cleanup. Keep it as unavailable-source evidence so a
            // later repair cannot replace upstream dimensions with degraded fallback values.
            summary.skipped_unmaterialized_batches += 1;
            continue;
        }

        if candidate.dataset == HOURLY_ROLLUP_DATASET_INVOCATIONS
            && (!long_term_stats_status
                .as_deref()
                .is_some_and(|status| matches!(status, "ready" | "empty"))
                || !long_term_stats_archive_files
                    .contains(&(candidate.file_path.clone(), candidate.sha256.clone())))
        {
            summary.skipped_unmaterialized_batches += 1;
            continue;
        }

        if candidate.dataset == "pool_upstream_request_attempts"
            && !long_term_stats_attempt_archive_files
                .contains(&(candidate.file_path.clone(), candidate.sha256.clone()))
        {
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

        if file_missing {
            // These manifests predate the two-phase cleanup protocol. The existing retention
            // gates above already prove that this legacy batch is eligible; retain the old
            // NotFound-finalization behavior without inventing a source boundary from metadata.
            if stage_archive_batch_deletion(
                pool,
                candidate.id,
                &candidate.dataset,
                &candidate.file_path,
                &candidate.sha256,
                None,
            )
            .await?
                && finalize_archive_batch_file_deletion(
                    pool,
                    candidate.id,
                    &candidate.dataset,
                    &candidate.file_path,
                    &candidate.sha256,
                )
                .await?
            {
                summary.deleted_archive_batches += 1;
            }
            continue;
        }

        let integrity_source_safe_start = if matches!(
            candidate.dataset.as_str(),
            HOURLY_ROLLUP_DATASET_INVOCATIONS | "pool_upstream_request_attempts"
        ) {
            match crate::long_term_stats::long_term_integrity_source_safe_start_for_archive_cleanup(
                pool,
                &candidate.dataset,
                &candidate.file_path,
                candidate.coverage_end_at.as_deref(),
            )
            .await
            {
                Ok(source_safe_start) => source_safe_start,
                Err(error) => {
                    warn!(
                        dataset = candidate.dataset,
                        file_path = candidate.file_path,
                        error = %error,
                        "could not prove long-term source boundary; retaining legacy archive batch"
                    );
                    summary.skipped_unmaterialized_batches += 1;
                    continue;
                }
            }
        } else {
            None
        };

        if stage_archive_batch_deletion(
            pool,
            candidate.id,
            &candidate.dataset,
            &candidate.file_path,
            &candidate.sha256,
            integrity_source_safe_start,
        )
        .await?
            && finalize_archive_batch_file_deletion(
                pool,
                candidate.id,
                &candidate.dataset,
                &candidate.file_path,
                &candidate.sha256,
            )
            .await?
        {
            summary.deleted_archive_batches += 1;
        }
    }

    Ok(summary)
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
                super::super::retention::acquire_retention_write_admission("quota_compaction")
                    .await
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
    }

    Ok((rows_archived, archive_batches))
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

    #[test]
    fn historical_rollup_materialization_budget_exhausted_only_when_bounded_budget_is_zero() {
        assert!(historical_rollup_materialization_budget_exhausted(
            Instant::now(),
            Some(Duration::ZERO),
        ));
        assert!(!historical_rollup_materialization_budget_exhausted(
            Instant::now(),
            None,
        ));
        assert!(!historical_rollup_materialization_budget_exhausted(
            Instant::now(),
            Some(Duration::from_secs(1)),
        ));
    }

    #[tokio::test]
    async fn startup_candidates_include_materialized_invocation_archive_missing_global_summary_proof()
     {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("memory pool");
        crate::schema::ensure_schema(&pool)
            .await
            .expect("current schema");
        sqlx::query(
            r#"
            INSERT INTO archive_batches (
                dataset,
                month_key,
                file_path,
                sha256,
                row_count,
                status,
                summary_source_kind,
                coverage_start_at,
                coverage_end_at,
                historical_rollups_materialized_at
            )
            VALUES (
                'codex_invocations',
                '2026-08',
                '/legacy/missing-global-summary-proof.sqlite.gz',
                'legacy-summary-proof',
                1,
                'completed',
                'unknown',
                '2026-08-01 00:00:00',
                '2026-08-01 01:00:00',
                datetime('now')
            )
            "#,
        )
        .execute(&pool)
        .await
        .expect("seed materialized legacy invocation archive");
        for target in [
            HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_STATS_HOURLY,
            HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE_BREAKDOWN,
        ] {
            sqlx::query(
                "INSERT INTO hourly_rollup_archive_replay \
                 (target, dataset, file_path, archive_sha256) \
                 VALUES (?1, 'codex_invocations', \
                         '/legacy/missing-global-summary-proof.sqlite.gz', \
                         'legacy-summary-proof')",
            )
            .bind(target)
            .execute(&pool)
            .await
            .expect("seed retained summary proof");
        }
        crate::schema::ensure_schema(&pool)
            .await
            .expect("upgrade legacy archive schema");

        let candidates = load_historical_rollup_startup_candidates(&pool, 0)
            .await
            .expect("load startup candidates");

        assert_eq!(candidates.len(), 1);
        assert_eq!(
            candidates[0].file_path,
            "/legacy/missing-global-summary-proof.sqlite.gz"
        );
    }

    #[tokio::test]
    async fn summary_archive_snapshot_backfill_prioritizes_current_30d_horizon() {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("memory pool");
        crate::schema::ensure_schema(&pool)
            .await
            .expect("current schema");
        let old_start = format_utc_iso(Utc::now() - ChronoDuration::days(90));
        let old_end = format_utc_iso(Utc::now() - ChronoDuration::days(89));
        let recent_start = format_utc_iso(Utc::now() - ChronoDuration::days(2));
        let recent_end = format_utc_iso(Utc::now() - ChronoDuration::days(1));
        for (id, path, sha, start, end) in [
            (
                1_i64,
                "/legacy/old-summary.sqlite.gz",
                "old-summary",
                old_start,
                old_end,
            ),
            (
                2_i64,
                "/legacy/recent-summary.sqlite.gz",
                "recent-summary",
                recent_start,
                recent_end,
            ),
        ] {
            sqlx::query(
                "INSERT INTO archive_batches \
                 (id, dataset, month_key, file_path, sha256, row_count, status, summary_source_kind, \
                  coverage_start_at, coverage_end_at) \
                 VALUES (?1, 'codex_invocations', '2026-08', ?2, ?3, 1, 'completed', 'unknown', ?4, ?5)",
            )
            .bind(id)
            .bind(path)
            .bind(sha)
            .bind(start)
            .bind(end)
            .execute(&pool)
            .await
            .expect("seed Snapshot V2 candidate");
        }

        let candidates = load_summary_archive_snapshot_backfill_candidates(&pool, 0, 1)
            .await
            .expect("load 30d-prioritized candidates");
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].id, 2);
    }

    #[tokio::test]
    async fn completed_invocation_archive_requires_summary_publication_proof() {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("memory pool");
        crate::schema::ensure_schema(&pool)
            .await
            .expect("current schema");
        sqlx::query(
            r#"
            INSERT INTO archive_batches (
                dataset,
                month_key,
                file_path,
                sha256,
                row_count,
                status,
                summary_source_kind,
                coverage_start_at,
                coverage_end_at,
                historical_rollups_materialized_at
            )
            VALUES (
                'codex_invocations',
                '2026-08',
                '/staged/missing-summary-proof.sqlite.gz',
                'staged-summary-proof',
                1,
                'materializing',
                'authoritative',
                '2026-08-01 00:00:00',
                '2026-08-01 01:00:00',
                datetime('now')
            )
            "#,
        )
        .execute(&pool)
        .await
        .expect("seed staged invocation archive");

        let error = sqlx::query(
            "UPDATE archive_batches SET status = 'completed' \
             WHERE dataset = 'codex_invocations' \
               AND file_path = '/staged/missing-summary-proof.sqlite.gz'",
        )
        .execute(&pool)
        .await
        .expect_err("publication must require every Summary proof");
        assert!(
            error
                .to_string()
                .contains("requires Summary publication proof")
        );
    }

    #[tokio::test]
    async fn summary_archive_snapshot_backfill_localizes_missing_authority_and_advances_cursor() {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("memory pool");
        crate::schema::ensure_schema(&pool)
            .await
            .expect("current schema");
        sqlx::query(
            r#"
            INSERT INTO archive_batches (
                id, dataset, month_key, file_path, sha256, row_count, status,
                summary_source_kind, coverage_start_at, coverage_end_at
            )
            VALUES (1, 'codex_invocations', '2026-08',
                    '/legacy/missing-summary-authority.sqlite.gz', 'missing-authority', 1,
                    'completed', 'unknown', '2026-08-01 00:00:00',
                    '2026-08-01 01:00:00')
            "#,
        )
        .execute(&pool)
        .await
        .expect("seed missing Summary archive authority");

        let result = backfill_summary_archive_snapshots_v2_window(&pool, Duration::from_secs(1))
            .await
            .expect("run bounded Summary Snapshot backfill");
        assert_eq!(result.scanned_archive_batches, 1);
        assert_eq!(result.unavailable_archive_batches, 1);
        assert_eq!(result.materialized_archive_batches, 0);
        assert_eq!(result.next_cursor_id, 1);

        let outcome = sqlx::query_as::<_, (String, String)>(
            "SELECT disposition, failure_kind
             FROM summary_archive_snapshot_backfill_outcome
             WHERE archive_batch_id = 1 AND manifest_sha256 = 'missing-authority'",
        )
        .fetch_one(&pool)
        .await
        .expect("load missing-source outcome");
        assert_eq!(
            outcome,
            ("unavailable".to_string(), "missing_source".to_string())
        );
        let checkpoint = sqlx::query_as::<_, (i64, i64, i64)>(
            "SELECT next_archive_batch_id, manifest_high_watermark_id, completed
             FROM summary_archive_snapshot_backfill_checkpoint WHERE scope = ?1",
        )
        .bind(SUMMARY_ARCHIVE_SNAPSHOT_BACKFILL_SCOPE)
        .fetch_one(&pool)
        .await
        .expect("load Summary Snapshot backfill checkpoint");
        assert_eq!(checkpoint, (1, 1, 1));
    }

    #[tokio::test]
    async fn summary_archive_snapshot_backfill_reuses_verified_v2_without_raw_source() {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("memory pool");
        crate::schema::ensure_schema(&pool)
            .await
            .expect("current schema");
        let manifest_sha256 = "verified-v2-authority";
        sqlx::query(
            r#"
            INSERT INTO archive_batches (
                id, dataset, month_key, file_path, sha256, row_count, status,
                summary_source_kind, coverage_start_at, coverage_end_at
            )
            VALUES (1, 'codex_invocations', '2026-08',
                    '/legacy/already-snapshotted.sqlite.gz', ?1, 1,
                    'completed', 'unknown', '2026-08-01 00:00:00',
                    '2026-08-01 00:00:00')
            "#,
        )
        .bind(manifest_sha256)
        .execute(&pool)
        .await
        .expect("seed verified Summary archive authority");
        let payload = serde_json::to_vec(&vec![SummaryArchiveSnapshotV2Record {
            id: 1,
            invoke_id: "verified-v2-invocation".to_string(),
            occurred_at: "2026-08-01 00:00:00".to_string(),
            source: "proxy".to_string(),
            model: None,
            response_model: None,
            input_tokens: 0,
            output_tokens: 0,
            cache_input_tokens: 0,
            reasoning_tokens: 0,
            reasoning_effort: None,
            total_tokens: 0,
            cost: None,
            cost_input: None,
            cost_cache_write: None,
            cost_cache_read: None,
            cost_output: None,
            cost_reasoning: None,
            status: "success".to_string(),
            error_message: None,
            failure_kind: None,
            failure_class: None,
            is_actionable: false,
            upstream_account_id: None,
        }])
        .expect("encode verified V2 payload");
        let page = SummaryArchiveSnapshotPage {
            archive_batch_id: 1,
            manifest_sha256: manifest_sha256.to_string(),
            page_index: 0,
            coverage_start: "2026-08-01 00:00:00".to_string(),
            coverage_end: "2026-08-01 00:00:00".to_string(),
            row_count: 1,
            payload: zstd::stream::encode_all(payload.as_slice(), 1)
                .expect("compress verified V2 payload"),
        };
        let mut tx = pool.begin().await.expect("begin V2 proof transaction");
        store_summary_archive_snapshot_page_v2_tx(tx.as_mut(), &page)
            .await
            .expect("store verified V2 page");
        tx.commit().await.expect("commit verified V2 page");

        let result = backfill_summary_archive_snapshots_v2_window(&pool, Duration::from_secs(1))
            .await
            .expect("run V2 proof backfill");
        assert_eq!(result.materialized_archive_batches, 1);
        assert_eq!(result.unavailable_archive_batches, 0);
        let disposition: String = sqlx::query_scalar(
            "SELECT disposition FROM summary_archive_snapshot_backfill_outcome
             WHERE archive_batch_id = 1 AND manifest_sha256 = ?1",
        )
        .bind(manifest_sha256)
        .fetch_one(&pool)
        .await
        .expect("load verified V2 outcome");
        assert_eq!(disposition, "complete");
    }

    #[tokio::test]
    async fn summary_archive_snapshot_backfill_retry_timestamp_is_sqlite_comparable() {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("memory pool");
        crate::schema::ensure_schema(&pool)
            .await
            .expect("current schema");
        sqlx::query(
            r#"
            INSERT INTO archive_batches (
                id, dataset, month_key, file_path, sha256, row_count, status,
                summary_source_kind, coverage_start_at, coverage_end_at
            )
            VALUES (1, 'codex_invocations', '2026-08',
                    '/legacy/retryable-summary-authority.sqlite.gz', 'retryable', 1,
                    'completed', 'unknown', '2026-08-01 00:00:00',
                    '2026-08-01 01:00:00')
            "#,
        )
        .execute(&pool)
        .await
        .expect("seed retryable Summary archive authority");
        let candidate = load_summary_archive_snapshot_backfill_candidates(&pool, 0, 1)
            .await
            .expect("load candidate before outcome")[0]
            .clone();
        record_summary_archive_snapshot_backfill_outcome(
            &pool,
            &candidate,
            "unavailable",
            "missing_source",
            0,
            0,
        )
        .await
        .expect("persist retry outcome");
        let candidates = load_summary_archive_snapshot_backfill_candidates(&pool, 0, 1)
            .await
            .expect("load candidates after retry outcome");
        assert!(
            candidates.is_empty(),
            "future RFC3339 retry timestamp must be compared numerically by SQLite"
        );
    }

    #[tokio::test]
    async fn direct_authoritative_invocation_archive_publication_requires_summary_proof() {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("memory pool");
        crate::schema::ensure_schema(&pool)
            .await
            .expect("current schema");

        let error = sqlx::query(
            r#"
            INSERT INTO archive_batches (
                dataset,
                month_key,
                file_path,
                sha256,
                row_count,
                status,
                summary_source_kind,
                coverage_start_at,
                coverage_end_at,
                historical_rollups_materialized_at
            )
            VALUES (
                'codex_invocations',
                '2026-08',
                '/authoritative/missing-summary-proof.sqlite.gz',
                'missing-summary-proof',
                1,
                'completed',
                'authoritative',
                '2026-08-01 00:00:00',
                '2026-08-01 01:00:00',
                datetime('now')
            )
            "#,
        )
        .execute(&pool)
        .await
        .expect_err("direct authoritative publication must require every Summary proof");
        assert!(
            error
                .to_string()
                .contains("requires Summary publication proof")
        );
    }

    #[tokio::test]
    async fn legacy_segment_mirror_classification_requires_contiguous_live_id_coverage() {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("memory pool");
        crate::schema::ensure_schema(&pool)
            .await
            .expect("current schema");
        for id in [41_i64, 42_i64, 43_i64] {
            sqlx::query(
                "INSERT INTO codex_invocations \
                 (id, invoke_id, occurred_at, source, status, payload, raw_response) \
                 VALUES (?1, ?2, '2026-08-01 00:00:00', 'xy', 'success', '{}', '')",
            )
            .bind(id)
            .bind(format!("legacy-mirror-{id}"))
            .execute(&pool)
            .await
            .expect("seed retained live invocation");
        }
        for (file_path, part_key) in [
            (
                "/legacy/proven-live-mirror.sqlite.gz",
                "part-0000000000000029-000000000000002a-0123456789abcdef",
            ),
            (
                "/legacy/ambiguous-source.sqlite.gz",
                "part-000000000000002b-000000000000002c-0123456789abcdef",
            ),
        ] {
            sqlx::query(
                r#"
                INSERT INTO archive_batches (
                    dataset,
                    month_key,
                    part_key,
                    file_path,
                    sha256,
                    row_count,
                    status,
                    layout,
                    coverage_start_at,
                    coverage_end_at,
                    historical_rollups_materialized_at
                )
                VALUES (
                    'codex_invocations',
                    '2026-08',
                    ?1,
                    ?2,
                    ?3,
                    1,
                    'completed',
                    'segment_v1',
                    '2026-08-01 00:00:00',
                    '2026-08-01 01:00:00',
                    datetime('now')
                )
                "#,
            )
            .bind(part_key)
            .bind(file_path)
            .bind(format!("legacy-{part_key}"))
            .execute(&pool)
            .await
            .expect("seed legacy segment manifest");
        }

        crate::schema::ensure_schema(&pool)
            .await
            .expect("classify legacy segment manifests");
        let states = sqlx::query_as::<_, (String, String)>(
            "SELECT file_path, summary_source_kind FROM archive_batches ORDER BY file_path",
        )
        .fetch_all(&pool)
        .await
        .expect("load classified archive source kinds");
        assert_eq!(
            states,
            vec![
                (
                    "/legacy/ambiguous-source.sqlite.gz".to_string(),
                    "unknown".to_string(),
                ),
                (
                    "/legacy/proven-live-mirror.sqlite.gz".to_string(),
                    "live_mirror".to_string(),
                ),
            ]
        );
    }

    #[tokio::test]
    async fn finalization_keeps_a_reactivated_legacy_archive_file() {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("memory pool");
        crate::schema::ensure_schema(&pool)
            .await
            .expect("full schema");
        let archive_path = std::env::temp_dir().join(format!(
            "codex-vibe-monitor-reactivated-archive-{}-{}.sqlite.gz",
            std::process::id(),
            Utc::now().timestamp_nanos_opt().unwrap_or_default(),
        ));
        fs::write(&archive_path, b"old archive content").expect("write old archive file");
        let old_sha256 = sha256_hex_file(&archive_path).expect("hash old archive file");
        let archive_path_string = archive_path.to_string_lossy().to_string();
        sqlx::query(
            r#"
            INSERT INTO archive_batches (
                id,
                dataset,
                month_key,
                file_path,
                sha256,
                row_count,
                status,
                created_at
            )
            VALUES (1, 'codex_quota_snapshots', '2025-01', ?1, ?2, 1, 'completed', datetime('now'))
            "#,
        )
        .bind(&archive_path_string)
        .bind(&old_sha256)
        .execute(&pool)
        .await
        .expect("insert staged archive manifest");
        assert!(
            stage_archive_batch_deletion(
                &pool,
                1,
                "codex_quota_snapshots",
                &archive_path_string,
                &old_sha256,
                None,
            )
            .await
            .expect("stage old archive deletion")
        );

        fs::write(&archive_path, b"replacement archive content")
            .expect("replace archive file before manifest rewrite");
        let replacement_sha256 = sha256_hex_file(&archive_path).expect("hash replacement archive");
        sqlx::query("UPDATE archive_batches SET sha256 = ?1, cleanup_state = ?2 WHERE id = 1")
            .bind(&replacement_sha256)
            .bind(ARCHIVE_CLEANUP_STATE_ACTIVE)
            .execute(&pool)
            .await
            .expect("reactivate rewritten archive manifest");

        let finalized = finalize_archive_batch_file_deletion(
            &pool,
            1,
            "codex_quota_snapshots",
            &archive_path_string,
            &old_sha256,
        )
        .await
        .expect("ignore stale archive deletion finalizer");
        assert!(!finalized);
        assert_eq!(
            fs::read(&archive_path).expect("read replacement archive"),
            b"replacement archive content"
        );
        let manifest: (String, String) =
            sqlx::query_as("SELECT sha256, cleanup_state FROM archive_batches WHERE id = 1")
                .fetch_one(&pool)
                .await
                .expect("load reactivated archive manifest");
        assert_eq!(manifest.0, replacement_sha256);
        assert_eq!(manifest.1, ARCHIVE_CLEANUP_STATE_ACTIVE);

        let _ = fs::remove_file(&archive_path);
    }

    #[tokio::test]
    async fn failed_file_removal_does_not_advance_staged_source_boundary() {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("memory pool");
        crate::schema::ensure_schema(&pool)
            .await
            .expect("full schema");
        let archive_path = std::env::temp_dir().join(format!(
            "codex-vibe-monitor-pending-source-boundary-{}-{}.sqlite.gz",
            std::process::id(),
            Utc::now().timestamp_nanos_opt().unwrap_or_default(),
        ));
        fs::write(&archive_path, b"pending archive content").expect("write pending archive file");
        let archive_sha256 = sha256_hex_file(&archive_path).expect("hash pending archive file");
        let archive_path_string = archive_path.to_string_lossy().to_string();
        sqlx::query(
            r#"
            INSERT INTO archive_batches (
                id,
                dataset,
                month_key,
                file_path,
                sha256,
                row_count,
                status,
                cleanup_state,
                cleanup_source_safe_start_date,
                created_at
            )
            VALUES (1, 'codex_quota_snapshots', '2025-01', ?1, ?2, 1, 'completed', 'delete_pending', '2025-01-04', datetime('now'))
            "#,
        )
        .bind(&archive_path_string)
        .bind(&archive_sha256)
        .execute(&pool)
        .await
        .expect("insert pending archive manifest");

        let finalized = finalize_archive_batch_file_deletion_with_remove(
            &pool,
            1,
            "codex_quota_snapshots",
            &archive_path_string,
            &archive_sha256,
            |_| {
                Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "forced archive deletion failure",
                ))
            },
        )
        .await
        .expect("failed file removal should remain retryable");
        assert!(!finalized);
        assert!(archive_path.exists());
        let integrity_source_start: Option<String> = sqlx::query_scalar(
            "SELECT integrity_source_start_date FROM long_term_stats_state WHERE id = 1",
        )
        .fetch_one(&pool)
        .await
        .expect("load source boundary after failed removal");
        assert!(integrity_source_start.is_none());

        let finalized = finalize_archive_batch_file_deletion(
            &pool,
            1,
            "codex_quota_snapshots",
            &archive_path_string,
            &archive_sha256,
        )
        .await
        .expect("retry pending archive deletion");
        assert!(finalized);
        let integrity_source_start: Option<String> = sqlx::query_scalar(
            "SELECT integrity_source_start_date FROM long_term_stats_state WHERE id = 1",
        )
        .fetch_one(&pool)
        .await
        .expect("load source boundary after finalized removal");
        assert_eq!(integrity_source_start.as_deref(), Some("2025-01-04"));
    }
}
