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

pub(crate) async fn prune_old_invocation_details(
    pool: &Pool<Sqlite>,
    config: &AppConfig,
    raw_path_fallback_root: Option<&Path>,
    dry_run: bool,
) -> Result<(usize, usize, usize)> {
    let prune_cutoff = shanghai_local_cutoff_string(config.invocation_success_full_days);
    let archive_cutoff = shanghai_local_cutoff_string(config.invocation_max_days);
    let spec = archive_table_spec("codex_invocations");
    let success_like_condition = invocation_status_is_success_like_sql("status", "error_message");
    if dry_run {
        let sql = format!(
            r#"
            SELECT id, occurred_at, request_raw_path, response_raw_path,
                   COALESCE(length(payload), 0) + COALESCE(length(raw_response), 0) + 512
                       AS estimated_write_bytes
            FROM codex_invocations
            WHERE {success_like_condition}
              AND detail_level = ?1
              AND occurred_at < ?2
              AND occurred_at >= ?3
            ORDER BY occurred_at ASC, id ASC
            "#,
            success_like_condition = success_like_condition,
        );
        let candidates = sqlx::query_as::<_, InvocationDetailPruneCandidate>(&sql)
            .bind(DETAIL_LEVEL_FULL)
            .bind(&prune_cutoff)
            .bind(&archive_cutoff)
            .fetch_all(pool)
            .await?;
        let mut by_group: BTreeMap<String, usize> = BTreeMap::new();
        for candidate in &candidates {
            let group_key = invocation_archive_group_key(config, &candidate.occurred_at)?;
            *by_group.entry(group_key).or_default() += 1;
        }
        for (group_key, rows) in &by_group {
            info!(
                dataset = spec.dataset,
                archive_group = group_key,
                rows = *rows,
                reason = DETAIL_PRUNE_REASON_SUCCESS_OVER_30D,
                "retention dry-run planned invocation detail prune archive batch"
            );
        }
        let raw_paths = candidates
            .iter()
            .flat_map(|candidate| {
                [
                    candidate.request_raw_path.clone(),
                    candidate.response_raw_path.clone(),
                ]
            })
            .collect::<Vec<_>>();
        return Ok((
            candidates.len(),
            by_group.len(),
            count_existing_proxy_raw_paths(&raw_paths, raw_path_fallback_root),
        ));
    }

    let mut rows_pruned = 0usize;
    let mut archive_batches = 0usize;
    let mut raw_files_removed = 0usize;

    loop {
        let sql = format!(
            r#"
            SELECT id, occurred_at, request_raw_path, response_raw_path,
                   COALESCE(length(payload), 0) + COALESCE(length(raw_response), 0) + 512
                       AS estimated_write_bytes
            FROM codex_invocations
            WHERE {success_like_condition}
              AND detail_level = ?1
              AND occurred_at < ?2
              AND occurred_at >= ?3
            ORDER BY occurred_at ASC, id ASC
            LIMIT ?4
            "#,
            success_like_condition = success_like_condition,
        );
        let candidate_limit = retention_candidate_limit(config, "invocation_detail_prune");
        let candidates = sqlx::query_as::<_, InvocationDetailPruneCandidate>(&sql)
            .bind(DETAIL_LEVEL_FULL)
            .bind(&prune_cutoff)
            .bind(&archive_cutoff)
            .bind(candidate_limit as i64)
            .fetch_all(pool)
            .await?;

        if candidates.is_empty() {
            break;
        }

        let candidate_remaining_hint = usize::from(candidates.len() >= candidate_limit);
        let mut by_group: BTreeMap<String, Vec<InvocationDetailPruneCandidate>> = BTreeMap::new();
        for candidate in candidates {
            let group_key = invocation_archive_group_key(config, &candidate.occurred_at)?;
            by_group.entry(group_key).or_default().push(candidate);
        }

        for (group_key, group) in by_group {
            let group = take_retention_micro_batch(group, |candidate| {
                candidate.estimated_write_bytes.max(1) as usize
            });
            let prepare_started = Instant::now();
            let ids = group
                .iter()
                .map(|candidate| candidate.id)
                .collect::<Vec<_>>();
            let raw_paths = group
                .iter()
                .flat_map(|candidate| {
                    [
                        candidate.request_raw_path.clone(),
                        candidate.response_raw_path.clone(),
                    ]
                })
                .collect::<Vec<_>>();
            let Some(mut archive_outcome) = retention_prepared_batch_or_deferred(
                match archive_layout_for_dataset(config, spec.dataset) {
                    ArchiveBatchLayout::LegacyMonth => {
                        archive_rows_into_month_batch(pool, config, spec, &group_key, &ids).await
                    }
                    ArchiveBatchLayout::SegmentV1 => {
                        archive_rows_into_segment_batch(pool, config, spec, &group_key, &ids).await
                    }
                },
            )?
            else {
                return Ok((rows_pruned, archive_batches, raw_files_removed));
            };
            set_archive_batch_coverage_from_local_rows(
                &mut archive_outcome,
                group.iter().map(|candidate| candidate.occurred_at.as_str()),
                Some(config.invocation_archive_ttl_days),
            )?;
            archive_outcome.summary_source_kind = SUMMARY_ARCHIVE_SOURCE_KIND_LIVE_MIRROR;
            let pruned_at = format_naive(Utc::now().with_timezone(&Shanghai).naive_local());
            let prepare_elapsed = prepare_started.elapsed();
            let Some(admission) =
                acquire_retention_write_admission("invocation_detail_prune").await
            else {
                return Ok((rows_pruned, archive_batches, raw_files_removed));
            };
            let execute_started = Instant::now();
            let mut tx = pool.begin().await?;
            upsert_archive_batch_manifest(tx.as_mut(), &archive_outcome).await?;
            mark_archive_batch_historical_rollups_materialized_tx(
                tx.as_mut(),
                spec.dataset,
                &archive_outcome.file_path,
            )
            .await?;
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
                for id in &ids {
                    separated.push_bind(id);
                }
            }
            query.push(")");
            query.build().execute(tx.as_mut()).await?;
            if let Some(latest) = group
                .iter()
                .map(|candidate| candidate.occurred_at.as_str())
                .max()
            {
                record_parallel_work_unrecoverable_detail_tx(tx.as_mut(), latest).await?;
            }
            let commit_started = Instant::now();
            tx.commit().await?;
            retention_record_commit!(
                "invocation_detail_prune",
                admission.admission_mode(),
                group.len(),
                group
                    .iter()
                    .map(|candidate| candidate.estimated_write_bytes.max(1) as usize)
                    .sum(),
                prepare_elapsed,
                admission.lock_wait(),
                commit_started.duration_since(execute_started),
                commit_started.elapsed(),
                admission.p1_waiter_count,
                candidate_remaining_hint,
            );
            drop(admission);
            rows_pruned += group.len();
            archive_batches += 1;

            let raw_paths = filter_unreferenced_proxy_raw_paths(pool, &raw_paths).await?;
            raw_files_removed += delete_proxy_raw_paths(&raw_paths, raw_path_fallback_root)?;
        }
    }

    Ok((rows_pruned, archive_batches, raw_files_removed))
}
