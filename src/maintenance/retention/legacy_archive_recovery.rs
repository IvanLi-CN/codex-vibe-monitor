use super::*;

pub(crate) async fn reconcile_legacy_retention_archive_segments(
    pool: &Pool<Sqlite>,
    config: &AppConfig,
) -> Result<()> {
    retention_recovery_set_stage("legacy_reconcile");

    let not_due: i64 = sqlx::query_scalar(
        "SELECT COALESCE(next_retry_at > datetime('now'), 0)
         FROM retention_recovery_cursors WHERE scope = 'legacy_archive_segments'",
    )
    .fetch_optional(pool)
    .await?
    .unwrap_or(0);
    if not_due != 0 {
        return Ok(());
    }

    // Probe pressure and write admission before touching the archive tree. The permit is
    // released immediately so archive verification does not hold the coordinator across file
    // I/O; each database mutation below still takes its own short admission.
    let Some(admission) =
        acquire_retention_write_admission("retention_recovery_legacy_quarantine").await
    else {
        retention_recovery_persist_pressure_defer(
            pool,
            "legacy_archive_segments",
            "legacy_reconcile",
        )
        .await?;
        return Err(retention_write_deferred(
            "retention_recovery_legacy_quarantine",
        ));
    };
    drop(admission);

    let archive_root = resolved_archive_dir(config).join("codex_invocations");
    let cursor = sqlx::query_scalar::<_, String>(
        "SELECT cursor FROM retention_recovery_cursors WHERE scope = 'legacy_archive_segments'",
    )
    .fetch_optional(pool)
    .await?
    .unwrap_or_default();
    let RetentionArchiveCandidateScan {
        candidates,
        progress,
    } = collect_retention_archive_candidates_after_cursor(
        &archive_root,
        cursor.as_str(),
        RETENTION_RECOVERY_LEGACY_SCAN_BATCH,
    )?;
    for path in &candidates {
        #[cfg(test)]
        retention_test_legacy_archive_io_event();
        let file_path = path.to_string_lossy().to_string();
        let metadata = match fs::metadata(path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => {
                retention_recovery_record_failure(
                    "legacy_reconcile",
                    &anyhow!("legacy archive metadata check failed: {}", error.kind()),
                );
                continue;
            }
        };
        let _archive_lock = retention_archive_file_lock_async(path).await?;
        let sha256 = match sha256_hex_file_with_deadline(
            path,
            retention_archive_file_operation_deadline(),
        ) {
            Ok(sha256) => Some(sha256),
            Err(error) if is_retention_archive_file_work_transient(&error) => {
                retention_recovery_record_deferred("legacy_reconcile");
                return Err(error);
            }
            Err(error) => {
                retention_recovery_record_failure("legacy_reconcile", &error);
                None
            }
        };
        let verified = if let Some(sha256) = sha256.as_deref() {
            match verify_legacy_retention_archive_segment(pool, config, path, sha256).await {
                Ok(verified) => verified,
                Err(error) if is_retention_archive_file_work_transient(&error) => {
                    retention_recovery_record_deferred("legacy_reconcile");
                    return Err(error);
                }
                Err(error) => {
                    retention_recovery_record_failure("legacy_reconcile", &error);
                    None
                }
            }
        } else {
            None
        };
        let Some(admission) =
            acquire_retention_write_admission("retention_recovery_legacy_quarantine").await
        else {
            retention_recovery_persist_pressure_defer(
                pool,
                "legacy_archive_segments",
                "legacy_reconcile",
            )
            .await?;
            return Err(retention_write_deferred(
                "retention_recovery_legacy_quarantine",
            ));
        };
        let mut tx = pool.begin().await?;
        let referenced = sqlx::query_scalar::<_, i64>(
            r#"
            SELECT EXISTS(
                SELECT 1 FROM archive_batches WHERE dataset = 'codex_invocations' AND file_path = ?1
            )
            OR EXISTS(
                SELECT 1 FROM retention_prepared_archives WHERE dataset = 'codex_invocations' AND file_path = ?1
            )
            "#,
        )
        .bind(&file_path)
        .fetch_one(tx.as_mut())
        .await?;
        if referenced != 0 {
            tx.rollback().await?;
            drop(admission);
            continue;
        }
        if let Some((descriptor, verified_sha)) = verified {
            let source_ids_json = descriptor.source_ids_json.clone();
            let (coverage_start_at, coverage_end_at): (Option<String>, Option<String>) =
                sqlx::query_as(
                    "SELECT MIN(occurred_at), MAX(occurred_at) FROM codex_invocations \
                     WHERE id IN (SELECT value FROM json_each(?1))",
                )
                .bind(&source_ids_json)
                .fetch_one(tx.as_mut())
                .await?;
            let archive_expires_at = coverage_end_at
                .as_deref()
                .map(|value| {
                    shanghai_archive_expiry_from_reference_timestamp(
                        value,
                        config.invocation_archive_ttl_days,
                    )
                })
                .transpose()?;
            let row_count = serde_json::from_str::<Vec<i64>>(&source_ids_json)?.len() as i64;
            sqlx::query(
                r#"
                INSERT INTO archive_batches (
                    dataset, month_key, day_key, part_key, file_path, sha256, row_count,
                    status, layout, codec, writer_version, cleanup_state,
                    coverage_start_at, coverage_end_at, archive_expires_at,
                    summary_source_kind, historical_rollups_materialized_at
                )
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12,
                        ?13, ?14, ?15, ?16, datetime('now'))
                "#,
            )
            .bind(descriptor.dataset)
            .bind(&descriptor.month_key)
            .bind(&descriptor.day_key)
            .bind(&descriptor.part_key)
            .bind(&descriptor.file_path)
            .bind(verified_sha)
            .bind(row_count)
            .bind(ARCHIVE_STATUS_COMPLETED)
            .bind(ARCHIVE_LAYOUT_SEGMENT_V1)
            .bind(ARCHIVE_FILE_CODEC_GZIP)
            .bind(ARCHIVE_WRITER_VERSION_SEGMENT_V1)
            .bind(ARCHIVE_CLEANUP_STATE_ACTIVE)
            .bind(coverage_start_at)
            .bind(coverage_end_at)
            .bind(archive_expires_at)
            .bind(SUMMARY_ARCHIVE_SOURCE_KIND_LIVE_MIRROR)
            .execute(tx.as_mut())
            .await?;
            sqlx::query("DELETE FROM retention_prepared_archives WHERE prepared_key = ?1")
                .bind(&descriptor.prepared_key)
                .execute(tx.as_mut())
                .await?;
        } else {
            sqlx::query(
                r#"
                INSERT OR IGNORE INTO retention_prepared_archives (
                    prepared_key, dataset, month_key, file_path, source_ids_json,
                    source_identity_sha256, state, artifact_sha256, artifact_bytes,
                    attempt_count, last_failure_stage, last_failure_fingerprint, quarantined_at
                )
                VALUES (?1, 'codex_invocations', '', ?2, '[]', 'legacy-unverified',
                        ?3, ?4, ?5, 0, 'legacy_reconcile', ?6, datetime('now'))
                "#,
            )
            .bind(format!("legacy:{file_path}"))
            .bind(&file_path)
            .bind(RETENTION_RECOVERY_STATE_QUARANTINED)
            .bind(sha256)
            .bind(metadata.len() as i64)
            .bind({
                let digest = Sha256::digest(b"legacy_archive_identity_unverified");
                format!("{digest:x}")[..16].to_string()
            })
            .execute(tx.as_mut())
            .await?;
        }
        tx.commit().await?;
        drop(admission);
    }
    let progress_after_cursor = progress.as_deref().filter(|path| *path > cursor.as_str());
    if let Some(progress) = progress_after_cursor {
        advance_retention_recovery_cursor(pool, cursor.as_str(), progress).await?;
    } else if !cursor.is_empty() {
        // Wrap when the tail is exhausted so earlier-arriving files are eventually revisited.
        let Some(admission) = acquire_retention_write_admission("retention_recovery_cursor").await
        else {
            retention_recovery_persist_pressure_defer(
                pool,
                "legacy_archive_segments",
                "legacy_reconcile",
            )
            .await?;
            return Err(retention_write_deferred("retention_recovery_cursor"));
        };
        sqlx::query(
            r#"
            UPDATE retention_recovery_cursors
            SET cursor = '', updated_at = datetime('now')
            WHERE scope = 'legacy_archive_segments' AND cursor = ?1
            "#,
        )
        .bind(&cursor)
        .execute(pool)
        .await?;
        drop(admission);
    }
    retention_recovery_schedule_legacy_success(pool, !candidates.is_empty()).await?;
    retention_recovery_refresh_counts(pool, config).await?;
    Ok(())
}

