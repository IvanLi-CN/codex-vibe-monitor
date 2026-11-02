use super::*;
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

#[derive(Debug, FromRow)]
pub(crate) struct ArchiveBatchCleanupCandidate {
    id: i64,
    dataset: String,
    file_path: String,
    sha256: String,
    summary_source_kind: String,
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

pub(super) async fn stage_archive_batch_deletion(
    pool: &Pool<Sqlite>,
    archive_batch_id: i64,
    dataset: &str,
    file_path: &str,
    expected_sha256: &str,
    source_safe_start: Option<NaiveDate>,
) -> Result<bool> {
    let _archive_lock =
        super::super::retention::retention_archive_file_lock_async(Path::new(file_path))
            .await
            .context("failed to lock archive cleanup path")?;
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

pub(super) async fn finalize_archive_batch_file_deletion(
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

pub(super) async fn finalize_archive_batch_file_deletion_with_remove<F>(
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
    let parent_identity =
        super::super::retention::retention_archive_parent_identity(Path::new(file_path));
    let mut archive_lock =
        super::super::retention::retention_archive_file_lock_async(Path::new(file_path))
            .await
            .context("failed to lock archive cleanup path")?;
    let Some(_archive_operation_lease) =
        crate::try_acquire_archive_file_operation_lease(Path::new(file_path))
    else {
        warn!(
            dataset,
            file_path, "archive file deletion deferred while a file operation is active"
        );
        return Ok(false);
    };
    let Some(file_phase) = crate::try_acquire_file_phase_permit() else {
        return Ok(false);
    };
    let file_sha256 = if Path::new(file_path).exists() {
        match sha256_hex_file_with_deadline(
            Path::new(file_path),
            super::super::retention::retention_archive_file_operation_deadline(),
        ) {
            Ok(value) => Some(value),
            Err(error)
                if super::super::retention::is_retention_archive_file_work_transient(&error) =>
            {
                return Err(error);
            }
            Err(error) => {
                warn!(
                    dataset,
                    error_fingerprint = %super::super::retention::retention_error_fingerprint(&error),
                    "archive file identity could not be verified; retaining pending metadata"
                );
                return Ok(false);
            }
        }
    } else {
        None
    };
    drop(file_phase);
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

    if parent_identity
        != super::super::retention::retention_archive_parent_identity(Path::new(file_path))
    {
        tx.rollback().await?;
        return Ok(false);
    }

    // A missing parent cannot be fenced by the initial lock attempt. Re-check it after taking
    // the SQLite writer lock and acquire the recreated directory before deleting anything.
    if !archive_lock.is_held() {
        let Some(parent) = Path::new(file_path).parent() else {
            tx.rollback().await?;
            return Ok(false);
        };
        if parent.exists() {
            archive_lock =
                super::super::retention::retention_archive_file_lock_async(Path::new(file_path))
                    .await
                    .context("failed to lock recreated archive cleanup path")?;
            if !archive_lock.is_held() {
                tx.rollback().await?;
                return Ok(false);
            }
            let Some(file_phase) = crate::try_acquire_file_phase_permit() else {
                tx.rollback().await?;
                return Ok(false);
            };
            let current_sha256 = if Path::new(file_path).is_file() {
                Some(sha256_hex_file_with_deadline(
                    Path::new(file_path),
                    super::super::retention::retention_archive_file_operation_deadline(),
                )?)
            } else {
                None
            };
            drop(file_phase);
            if current_sha256 != file_sha256 {
                tx.rollback().await?;
                return Ok(false);
            }
        } else {
            // A missing parent cannot be fenced. Keep the manifest pending so a concurrent
            // writer may recreate the directory and publish under its own directory lock.
            tx.rollback().await?;
            return Ok(false);
        }
    }

    let summary_source_kind = sqlx::query_scalar::<_, String>(
        "SELECT COALESCE(summary_source_kind, 'unknown') FROM archive_batches \
         WHERE id = ?1 AND dataset = ?2 AND file_path = ?3 AND sha256 = ?4 \
           AND status = ?5 AND cleanup_state = ?6",
    )
    .bind(archive_batch_id)
    .bind(dataset)
    .bind(file_path)
    .bind(expected_sha256)
    .bind(ARCHIVE_STATUS_COMPLETED)
    .bind(ARCHIVE_CLEANUP_STATE_DELETE_PENDING)
    .fetch_optional(tx.as_mut())
    .await?;
    let is_live_mirror = summary_source_kind.as_deref()
        == Some(crate::maintenance::retention::SUMMARY_ARCHIVE_SOURCE_KIND_LIVE_MIRROR);
    if dataset == HOURLY_ROLLUP_DATASET_INVOCATIONS && !is_live_mirror {
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

    if file_sha256.as_deref() != Some(expected_sha256) && file_sha256.is_some() {
        warn!(
            dataset,
            expected_sha256,
            file_sha256 = ?file_sha256,
            "archive file identity changed after deletion was staged; retaining reactivated manifest"
        );
        tx.rollback().await?;
        return Ok(false);
    }

    let Some(file_phase) = crate::try_acquire_file_phase_permit() else {
        tx.rollback().await?;
        return Ok(false);
    };
    match remove_file(file_path) {
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => {
            warn!(
                dataset,
                error = %error,
                "archive file deletion is pending; retaining metadata for a later retry"
            );
            tx.rollback().await?;
            return Ok(false);
        }
    }
    drop(file_phase);
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
    let prepared_archive_guard = if sqlx::query_scalar::<_, i64>(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'retention_prepared_archives')",
    )
    .fetch_one(pool)
    .await?
        != 0
    {
        "AND NOT EXISTS (
              SELECT 1
              FROM retention_prepared_archives AS prepared
              WHERE prepared.file_path = archive_batches.file_path
                AND prepared.state IN ('preparing', 'published')
          )"
    } else {
        ""
    };
    let candidates = sqlx::query_as::<_, ArchiveBatchCleanupCandidate>(&format!(
        r#"
        SELECT id, dataset, file_path, sha256,
               COALESCE(summary_source_kind, 'unknown') AS summary_source_kind,
               cleanup_state, historical_rollups_materialized_at, coverage_end_at
        FROM archive_batches
        WHERE status = ?1
          AND sha256 IS NOT NULL
          AND TRIM(sha256) <> ''
          AND replacement_staged_path IS NULL
          {prepared_archive_guard}
          AND archive_expires_at IS NOT NULL
          AND archive_expires_at < ?2
        ORDER BY archive_expires_at ASC, id ASC
        LIMIT ?3
        "#,
    ))
    .bind(ARCHIVE_STATUS_COMPLETED)
    .bind(&cutoff)
    .bind(super::super::retention::retention_candidate_limit(config, "archive_cleanup") as i64)
    .fetch_all(pool)
    .await?;
    let materialized_pool_upstream_cache_files = sqlx::query_as::<_, (String, String)>(
        r#"
        SELECT file_path, archive_sha256
        FROM hourly_rollup_archive_replay
        WHERE target = ?1
          AND dataset = 'pool_upstream_request_attempts'
          AND archive_sha256 IS NOT NULL
          AND TRIM(archive_sha256) <> ''
        "#,
    )
    .bind(POOL_UPSTREAM_NODE_HEALTH_ARCHIVE_REPLAY_TARGET)
    .fetch_all(pool)
    .await?
    .into_iter()
    .collect::<HashSet<_>>();
    let materialized_pool_upstream_hourly_files = sqlx::query_as::<_, (String, String)>(
        r#"
        SELECT file_path, archive_sha256
        FROM hourly_rollup_archive_replay
        WHERE target = ?1
          AND dataset = 'pool_upstream_request_attempts'
          AND archive_sha256 IS NOT NULL
          AND TRIM(archive_sha256) <> ''
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
        if candidate.dataset == HOURLY_ROLLUP_DATASET_INVOCATIONS
            && candidate.summary_source_kind
                == crate::maintenance::retention::SUMMARY_ARCHIVE_SOURCE_KIND_LIVE_MIRROR
        {
            let mirror_path = Path::new(&candidate.file_path);
            let owned = crate::maintenance::retention::retention_archive_path_is_within_root(
                config,
                mirror_path,
            ) && (!mirror_path.exists()
                || crate::maintenance::retention::retention_archive_path_is_owned(
                    config,
                    mirror_path,
                ));
            if !owned {
                warn!(
                    dataset = candidate.dataset,
                    "retention live-mirror cleanup rejected an archive path outside its owned root"
                );
                continue;
            }
            // A live mirror is deliberately outside Summary authority. Its TTL is sufficient
            // cleanup evidence once the completed manifest is staged; it has no rollup or
            // Snapshot proof to wait for.
            eligible_candidates.push(candidate);
            continue;
        }
        if candidate.cleanup_state == ARCHIVE_CLEANUP_STATE_DELETE_PENDING {
            if candidate.dataset == HOURLY_ROLLUP_DATASET_INVOCATIONS
                && !summary_archive_snapshot_cleanup_gate_satisfied(
                    pool,
                    candidate.id,
                    &candidate.sha256,
                )
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
                || !materialized_pool_upstream_cache_files
                    .contains(&(candidate.file_path.clone(), candidate.sha256.clone()))
                || !materialized_pool_upstream_hourly_files
                    .contains(&(candidate.file_path.clone(), candidate.sha256.clone())))
        {
            continue;
        }
        if candidate.dataset == "pool_upstream_request_attempts"
            && !long_term_stats_attempt_archive_files
                .contains(&(candidate.file_path.clone(), candidate.sha256.clone()))
        {
            continue;
        }
        if candidate.dataset == HOURLY_ROLLUP_DATASET_INVOCATIONS
            && !long_term_stats_archive_files
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
            && !summary_archive_snapshot_cleanup_gate_satisfied(
                pool,
                candidate.id,
                &candidate.sha256,
            )
            .await?
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
                archive_batch_id = candidate.id,
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
                crate::record_managed_task_processed_work(&["prune_archive_batches"], 1);
            }
            continue;
        }
        if candidate.dataset == HOURLY_ROLLUP_DATASET_INVOCATIONS
            && candidate.summary_source_kind
                == crate::maintenance::retention::SUMMARY_ARCHIVE_SOURCE_KIND_LIVE_MIRROR
        {
            // Mirrors do not own historical rollup boundaries. Stage and finalize them with no
            // source-safe-start marker, while the finalizer still rechecks file identity.
            let staged = stage_archive_batch_deletion(
                pool,
                candidate.id,
                &candidate.dataset,
                &candidate.file_path,
                &candidate.sha256,
                None,
            )
            .await?;
            if staged
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
                crate::record_managed_task_processed_work(&["prune_archive_batches"], 1);
            }
            continue;
        }
        let file_missing = match fs::metadata(&candidate.file_path) {
            Ok(_) => false,
            Err(error) if error.kind() == io::ErrorKind::NotFound => true,
            Err(error) => {
                warn!(
                    dataset = candidate.dataset,
                    archive_batch_id = candidate.id,
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
                crate::record_managed_task_processed_work(&["prune_archive_batches"], 1);
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
                        archive_batch_id = candidate.id,
                        error = %error,
                        "could not prove long-term source boundary; retaining expired archive batch"
                    );
                    continue;
                }
            }
        } else {
            None
        };
        let staged = stage_archive_batch_deletion(
            pool,
            candidate.id,
            &candidate.dataset,
            &candidate.file_path,
            &candidate.sha256,
            integrity_source_safe_start,
        )
        .await?;
        if staged
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
            crate::record_managed_task_processed_work(&["prune_archive_batches"], 1);
        }
    }

    Ok(deleted)
}

