async fn compress_cold_pool_attempt_response_raw_lane(
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
            let outcome = match process_raw_compression_candidate(
                pool,
                config,
                raw_path_fallback_root,
                dry_run,
                "attempt_response",
                &candidate,
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
            accumulate_raw_compression_summary_for_outcome(&mut summary, &outcome);
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

            let outcome = match process_raw_compression_candidate(
                pool,
                config,
                raw_path_fallback_root,
                dry_run,
                field.label(),
                &candidate,
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

            accumulate_raw_compression_summary_for_outcome(&mut summary, &outcome);

            if rows_processed >= batch_limit {
                break;
            }
        }
    }

    let hit_batch_limit = rows_processed >= batch_limit;
    Ok((summary, hit_batch_limit))
}

async fn process_raw_compression_candidate(
    pool: &Pool<Sqlite>,
    config: &AppConfig,
    raw_path_fallback_root: Option<&Path>,
    dry_run: bool,
    field_name: &str,
    candidate: &InvocationRawCompressionFieldCandidate,
) -> Result<RawCompressionFileOutcome> {
    let outcome = maybe_compress_proxy_raw_path(
        pool,
        candidate.id,
        field_name,
        Some(candidate.raw_path.as_str()),
        config.proxy_raw_compression,
        raw_path_fallback_root,
        dry_run,
    )
    .await?;
    let next_path = outcome
        .new_db_path
        .clone()
        .unwrap_or_else(|| candidate.raw_path.clone());
    let next_codec = outcome
        .new_codec
        .clone()
        .unwrap_or_else(|| raw_codec_from_path(Some(next_path.as_str())));
    if !dry_run && (next_path != candidate.raw_path || !raw_codec_is_identity(Some(&next_codec))) {
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
    Ok(outcome)
}

fn accumulate_raw_compression_summary_for_outcome(
    summary: &mut RawCompressionPassSummary,
    outcome: &RawCompressionFileOutcome,
) {
    if outcome.candidate_counted {
        summary.files_considered += 1;
    }
    if outcome.compressed {
        summary.files_compressed += 1;
    }
    summary.bytes_before += outcome.bytes_before;
    summary.bytes_after += outcome.bytes_after;
    summary.estimated_bytes_after += outcome.estimated_bytes_after;
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
        let Some(updated) = update_proxy_raw_path_reference_batch(
            pool,
            old_path,
            next_path,
            next_codec,
            by_kind,
            candidate_remaining_hint,
        )
        .await?
        else {
            return Ok(false);
        };
        if updated == 0 {
            break;
        }
    }
    debug!(
        old_path,
        next_path, next_codec, "propagated shared proxy raw path replacement"
    );
    Ok(true)
}

async fn update_proxy_raw_path_reference_batch(
    pool: &Pool<Sqlite>,
    old_path: &str,
    next_path: &str,
    next_codec: &str,
    by_kind: BTreeMap<String, Vec<i64>>,
    candidate_remaining_hint: usize,
) -> Result<Option<usize>> {
    let Some(admission) = acquire_retention_write_admission("raw_path_reference_update").await
    else {
        return Ok(None);
    };
    let execute_started = Instant::now();
    let mut tx = pool.begin().await?;
    let mut updated = 0usize;
    for (reference_kind, ids) in by_kind {
        let (table, path_column, codec_column) = match reference_kind.as_str() {
            "invocation_request" => ("codex_invocations", "request_raw_path", "request_raw_codec"),
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
    Ok(Some(updated))
}

async fn wake_retention_startup_backfill_tasks(
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

async fn reset_retention_raw_payload_metrics_inventory(state: &AppState) -> Result<bool> {
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

async fn reset_retention_raw_payload_metrics_inventory_batch(
    state: &AppState,
) -> Result<Option<bool>> {
    let Some(admission) = acquire_retention_write_admission("raw_metrics_inventory_reset").await
    else {
        return Ok(None);
    };
    let candidate_limit = retention_candidate_limit(&state.config, "raw_metrics_inventory_reset");
    let execute_started = Instant::now();
    let outcome = reset_system_raw_payload_metrics_inventory_batch(state, candidate_limit).await?;
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

async fn filter_unreferenced_proxy_raw_paths(
    pool: &Pool<Sqlite>,
    raw_paths: &[Option<String>],
) -> Result<Vec<Option<String>>> {
    let candidates = raw_paths
        .iter()
        .flatten()
        .cloned()
        .collect::<std::collections::BTreeSet<_>>();
    let mut unreferenced = Vec::with_capacity(candidates.len());
    for path in candidates {
        let referenced = sqlx::query_scalar::<_, i64>(
            r#"
            SELECT EXISTS(
              SELECT 1 FROM proxy_raw_payload_blob_links
              WHERE raw_path = ?1
              UNION ALL
              SELECT 1 FROM codex_invocations
              WHERE request_raw_path = ?1 OR response_raw_path = ?1
              UNION ALL
              SELECT 1 FROM pool_upstream_request_attempts
              WHERE response_raw_path = ?1
            )
            "#,
        )
        .bind(&path)
        .fetch_one(pool)
        .await?;
        if referenced == 0 {
            unreferenced.push(Some(path));
        }
    }
    Ok(unreferenced)
}
