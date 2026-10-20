use super::*;

#[derive(Debug, Clone)]
pub(crate) struct RetentionPreparedArchiveDescriptor {
    pub(crate) prepared_key: String,
    pub(crate) dataset: &'static str,
    pub(crate) month_key: String,
    pub(crate) day_key: Option<String>,
    pub(crate) part_key: Option<String>,
    pub(crate) file_path: String,
    pub(crate) source_ids_json: String,
    pub(crate) source_identity_sha256: String,
    pub(crate) publication_kind: &'static str,
}

pub(crate) fn retention_prepared_archive_descriptor(
    config: &AppConfig,
    dataset: &'static str,
    group_key: &str,
    candidates: &[InvocationArchiveCandidate],
    source_identity_sha256: String,
    publication_kind: &'static str,
) -> Result<RetentionPreparedArchiveDescriptor> {
    let mut ids = candidates
        .iter()
        .map(|candidate| candidate.id)
        .collect::<Vec<_>>();
    ids.sort_unstable();
    ids.dedup();
    if ids.is_empty() {
        bail!("retention prepared archive requires source ids");
    }
    let source_ids_json =
        serde_json::to_string(&ids).context("encode retention prepared archive source ids")?;
    let layout = archive_layout_for_dataset(config, dataset);
    let (month_key, day_key, part_key, file_path) = match layout {
        ArchiveBatchLayout::LegacyMonth => {
            let file_path = archive_batch_file_path(config, dataset, group_key)?;
            (
                group_key.to_string(),
                None,
                None,
                file_path.to_string_lossy().to_string(),
            )
        }
        ArchiveBatchLayout::SegmentV1 => {
            let part_key = archive_segment_part_key_for_ids(&ids)?;
            let file_path = archive_segment_file_path(
                config,
                dataset,
                group_key,
                &part_key,
                config.invocation_archive_codec,
            )?;
            (
                archive_month_key_from_day_key(group_key)?,
                Some(group_key.to_string()),
                Some(part_key),
                file_path.to_string_lossy().to_string(),
            )
        }
    };
    Ok(RetentionPreparedArchiveDescriptor {
        prepared_key: format!("{dataset}:{file_path}:{source_identity_sha256}"),
        dataset,
        month_key,
        day_key,
        part_key,
        file_path,
        source_ids_json,
        source_identity_sha256,
        publication_kind,
    })
}

pub(crate) fn retention_recovery_path_matches(
    config: &AppConfig,
    actual: &str,
    expected: &Path,
) -> bool {
    let actual = Path::new(actual);
    actual == expected
        || (actual.is_relative()
            && config
                .database_path
                .parent()
                .is_some_and(|parent| parent.join(actual) == expected))
}

pub(crate) fn retention_recovery_publication_kind_from_path(
    config: &AppConfig,
    file_path: &str,
    month_key: &str,
    day_key: Option<&str>,
    part_key: Option<&str>,
    source_ids_json: &str,
    source_identity_sha256: &str,
) -> Option<&'static str> {
    let source_ids = serde_json::from_str::<Vec<i64>>(source_ids_json).ok()?;
    let group_key = day_key.unwrap_or(month_key);
    if !source_ids.is_empty()
        && retention_live_mirror_archive_path(
            config,
            group_key,
            &source_ids,
            source_identity_sha256,
        )
        .ok()
        .is_some_and(|expected| retention_recovery_path_matches(config, file_path, &expected))
    {
        return Some(RETENTION_RECOVERY_PUBLICATION_DETAIL_PRUNE);
    }

    let expected = match (day_key, part_key) {
        (Some(day_key), Some(part_key)) => archive_segment_file_path(
            config,
            "codex_invocations",
            day_key,
            part_key,
            config.invocation_archive_codec,
        )
        .ok(),
        (None, None) => archive_batch_file_path(config, "codex_invocations", month_key).ok(),
        _ => None,
    }?;
    retention_recovery_path_matches(config, file_path, &expected)
        .then_some(RETENTION_RECOVERY_PUBLICATION_INVOCATION_ARCHIVE)
}

pub(crate) fn retention_live_mirror_archive_path(
    config: &AppConfig,
    group_key: &str,
    ids: &[i64],
    source_identity_sha256: &str,
) -> Result<PathBuf> {
    match archive_layout_for_dataset(config, "codex_invocations") {
        ArchiveBatchLayout::LegacyMonth => {
            let standard_path = archive_batch_file_path(config, "codex_invocations", group_key)?;
            let file_name = format!(
                "codex_invocations-{group_key}-live-mirror-{}.sqlite.gz",
                &source_identity_sha256[..source_identity_sha256.len().min(16)]
            );
            Ok(standard_path.with_file_name(file_name))
        }
        ArchiveBatchLayout::SegmentV1 => {
            let part_key = archive_segment_part_key_for_ids(ids)?;
            let standard_path = archive_segment_file_path(
                config,
                "codex_invocations",
                group_key,
                &part_key,
                config.invocation_archive_codec,
            )?;
            Ok(standard_path.with_file_name(format!(
                "{part_key}.live-mirror.sqlite.{}",
                config.invocation_archive_codec.file_extension()
            )))
        }
    }
}

pub(crate) async fn load_retention_recovery_expired_backlog(
    pool: &Pool<Sqlite>,
    cutoff: &str,
) -> Result<(i64, Option<String>)> {
    let query_budget = retention_run_remaining_budget()
        .unwrap_or(RETENTION_BACKLOG_OBSERVER_QUERY_BUDGET)
        .min(RETENTION_BACKLOG_OBSERVER_QUERY_BUDGET);
    if query_budget.is_zero() {
        return Err(anyhow!(
            "retention recovery backlog query budget expired before connection acquisition"
        ));
    }
    let deadline = Instant::now() + query_budget;
    let mut connection = tokio::time::timeout(query_budget, pool.acquire())
        .await
        .map_err(|_| {
            anyhow!(
                "retention recovery backlog connection acquisition exceeded {}ms",
                query_budget.as_millis()
            )
        })??;
    let interrupted = Arc::new(AtomicBool::new(false));
    {
        let interrupted = interrupted.clone();
        let mut handle = tokio::time::timeout(
            deadline.saturating_duration_since(Instant::now()),
            connection.lock_handle(),
        )
        .await
        .map_err(|_| anyhow!("retention recovery backlog SQLite handle acquisition timed out"))??;
        handle.set_progress_handler(RETENTION_BACKLOG_OBSERVER_PROGRESS_OPS, move || {
            let within_budget = Instant::now() < deadline;
            if !within_budget {
                interrupted.store(true, Ordering::Release);
            }
            within_budget
        });
    }
    let query = sqlx::query_as::<_, (i64, Option<String>)>(
        r#"
            SELECT COUNT(*), MIN(occurred_at)
            FROM codex_invocations
            WHERE occurred_at < ?1
            "#,
    )
    .bind(cutoff)
    .fetch_one(&mut *connection);
    let result =
        tokio::time::timeout(deadline.saturating_duration_since(Instant::now()), query).await;
    if let Err(error) =
        clear_retention_backlog_observer_progress_handler(&mut connection, deadline).await
    {
        connection.close_on_drop();
        return Err(error);
    }
    let backlog = match result {
        Ok(Ok(backlog)) => backlog,
        Ok(Err(error)) if interrupted.load(Ordering::Acquire) => {
            connection.close_on_drop();
            return Err(anyhow!(
                "retention recovery backlog SQLite query was cancelled: {error}"
            ));
        }
        Ok(Err(error)) => {
            connection.close_on_drop();
            return Err(error.into());
        }
        Err(_) => {
            connection.close_on_drop();
            return Err(anyhow!(
                "retention recovery backlog query exceeded {}ms",
                query_budget.as_millis()
            ));
        }
    };
    drop(connection);
    Ok(backlog)
}

#[cfg(test)]
pub(crate) async fn retention_test_refresh_recovery_counts(
    pool: &Pool<Sqlite>,
    config: &AppConfig,
) -> Result<RetentionRecoveryHealthSnapshot> {
    retention_recovery_refresh_counts(pool, config).await?;
    Ok(retention_recovery_health_snapshot())
}

pub(crate) async fn retention_recovery_refresh_counts(
    pool: &Pool<Sqlite>,
    config: &AppConfig,
) -> Result<()> {
    let (prepared_count, quarantined_count, next_retry_at) =
        sqlx::query_as::<_, (i64, i64, Option<String>)>(
            r#"
        SELECT
            COALESCE(SUM(CASE WHEN state IN ('preparing', 'published') THEN 1 ELSE 0 END), 0),
            COALESCE(SUM(CASE WHEN state = 'quarantined' THEN 1 ELSE 0 END), 0),
            MIN(next_retry_at)
        FROM retention_prepared_archives
        "#,
        )
        .fetch_one(pool)
        .await?;
    let scheduler = sqlx::query_as::<
        _,
        (
            Option<String>,
            i64,
            Option<String>,
            Option<String>,
            Option<String>,
        ),
    >(
        "SELECT next_retry_at, consecutive_failure_count, last_failure_fingerprint, defer_reason, last_progress_at
         FROM retention_recovery_cursors WHERE scope = ?1",
    )
    .bind(RETENTION_RECOVERY_PREPARED_SCOPE)
    .fetch_optional(pool)
    .await?;
    let legacy_scheduler = sqlx::query_as::<
        _,
        (
            Option<String>,
            i64,
            Option<String>,
            Option<String>,
            Option<String>,
        ),
    >(
        "SELECT next_retry_at, consecutive_failure_count, last_failure_fingerprint, defer_reason, last_progress_at
         FROM retention_recovery_cursors WHERE scope = 'legacy_archive_segments'",
    )
    .fetch_optional(pool)
    .await?;
    let latest_failure_stage: Option<String> = sqlx::query_scalar(
        "SELECT last_failure_stage
         FROM retention_prepared_archives
         WHERE state IN ('preparing', 'published', 'quarantined')
           AND last_failure_stage IS NOT NULL
         ORDER BY updated_at DESC, prepared_key DESC
         LIMIT 1",
    )
    .fetch_optional(pool)
    .await?;
    let cutoff = shanghai_local_cutoff_string(config.invocation_max_days);
    let (expired_backlog_count, oldest_backlog_at) =
        load_retention_recovery_expired_backlog(pool, &cutoff).await?;
    let now = Utc::now();
    let oldest_backlog_age_secs = oldest_backlog_at
        .as_deref()
        .and_then(parse_to_utc_datetime)
        .map(|occurred_at| now.signed_duration_since(occurred_at).num_seconds().max(0) as u64);
    let mut health = RETENTION_RECOVERY_HEALTH
        .lock()
        .expect("retention recovery health");
    health.prepared_count = Some(prepared_count.max(0) as usize);
    health.quarantined_count = Some(quarantined_count.max(0) as usize);
    health.expired_backlog_count = Some(expired_backlog_count.max(0) as usize);
    health.oldest_backlog_age_secs = oldest_backlog_age_secs;
    health.next_retry_at = [
        scheduler.as_ref().and_then(|row| row.0.clone()),
        legacy_scheduler.as_ref().and_then(|row| row.0.clone()),
        next_retry_at,
    ]
    .into_iter()
    .flatten()
    .min();
    let scheduler = if legacy_scheduler.as_ref().is_some_and(|row| row.3.is_some()) {
        legacy_scheduler.map(|row| ("legacy_archive_segments", row))
    } else {
        scheduler.map(|row| (RETENTION_RECOVERY_PREPARED_SCOPE, row))
    };
    if let Some((scope, (_, count, fingerprint, defer_reason, last_progress_at))) = scheduler {
        health.consecutive_failure_count = Some(count.max(0) as u32);
        health.failure_fingerprint = fingerprint;
        health.failure_stage = if scope == "legacy_archive_segments" {
            Some("legacy_reconcile".to_string())
        } else {
            latest_failure_stage
        };
        health.defer_reason = defer_reason;
        if last_progress_at.is_some() {
            health.last_progress_at = last_progress_at;
        }
    } else {
        health.failure_stage = latest_failure_stage;
    }
    if health.defer_reason.is_some() {
        health.state = "deferred".to_string();
    } else if health.quarantined_count.is_some_and(|count| count > 0) {
        health.state = "degraded".to_string();
    } else if health.prepared_count.is_some_and(|count| count > 0)
        || health.expired_backlog_count.is_some_and(|count| count > 0)
    {
        if !matches!(health.state.as_str(), "degraded" | "deferred") {
            health.state = "recovering".to_string();
        }
    } else if health.prepared_count == Some(0)
        && health.quarantined_count == Some(0)
        && health.expired_backlog_count == Some(0)
        && matches!(
            health.state.as_str(),
            "unknown" | "recovering" | "deferred" | "degraded"
        )
    {
        health.state = "healthy".to_string();
        health.failure_stage = None;
        health.failure_fingerprint = None;
        health.consecutive_failure_count = Some(0);
    }
    Ok(())
}

