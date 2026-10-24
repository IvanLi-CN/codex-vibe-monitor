use super::*;

pub(crate) async fn compress_cold_proxy_raw_payloads(
    pool: &Pool<Sqlite>,
    config: &AppConfig,
    raw_path_fallback_root: Option<&Path>,
    dry_run: bool,
) -> Result<RawCompressionPassSummary> {
    compress_cold_proxy_raw_payloads_with_budget(
        pool,
        config,
        raw_path_fallback_root,
        dry_run,
        Some(config.retention_catchup_budget),
    )
    .await
}

pub(crate) async fn compress_cold_proxy_raw_payloads_with_budget(
    pool: &Pool<Sqlite>,
    config: &AppConfig,
    raw_path_fallback_root: Option<&Path>,
    dry_run: bool,
    catchup_budget: Option<Duration>,
) -> Result<RawCompressionPassSummary> {
    if config.proxy_raw_compression == RawCompressionCodec::None {
        return Ok(RawCompressionPassSummary::default());
    }

    let mut summary = RawCompressionPassSummary::default();
    let started_at = Instant::now();
    let batch_limit = if dry_run {
        i64::MAX as usize
    } else {
        retention_candidate_limit(config, "raw_compression")
    };

    loop {
        let (request_summary, request_hit_batch_limit) = compress_cold_proxy_raw_payload_lane(
            pool,
            config,
            raw_path_fallback_root,
            dry_run,
            RawPayloadField::Request,
            batch_limit,
        )
        .await?;
        accumulate_raw_compression_summary(&mut summary, request_summary);

        let (response_summary, response_hit_batch_limit) = compress_cold_proxy_raw_payload_lane(
            pool,
            config,
            raw_path_fallback_root,
            dry_run,
            RawPayloadField::Response,
            batch_limit,
        )
        .await?;
        accumulate_raw_compression_summary(&mut summary, response_summary);

        let (attempt_summary, attempt_hit_batch_limit) =
            compress_cold_pool_attempt_response_raw_lane(
                pool,
                config,
                raw_path_fallback_root,
                dry_run,
                batch_limit,
            )
            .await?;
        accumulate_raw_compression_summary(&mut summary, attempt_summary);

        if !request_hit_batch_limit && !response_hit_batch_limit && !attempt_hit_batch_limit {
            break;
        }
        if dry_run {
            break;
        }
        if let Some(limit) = catchup_budget
            && started_at.elapsed() >= limit
        {
            break;
        }
    }

    Ok(summary)
}