async fn summary_archive_snapshot_cleanup_gate_satisfied(
    pool: &Pool<Sqlite>,
    archive_batch_id: i64,
    manifest_sha256: &str,
) -> Result<bool> {
    let v2_page_exists = sqlx::query_scalar::<_, i64>(
        "SELECT EXISTS(SELECT 1 FROM summary_archive_snapshot \
         WHERE archive_batch_id = ?1 AND manifest_sha256 = ?2 AND format_version = 2)",
    )
    .bind(archive_batch_id)
    .bind(manifest_sha256)
    .fetch_one(pool)
    .await?
        != 0;
    if !v2_page_exists {
        // A legacy archive without even an intermediate V2 page is still the only raw
        // authority.  Do not let retention cleanup race the recovery supervisor; raw deletion
        // is admitted only after a final V2 proof exists.
        return Ok(false);
    }
    let proof_marker_exists = sqlx::query_scalar::<_, i64>(
        "SELECT EXISTS(SELECT 1 FROM summary_archive_snapshot_v2_proof \
         WHERE archive_batch_id = ?1 AND manifest_sha256 = ?2)",
    )
    .bind(archive_batch_id)
    .bind(manifest_sha256)
    .fetch_one(pool)
    .await?
        != 0;
    if !proof_marker_exists {
        return Ok(false);
    }
    summary_archive_snapshot_has_final_proof_read_only(pool, archive_batch_id, manifest_sha256)
        .await
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
    query.push(" AND sha256 IS NOT NULL AND TRIM(sha256) <> ''");
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
                crate::record_managed_task_processed_work(
                    &["prune_archive_batches", "prune_legacy_archive_batches"],
                    1,
                );
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
                archive_batch_id = candidate.id,
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
                crate::record_managed_task_processed_work(
                    &["prune_archive_batches", "prune_legacy_archive_batches"],
                    1,
                );
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
                        archive_batch_id = candidate.id,
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
            crate::record_managed_task_processed_work(
                &["prune_archive_batches", "prune_legacy_archive_batches"],
                1,
            );
        }
    }

    Ok(summary)
}