pub(crate) async fn retention_recovery_record_preparing(
    pool: &Pool<Sqlite>,
    descriptor: &RetentionPreparedArchiveDescriptor,
) -> Result<()> {
    retention_recovery_set_stage("preparing");
    let Some(admission) = acquire_retention_write_admission("retention_recovery_prepare").await
    else {
        return Err(retention_write_deferred("retention_recovery_prepare"));
    };
    let mut tx = pool.begin_with("BEGIN IMMEDIATE").await?;
    let existing = sqlx::query_as::<_, (String, String)>(
        "SELECT prepared_key, source_identity_sha256 FROM retention_prepared_archives WHERE dataset = ?1 AND file_path = ?2",
    )
    .bind(descriptor.dataset)
    .bind(&descriptor.file_path)
    .fetch_optional(tx.as_mut())
    .await?;
    if let Some((existing_key, existing_identity)) = existing
        && existing_identity != descriptor.source_identity_sha256
    {
        let source_ids = serde_json::from_str::<Vec<i64>>(&descriptor.source_ids_json)
            .context("decode retention prepared archive source ids")?;
        let compatible_identity = invocation_archive_source_identity_matches(
            tx.as_mut(),
            InvocationArchiveIdentityDatabase::Main,
            &source_ids,
            &existing_identity,
        )
        .await?;
        if !compatible_identity {
            tx.rollback().await?;
            drop(admission);
            let error = anyhow!("retention prepared archive identity collision");
            retention_recovery_persist_failure(pool, &existing_key, "preparing", &error).await?;
            return Err(retention_recovery_failure_persisted(&existing_key, error));
        }
    }
    sqlx::query(
        r#"
        INSERT INTO retention_prepared_archives (
            prepared_key,
            dataset,
            month_key,
            day_key,
            part_key,
            file_path,
            source_ids_json,
            source_identity_sha256,
            publication_kind,
            state,
            attempt_count,
            next_retry_at
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, 1, NULL)
        ON CONFLICT(dataset, file_path) DO UPDATE SET
            prepared_key = excluded.prepared_key,
            dataset = excluded.dataset,
            month_key = excluded.month_key,
            day_key = excluded.day_key,
            part_key = excluded.part_key,
            file_path = excluded.file_path,
            source_ids_json = excluded.source_ids_json,
            source_identity_sha256 = excluded.source_identity_sha256,
            publication_kind = excluded.publication_kind,
            state = CASE
                WHEN retention_prepared_archives.state = 'quarantined'
                    THEN retention_prepared_archives.state
                ELSE excluded.state
            END,
            attempt_count = retention_prepared_archives.attempt_count + 1,
            updated_at = datetime('now')
        WHERE retention_prepared_archives.next_retry_at IS NULL
           OR retention_prepared_archives.next_retry_at <= datetime('now')
        "#,
    )
    .bind(&descriptor.prepared_key)
    .bind(descriptor.dataset)
    .bind(&descriptor.month_key)
    .bind(&descriptor.day_key)
    .bind(&descriptor.part_key)
    .bind(&descriptor.file_path)
    .bind(&descriptor.source_ids_json)
    .bind(&descriptor.source_identity_sha256)
    .bind(descriptor.publication_kind)
    .bind(RETENTION_RECOVERY_STATE_PREPARING)
    .execute(tx.as_mut())
    .await?;
    let (state, retry_due): (String, i64) = sqlx::query_as(
        r#"
        SELECT state,
               CASE WHEN next_retry_at IS NOT NULL AND next_retry_at > datetime('now') THEN 1 ELSE 0 END
        FROM retention_prepared_archives
        WHERE prepared_key = ?1
        "#,
    )
    .bind(&descriptor.prepared_key)
    .fetch_one(tx.as_mut())
    .await?
    ;
    if state == RETENTION_RECOVERY_STATE_QUARANTINED || retry_due != 0 {
        tx.rollback().await?;
        return Err(retention_write_deferred("retention_recovery_backoff"));
    }
    tx.commit().await?;
    drop(admission);
    Ok(())
}

pub(crate) async fn retention_recovery_persist_failure(
    pool: &Pool<Sqlite>,
    prepared_key: &str,
    stage: &'static str,
    error: &anyhow::Error,
) -> Result<()> {
    retention_recovery_record_failure(stage, error);
    let fingerprint = retention_recovery_health_snapshot()
        .failure_fingerprint
        .unwrap_or_default();
    let error_text = error.to_string();
    let quarantine = error_text.contains("identity collision")
        || error_text.contains("verification failed")
        || error_text.contains("publication kind")
        || error_text.contains("ownership verification")
        || error_text.contains("semantic proof validation failed");
    let Some(admission) = acquire_retention_write_admission("retention_recovery_failure").await
    else {
        retention_recovery_persist_scheduler_cursor_without_pressure(
            pool,
            RETENTION_RECOVERY_PRESSURE_RETRY_SECS,
            Some(RETENTION_RECOVERY_DEFER_SQLITE_PRESSURE),
            false,
            Some(&fingerprint),
        )
        .await?;
        retention_recovery_record_deferred(stage);
        return Err(retention_write_deferred("retention_recovery_failure"));
    };
    let execute_started = Instant::now();
    let observed_artifact_sha256 = sqlx::query_scalar::<_, Option<String>>(
        "SELECT artifact_sha256 FROM retention_prepared_archives WHERE prepared_key = ?1",
    )
    .bind(prepared_key)
    .fetch_optional(pool)
    .await?
    .flatten();
    let attempt_count = sqlx::query_scalar::<_, i64>(
        "SELECT attempt_count FROM retention_prepared_archives WHERE prepared_key = ?1",
    )
    .bind(prepared_key)
    .fetch_optional(pool)
    .await?
    .unwrap_or(0)
    .max(0) as u32;
    let retry_seconds =
        RETENTION_RECOVERY_FAILURE_BACKOFF_SECS[attempt_count.saturating_sub(1).min(4) as usize];
    // Preserve the first quarantine timestamp so repeated identity collisions cannot postpone
    // the bounded cleanup window forever.
    sqlx::query(
        r#"
        UPDATE retention_prepared_archives
        SET state = CASE WHEN ?1 THEN ?2 ELSE state END,
            quarantined_at = CASE
                WHEN ?1 AND quarantined_at IS NULL THEN datetime('now')
                ELSE quarantined_at
            END,
            last_failure_stage = ?3,
            last_failure_fingerprint = ?4,
            next_retry_at = CASE WHEN ?1 THEN NULL ELSE datetime('now', ?5) END,
            attempt_count = attempt_count + 1,
            updated_at = datetime('now')
        WHERE prepared_key = ?6
          AND (
              state = 'preparing'
              OR (
                  state = 'published'
                  AND (
                      artifact_sha256 IS NULL
                      OR ?7 IN ('finalizing', 'publishing', 'prepared_reconcile', 'legacy_reconcile')
                  )
              )
          )
          AND artifact_sha256 IS ?8
        "#,
    )
    .bind(quarantine)
    .bind(RETENTION_RECOVERY_STATE_QUARANTINED)
    .bind(stage)
    .bind(&fingerprint)
    .bind(format!("+{retry_seconds} seconds"))
    .bind(prepared_key)
    .bind(stage)
    .bind(observed_artifact_sha256.as_deref())
    .execute(pool)
    .await?;
    sqlx::query(
        "UPDATE retention_recovery_cursors
         SET next_retry_at = datetime('now', ?1),
             consecutive_failure_count = consecutive_failure_count + 1,
             last_failure_fingerprint = ?2,
             defer_reason = ?3,
             updated_at = datetime('now')
         WHERE scope = ?4",
    )
    .bind(format!("+{retry_seconds} seconds"))
    .bind(&fingerprint)
    .bind(RETENTION_RECOVERY_DEFER_RETRY_BACKOFF)
    .bind(RETENTION_RECOVERY_PREPARED_SCOPE)
    .execute(pool)
    .await?;
    retention_record_commit!(
        "retention_recovery_failure",
        admission.admission_mode(),
        1,
        128,
        Duration::ZERO,
        admission.lock_wait(),
        execute_started.elapsed(),
        Duration::ZERO,
        admission.p1_waiter_count,
        0,
    );
    drop(admission);
    Ok(())
}