pub(crate) async fn compress_cold_pool_attempt_response_raw_lane(
    pool: &Pool<Sqlite>,
    config: &AppConfig,
    raw_path_fallback_root: Option<&Path>,
    dry_run: bool,
    batch_limit: usize,
) -> Result<(RawCompressionPassSummary, bool)> {
    let cutoff = shanghai_local_cutoff_for_age_secs_string(config.proxy_raw_hot_secs);
    let archive_cutoff =
        shanghai_local_cutoff_string(config.pool_upstream_request_attempts_retention_days);
    let mut summary = RawCompressionPassSummary::default();
    let mut rows_processed = 0usize;
    let mut last_seen_occurred_at: Option<String> = None;
    let mut last_seen_id = 0_i64;

    while rows_processed < batch_limit {
        let candidates = sqlx::query_as::<_, InvocationRawCompressionFieldCandidate>(
            r#"
            SELECT id, occurred_at, response_raw_path AS raw_path
            FROM pool_upstream_request_attempts
            WHERE occurred_at < ?1
              AND occurred_at >= ?2
              AND response_raw_path IS NOT NULL
              AND response_raw_codec = ?3
              AND (?4 IS NULL OR occurred_at > ?4 OR (occurred_at = ?4 AND id > ?5))
            ORDER BY occurred_at ASC, id ASC
            LIMIT ?6
            "#,
        )
        .bind(&cutoff)
        .bind(&archive_cutoff)
        .bind(RAW_CODEC_IDENTITY)
        .bind(last_seen_occurred_at.as_deref())
        .bind(last_seen_id)
        .bind((batch_limit - rows_processed).max(1) as i64)
        .fetch_all(pool)
        .await?;

        if candidates.is_empty() {
            break;
        }

        for candidate in candidates {
            last_seen_occurred_at = Some(candidate.occurred_at.clone());
            last_seen_id = candidate.id;
            rows_processed += 1;
            if !dry_run && !prepare_raw_compression_inventory_reset(pool).await? {
                continue;
            }
            // Cold compression and orphan removal both mutate this raw directory. Keep the
            // shared fence through compression, reference replacement, and source cleanup so an
            // exclusive orphan sweep cannot observe or remove a half-published replacement.
            let _compression_fence = match acquire_raw_compression_fence(
                &candidate.raw_path,
                raw_path_fallback_root,
            )
            .await
            {
                Ok(fence) => fence,
                Err(error) => {
                    warn!(
                        invocation_id = candidate.id,
                        field = "attempt_response",
                        error = %error,
                        "failed to acquire raw compression directory fence; continuing retention"
                    );
                    continue;
                }
            };
            let outcome = match maybe_compress_proxy_raw_path(
                pool,
                candidate.id,
                "attempt_response",
                Some(candidate.raw_path.as_str()),
                config.proxy_raw_compression,
                raw_path_fallback_root,
                dry_run,
            )
            .await
            {
                Ok(outcome) => outcome,
                Err(err) => {
                    warn!(
                        invocation_id = candidate.id,
                        field = "attempt_response",
                        error = %err,
                        "failed to cold-compress raw payload file; continuing retention"
                    );
                    continue;
                }
            };
            let next_path = outcome
                .new_db_path
                .clone()
                .unwrap_or_else(|| candidate.raw_path.clone());
            let next_codec = outcome
                .new_codec
                .clone()
                .unwrap_or_else(|| raw_codec_from_path(Some(next_path.as_str())));
            if !dry_run
                && (next_path != candidate.raw_path || !raw_codec_is_identity(Some(&next_codec)))
            {
                let references_updated = replace_proxy_raw_path_references(
                    pool,
                    config,
                    &candidate.raw_path,
                    &next_path,
                    &next_codec,
                )
                .await?;
                if let Some(path) = outcome.old_exact_path.as_deref()
                    && references_updated
                    && next_path != candidate.raw_path
                {
                    delete_exact_proxy_raw_path(Some(path), raw_path_fallback_root)?;
                }
            }
            if outcome.candidate_counted {
                summary.files_considered += 1;
            }
            if outcome.compressed {
                summary.files_compressed += 1;
                crate::record_managed_task_processed_work(&["raw_compression"], 1);
            }
            summary.bytes_before += outcome.bytes_before;
            summary.bytes_after += outcome.bytes_after;
            summary.estimated_bytes_after += outcome.estimated_bytes_after;
            if rows_processed >= batch_limit {
                break;
            }
        }
    }
    Ok((summary, rows_processed >= batch_limit))
}

pub(crate) fn accumulate_raw_compression_summary(
    target: &mut RawCompressionPassSummary,
    next: RawCompressionPassSummary,
) {
    target.files_considered += next.files_considered;
    target.files_compressed += next.files_compressed;
    target.bytes_before += next.bytes_before;
    target.bytes_after += next.bytes_after;
    target.estimated_bytes_after += next.estimated_bytes_after;
}

pub(crate) async fn prepare_raw_compression_inventory_reset(pool: &Pool<Sqlite>) -> Result<bool> {
    let Some(admission) = acquire_retention_write_admission("raw_metrics_inventory_reset").await
    else {
        return Ok(false);
    };
    mark_retention_raw_inventory_reset_intent(pool).await?;
    drop(admission);
    Ok(true)
}

