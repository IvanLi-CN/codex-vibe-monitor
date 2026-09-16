use super::*;
use futures_util::{StreamExt, stream};
use sha2_resumable::{
    Sha256 as ResumableSha256,
    digest::{
        Digest as ResumableDigest,
        common::hazmat::{SerializableState, SerializedState},
    },
};
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

    if dataset == HOURLY_ROLLUP_DATASET_INVOCATIONS {
        let proof_exists = sqlx::query_scalar::<_, i64>(
            "SELECT EXISTS(SELECT 1 FROM summary_archive_snapshot_v2_proof \
             WHERE archive_batch_id = ?1 AND manifest_sha256 = ?2)",
        )
        .bind(archive_batch_id)
        .bind(expected_sha256)
        .fetch_one(tx.as_mut())
        .await?
            != 0;
        // Re-validate the complete V2 proof under the same writer transaction that stages raw
        // deletion. This closes the race where a recovery pass deletes or rewrites pages after
        // the outer cleanup precheck but before finalization.
        if !proof_exists
            || !summary_archive_snapshot_has_proof_tx(
                tx.as_mut(),
                archive_batch_id,
                expected_sha256,
            )
            .await?
        {
            tx.rollback().await?;
            return Ok(false);
        }
    }
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
    let metadata_deleted = delete_archive_batch_metadata_tx(
        tx.as_mut(),
        archive_batch_id,
        dataset,
        file_path,
        expected_sha256,
    )
    .await?;
    if !metadata_deleted {
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