pub(crate) async fn retention_recovery_persist_latest_failure_best_effort(
    pool: &Pool<Sqlite>,
    stage: &'static str,
    error: &anyhow::Error,
) {
    let persist_result = async {
        let prepared_key = retention_recovery_current_prepared_key();
        if let Some(prepared_key) = prepared_key {
            retention_recovery_persist_failure(pool, &prepared_key, stage, error).await?;
        } else {
            retention_recovery_record_failure(stage, error);
        }
        Ok::<(), anyhow::Error>(())
    }
    .await;

    if let Err(persist_error) = persist_result {
        if is_retention_write_deferred(&persist_error) {
            retention_recovery_record_deferred(stage);
        } else {
            retention_recovery_record_failure(stage, error);
        }
        retention_recovery_log_event(
            tracing::Level::WARN,
            "failure_persist",
            "retention recovery failure state could not be persisted; continuing independent stages",
        );
    }
}

pub(crate) async fn retention_recovery_persist_prepared_failure_best_effort(
    pool: &Pool<Sqlite>,
    prepared_key: &str,
    error: &anyhow::Error,
) -> bool {
    match retention_recovery_persist_failure(pool, prepared_key, "prepared_reconcile", error).await
    {
        Ok(()) => true,
        Err(persist_error) if is_retention_write_deferred(&persist_error) => false,
        Err(persist_error) => {
            retention_recovery_record_failure("prepared_reconcile", &persist_error);
            true
        }
    }
}

pub(crate) async fn retention_recovery_persist_current_prepared_failure_best_effort(
    pool: &Pool<Sqlite>,
    error: &anyhow::Error,
) -> bool {
    let Some(prepared_key) = retention_recovery_current_prepared_key() else {
        retention_recovery_record_failure("prepared_reconcile", error);
        return true;
    };
    retention_recovery_persist_prepared_failure_best_effort(pool, &prepared_key, error).await
}

pub(crate) async fn retention_recovery_mark_published(
    pool: &Pool<Sqlite>,
    descriptor: &RetentionPreparedArchiveDescriptor,
    artifact_sha256: &str,
) -> Result<()> {
    retention_recovery_set_stage("publishing");
    let artifact_bytes = fs::metadata(&descriptor.file_path)
        .context("failed to inspect published archive artifact")?
        .len();
    let Some(admission) = acquire_retention_write_admission("retention_recovery_publish").await
    else {
        return Err(retention_write_deferred("retention_recovery_publish"));
    };
    let updated = sqlx::query(
        r#"
        UPDATE retention_prepared_archives
        SET state = ?1,
            artifact_sha256 = ?2,
            artifact_bytes = ?3,
            next_retry_at = NULL,
            last_failure_stage = NULL,
            last_failure_fingerprint = NULL,
            updated_at = datetime('now')
        WHERE prepared_key = ?4
          AND state = 'preparing'
          AND staged_file_path IS NULL
        "#,
    )
    .bind(RETENTION_RECOVERY_STATE_PUBLISHED)
    .bind(artifact_sha256)
    .bind(artifact_bytes as i64)
    .bind(&descriptor.prepared_key)
    .execute(pool)
    .await?;
    drop(admission);
    if updated.rows_affected() == 0 {
        return Err(anyhow!(
            "prepared archive journal changed during publication"
        ));
    }
    Ok(())
}

pub(crate) async fn retention_recovery_verify_publication_tx(
    tx: &mut sqlx::SqliteConnection,
    descriptor: &RetentionPreparedArchiveDescriptor,
    artifact_sha256: &str,
) -> Result<()> {
    let journal_identity = sqlx::query_scalar::<_, String>(
        "SELECT source_identity_sha256 FROM retention_prepared_archives WHERE prepared_key = ?1 AND state = 'published'",
    )
    .bind(&descriptor.prepared_key)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| anyhow!("published retention archive journal entry is missing"))?;
    let journal_publication_kind = sqlx::query_scalar::<_, Option<String>>(
        "SELECT publication_kind FROM retention_prepared_archives WHERE prepared_key = ?1",
    )
    .bind(&descriptor.prepared_key)
    .fetch_one(&mut *tx)
    .await?;
    if journal_publication_kind
        .as_deref()
        .is_some_and(|kind| kind != descriptor.publication_kind)
    {
        bail!("retention prepared archive publication kind verification failed");
    }
    sqlx::query(
        "UPDATE retention_prepared_archives
         SET publication_kind = ?1
         WHERE prepared_key = ?2 AND publication_kind IS NULL",
    )
    .bind(descriptor.publication_kind)
    .bind(&descriptor.prepared_key)
    .execute(&mut *tx)
    .await?;
    let expected_sha = sqlx::query_scalar::<_, Option<String>>(
        "SELECT artifact_sha256 FROM retention_prepared_archives WHERE prepared_key = ?1",
    )
    .bind(&descriptor.prepared_key)
    .fetch_one(&mut *tx)
    .await?
    .ok_or_else(|| anyhow!("retention prepared archive artifact digest is missing"))?;
    if expected_sha != artifact_sha256 {
        bail!("retention prepared archive artifact digest verification failed");
    }
    let source_ids = serde_json::from_str::<Vec<i64>>(&descriptor.source_ids_json)
        .context("decode retention prepared archive source ids")?;
    let source_identity_matches_journal = journal_identity == descriptor.source_identity_sha256
        && invocation_archive_source_identity_matches(
            tx,
            InvocationArchiveIdentityDatabase::Main,
            &source_ids,
            &journal_identity,
        )
        .await?;
    if !source_identity_matches_journal {
        bail!(
            "retention prepared archive source identity verification failed: live rows no longer match journal"
        );
    }
    Ok(())
}

pub(crate) async fn verify_prepared_retention_archive_artifact(
    pool: &Pool<Sqlite>,
    descriptor: &RetentionPreparedArchiveDescriptor,
    archive_path: &Path,
    expected_sha256: &str,
) -> Result<i64> {
    if descriptor.dataset != "codex_invocations" {
        bail!("unsupported prepared archive dataset for recovery verification");
    }
    let temp_path = PathBuf::from(format!(
        "{}.{}.recovery.sqlite",
        archive_path.display(),
        retention_temp_suffix()
    ));
    let _temp_cleanup = TempSqliteCleanup::new(temp_path.clone());
    inflate_gzip_sqlite_file(archive_path, &temp_path)?;
    let mut archive_db = open_archive_sqlite_connection(&temp_path).await?;
    ensure_codex_invocations_archive_schema_direct(&mut archive_db).await?;
    let archive_ids =
        sqlx::query_scalar::<_, i64>("SELECT id FROM codex_invocations ORDER BY id ASC")
            .fetch_all(&mut archive_db)
            .await?;
    let expected_ids = serde_json::from_str::<Vec<i64>>(&descriptor.source_ids_json)?;
    if descriptor.part_key.is_some() && archive_ids != expected_ids {
        bail!("prepared archive source identity verification failed");
    }
    if descriptor.part_key.is_none()
        && expected_ids
            .iter()
            .any(|expected_id| archive_ids.binary_search(expected_id).is_err())
    {
        bail!("prepared archive source identity verification failed");
    }
    let identity_ids = if descriptor.part_key.is_some() {
        archive_ids.as_slice()
    } else {
        expected_ids.as_slice()
    };
    let archive_identity_matches = invocation_archive_source_identity_matches(
        &mut archive_db,
        InvocationArchiveIdentityDatabase::Main,
        identity_ids,
        &descriptor.source_identity_sha256,
    )
    .await?;
    archive_db.close().await?;
    if !archive_identity_matches {
        bail!("prepared archive source identity verification failed");
    }
    let mut source_connection = pool.acquire().await?;
    let source_identity_matches = invocation_archive_source_identity_matches(
        &mut source_connection,
        InvocationArchiveIdentityDatabase::Main,
        &expected_ids,
        &descriptor.source_identity_sha256,
    )
    .await?;
    drop(source_connection);
    if !source_identity_matches {
        bail!("prepared archive live source identity verification failed");
    }
    if sha256_hex_file(archive_path)? != expected_sha256 {
        bail!("prepared archive artifact digest verification failed");
    }
    Ok(archive_ids.len() as i64)
}

pub(crate) fn retention_recovery_archive_outcome(
    config: &AppConfig,
    descriptor: &RetentionPreparedArchiveDescriptor,
    candidates: &[InvocationArchiveCandidate],
    artifact_sha256: &str,
    row_count: i64,
) -> Result<ArchiveBatchOutcome> {
    let (layout, writer_version) = if descriptor.part_key.is_some() {
        (ARCHIVE_LAYOUT_SEGMENT_V1, ARCHIVE_WRITER_VERSION_SEGMENT_V1)
    } else {
        (
            ARCHIVE_LAYOUT_LEGACY_MONTH,
            ARCHIVE_WRITER_VERSION_LEGACY_MONTH_V1,
        )
    };
    let mut outcome = ArchiveBatchOutcome {
        dataset: descriptor.dataset,
        month_key: descriptor.month_key.clone(),
        day_key: descriptor.day_key.clone(),
        part_key: descriptor.part_key.clone(),
        file_path: descriptor.file_path.clone(),
        sha256: artifact_sha256.to_string(),
        source_identity_sha256: Some(descriptor.source_identity_sha256.clone()),
        row_count,
        upstream_last_activity: Vec::new(),
        coverage_start_at: None,
        coverage_end_at: None,
        archive_expires_at: None,
        summary_source_kind: if descriptor.publication_kind
            == RETENTION_RECOVERY_PUBLICATION_DETAIL_PRUNE
        {
            SUMMARY_ARCHIVE_SOURCE_KIND_LIVE_MIRROR
        } else {
            SUMMARY_ARCHIVE_SOURCE_KIND_AUTHORITATIVE
        },
        layout,
        codec: ARCHIVE_FILE_CODEC_GZIP,
        writer_version,
        cleanup_state: ARCHIVE_CLEANUP_STATE_ACTIVE,
        superseded_by: None,
    };
    set_archive_batch_coverage_from_local_rows(
        &mut outcome,
        candidates
            .iter()
            .map(|candidate| candidate.occurred_at.as_str()),
        (descriptor.publication_kind == RETENTION_RECOVERY_PUBLICATION_DETAIL_PRUNE)
            .then_some(config.invocation_archive_ttl_days),
    )?;
    if descriptor.publication_kind == RETENTION_RECOVERY_PUBLICATION_INVOCATION_ARCHIVE {
        outcome.archive_expires_at = Some(shanghai_archive_expiry_from_reference_timestamp(
            &format_utc_iso(Utc::now()),
            config.invocation_archive_ttl_days,
        )?);
    }
    Ok(outcome)
}