pub(crate) async fn compress_cold_proxy_raw_payload_lane(
    pool: &Pool<Sqlite>,
    config: &AppConfig,
    raw_path_fallback_root: Option<&Path>,
    dry_run: bool,
    field: RawPayloadField,
    batch_limit: usize,
) -> Result<(RawCompressionPassSummary, bool)> {
    let cutoff = shanghai_local_cutoff_for_age_secs_string(config.proxy_raw_hot_secs);
    let prune_cutoff = shanghai_local_cutoff_string(config.invocation_success_full_days);
    let archive_cutoff = shanghai_local_cutoff_string(config.invocation_max_days);
    let success_like_condition = invocation_status_is_success_like_sql("status", "error_message");
    let sql = format!(
        r#"
        SELECT id, occurred_at, {path_column} AS raw_path
        FROM codex_invocations
        WHERE occurred_at < ?1
          AND occurred_at >= ?2
          AND (
            NOT {success_like_condition}
            OR detail_level IS NULL
            OR detail_level != ?3
            OR occurred_at >= ?4
          )
          AND {path_column} IS NOT NULL
          AND {codec_column} = ?5
          AND (
            ?6 IS NULL
            OR occurred_at > ?6
            OR (occurred_at = ?6 AND id > ?7)
          )
        ORDER BY occurred_at ASC, id ASC
        LIMIT ?8
        "#,
        path_column = field.path_column(),
        codec_column = field.codec_column(),
        success_like_condition = success_like_condition,
    );

    let mut summary = RawCompressionPassSummary::default();
    let mut rows_processed = 0usize;
    let mut last_seen_occurred_at: Option<String> = None;
    let mut last_seen_id = 0_i64;

    while rows_processed < batch_limit {
        let remaining = (batch_limit - rows_processed) as i64;
        let candidates = sqlx::query_as::<_, InvocationRawCompressionFieldCandidate>(&sql)
            .bind(&cutoff)
            .bind(&archive_cutoff)
            .bind(DETAIL_LEVEL_FULL)
            .bind(&prune_cutoff)
            .bind(RAW_CODEC_IDENTITY)
            .bind(last_seen_occurred_at.as_deref())
            .bind(last_seen_id)
            .bind(remaining.max(1))
            .fetch_all(pool)
            .await?;

        if candidates.is_empty() {
            break;
        }

        for candidate in candidates {
            last_seen_occurred_at = Some(candidate.occurred_at.clone());
            last_seen_id = candidate.id;
            rows_processed += 1;

            if !dry_run && !prepare_raw_compression_inventory_reset(pool).await? {
                continue;
            }

            // Keep the shared raw-directory fence until the DB references and old source path
            // have been updated, matching the orphan sweep's exclusive deletion window.
            let _compression_fence = match acquire_raw_compression_fence(
                &candidate.raw_path,
                raw_path_fallback_root,
            )
            .await
            {
                Ok(fence) => fence,
                Err(error) => {
                    warn!(
                        invocation_id = candidate.id,
                        field = field.label(),
                        error = %error,
                        "failed to acquire raw compression directory fence; continuing retention"
                    );
                    continue;
                }
            };

            let outcome = match maybe_compress_proxy_raw_path(
                pool,
                candidate.id,
                field.label(),
                Some(candidate.raw_path.as_str()),
                config.proxy_raw_compression,
                raw_path_fallback_root,
                dry_run,
            )
            .await
            {
                Ok(outcome) => outcome,
                Err(err) => {
                    warn!(
                        invocation_id = candidate.id,
                        field = field.label(),
                        error = %err,
                        "failed to cold-compress raw payload file; continuing retention"
                    );
                    continue;
                }
            };

            let next_path = outcome
                .new_db_path
                .clone()
                .unwrap_or_else(|| candidate.raw_path.clone());
            let next_codec = outcome
                .new_codec
                .clone()
                .unwrap_or_else(|| raw_codec_from_path(Some(next_path.as_str())));

            if !dry_run
                && (next_path != candidate.raw_path || !raw_codec_is_identity(Some(&next_codec)))
            {
                let references_updated = replace_proxy_raw_path_references(
                    pool,
                    config,
                    &candidate.raw_path,
                    &next_path,
                    &next_codec,
                )
                .await?;

                if let Some(path) = outcome.old_exact_path.as_deref()
                    && references_updated
                    && next_path != candidate.raw_path
                {
                    delete_exact_proxy_raw_path(Some(path), raw_path_fallback_root)?;
                }
            }

            if outcome.candidate_counted {
                summary.files_considered += 1;
            }
            if outcome.compressed {
                summary.files_compressed += 1;
                crate::record_managed_task_processed_work(&["raw_compression"], 1);
            }
            summary.bytes_before += outcome.bytes_before;
            summary.bytes_after += outcome.bytes_after;
            summary.estimated_bytes_after += outcome.estimated_bytes_after;

            if rows_processed >= batch_limit {
                break;
            }
        }
    }

    let hit_batch_limit = rows_processed >= batch_limit;
    Ok((summary, hit_batch_limit))
}