pub(crate) async fn retention_recovery_schedule_legacy_success(
    pool: &Pool<Sqlite>,
    progressed: bool,
) -> Result<()> {
    let retry_secs = if progressed {
        RETENTION_RECOVERY_PROGRESS_RETRY_SECS
    } else {
        RETENTION_RECOVERY_EMPTY_RETRY_SECS
    };
    let Some(admission) =
        acquire_retention_write_admission("retention_recovery_legacy_schedule").await
    else {
        retention_recovery_persist_pressure_defer(
            pool,
            "legacy_archive_segments",
            "legacy_reconcile",
        )
        .await?;
        return Err(retention_write_deferred(
            "retention_recovery_legacy_schedule",
        ));
    };
    sqlx::query(
        "UPDATE retention_recovery_cursors
         SET next_retry_at = datetime('now', ?1),
             consecutive_failure_count = 0,
             last_failure_fingerprint = NULL,
             defer_reason = NULL,
             last_progress_at = CASE WHEN ?2 = 1 THEN datetime('now') ELSE last_progress_at END,
             updated_at = datetime('now')
         WHERE scope = 'legacy_archive_segments'",
    )
    .bind(format!("+{retry_secs} seconds"))
    .bind(i64::from(progressed))
    .execute(pool)
    .await?;
    drop(admission);
    Ok(())
}