pub(crate) async fn retention_recovery_update_detail_rows_tx(
    tx: &mut sqlx::SqliteConnection,
    ids: &[i64],
    candidates: &[InvocationArchiveCandidate],
) -> Result<()> {
    let prompt_cache_keys = load_prompt_cache_keys_for_invocation_ids_tx(tx, ids).await?;
    let prompt_cache_key_refs = prompt_cache_keys
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>();
    if !prompt_cache_key_refs.is_empty() {
        mark_prompt_cache_conversation_stats_stale_on_connection(tx).await?;
    }
    let pruned_at = format_naive(Utc::now().with_timezone(&Shanghai).naive_local());
    let mut query = QueryBuilder::<Sqlite>::new(
        "UPDATE codex_invocations SET payload = CASE WHEN json_valid(payload) AND (json_extract(payload, '$.upstreamAccountId') IS NOT NULL OR json_extract(payload, '$.requestModel') IS NOT NULL OR json_extract(payload, '$.responseModel') IS NOT NULL OR json_extract(payload, '$.reasoningEffort') IS NOT NULL OR json_extract(payload, '$.requestCompressionAlgorithm') IS NOT NULL) THEN json_patch(json_patch(json_patch(json_patch(json_patch('{}', CASE WHEN json_extract(payload, '$.upstreamAccountId') IS NOT NULL THEN json_object('upstreamAccountId', json_extract(payload, '$.upstreamAccountId')) ELSE '{}' END), CASE WHEN json_extract(payload, '$.requestModel') IS NOT NULL THEN json_object('requestModel', json_extract(payload, '$.requestModel')) ELSE '{}' END), CASE WHEN json_extract(payload, '$.responseModel') IS NOT NULL THEN json_object('responseModel', json_extract(payload, '$.responseModel')) ELSE '{}' END), CASE WHEN json_extract(payload, '$.reasoningEffort') IS NOT NULL THEN json_object('reasoningEffort', json_extract(payload, '$.reasoningEffort')) ELSE '{}' END), CASE WHEN json_extract(payload, '$.requestCompressionAlgorithm') IS NOT NULL THEN json_object('requestCompressionAlgorithm', json_extract(payload, '$.requestCompressionAlgorithm')) ELSE '{}' END) ELSE NULL END, raw_response = '', request_raw_path = NULL, request_raw_codec = 'identity', request_raw_size = NULL, request_raw_truncated = 0, request_raw_truncated_reason = NULL, response_raw_path = NULL, response_raw_codec = 'identity', response_raw_size = NULL, response_raw_truncated = 0, response_raw_truncated_reason = NULL, detail_level = ",
    );
    query
        .push_bind(DETAIL_LEVEL_STRUCTURED_ONLY)
        .push(", detail_pruned_at = ")
        .push_bind(pruned_at)
        .push(", detail_prune_reason = ")
        .push_bind(DETAIL_PRUNE_REASON_SUCCESS_OVER_30D)
        .push(" WHERE id IN (");
    {
        let mut separated = query.separated(", ");
        for id in ids {
            separated.push_bind(id);
        }
    }
    query.push(")");
    query.build().execute(&mut *tx).await?;
    // The source mutation and durable invalidation trigger commit together. The materialization
    // owner refreshes these keys outside the retention publication transaction.
    if let Some(latest) = candidates
        .iter()
        .map(|candidate| candidate.occurred_at.as_str())
        .max()
    {
        record_parallel_work_unrecoverable_detail_tx(&mut *tx, latest).await?;
    }
    Ok(())
}

pub(crate) async fn load_prompt_cache_keys_for_invocation_ids_tx(
    tx: &mut sqlx::SqliteConnection,
    ids: &[i64],
) -> Result<Vec<String>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let ids_json = serde_json::to_string(ids)?;
    sqlx::query_scalar::<_, String>(&format!(
        "SELECT DISTINCT {INVOCATION_PROMPT_CACHE_KEY_EXPR_SQL} AS prompt_cache_key \
         FROM codex_invocations \
         WHERE id IN (SELECT value FROM json_each(?1)) \
           AND {INVOCATION_PROMPT_CACHE_KEY_EXPR_SQL} IS NOT NULL \
           AND {INVOCATION_PROMPT_CACHE_KEY_EXPR_SQL} <> ''"
    ))
    .bind(ids_json)
    .fetch_all(&mut *tx)
    .await
    .context("failed to load prompt-cache keys before invocation detail prune")
}

pub(crate) async fn retention_recovery_finalize_published(
    pool: &Pool<Sqlite>,
    config: &AppConfig,
    descriptor: &RetentionPreparedArchiveDescriptor,
    path: &Path,
    artifact_sha256: &str,
) -> Result<usize> {
    let source_ids = serde_json::from_str::<Vec<i64>>(&descriptor.source_ids_json)
        .context("decode prepared archive source ids for finalization")?;
    let candidates = load_invocation_archive_candidates_by_ids(pool, &source_ids).await?;
    if candidates.len() != source_ids.len() {
        bail!("prepared archive source identity verification failed: source rows are missing");
    }
    let row_count =
        verify_prepared_retention_archive_artifact(pool, descriptor, path, artifact_sha256).await?;
    let outcome = retention_recovery_archive_outcome(
        config,
        descriptor,
        &candidates,
        artifact_sha256,
        row_count,
    )?;
    let raw_paths = candidates
        .iter()
        .flat_map(|candidate| {
            [
                candidate.request_raw_path.clone(),
                candidate.response_raw_path.clone(),
            ]
        })
        .collect::<Vec<_>>();
    let Some(admission) = acquire_retention_write_admission("retention_recovery_finalize").await
    else {
        return Err(retention_write_deferred("retention_recovery_finalize"));
    };
    let mut tx = pool.begin().await?;
    if descriptor.publication_kind == RETENTION_RECOVERY_PUBLICATION_DETAIL_PRUNE {
        upsert_archive_batch_manifest(tx.as_mut(), &outcome).await?;
        mark_archive_batch_historical_rollups_materialized_tx(
            tx.as_mut(),
            descriptor.dataset,
            &outcome.file_path,
        )
        .await?;
        retention_recovery_verify_publication_tx(tx.as_mut(), descriptor, artifact_sha256).await?;
        retention_recovery_update_detail_rows_tx(tx.as_mut(), &source_ids, &candidates).await?;
    } else if descriptor.publication_kind == RETENTION_RECOVERY_PUBLICATION_INVOCATION_ARCHIVE {
        let materialized_rows = candidates
            .iter()
            .map(invocation_archive_candidate_to_hourly_source_record)
            .collect::<Vec<_>>();
        stage_invocation_archive_batch_manifest(tx.as_mut(), &outcome).await?;
        mark_archive_batch_historical_rollups_materialized_tx(
            tx.as_mut(),
            descriptor.dataset,
            &outcome.file_path,
        )
        .await?;
        upsert_invocation_rollups(tx.as_mut(), &candidates).await?;
        let snapshot_archive_batch_id = load_archive_batch_id_for_file_tx(
            tx.as_mut(),
            descriptor.dataset,
            &outcome.month_key,
            &outcome.file_path,
        )
        .await?;
        let invoke_ids_by_row_id = candidates
            .iter()
            .map(|candidate| (candidate.id, candidate.invoke_id.clone()))
            .collect::<HashMap<_, _>>();
        let snapshot_page = SummaryArchiveSnapshotPage {
            archive_batch_id: snapshot_archive_batch_id,
            manifest_sha256: outcome.sha256.clone(),
            page_index: 0,
            coverage_start: outcome.coverage_start_at.clone().ok_or_else(|| {
                anyhow!("authoritative archive is missing Snapshot start coverage")
            })?,
            coverage_end: outcome
                .coverage_end_at
                .clone()
                .ok_or_else(|| anyhow!("authoritative archive is missing Snapshot end coverage"))?,
            row_count: u32::try_from(materialized_rows.len())
                .context("Summary Archive Snapshot row count overflow")?,
            payload: encode_summary_archive_snapshot_v2_payload(
                &materialized_rows,
                &invoke_ids_by_row_id,
            )?,
        };
        store_summary_archive_snapshot_page_v2_tx(tx.as_mut(), &snapshot_page).await?;
        if !summary_archive_snapshot_has_proof_tx(
            tx.as_mut(),
            snapshot_archive_batch_id,
            &outcome.sha256,
        )
        .await?
        {
            bail!("retention Summary Snapshot V2 semantic proof validation failed");
        }
        store_summary_archive_snapshot_v2_final_proof_tx(
            tx.as_mut(),
            snapshot_archive_batch_id,
            &outcome.sha256,
        )
        .await?;
        retention_recovery_verify_publication_tx(tx.as_mut(), descriptor, artifact_sha256).await?;
        let ids_json = serde_json::to_string(&source_ids)?;
        let archived_prompt_cache_keys = sqlx::query_scalar::<_, String>(&format!(
            "SELECT DISTINCT {INVOCATION_PROMPT_CACHE_KEY_EXPR_SQL} AS prompt_cache_key \
             FROM codex_invocations \
             WHERE id IN (SELECT value FROM json_each(?1)) \
               AND {INVOCATION_PROMPT_CACHE_KEY_EXPR_SQL} IS NOT NULL \
               AND {INVOCATION_PROMPT_CACHE_KEY_EXPR_SQL} <> ''"
        ))
        .bind(ids_json)
        .fetch_all(tx.as_mut())
        .await?;
        let archived_prompt_cache_key_refs = archived_prompt_cache_keys
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>();
        if !archived_prompt_cache_key_refs.is_empty() {
            mark_prompt_cache_conversation_stats_stale_on_connection(tx.as_mut()).await?;
        }
        delete_rows_by_ids(tx.as_mut(), descriptor.dataset, &source_ids).await?;
        // DELETE triggers persist the affected keys. Prompt-cache materialization is deliberately
        // outside this archive publication transaction.
        mark_retention_archived_hourly_rollup_targets_tx(
            tx.as_mut(),
            descriptor.dataset,
            &materialized_rows,
            &[],
        )
        .await?;
        for target in INVOCATION_HOURLY_ROLLUP_TARGETS {
            mark_hourly_rollup_archive_replayed_tx(
                tx.as_mut(),
                target,
                descriptor.dataset,
                &outcome.file_path,
            )
            .await?;
        }
        finalize_invocation_archive_batch_publication_tx(tx.as_mut(), &outcome.file_path).await?;
    } else {
        bail!("unsupported prepared archive publication kind");
    }
    retention_recovery_delete_tx(tx.as_mut(), &descriptor.prepared_key).await?;
    let raw_paths =
        filter_unreferenced_proxy_raw_paths(tx.as_mut(), &raw_paths, config.database_path.parent())
            .await?;
    tx.commit().await?;
    let inventory_intent_recorded = if raw_paths.is_empty() {
        true
    } else {
        match mark_retention_raw_inventory_reset_intent(pool).await {
            Ok(()) => true,
            Err(error) => {
                warn!(
                    error = %error,
                    "published retention recovery committed but raw inventory reset intent was not recorded; retaining raw files for a later orphan sweep"
                );
                false
            }
        }
    };
    let removed = if inventory_intent_recorded {
        delete_proxy_raw_paths(&raw_paths, config.database_path.parent())?
    } else {
        0
    };
    drop(admission);
    retention_recovery_record_progress();
    Ok(removed)
}