pub(crate) async fn acquire_raw_compression_fence(
    raw_path: &str,
    raw_path_fallback_root: Option<&Path>,
) -> Result<Option<Arc<RetentionArchiveFileLock>>> {
    let lock_path = locate_existing_proxy_raw_path(raw_path, raw_path_fallback_root)
        .or_else(|| locate_existing_proxy_raw_compressed_path(raw_path, raw_path_fallback_root))
        .or_else(|| {
            resolved_raw_path_candidates(raw_path, raw_path_fallback_root)
                .into_iter()
                .next()
        });
    let Some(lock_path) = lock_path else {
        return Ok(None);
    };
    let lock_description = lock_path.display().to_string();
    let fence =
        tokio::task::spawn_blocking(move || acquire_retention_raw_exclusive_fence(&lock_path))
            .await
            .map_err(|error| anyhow!("raw compression fence worker failed: {error}"))?
            .with_context(|| {
                format!(
                    "failed to acquire raw compression directory fence: {}",
                    lock_description
                )
            })?;
    Ok(Some(fence))
}

pub(crate) async fn maybe_compress_proxy_raw_path(
    _pool: &Pool<Sqlite>,
    invocation_id: i64,
    field_name: &str,
    raw_path: Option<&str>,
    codec: RawCompressionCodec,
    raw_path_fallback_root: Option<&Path>,
    dry_run: bool,
) -> Result<RawCompressionFileOutcome> {
    let Some(raw_path) = raw_path else {
        return Ok(RawCompressionFileOutcome::default());
    };
    if codec == RawCompressionCodec::None || raw_path.ends_with(".gz") {
        return Ok(RawCompressionFileOutcome {
            new_db_path: Some(raw_path.to_string()),
            new_codec: Some(RAW_CODEC_GZIP.to_string()),
            ..RawCompressionFileOutcome::default()
        });
    }

    let Some(source_path) = locate_existing_proxy_raw_path(raw_path, raw_path_fallback_root) else {
        let existing_compressed =
            locate_existing_proxy_raw_compressed_path(raw_path, raw_path_fallback_root);
        if existing_compressed.is_some() {
            return Ok(RawCompressionFileOutcome {
                new_db_path: Some(raw_payload_compressed_db_path(raw_path)),
                new_codec: Some(RAW_CODEC_GZIP.to_string()),
                ..RawCompressionFileOutcome::default()
            });
        }
        warn!(
            invocation_id,
            field = field_name,
            raw_path,
            "skipping raw cold compression because source raw file is missing"
        );
        return Ok(RawCompressionFileOutcome {
            new_db_path: Some(raw_path.to_string()),
            new_codec: Some(raw_codec_from_path(Some(raw_path))),
            ..RawCompressionFileOutcome::default()
        });
    };

    let source_meta = fs::metadata(&source_path).with_context(|| {
        format!(
            "failed to inspect raw payload before cold compression: {}",
            source_path.display()
        )
    })?;
    if !source_meta.is_file() {
        return Ok(RawCompressionFileOutcome {
            new_db_path: Some(raw_path.to_string()),
            new_codec: Some(raw_codec_from_path(Some(raw_path))),
            ..RawCompressionFileOutcome::default()
        });
    }

    let target_db_path = raw_payload_compressed_db_path(raw_path);
    let target_path = raw_payload_compressed_file_path(&source_path);
    let bytes_before = source_meta.len();
    if target_path.exists() {
        return Ok(RawCompressionFileOutcome {
            candidate_counted: true,
            bytes_before,
            new_db_path: Some(target_db_path),
            new_codec: Some(RAW_CODEC_GZIP.to_string()),
            old_exact_path: Some(source_path),
            ..RawCompressionFileOutcome::default()
        });
    }
    if dry_run {
        let estimated_bytes_after = estimate_gzip_file_size(&source_path)?;
        return Ok(RawCompressionFileOutcome {
            candidate_counted: true,
            bytes_before,
            estimated_bytes_after,
            new_db_path: Some(target_db_path),
            new_codec: Some(RAW_CODEC_GZIP.to_string()),
            old_exact_path: Some(source_path),
            ..RawCompressionFileOutcome::default()
        });
    }

    let bytes_after = compress_file_to_gzip(&source_path, &target_path)?;
    Ok(RawCompressionFileOutcome {
        candidate_counted: true,
        compressed: true,
        bytes_before,
        bytes_after,
        new_db_path: Some(target_db_path),
        new_codec: Some(RAW_CODEC_GZIP.to_string()),
        old_exact_path: Some(source_path),
        ..RawCompressionFileOutcome::default()
    })
}