pub(crate) async fn advance_retention_recovery_cursor(
    pool: &Pool<Sqlite>,
    observed_cursor: &str,
    progress: &str,
) -> Result<()> {
    advance_retention_recovery_cursor_for_scope(
        pool,
        "legacy_archive_segments",
        observed_cursor,
        progress,
    )
    .await
}

pub(crate) async fn advance_retention_recovery_cursor_for_scope(
    pool: &Pool<Sqlite>,
    scope: &str,
    observed_cursor: &str,
    progress: &str,
) -> Result<()> {
    let Some(admission) = acquire_retention_write_admission("retention_recovery_cursor").await
    else {
        retention_recovery_persist_pressure_defer(pool, scope, "legacy_reconcile").await?;
        return Err(retention_write_deferred("retention_recovery_cursor"));
    };
    sqlx::query(
        r#"
        INSERT INTO retention_recovery_cursors (scope, cursor, updated_at)
        VALUES (?1, ?2, datetime('now'))
        ON CONFLICT(scope) DO UPDATE SET
            cursor = CASE
                WHEN retention_recovery_cursors.cursor = ?3
                    AND excluded.cursor > retention_recovery_cursors.cursor
                    THEN excluded.cursor
                ELSE retention_recovery_cursors.cursor
            END,
            updated_at = CASE
                WHEN retention_recovery_cursors.cursor = ?3
                    AND excluded.cursor > retention_recovery_cursors.cursor
                    THEN excluded.updated_at
                ELSE retention_recovery_cursors.updated_at
            END
        "#,
    )
    .bind(scope)
    .bind(progress)
    .bind(observed_cursor)
    .execute(pool)
    .await?;
    drop(admission);
    Ok(())
}