pub(crate) async fn retention_recovery_delete_tx(
    tx: &mut sqlx::SqliteConnection,
    prepared_key: &str,
) -> Result<()> {
    sqlx::query("DELETE FROM retention_prepared_archives WHERE prepared_key = ?1")
        .bind(prepared_key)
        .execute(&mut *tx)
        .await?;
    Ok(())
}

pub(crate) async fn quarantine_published_retention_archive_if_unchanged(
    pool: &Pool<Sqlite>,
    prepared_key: &str,
    expected_artifact_sha256: Option<&str>,
    failure_fingerprint: Option<&str>,
) -> Result<bool> {
    let result = sqlx::query(
        r#"
        UPDATE retention_prepared_archives
        SET state = ?1,
            quarantined_at = datetime('now'),
            last_failure_stage = ?2,
            last_failure_fingerprint = ?3,
            updated_at = datetime('now')
        WHERE prepared_key = ?4
          AND state = ?5
          AND artifact_sha256 IS ?6
        "#,
    )
    .bind(RETENTION_RECOVERY_STATE_QUARANTINED)
    .bind("prepared_reconcile")
    .bind(failure_fingerprint)
    .bind(prepared_key)
    .bind(RETENTION_RECOVERY_STATE_PUBLISHED)
    .bind(expected_artifact_sha256)
    .execute(pool)
    .await?;
    Ok(result.rows_affected() == 1)
}

pub(crate) fn retention_archive_path_is_within_root(config: &AppConfig, path: &Path) -> bool {
    retention_archive_path_is_within(
        &resolved_archive_dir(config).join("codex_invocations"),
        path,
    )
}

pub(crate) fn retention_archive_path_is_within_archive_root(
    config: &AppConfig,
    path: &Path,
) -> bool {
    retention_archive_path_is_within(&resolved_archive_dir(config), path)
}

pub(crate) fn retention_archive_path_is_within(root: &Path, path: &Path) -> bool {
    let Ok(root) = fs::canonicalize(root) else {
        return false;
    };
    if path
        .components()
        .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        return false;
    }
    let candidates = if path.is_absolute() {
        vec![path.to_path_buf()]
    } else {
        vec![path.to_path_buf(), root.join(path)]
    };
    candidates.into_iter().any(|candidate| {
        let mut current = candidate.as_path();
        loop {
            if let Ok(canonical) = fs::canonicalize(current) {
                return canonical.starts_with(&root);
            }
            current = match current.parent() {
                Some(parent) if parent != current => parent,
                _ => return false,
            };
        }
    })
}

pub(crate) fn retention_archive_path_is_owned(config: &AppConfig, path: &Path) -> bool {
    let Ok(root) = fs::canonicalize(resolved_archive_dir(config).join("codex_invocations")) else {
        return false;
    };
    if !retention_archive_path_is_within_root(config, path) {
        return false;
    }
    let candidates = if path.is_absolute() {
        vec![path.to_path_buf()]
    } else {
        vec![path.to_path_buf(), root.join(path)]
    };
    candidates.into_iter().any(|candidate| {
        let Ok(metadata) = fs::symlink_metadata(&candidate) else {
            return false;
        };
        if !metadata.file_type().is_file() {
            return false;
        }
        let Ok(canonical_candidate) = fs::canonicalize(&candidate) else {
            return false;
        };
        canonical_candidate.starts_with(&root)
    })
}

pub(crate) async fn retention_recovery_schedule_prepared_cursor(
    pool: &Pool<Sqlite>,
    retry_secs: i64,
    defer_reason: Option<&str>,
    reset_failure_count: bool,
    failure_fingerprint: Option<&str>,
) -> Result<()> {
    let Some(admission) = acquire_retention_write_admission("retention_recovery_scheduler").await
    else {
        if let Some(reason) = defer_reason {
            // The scheduler row is the one bounded state transition that must survive a
            // pressure-gated restart. It does not inspect archive data or touch the filesystem,
            // so persist it through the foreground-aware SQLite coordinator without consuming a
            // background pressure slot.
            retention_recovery_persist_scheduler_cursor_without_pressure(
                pool,
                retry_secs,
                Some(reason),
                reset_failure_count,
                failure_fingerprint,
            )
            .await?;
            let mut health = RETENTION_RECOVERY_HEALTH
                .lock()
                .expect("retention recovery health");
            health.state = "deferred".to_string();
            health.stage = Some("prepared_reconcile".to_string());
            health.defer_reason = Some(reason.to_string());
            health.next_retry_at = Some(format_utc_iso(
                Utc::now() + chrono::Duration::seconds(retry_secs),
            ));
        }
        return Ok(());
    };
    let next_retry_at = if retry_secs > 0 {
        Some(format!("+{retry_secs} seconds"))
    } else {
        None
    };
    let query = if reset_failure_count {
        "UPDATE retention_recovery_cursors
         SET next_retry_at = CASE WHEN ?1 IS NULL THEN NULL ELSE datetime('now', ?1) END,
             consecutive_failure_count = 0,
             last_failure_fingerprint = ?2,
             defer_reason = ?3,
             last_progress_at = CASE WHEN ?4 = 1 THEN datetime('now') ELSE last_progress_at END,
             updated_at = datetime('now')
         WHERE scope = ?5"
    } else {
        "UPDATE retention_recovery_cursors
         SET next_retry_at = CASE WHEN ?1 IS NULL THEN NULL ELSE datetime('now', ?1) END,
             last_failure_fingerprint = ?2,
             defer_reason = ?3,
             last_progress_at = CASE WHEN ?4 = 1 THEN datetime('now') ELSE last_progress_at END,
             updated_at = datetime('now')
         WHERE scope = ?5"
    };
    sqlx::query(query)
        .bind(next_retry_at)
        .bind(failure_fingerprint)
        .bind(defer_reason)
        .bind(i64::from(reset_failure_count))
        .bind(RETENTION_RECOVERY_PREPARED_SCOPE)
        .execute(pool)
        .await?;
    drop(admission);
    Ok(())
}

pub(crate) async fn retention_recovery_persist_scheduler_cursor_without_pressure(
    pool: &Pool<Sqlite>,
    retry_secs: i64,
    defer_reason: Option<&str>,
    reset_failure_count: bool,
    failure_fingerprint: Option<&str>,
) -> Result<()> {
    retention_recovery_persist_scheduler_cursor_without_pressure_for_scope(
        pool,
        RETENTION_RECOVERY_PREPARED_SCOPE,
        retry_secs,
        defer_reason,
        reset_failure_count,
        failure_fingerprint,
    )
    .await
}

pub(crate) async fn retention_recovery_persist_scheduler_cursor_without_pressure_for_scope(
    pool: &Pool<Sqlite>,
    scope: &str,
    retry_secs: i64,
    defer_reason: Option<&str>,
    reset_failure_count: bool,
    failure_fingerprint: Option<&str>,
) -> Result<()> {
    let (write_permit, _) = acquire_retention_write_coordinator("retention_recovery_scheduler")
        .await
        .ok_or_else(|| retention_write_deferred("retention_recovery_scheduler"))?;
    let next_retry_at = if retry_secs > 0 {
        Some(format!("+{retry_secs} seconds"))
    } else {
        None
    };
    let query = if reset_failure_count {
        "UPDATE retention_recovery_cursors
         SET next_retry_at = CASE WHEN ?1 IS NULL THEN NULL ELSE datetime('now', ?1) END,
             consecutive_failure_count = 0,
             last_failure_fingerprint = ?2,
             defer_reason = ?3,
             last_progress_at = CASE WHEN ?4 = 1 THEN datetime('now') ELSE last_progress_at END,
             updated_at = datetime('now')
         WHERE scope = ?5"
    } else {
        "UPDATE retention_recovery_cursors
         SET next_retry_at = CASE WHEN ?1 IS NULL THEN NULL ELSE datetime('now', ?1) END,
             last_failure_fingerprint = ?2,
             defer_reason = ?3,
             last_progress_at = CASE WHEN ?4 = 1 THEN datetime('now') ELSE last_progress_at END,
             updated_at = datetime('now')
         WHERE scope = ?5"
    };
    sqlx::query(query)
        .bind(next_retry_at)
        .bind(failure_fingerprint)
        .bind(defer_reason)
        .bind(i64::from(reset_failure_count))
        .bind(scope)
        .execute(pool)
        .await?;
    drop(write_permit);
    Ok(())
}

pub(crate) async fn retention_recovery_mark_prepared_progress(pool: &Pool<Sqlite>) -> Result<()> {
    let (write_permit, _) = acquire_retention_write_coordinator("retention_recovery_progress")
        .await
        .ok_or_else(|| retention_write_deferred("retention_recovery_progress"))?;
    sqlx::query(
        "UPDATE retention_recovery_cursors
         SET last_progress_at = datetime('now'), updated_at = datetime('now')
         WHERE scope = ?1",
    )
    .bind(RETENTION_RECOVERY_PREPARED_SCOPE)
    .execute(pool)
    .await?;
    drop(write_permit);
    Ok(())
}

pub(crate) async fn retention_recovery_persist_pressure_defer(
    pool: &Pool<Sqlite>,
    scope: &str,
    stage: &'static str,
) -> Result<()> {
    retention_recovery_persist_scheduler_cursor_without_pressure_for_scope(
        pool,
        scope,
        RETENTION_RECOVERY_PRESSURE_RETRY_SECS,
        Some(RETENTION_RECOVERY_DEFER_SQLITE_PRESSURE),
        false,
        None,
    )
    .await?;
    retention_recovery_record_deferred(stage);
    Ok(())
}

#[cfg(test)]
pub(crate) async fn reconcile_retention_prepared_archives_for_test(
    pool: &Pool<Sqlite>,
    config: &AppConfig,
) -> Result<()> {
    reconcile_retention_prepared_archives(pool, config).await
}