pub(crate) fn compress_file_to_gzip(source: &Path, destination: &Path) -> Result<u64> {
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent).with_context(|| {
            format!(
                "failed to create raw compression directory {}",
                parent.display()
            )
        })?;
    }

    let temp_destination = PathBuf::from(format!("{}.tmp", destination.display()));
    if temp_destination.exists() {
        let _ = fs::remove_file(&temp_destination);
    }

    let result = (|| -> Result<u64> {
        let input = fs::File::open(source)
            .with_context(|| format!("failed to open raw payload {}", source.display()))?;
        let output = fs::File::create(&temp_destination).with_context(|| {
            format!(
                "failed to create compressed raw payload {}",
                temp_destination.display()
            )
        })?;
        let mut reader = io::BufReader::new(input);
        let counting_writer = CountingWriter::new(io::BufWriter::new(output));
        let mut encoder = GzEncoder::new(counting_writer, Compression::default());
        io::copy(&mut reader, &mut encoder).with_context(|| {
            format!(
                "failed to compress raw payload {} into {}",
                source.display(),
                temp_destination.display()
            )
        })?;
        let mut counting_writer = encoder.finish().with_context(|| {
            format!(
                "failed to finish raw payload compression {}",
                temp_destination.display()
            )
        })?;
        counting_writer.flush()?;
        let bytes_after = counting_writer.bytes_written();
        let mut output = counting_writer.inner;
        output.flush()?;
        fs::rename(&temp_destination, destination).with_context(|| {
            format!(
                "failed to move compressed raw payload into place: {} -> {}",
                temp_destination.display(),
                destination.display()
            )
        })?;
        Ok(bytes_after)
    })();

    if result.is_err() {
        let _ = fs::remove_file(&temp_destination);
    }
    result
}

pub(crate) fn estimate_gzip_file_size(source: &Path) -> Result<u64> {
    let input = fs::File::open(source)
        .with_context(|| format!("failed to open raw payload {}", source.display()))?;
    let mut reader = io::BufReader::new(input);
    let counting_writer = CountingWriter::new(io::sink());
    let mut encoder = GzEncoder::new(counting_writer, Compression::default());
    io::copy(&mut reader, &mut encoder).with_context(|| {
        format!(
            "failed to estimate gzip size for raw payload {}",
            source.display()
        )
    })?;
    let counting_writer = encoder.finish().with_context(|| {
        format!(
            "failed to finish gzip size estimate for raw payload {}",
            source.display()
        )
    })?;
    Ok(counting_writer.bytes_written())
}

pub(crate) fn raw_payload_compressed_db_path(raw_path: &str) -> String {
    if raw_path.ends_with(".gz") {
        raw_path.to_string()
    } else {
        format!("{raw_path}.gz")
    }
}

pub(crate) fn raw_codec_from_path(raw_path: Option<&str>) -> String {
    match raw_path {
        Some(path) if path.ends_with(".gz") => RAW_CODEC_GZIP.to_string(),
        _ => RAW_CODEC_IDENTITY.to_string(),
    }
}

pub(crate) fn raw_codec_is_identity(raw_codec: Option<&str>) -> bool {
    matches!(raw_codec, Some(RAW_CODEC_IDENTITY) | None)
}

pub(crate) fn raw_payload_compressed_file_path(path: &Path) -> PathBuf {
    PathBuf::from(format!("{}.gz", path.display()))
}