/// Retire obsolete, unowned artifacts without reading prior source IDs or resuming work.
pub(crate) async fn retire_task_local_quarantines(
    pool: &Pool<Sqlite>,
    config: &AppConfig,
) -> Result<()> {
    let rows = sqlx::query_as::<_, (String, String)>(
        "SELECT prepared_key,file_path FROM retention_prepared_archives
         WHERE state='quarantined' AND quarantined_at IS NOT NULL
         AND julianday('now')-julianday(quarantined_at)>= (?1 / 86400.0)
         ORDER BY id LIMIT 32",
    )
    .bind(RETENTION_RECOVERY_QUARANTINE_GRACE_SECS)
    .fetch_all(pool)
    .await?;
    for (key, file) in rows {
        if retention_run_budget_expired() {
            break;
        }
        let path = Path::new(&file);
        let _lock = retention_archive_file_lock(path)?;
        let Some(_admission) = acquire_retention_write_admission("legacy_quarantine_cleanup").await
        else {
            break;
        };
        let mut tx = pool.begin().await?;
        let owned: bool = sqlx::query_scalar(
            "SELECT NOT EXISTS(SELECT 1 FROM archive_batches WHERE file_path=?1)
             AND NOT EXISTS(SELECT 1 FROM retention_prepared_archives WHERE file_path=?1
                 AND prepared_key<>?2 AND state IN ('preparing','published'))
             AND EXISTS(SELECT 1 FROM retention_prepared_archives WHERE prepared_key=?2
                 AND state='quarantined' AND julianday('now')-julianday(quarantined_at)>= (?3 / 86400.0))",
        ).bind(&file).bind(&key).bind(RETENTION_RECOVERY_QUARANTINE_GRACE_SECS).fetch_one(tx.as_mut()).await?;
        if owned && (!path.exists() || retention_archive_path_is_owned(config, path)) {
            match fs::remove_file(path) {
                Ok(()) => (),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
                Err(error) => return Err(error.into()),
            }
            sqlx::query("DELETE FROM retention_prepared_archives WHERE prepared_key=?1")
                .bind(&key)
                .execute(tx.as_mut())
                .await?;
        }
        tx.commit().await?;
    }
    Ok(())
}