pub(crate) async fn replace_proxy_raw_path_references(
    pool: &Pool<Sqlite>,
    config: &AppConfig,
    old_path: &str,
    next_path: &str,
    next_codec: &str,
) -> Result<bool> {
    loop {
        let candidate_limit = retention_candidate_limit(config, "raw_path_reference_update")
            .min(RETENTION_WRITE_MAX_ROWS);
        let mut candidates = sqlx::query_as::<_, RawPathReferenceCandidate>(
            r#"
            SELECT reference_kind, id
            FROM (
                SELECT 'invocation_request' AS reference_kind, id
                FROM codex_invocations
                WHERE request_raw_path = ?1
                UNION ALL
                SELECT 'invocation_response' AS reference_kind, id
                FROM codex_invocations
                WHERE response_raw_path = ?1
                UNION ALL
                SELECT 'attempt_response' AS reference_kind, id
                FROM pool_upstream_request_attempts
                WHERE response_raw_path = ?1
            )
            ORDER BY reference_kind ASC, id ASC
            LIMIT ?2
            "#,
        )
        .bind(old_path)
        .bind(candidate_limit.saturating_add(1) as i64)
        .fetch_all(pool)
        .await?;
        if candidates.is_empty() {
            break;
        }

        let candidate_remaining_hint = usize::from(candidates.len() > candidate_limit);
        candidates.truncate(candidate_limit);
        let mut by_kind = BTreeMap::<String, Vec<i64>>::new();
        for candidate in &candidates {
            by_kind
                .entry(candidate.reference_kind.clone())
                .or_default()
                .push(candidate.id);
        }
        let Some(admission) = acquire_retention_write_admission("raw_path_reference_update").await
        else {
            return Ok(false);
        };
        let execute_started = Instant::now();
        let mut tx = pool.begin().await?;
        let mut updated = 0usize;
        for (reference_kind, ids) in by_kind {
            let (table, path_column, codec_column) = match reference_kind.as_str() {
                "invocation_request" => {
                    ("codex_invocations", "request_raw_path", "request_raw_codec")
                }
                "invocation_response" => (
                    "codex_invocations",
                    "response_raw_path",
                    "response_raw_codec",
                ),
                "attempt_response" => (
                    "pool_upstream_request_attempts",
                    "response_raw_path",
                    "response_raw_codec",
                ),
                _ => continue,
            };
            let mut update =
                QueryBuilder::<Sqlite>::new(format!("UPDATE {table} SET {path_column} = "));
            update
                .push_bind(next_path)
                .push(format!(", {codec_column} = "))
                .push_bind(next_codec)
                .push(format!(" WHERE {path_column} = "))
                .push_bind(old_path)
                .push(" AND id IN (");
            {
                let mut separated = update.separated(", ");
                for id in &ids {
                    separated.push_bind(id);
                }
            }
            update.push(")");
            updated += update.build().execute(tx.as_mut()).await?.rows_affected() as usize;
        }
        let commit_started = Instant::now();
        tx.commit().await?;
        retention_record_commit!(
            "raw_path_reference_update",
            admission.admission_mode(),
            updated,
            updated.saturating_mul(192),
            Duration::ZERO,
            admission.lock_wait(),
            commit_started.duration_since(execute_started),
            commit_started.elapsed(),
            admission.p1_waiter_count,
            candidate_remaining_hint,
        );
        drop(admission);
    }
    debug!(
        old_path,
        next_path, next_codec, "propagated shared proxy raw path replacement"
    );
    Ok(true)
}

pub(crate) async fn wake_retention_startup_backfill_tasks(
    pool: &Pool<Sqlite>,
    tasks: &[StartupBackfillTask],
    wake_reason: &'static str,
) -> Result<u64> {
    let Some(admission) = acquire_retention_write_admission("archive_backfill_wake").await else {
        return Ok(0);
    };
    let execute_started = Instant::now();
    let woken = wake_startup_backfill_tasks(pool, tasks, wake_reason).await?;
    retention_record_commit!(
        "archive_backfill_wake",
        admission.admission_mode(),
        woken as usize,
        tasks.len().saturating_mul(256),
        Duration::ZERO,
        admission.lock_wait(),
        execute_started.elapsed(),
        Duration::ZERO,
        admission.p1_waiter_count,
        0,
    );
    Ok(woken)
}

pub(crate) async fn reset_retention_raw_payload_metrics_inventory(
    state: &AppState,
) -> Result<bool> {
    loop {
        let Some(complete) = reset_retention_raw_payload_metrics_inventory_batch(state).await?
        else {
            return Ok(false);
        };
        if complete {
            return Ok(true);
        }
    }
}

pub(crate) async fn reset_retention_raw_payload_metrics_inventory_batch(
    state: &AppState,
) -> Result<Option<bool>> {
    let Some(admission) = acquire_retention_write_admission("raw_metrics_inventory_reset").await
    else {
        return Ok(None);
    };
    let candidate_limit = retention_candidate_limit(&state.config, "raw_metrics_inventory_reset");
    let execute_started = Instant::now();
    let outcome = reset_system_raw_payload_metrics_inventory_batch(state, candidate_limit).await?;
    crate::record_managed_task_discovered_work_delta(
        &["raw_payload_metrics_inventory"],
        i64::try_from(outcome.removed_path_count).unwrap_or(i64::MAX),
    );
    crate::record_managed_task_processed_work(
        &["raw_payload_metrics_inventory"],
        i64::try_from(outcome.removed_path_count).unwrap_or(i64::MAX),
    );
    retention_record_commit!(
        "raw_metrics_inventory_reset",
        admission.admission_mode(),
        outcome.removed_path_count.saturating_add(1),
        outcome
            .removed_path_count
            .saturating_mul(128)
            .saturating_add(256),
        Duration::ZERO,
        admission.lock_wait(),
        execute_started.elapsed(),
        Duration::ZERO,
        admission.p1_waiter_count,
        usize::from(!outcome.complete),
    );
    Ok(Some(outcome.complete))
}

pub(crate) async fn resume_retention_raw_payload_metrics_inventory_reset(
    state: &AppState,
) -> Result<bool> {
    if !crate::system_raw_payload_metrics_inventory_reset_pending(&state.pool).await? {
        return Ok(true);
    }
    reset_retention_raw_payload_metrics_inventory_batch(state)
        .await
        .map(|outcome| outcome.unwrap_or(false))
}

pub(crate) fn locate_existing_proxy_raw_path(
    path: &str,
    fallback_root: Option<&Path>,
) -> Option<PathBuf> {
    resolved_raw_path_candidates(path, fallback_root)
        .into_iter()
        .find(|candidate| candidate.exists())
}

pub(crate) fn locate_existing_proxy_raw_compressed_path(
    path: &str,
    fallback_root: Option<&Path>,
) -> Option<PathBuf> {
    resolved_raw_path_candidates(&raw_payload_compressed_db_path(path), fallback_root)
        .into_iter()
        .find(|candidate| candidate.exists())
}

pub(crate) fn delete_exact_proxy_raw_path(
    raw_path: Option<&Path>,
    raw_path_fallback_root: Option<&Path>,
) -> Result<()> {
    let Some(raw_path) = raw_path else {
        return Ok(());
    };
    let raw_path = raw_path.to_string_lossy();
    for candidate in resolved_raw_path_candidates(&raw_path, raw_path_fallback_root) {
        match fs::remove_file(&candidate) {
            Ok(_) => return Ok(()),
            Err(err) if err.kind() == io::ErrorKind::NotFound => continue,
            Err(err) => {
                warn!(
                    path = %candidate.display(),
                    error = %err,
                    "failed to remove replaced raw payload after cold compression"
                );
                return Ok(());
            }
        }
    }
    Ok(())
}

pub(crate) fn raw_path_ledger_aliases(path: &str, fallback_root: Option<&Path>) -> Vec<String> {
    let mut aliases = std::collections::BTreeSet::new();
    let mut add_path = |candidate: &str| {
        aliases.insert(candidate.to_string());
        if let Some(alternate_path) = raw_payload_alternate_db_path(candidate) {
            aliases.insert(alternate_path);
        }
    };
    add_path(path);
    let absolute_root =
        fallback_root.map(|root| std::path::absolute(root).unwrap_or_else(|_| root.to_path_buf()));
    let fallback_root_relative = fallback_root.and_then(|root| {
        if root.is_absolute() {
            std::env::current_dir()
                .ok()
                .and_then(|cwd| root.strip_prefix(cwd).ok().map(Path::to_path_buf))
        } else {
            Some(root.to_path_buf())
        }
    });
    let path = Path::new(path);
    if !path.is_absolute() {
        if let Some(root) = fallback_root_relative.as_deref()
            && let Ok(relative) = path.strip_prefix(root)
        {
            add_path(&relative.to_string_lossy());
        }
        if let Some(root) = fallback_root
            && !root.is_absolute()
        {
            add_path(&root.join(path).to_string_lossy());
        }
    }
    if let Some(root) = absolute_root.as_deref() {
        if path.is_absolute() {
            if let Ok(cwd) = std::env::current_dir()
                && let Ok(relative) = path.strip_prefix(cwd)
            {
                add_path(&relative.to_string_lossy());
            }
            if let Ok(relative) = path.strip_prefix(root) {
                add_path(&relative.to_string_lossy());
                if let Some(fallback_root) = fallback_root
                    && !fallback_root.is_absolute()
                {
                    add_path(&fallback_root.join(relative).to_string_lossy());
                }
            }
        } else {
            let cwd_absolute = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
            add_path(&cwd_absolute.to_string_lossy());
            let relative = fallback_root_relative
                .as_deref()
                .and_then(|root_relative| match path.strip_prefix(root_relative) {
                    Ok(relative) => Some(relative),
                    Err(_) => {
                        add_path(&root_relative.join(path).to_string_lossy());
                        None
                    }
                })
                .unwrap_or(path);
            let absolute =
                std::path::absolute(root.join(relative)).unwrap_or_else(|_| root.join(relative));
            add_path(&absolute.to_string_lossy());
        }
    }
    aliases.into_iter().collect()
}