pub(crate) async fn reconcile_retention_prepared_archives(
    pool: &Pool<Sqlite>,
    config: &AppConfig,
) -> Result<()> {
    if retention_run_remaining_budget().is_some() {
        // Compatibility isolation only. A current task never continues source mutations
        // from the previous program's prepared artifact or source_ids cursor.
        let pending: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM retention_prepared_archives
             WHERE state IN ('preparing','published'))",
        )
        .fetch_one(pool)
        .await?;
        if !pending {
            return retire_task_local_quarantines(pool, config).await;
        }
        let Some(_admission) = acquire_retention_write_admission("legacy_prepared_isolation").await
        else {
            return Ok(());
        };
        sqlx::query("UPDATE retention_prepared_archives
             SET state = 'quarantined', quarantined_at = COALESCE(quarantined_at, datetime('now')),
                 next_retry_at = NULL, last_failure_stage = 'task_local_reselection', updated_at = datetime('now')
             WHERE id IN (SELECT id FROM retention_prepared_archives
                 WHERE state IN ('preparing', 'published') ORDER BY id LIMIT 32)")
            .execute(pool).await?;
        drop(_admission);
        retire_task_local_quarantines(pool, config).await?;
        return Ok(());
    }
    retention_recovery_set_stage("prepared_reconcile");
    let not_due: i64 = sqlx::query_scalar(
        "SELECT COALESCE(next_retry_at > datetime('now'), 0)
         FROM retention_recovery_cursors WHERE scope = ?1",
    )
    .bind(RETENTION_RECOVERY_PREPARED_SCOPE)
    .fetch_optional(pool)
    .await?
    .unwrap_or(0);
    if not_due != 0 {
        return Ok(());
    }
    let Some(probe) = acquire_retention_write_admission("retention_recovery_probe").await else {
        retention_recovery_persist_scheduler_cursor_without_pressure(
            pool,
            RETENTION_RECOVERY_PRESSURE_RETRY_SECS,
            Some(RETENTION_RECOVERY_DEFER_SQLITE_PRESSURE),
            false,
            None,
        )
        .await?;
        retention_recovery_record_deferred("prepared_reconcile");
        return Ok(());
    };
    drop(probe);
    let rows = sqlx::query_as::<
        _,
        (
            String,
            String,
            String,
            Option<String>,
            Option<String>,
            String,
            Option<String>,
            Option<String>,
            String,
            String,
            Option<String>,
            Option<String>,
        ),
    >(
        r#"
        SELECT prepared_key, file_path, state, artifact_sha256, quarantined_at,
               month_key, day_key, part_key, source_ids_json, source_identity_sha256,
               staged_file_path, publication_kind
        FROM retention_prepared_archives
        WHERE (
            state IN ('preparing', 'published')
            AND (next_retry_at IS NULL OR next_retry_at <= datetime('now'))
        ) OR (
            state = 'quarantined'
            AND quarantined_at IS NOT NULL
            AND julianday('now') - julianday(quarantined_at) >= (?1 / 86400.0)
            AND NOT EXISTS (
                SELECT 1 FROM archive_batches
                WHERE archive_batches.file_path = retention_prepared_archives.file_path
            )
            AND NOT EXISTS (
                SELECT 1 FROM retention_prepared_archives AS active
                WHERE active.file_path = retention_prepared_archives.file_path
                  AND active.prepared_key <> retention_prepared_archives.prepared_key
                  AND active.state IN ('preparing', 'published')
            )
        )
        ORDER BY CASE
                     WHEN state = 'published' THEN 0
                     WHEN state = 'quarantined' THEN 1
                     ELSE 2
                 END,
                 updated_at ASC, prepared_key ASC
        LIMIT ?2
        "#,
    )
    .bind(RETENTION_RECOVERY_QUARANTINE_GRACE_SECS)
    .bind(RETENTION_RECOVERY_LEGACY_SCAN_BATCH as i64)
    .fetch_all(pool)
    .await?;
    let mut progressed = false;
    let mut encountered_failure = false;
    macro_rules! prepared_item_failure {
        ($error:expr) => {{
            encountered_failure = true;
            let error = anyhow!($error);
            if !retention_recovery_persist_current_prepared_failure_best_effort(pool, &error).await
            {
                retention_recovery_clear_current_prepared_key();
                return Ok(());
            }
            continue;
        }};
    }
    for (
        prepared_key,
        file_path,
        state,
        artifact_sha256,
        quarantined_at,
        month_key,
        day_key,
        part_key,
        source_ids_json,
        source_identity_sha256,
        staged_file_path,
        publication_kind,
    ) in rows
    {
        retention_recovery_set_current_prepared_key(&prepared_key);
        let path = Path::new(&file_path);
        if !retention_archive_path_is_within_root(config, path) {
            if state == RETENTION_RECOVERY_STATE_PREPARING
                || state == RETENTION_RECOVERY_STATE_PUBLISHED
            {
                let error = anyhow!("prepared archive path failed ownership verification");
                prepared_item_failure!(error);
            }
            continue;
        }
        if staged_file_path.is_some() {
            // The restore decision and the cleanup of its journal pointer must share the same
            // directory fence as the publisher. A pre-lock snapshot could otherwise overwrite a
            // newer retry artifact or clear a newly written staging path.
            let _archive_lock = match retention_archive_file_lock(path) {
                Ok(lock) => lock,
                Err(error) => prepared_item_failure!(error),
            };
            let staged_row = match sqlx::query_as::<_, (String, Option<String>)>(
                "SELECT state, staged_file_path FROM retention_prepared_archives WHERE prepared_key = ?1",
            )
            .bind(&prepared_key)
            .fetch_optional(pool)
            .await
            {
                Ok(row) => row,
                Err(error) => prepared_item_failure!(error),
            };
            let Some((current_state, Some(staged_file_path))) = staged_row else {
                continue;
            };
            let state = current_state;
            let staged_path = Path::new(&staged_file_path);
            if !retention_archive_path_is_within_root(config, staged_path)
                || !replacement_staging_path_is_owned(path, staged_path)
            {
                let error = anyhow!("prepared archive staging path failed ownership verification");
                prepared_item_failure!(error);
            }
            let archive_staged_path = match sqlx::query_scalar::<_, Option<String>>(
                "SELECT replacement_staged_path FROM archive_batches
                 WHERE file_path = ?1 ORDER BY id DESC LIMIT 1",
            )
            .bind(&file_path)
            .fetch_optional(pool)
            .await
            {
                Ok(value) => value.flatten(),
                Err(error) => prepared_item_failure!(error),
            };
            if archive_staged_path.as_deref() != Some(staged_file_path.as_str()) {
                // The prepared pointer was durable but the archive manifest pointer was not;
                // no rename could have started, so clear the orphaned pointer and retry.
                if let Err(error) =
                    clear_retention_staged_file_path(pool, &prepared_key, &staged_file_path, true)
                        .await
                {
                    prepared_item_failure!(error);
                }
                continue;
            }
            let manifest_sha = match sqlx::query_scalar::<_, Option<String>>(
                "SELECT sha256 FROM archive_batches WHERE file_path = ?1 ORDER BY id DESC LIMIT 1",
            )
            .bind(&file_path)
            .fetch_optional(pool)
            .await
            {
                Ok(value) => value.flatten(),
                Err(error) => prepared_item_failure!(error),
            };
            let current_sha = if path.is_file() {
                match sha256_hex_file(path) {
                    Ok(sha) => Some(sha),
                    Err(error) => prepared_item_failure!(error),
                }
            } else {
                None
            };
            if manifest_sha.is_some() && manifest_sha == current_sha {
                if staged_path.is_file()
                    && let Err(error) = fs::remove_file(staged_path)
                        .context("failed to remove committed legacy archive rollback copy")
                {
                    prepared_item_failure!(error);
                }
                if state == RETENTION_RECOVERY_STATE_PREPARING {
                    if let Err(error) = clear_retention_staged_file_path(
                        pool,
                        &prepared_key,
                        &staged_file_path,
                        true,
                    )
                    .await
                    {
                        prepared_item_failure!(error);
                    }
                    // The replacement either committed before the journal transition or never
                    // started. Re-run the normal writer from the durable manifest rather than
                    // treating the current artifact as a completed prepared publication.
                    continue;
                }
                if let Err(error) =
                    clear_retention_staged_file_path(pool, &prepared_key, &staged_file_path, false)
                        .await
                {
                    prepared_item_failure!(error);
                }
            } else if staged_path.is_file() {
                let Some(expected_sha) = manifest_sha.as_deref() else {
                    let error = anyhow!(
                        "prepared archive rollback cannot be verified without a manifest digest"
                    );
                    prepared_item_failure!(error);
                };
                if let Err(error) =
                    restore_staged_legacy_archive_file(staged_path, path, expected_sha)
                {
                    prepared_item_failure!(error);
                }
                if let Err(error) =
                    clear_retention_staged_file_path(pool, &prepared_key, &staged_file_path, true)
                        .await
                {
                    prepared_item_failure!(error);
                }
                // The manifest still describes the previous artifact. Leave the prepared row in
                // preparing state so the normal archive writer retries from the restored file.
                continue;
            }
        }
        // A budget/pressure stop can happen after the durable preparing journal is written but
        // before the archive artifact is linked into place. There is no ownership proof to
        // quarantine in that state: when the path is inside our archive root, has no manifest,
        // and is still missing, remove only this orphaned journal row so the source rows can be
        // prepared again on the next bounded pass.
        if state == RETENTION_RECOVERY_STATE_PREPARING && !path.exists() {
            let _archive_lock = match retention_archive_file_lock(path) {
                Ok(lock) => lock,
                Err(error) => prepared_item_failure!(error),
            };
            let manifest_exists = match sqlx::query_scalar::<_, i64>(
                "SELECT EXISTS(SELECT 1 FROM archive_batches WHERE file_path = ?1)",
            )
            .bind(&file_path)
            .fetch_one(pool)
            .await
            {
                Ok(value) => value != 0,
                Err(error) => prepared_item_failure!(error),
            };
            if !manifest_exists {
                let Some(admission) =
                    acquire_retention_write_admission("retention_recovery_missing_prepared").await
                else {
                    retention_recovery_persist_pressure_defer(
                        pool,
                        RETENTION_RECOVERY_PREPARED_SCOPE,
                        "prepared_reconcile",
                    )
                    .await?;
                    retention_recovery_clear_current_prepared_key();
                    return Ok(());
                };
                let deleted = sqlx::query(
                    "DELETE FROM retention_prepared_archives
                     WHERE prepared_key = ?1 AND state = 'preparing' AND file_path = ?2",
                )
                .bind(&prepared_key)
                .bind(&file_path)
                .execute(pool)
                .await?;
                drop(admission);
                progressed |= deleted.rows_affected() == 1;
                continue;
            }
        }
        if !retention_archive_path_is_owned(config, path)
            && state != RETENTION_RECOVERY_STATE_QUARANTINED
        {
            if state == RETENTION_RECOVERY_STATE_PREPARING
                || state == RETENTION_RECOVERY_STATE_PUBLISHED
            {
                let error = anyhow!("prepared archive path failed ownership verification");
                prepared_item_failure!(error);
            }
            continue;
        }
        if state == RETENTION_RECOVERY_STATE_PREPARING && path.is_file() {
            let _archive_lock = match retention_archive_file_lock(path) {
                Ok(lock) => lock,
                Err(error) => prepared_item_failure!(error),
            };
            let actual_sha = match sha256_hex_file(path) {
                Ok(sha) => sha,
                Err(error) => prepared_item_failure!(error),
            };
            let publication_kind = match publication_kind.as_deref() {
                Some(RETENTION_RECOVERY_PUBLICATION_DETAIL_PRUNE) => {
                    Some(RETENTION_RECOVERY_PUBLICATION_DETAIL_PRUNE)
                }
                Some(RETENTION_RECOVERY_PUBLICATION_INVOCATION_ARCHIVE) => {
                    Some(RETENTION_RECOVERY_PUBLICATION_INVOCATION_ARCHIVE)
                }
                Some(_) => {
                    let error = anyhow!("prepared archive publication kind is invalid");
                    prepared_item_failure!(error);
                }
                None => retention_recovery_publication_kind_from_path(
                    config,
                    &file_path,
                    &month_key,
                    day_key.as_deref(),
                    part_key.as_deref(),
                    &source_ids_json,
                    &source_identity_sha256,
                ),
            };
            let Some(publication_kind) = publication_kind else {
                let error = anyhow!("prepared archive publication kind could not be inferred");
                prepared_item_failure!(error);
            };
            let descriptor = RetentionPreparedArchiveDescriptor {
                prepared_key: prepared_key.clone(),
                dataset: "codex_invocations",
                month_key,
                day_key,
                part_key,
                file_path: file_path.clone(),
                source_ids_json,
                source_identity_sha256,
                publication_kind,
            };
            if let Err(error) =
                verify_prepared_retention_archive_artifact(pool, &descriptor, path, &actual_sha)
                    .await
            {
                let error = error.context("prepared archive artifact verification failed");
                prepared_item_failure!(error);
            }
            let Some(admission) =
                acquire_retention_write_admission("retention_recovery_reconcile").await
            else {
                retention_recovery_persist_pressure_defer(
                    pool,
                    RETENTION_RECOVERY_PREPARED_SCOPE,
                    "prepared_reconcile",
                )
                .await?;
                retention_recovery_clear_current_prepared_key();
                return Ok(());
            };
            let artifact_bytes = match fs::metadata(path) {
                Ok(metadata) => metadata.len() as i64,
                Err(error) => prepared_item_failure!(error),
            };
            let updated = match sqlx::query(
                r#"
                UPDATE retention_prepared_archives
                SET state = ?1,
                    artifact_sha256 = ?2,
                    artifact_bytes = ?3,
                    publication_kind = COALESCE(publication_kind, ?4),
                    updated_at = datetime('now')
                WHERE prepared_key = ?5
                  AND state = 'preparing'
                  AND staged_file_path IS NULL
                "#,
            )
            .bind(RETENTION_RECOVERY_STATE_PUBLISHED)
            .bind(actual_sha)
            .bind(artifact_bytes)
            .bind(publication_kind)
            .bind(&prepared_key)
            .execute(pool)
            .await
            {
                Ok(updated) => updated,
                Err(error) => prepared_item_failure!(error),
            };
            drop(admission);
            if updated.rows_affected() == 0 {
                continue;
            }
            progressed = true;
            continue;
        }
        if state == RETENTION_RECOVERY_STATE_PUBLISHED {
            let _archive_lock = match retention_archive_file_lock(path) {
                Ok(lock) => lock,
                Err(error) => prepared_item_failure!(error),
            };
            let valid = path.is_file()
                && artifact_sha256.as_deref().is_some_and(|expected| {
                    sha256_hex_file(path).ok().as_deref() == Some(expected)
                });
            if !valid {
                let error = anyhow!("published retention archive artifact verification failed");
                prepared_item_failure!(error);
            } else {
                let publication_kind = match publication_kind.as_deref() {
                    Some(RETENTION_RECOVERY_PUBLICATION_DETAIL_PRUNE) => {
                        Some(RETENTION_RECOVERY_PUBLICATION_DETAIL_PRUNE)
                    }
                    Some(RETENTION_RECOVERY_PUBLICATION_INVOCATION_ARCHIVE) => {
                        Some(RETENTION_RECOVERY_PUBLICATION_INVOCATION_ARCHIVE)
                    }
                    Some(_) => {
                        let error = anyhow!("prepared archive publication kind is invalid");
                        prepared_item_failure!(error);
                    }
                    None => retention_recovery_publication_kind_from_path(
                        config,
                        &file_path,
                        &month_key,
                        day_key.as_deref(),
                        part_key.as_deref(),
                        &source_ids_json,
                        &source_identity_sha256,
                    ),
                };
                let Some(publication_kind) = publication_kind else {
                    let error = anyhow!("prepared archive publication kind could not be inferred");
                    prepared_item_failure!(error);
                };
                let descriptor = RetentionPreparedArchiveDescriptor {
                    prepared_key: prepared_key.clone(),
                    dataset: "codex_invocations",
                    month_key,
                    day_key,
                    part_key,
                    file_path: file_path.clone(),
                    source_ids_json,
                    source_identity_sha256,
                    publication_kind,
                };
                if let Err(error) = retention_recovery_finalize_published(
                    pool,
                    config,
                    &descriptor,
                    path,
                    artifact_sha256.as_deref().unwrap_or_default(),
                )
                .await
                {
                    prepared_item_failure!(error);
                } else {
                    progressed = true;
                }
            }
        } else if state == RETENTION_RECOVERY_STATE_QUARANTINED {
            let expired = if quarantined_at.is_some() {
                match sqlx::query_scalar::<_, i64>(
                    r#"
                    SELECT COALESCE(
                        julianday('now') - julianday(quarantined_at) >= (?1 / 86400.0),
                        0
                    )
                    FROM retention_prepared_archives
                    WHERE prepared_key = ?2
                    "#,
                )
                .bind(RETENTION_RECOVERY_QUARANTINE_GRACE_SECS)
                .bind(&prepared_key)
                .fetch_optional(pool)
                .await
                {
                    Ok(value) => value.unwrap_or_default() != 0,
                    Err(error) => prepared_item_failure!(error),
                }
            } else {
                false
            };
            if expired {
                // Hold the archive lock before resolving ownership and keep it through the
                // delete/ledger transaction so a replacement cannot win between the check and
                // physical removal.
                let archive_lock = match retention_archive_file_lock(path) {
                    Ok(lock) => lock,
                    Err(error) => prepared_item_failure!(error),
                };
                let Some(admission) =
                    acquire_retention_write_admission("retention_recovery_quarantine_cleanup")
                        .await
                else {
                    drop(archive_lock);
                    retention_recovery_persist_pressure_defer(
                        pool,
                        RETENTION_RECOVERY_PREPARED_SCOPE,
                        "prepared_reconcile",
                    )
                    .await?;
                    retention_recovery_clear_current_prepared_key();
                    return Ok(());
                };
                let mut tx = match pool.begin().await {
                    Ok(tx) => tx,
                    Err(error) => prepared_item_failure!(error),
                };
                let (manifest_exists, active_prepared_exists) =
                    match sqlx::query_as::<_, (i64, i64)>(
                        r#"
                        SELECT
                            EXISTS(SELECT 1 FROM archive_batches WHERE file_path = ?1),
                            EXISTS(
                                SELECT 1 FROM retention_prepared_archives
                                WHERE file_path = ?1
                                  AND prepared_key <> ?2
                                  AND state IN ('preparing', 'published')
                            )
                        "#,
                    )
                    .bind(&file_path)
                    .bind(&prepared_key)
                    .fetch_one(tx.as_mut())
                    .await
                    {
                        Ok(row) => row,
                        Err(error) => prepared_item_failure!(error),
                    };
                let owned_artifact =
                    !path.exists() || retention_archive_path_is_owned(config, path);
                if manifest_exists == 0 && active_prepared_exists == 0 && owned_artifact {
                    if path.is_file()
                        && let Err(error) = fs::remove_file(path).with_context(
                            || "failed to remove expired quarantined archive artifact",
                        )
                    {
                        prepared_item_failure!(error);
                    }
                    let deleted = match sqlx::query(
                        "DELETE FROM retention_prepared_archives WHERE prepared_key = ?1",
                    )
                    .bind(prepared_key)
                    .execute(tx.as_mut())
                    .await
                    {
                        Ok(result) => result,
                        Err(error) => prepared_item_failure!(error),
                    };
                    progressed |= deleted.rows_affected() == 1;
                }
                if let Err(error) = tx.commit().await {
                    prepared_item_failure!(error);
                }
                drop(admission);
                drop(archive_lock);
            }
        }
    }
    if !encountered_failure {
        retention_recovery_schedule_prepared_cursor(
            pool,
            if progressed {
                RETENTION_RECOVERY_PROGRESS_RETRY_SECS
            } else {
                RETENTION_RECOVERY_EMPTY_RETRY_SECS
            },
            None,
            true,
            None,
        )
        .await?;
    } else if progressed {
        retention_recovery_mark_prepared_progress(pool).await?;
    }
    retention_recovery_clear_current_prepared_key();
    retention_recovery_refresh_counts(pool, config).await?;
    Ok(())
}