pub(crate) struct ProxyRawFileIdentity {
    pub(crate) path: PathBuf,
    pub(crate) identity: String,
    pub(crate) byte_size: u64,
}

pub(crate) struct ProxyRawDeletionPlan {
    pub(crate) paths: Vec<Option<String>>,
    pub(crate) fences: Vec<RetentionArchiveFileLock>,
    pub(crate) identities: Vec<ProxyRawFileIdentity>,
}

impl ProxyRawDeletionPlan {
    pub(crate) fn empty() -> Self {
        Self {
            paths: Vec::new(),
            fences: Vec::new(),
            identities: Vec::new(),
        }
    }
}

pub(crate) async fn filter_unreferenced_proxy_raw_paths(
    connection: &mut sqlx::SqliteConnection,
    raw_paths: &[Option<String>],
    fallback_root: Option<&Path>,
) -> Result<ProxyRawDeletionPlan> {
    let candidates = raw_paths
        .iter()
        .flatten()
        .cloned()
        .collect::<std::collections::BTreeSet<_>>();
    let mut plan = ProxyRawDeletionPlan {
        paths: Vec::with_capacity(candidates.len()),
        fences: Vec::new(),
        identities: Vec::new(),
    };
    let mut locked_parents = std::collections::BTreeSet::new();
    for path in candidates {
        let existing_paths = resolved_raw_path_read_candidates(&path, fallback_root)
            .into_iter()
            .filter(|candidate| {
                std::fs::metadata(candidate)
                    .map(|metadata| metadata.file_type().is_file())
                    .unwrap_or(false)
            })
            .collect::<Vec<_>>();
        let mut fence_available = true;
        for candidate in &existing_paths {
            let parent = candidate
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or_else(|| PathBuf::from("."));
            if locked_parents.contains(&parent) {
                continue;
            }
            match retention_archive_file_try_lock(candidate) {
                Ok(fence) => {
                    locked_parents.insert(parent);
                    plan.fences.push(fence);
                }
                Err(error) => {
                    warn!(
                        path = %candidate.display(),
                        error = %error,
                        "deferring raw deletion while the raw directory fence is busy"
                    );
                    fence_available = false;
                    break;
                }
            }
        }
        if !fence_available {
            // The transaction may still release the owning row. Keep the path in the
            // inventory-reset signal so the reconciler can retry it after the writer drains.
            plan.paths.push(Some(path));
            continue;
        }
        for candidate in &existing_paths {
            if let Ok(metadata) = std::fs::metadata(candidate)
                && metadata.file_type().is_file()
            {
                plan.identities.push(ProxyRawFileIdentity {
                    path: candidate.clone(),
                    identity: retention_raw_file_identity(&metadata),
                    byte_size: metadata.len(),
                });
            }
        }
        let mut referenced = 0;
        for ledger_path in raw_path_ledger_aliases(&path, fallback_root) {
            referenced = sqlx::query_scalar::<_, i64>(
                r#"
                SELECT EXISTS(
                  SELECT 1 FROM proxy_raw_payload_blob_links
                  WHERE raw_path = ?1
                )
                "#,
            )
            .bind(ledger_path)
            .fetch_one(&mut *connection)
            .await?;
            if referenced != 0 {
                break;
            }
        }
        if referenced == 0 {
            plan.paths.push(Some(path));
        }
    }
    Ok(plan)
}