pub(crate) async fn reconcile_staged_archive_replacements(
    pool: &Pool<Sqlite>,
    config: Option<&AppConfig>,
) -> Result<()> {
    let rows = sqlx::query_as::<_, (i64, String)>(
        "SELECT id, file_path
         FROM archive_batches
         WHERE replacement_staged_path IS NOT NULL
         ORDER BY id ASC LIMIT ?1",
    )
    .bind(RETENTION_RECOVERY_LEGACY_SCAN_BATCH as i64)
    .fetch_all(pool)
    .await?;
    if rows.is_empty() {
        return Ok(());
    }
    let Some(probe) = acquire_retention_write_admission("retention_recovery_staged_probe").await
    else {
        retention_recovery_persist_pressure_defer(
            pool,
            "legacy_archive_segments",
            "legacy_reconcile",
        )
        .await?;
        return Err(retention_write_deferred("retention_recovery_staged_probe"));
    };
    drop(probe);
    for (id, file_path) in rows {
        let path = Path::new(&file_path);
        let archive_lock = match retention_archive_file_lock(path) {
            Ok(lock) => lock,
            Err(error) => {
                retention_recovery_record_failure("legacy_reconcile", &error);
                continue;
            }
        };
        if !archive_lock.is_held() {
            continue;
        }
        let Some((current_file_path, staged_file_path, manifest_sha)) =
            sqlx::query_as::<_, (String, Option<String>, Option<String>)>(
                "SELECT file_path, replacement_staged_path, sha256
                 FROM archive_batches
                 WHERE id = ?1",
            )
            .bind(id)
            .fetch_optional(pool)
            .await?
        else {
            continue;
        };
        let Some(staged_file_path) = staged_file_path else {
            continue;
        };
        if current_file_path != file_path {
            continue;
        }
        let staged_path = Path::new(&staged_file_path);
        let Some(manifest_sha) = manifest_sha else {
            retention_recovery_record_failure(
                "legacy_reconcile",
                &anyhow!(
                    "archive replacement rollback cannot be verified without a manifest digest"
                ),
            );
            continue;
        };
        if config.is_some_and(|config| {
            !retention_archive_path_is_within_archive_root(config, path)
                || !retention_archive_path_is_within_archive_root(config, staged_path)
        }) || !replacement_staging_path_is_owned(path, staged_path)
        {
            retention_recovery_record_failure(
                "legacy_reconcile",
                &anyhow!("archive replacement staging path failed ownership verification"),
            );
            continue;
        }
        let current_sha = if path.is_file() {
            match sha256_hex_file(path) {
                Ok(sha) => Some(sha),
                Err(error) => {
                    retention_recovery_record_failure("legacy_reconcile", &error);
                    continue;
                }
            }
        } else {
            None
        };
        if current_sha.as_deref() == Some(manifest_sha.as_str()) {
            if staged_path.is_file()
                && let Err(error) = fs::remove_file(staged_path)
                    .context("failed to remove committed archive rollback copy")
            {
                retention_recovery_record_failure("legacy_reconcile", &error);
                continue;
            }
            if let Err(error) = clear_archive_batch_staged_path(pool, id, &staged_file_path).await {
                retention_recovery_record_failure("legacy_reconcile", &error);
            }
            continue;
        }
        if staged_path.is_file() {
            let staged_sha = match sha256_hex_file(staged_path) {
                Ok(sha) => sha,
                Err(error) => {
                    retention_recovery_record_failure("legacy_reconcile", &error);
                    continue;
                }
            };
            if current_sha.as_deref() == Some(staged_sha.as_str()) && staged_sha != manifest_sha {
                let Some(_admission) =
                    acquire_retention_write_admission("retention_recovery_manifest_adopt").await
                else {
                    retention_recovery_persist_pressure_defer(
                        pool,
                        "legacy_archive_segments",
                        "legacy_reconcile",
                    )
                    .await?;
                    return Err(retention_write_deferred(
                        "retention_recovery_manifest_adopt",
                    ));
                };
                sqlx::query(
                    "UPDATE archive_batches
                     SET sha256 = ?1
                     WHERE id = ?2 AND replacement_staged_path = ?3",
                )
                .bind(&staged_sha)
                .bind(id)
                .bind(&staged_file_path)
                .execute(pool)
                .await?;
                match fs::remove_file(staged_path) {
                    Ok(()) => {
                        if let Err(error) =
                            clear_archive_batch_staged_path(pool, id, &staged_file_path).await
                        {
                            retention_recovery_record_failure("legacy_reconcile", &error);
                        }
                    }
                    Err(error) if error.kind() == io::ErrorKind::NotFound => {
                        if let Err(error) =
                            clear_archive_batch_staged_path(pool, id, &staged_file_path).await
                        {
                            retention_recovery_record_failure("legacy_reconcile", &error);
                        }
                    }
                    Err(error) => {
                        retention_recovery_record_failure("legacy_reconcile", &error.into());
                    }
                }
                continue;
            }
            if let Err(error) = restore_staged_legacy_archive_file(staged_path, path, &manifest_sha)
            {
                retention_recovery_record_failure("legacy_reconcile", &error);
                continue;
            }
            if let Err(error) = clear_archive_batch_staged_path(pool, id, &staged_file_path).await {
                retention_recovery_record_failure("legacy_reconcile", &error);
            }
        } else {
            retention_recovery_record_failure(
                "legacy_reconcile",
                &anyhow!("archive replacement staging artifact is missing"),
            );
        }
    }
    Ok(())
}

#[cfg(test)]
pub(crate) async fn reconcile_staged_archive_replacements_for_test(
    pool: &Pool<Sqlite>,
    config: Option<&AppConfig>,
) -> Result<()> {
    reconcile_staged_archive_replacements(pool, config).await
}

pub(crate) fn replacement_staging_path_is_owned(final_path: &Path, staged_path: &Path) -> bool {
    let Some(final_parent) = final_path.parent() else {
        return false;
    };
    let Some(staged_parent) = staged_path.parent() else {
        return false;
    };
    if final_parent != staged_parent {
        return false;
    }
    let Some(final_name) = final_path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    let Some(staged_name) = staged_path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    if !staged_name.starts_with(&format!("{final_name}.")) || !staged_name.ends_with(".restore") {
        return false;
    }
    fs::canonicalize(final_parent).is_ok_and(|parent| {
        !fs::symlink_metadata(final_path).is_ok_and(|metadata| metadata.file_type().is_symlink())
            && !fs::symlink_metadata(staged_path)
                .is_ok_and(|metadata| metadata.file_type().is_symlink())
            && parent.is_dir()
    })
}

pub(crate) async fn clear_archive_batch_staged_path(
    pool: &Pool<Sqlite>,
    archive_batch_id: i64,
    expected_staged_file_path: &str,
) -> Result<()> {
    let Some(_admission) =
        acquire_retention_write_admission("retention_recovery_staged_path").await
    else {
        return Err(retention_write_deferred("retention_recovery_staged_path"));
    };
    sqlx::query(
        "UPDATE archive_batches SET replacement_staged_path = NULL
         WHERE id = ?1 AND replacement_staged_path = ?2",
    )
    .bind(archive_batch_id)
    .bind(expected_staged_file_path)
    .execute(pool)
    .await?;
    Ok(())
}

pub(crate) async fn clear_retention_staged_file_path(
    pool: &Pool<Sqlite>,
    prepared_key: &str,
    expected_staged_file_path: &str,
    reset_artifact: bool,
) -> Result<()> {
    let Some(_admission) =
        acquire_retention_write_admission("retention_recovery_staged_path").await
    else {
        return Err(retention_write_deferred("retention_recovery_staged_path"));
    };
    let query = if reset_artifact {
        "UPDATE retention_prepared_archives
         SET state = 'preparing', artifact_sha256 = NULL, artifact_bytes = NULL,
             staged_file_path = NULL, next_retry_at = NULL, updated_at = datetime('now')
         WHERE prepared_key = ?1 AND staged_file_path = ?2"
    } else {
        "UPDATE retention_prepared_archives
         SET staged_file_path = NULL, updated_at = datetime('now')
         WHERE prepared_key = ?1 AND staged_file_path = ?2"
    };
    sqlx::query(query)
        .bind(prepared_key)
        .bind(expected_staged_file_path)
        .execute(pool)
        .await?;
    Ok(())
}

pub(crate) fn restore_staged_legacy_archive_file(
    staged_path: &Path,
    final_path: &Path,
    expected_sha256: &str,
) -> Result<()> {
    let staged_sha256 = sha256_hex_file(staged_path)?;
    if staged_sha256 != expected_sha256 {
        bail!("archive rollback artifact digest verification failed");
    }
    match fs::rename(staged_path, final_path) {
        Ok(()) => sync_restored_archive_file(final_path),
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            fs::remove_file(final_path).with_context(|| {
                format!(
                    "failed to replace interrupted legacy archive {}",
                    final_path.display()
                )
            })?;
            fs::rename(staged_path, final_path)
                .with_context(|| {
                    format!(
                        "failed to restore interrupted legacy archive {}",
                        final_path.display()
                    )
                })
                .and_then(|()| sync_restored_archive_file(final_path))
        }
        Err(error) => Err(error).with_context(|| {
            format!(
                "failed to restore interrupted legacy archive {}",
                final_path.display()
            )
        }),
    }
}

pub(crate) fn sync_restored_archive_file(final_path: &Path) -> Result<()> {
    fs::File::open(final_path)
        .context("failed to open restored archive for sync")?
        .sync_all()
        .context("failed to sync restored archive")?;
    if let Some(parent) = final_path.parent() {
        fs::File::open(parent)
            .context("failed to open restored archive directory for sync")?
            .sync_all()
            .context("failed to sync restored archive directory")?;
    }
    Ok(())
}
