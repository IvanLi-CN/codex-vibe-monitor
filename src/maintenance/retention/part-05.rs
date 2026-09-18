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
        return dry_run_invocation_detail_prune(
            pool,
            config,
            raw_path_fallback_root,
            &prune_cutoff,
            &archive_cutoff,
            &success_like_condition,
            spec,
        )
        .await;
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
            let Some((pruned, batches, removed)) = prune_invocation_detail_group(
                pool,
                config,
                spec,
                &group_key,
                group,
                candidate_remaining_hint,
                raw_path_fallback_root,
            )
            .await?
            else {
                return Ok((rows_pruned, archive_batches, raw_files_removed));
            };
            rows_pruned += pruned;
            archive_batches += batches;
            raw_files_removed += removed;
        }
    }

    Ok((rows_pruned, archive_batches, raw_files_removed))
}

async fn dry_run_invocation_detail_prune(
    pool: &Pool<Sqlite>,
    config: &AppConfig,
    raw_path_fallback_root: Option<&Path>,
    prune_cutoff: &str,
    archive_cutoff: &str,
    success_like_condition: &str,
    spec: ArchiveTableSpec,
) -> Result<(usize, usize, usize)> {
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
        .bind(prune_cutoff)
        .bind(archive_cutoff)
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
    Ok((
        candidates.len(),
        by_group.len(),
        count_existing_proxy_raw_paths(&raw_paths, raw_path_fallback_root),
    ))
}

struct PreparedInvocationDetailPrune {
    outcome: ArchiveBatchOutcome,
    ids: Vec<i64>,
    raw_paths: Vec<Option<String>>,
    group_len: usize,
    estimated_write_bytes: usize,
    latest_occurred_at: Option<String>,
    pruned_at: String,
    prepare_elapsed: Duration,
    candidate_remaining_hint: usize,
}

async fn prune_invocation_detail_group(
    pool: &Pool<Sqlite>,
    config: &AppConfig,
    spec: ArchiveTableSpec,
    group_key: &str,
    group: Vec<InvocationDetailPruneCandidate>,
    candidate_remaining_hint: usize,
    raw_path_fallback_root: Option<&Path>,
) -> Result<Option<(usize, usize, usize)>> {
    let Some(batch) = prepare_invocation_detail_prune_group(
        pool,
        config,
        spec,
        group_key,
        group,
        candidate_remaining_hint,
    )
    .await?
    else {
        return Ok(None);
    };
    if !commit_invocation_detail_prune_batch(pool, spec, &batch).await? {
        return Ok(None);
    }
    let raw_paths = filter_unreferenced_proxy_raw_paths(pool, &batch.raw_paths).await?;
    let removed = delete_proxy_raw_paths(&raw_paths, raw_path_fallback_root)?;
    Ok(Some((batch.group_len, 1, removed)))
}

async fn prepare_invocation_detail_prune_group(
    pool: &Pool<Sqlite>,
    config: &AppConfig,
    spec: ArchiveTableSpec,
    group_key: &str,
    group: Vec<InvocationDetailPruneCandidate>,
    candidate_remaining_hint: usize,
) -> Result<Option<PreparedInvocationDetailPrune>> {
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
    let Some(mut outcome) = retention_prepared_batch_or_deferred(
        match archive_layout_for_dataset(config, spec.dataset) {
            ArchiveBatchLayout::LegacyMonth => {
                archive_rows_into_month_batch(pool, config, spec, group_key, &ids).await
            }
            ArchiveBatchLayout::SegmentV1 => {
                archive_rows_into_segment_batch(pool, config, spec, group_key, &ids).await
            }
        },
    )?
    else {
        return Ok(None);
    };
    set_archive_batch_coverage_from_local_rows(
        &mut outcome,
        group.iter().map(|candidate| candidate.occurred_at.as_str()),
        Some(config.invocation_archive_ttl_days),
    )?;
    outcome.summary_source_kind = SUMMARY_ARCHIVE_SOURCE_KIND_LIVE_MIRROR;
    Ok(Some(PreparedInvocationDetailPrune {
        outcome,
        ids,
        raw_paths,
        group_len: group.len(),
        estimated_write_bytes: group
            .iter()
            .map(|candidate| candidate.estimated_write_bytes.max(1) as usize)
            .sum(),
        latest_occurred_at: group
            .iter()
            .map(|candidate| candidate.occurred_at.clone())
            .max(),
        pruned_at: format_naive(Utc::now().with_timezone(&Shanghai).naive_local()),
        prepare_elapsed: prepare_started.elapsed(),
        candidate_remaining_hint,
    }))
}

async fn commit_invocation_detail_prune_batch(
    pool: &Pool<Sqlite>,
    spec: ArchiveTableSpec,
    batch: &PreparedInvocationDetailPrune,
) -> Result<bool> {
    let Some(admission) = acquire_retention_write_admission("invocation_detail_prune").await else {
        return Ok(false);
    };
    let execute_started = Instant::now();
    let mut tx = pool.begin().await?;
    upsert_archive_batch_manifest(tx.as_mut(), &batch.outcome).await?;
    mark_archive_batch_historical_rollups_materialized_tx(
        tx.as_mut(),
        spec.dataset,
        &batch.outcome.file_path,
    )
    .await?;
    let mut query = QueryBuilder::<Sqlite>::new(
        "UPDATE codex_invocations SET payload = CASE WHEN json_valid(payload) AND (json_extract(payload, '$.upstreamAccountId') IS NOT NULL OR json_extract(payload, '$.requestModel') IS NOT NULL OR json_extract(payload, '$.responseModel') IS NOT NULL OR json_extract(payload, '$.reasoningEffort') IS NOT NULL OR json_extract(payload, '$.requestCompressionAlgorithm') IS NOT NULL) THEN json_patch(json_patch(json_patch(json_patch(json_patch('{}', CASE WHEN json_extract(payload, '$.upstreamAccountId') IS NOT NULL THEN json_object('upstreamAccountId', json_extract(payload, '$.upstreamAccountId')) ELSE '{}' END), CASE WHEN json_extract(payload, '$.requestModel') IS NOT NULL THEN json_object('requestModel', json_extract(payload, '$.requestModel')) ELSE '{}' END), CASE WHEN json_extract(payload, '$.responseModel') IS NOT NULL THEN json_object('responseModel', json_extract(payload, '$.responseModel')) ELSE '{}' END), CASE WHEN json_extract(payload, '$.reasoningEffort') IS NOT NULL THEN json_object('reasoningEffort', json_extract(payload, '$.reasoningEffort')) ELSE '{}' END), CASE WHEN json_extract(payload, '$.requestCompressionAlgorithm') IS NOT NULL THEN json_object('requestCompressionAlgorithm', json_extract(payload, '$.requestCompressionAlgorithm')) ELSE '{}' END) ELSE NULL END, raw_response = '', request_raw_path = NULL, request_raw_codec = 'identity', request_raw_size = NULL, request_raw_truncated = 0, request_raw_truncated_reason = NULL, response_raw_path = NULL, response_raw_codec = 'identity', response_raw_size = NULL, response_raw_truncated = 0, response_raw_truncated_reason = NULL, detail_level = ",
    );
    query
        .push_bind(DETAIL_LEVEL_STRUCTURED_ONLY)
        .push(", detail_pruned_at = ")
        .push_bind(&batch.pruned_at)
        .push(", detail_prune_reason = ")
        .push_bind(DETAIL_PRUNE_REASON_SUCCESS_OVER_30D)
        .push(" WHERE id IN (");
    {
        let mut separated = query.separated(", ");
        for id in &batch.ids {
            separated.push_bind(id);
        }
    }
    query.push(")");
    query.build().execute(tx.as_mut()).await?;
    if let Some(latest) = batch.latest_occurred_at.as_deref() {
        record_parallel_work_unrecoverable_detail_tx(tx.as_mut(), latest).await?;
    }
    let commit_started = Instant::now();
    tx.commit().await?;
    retention_record_commit!(
        "invocation_detail_prune",
        admission.admission_mode(),
        batch.group_len,
        batch.estimated_write_bytes,
        batch.prepare_elapsed,
        admission.lock_wait(),
        commit_started.duration_since(execute_started),
        commit_started.elapsed(),
        admission.p1_waiter_count,
        batch.candidate_remaining_hint,
    );
    drop(admission);
    Ok(true)
}

pub(crate) async fn archive_old_invocations(
    pool: &Pool<Sqlite>,
    config: &AppConfig,
    raw_path_fallback_root: Option<&Path>,
    dry_run: bool,
) -> Result<(usize, usize, usize)> {
    let cutoff = shanghai_local_cutoff_string(config.invocation_max_days);
    let spec = archive_table_spec("codex_invocations");
    let candidate_limit = retention_candidate_limit(config, "invocation_archive");

    if dry_run {
        return dry_run_old_invocations(pool, config, raw_path_fallback_root, &cutoff, spec).await;
    }

    let mut rows_archived = 0usize;
    let mut archive_batches = 0usize;
    let mut raw_files_removed = 0usize;

    loop {
        let candidates = load_old_invocation_candidates(pool, &cutoff, candidate_limit).await?;
        if candidates.is_empty() {
            break;
        }
        let candidate_remaining_hint = usize::from(candidates.len() >= candidate_limit);
        let by_group = group_old_invocation_candidates(config, candidates)?;
        for (group_key, group) in by_group {
            let Some(prepared) =
                prepare_old_invocation_archive(pool, config, spec, &group_key, group).await?
            else {
                return Ok((rows_archived, archive_batches, raw_files_removed));
            };
            let Some(committed) =
                commit_old_invocation_archive(pool, spec, prepared, candidate_remaining_hint)
                    .await?
            else {
                return Ok((rows_archived, archive_batches, raw_files_removed));
            };
            rows_archived += committed.row_count;
            archive_batches += 1;
            let raw_paths = filter_unreferenced_proxy_raw_paths(pool, &committed.raw_paths).await?;
            raw_files_removed += delete_proxy_raw_paths(&raw_paths, raw_path_fallback_root)?;
        }
    }

    Ok((rows_archived, archive_batches, raw_files_removed))
}

async fn load_old_invocation_candidates(
    pool: &Pool<Sqlite>,
    cutoff: &str,
    candidate_limit: usize,
) -> Result<Vec<InvocationArchiveCandidate>> {
    sqlx::query_as::<_, InvocationArchiveCandidate>(
        r#"
        SELECT id, invoke_id, occurred_at, source, status, input_tokens, output_tokens,
               cache_input_tokens, reasoning_tokens, total_tokens, cost, first_token_ms,
               payload, request_raw_path, response_raw_path
        FROM codex_invocations
        WHERE occurred_at < ?1
        ORDER BY occurred_at ASC, id ASC
        LIMIT ?2
        "#,
    )
    .bind(cutoff)
    .bind(candidate_limit as i64)
    .fetch_all(pool)
    .await
    .map_err(Into::into)
}

fn group_old_invocation_candidates(
    config: &AppConfig,
    candidates: Vec<InvocationArchiveCandidate>,
) -> Result<BTreeMap<String, Vec<InvocationArchiveCandidate>>> {
    let mut by_group = BTreeMap::new();
    for candidate in candidates {
        let group_key = invocation_archive_group_key(config, &candidate.occurred_at)?;
        by_group
            .entry(group_key)
            .or_insert_with(Vec::new)
            .push(candidate);
    }
    Ok(by_group)
}

struct PreparedInvocationArchive {
    group: Vec<InvocationArchiveCandidate>,
    raw_paths: Vec<Option<String>>,
    ids: Vec<i64>,
    materialized_rows: Vec<InvocationHourlySourceRecord>,
    archive_outcome: ArchiveBatchOutcome,
    prepare_elapsed: Duration,
}

async fn prepare_old_invocation_archive(
    pool: &Pool<Sqlite>,
    config: &AppConfig,
    spec: ArchiveTableSpec,
    group_key: &str,
    group: Vec<InvocationArchiveCandidate>,
) -> Result<Option<PreparedInvocationArchive>> {
    let group = take_retention_micro_batch(group, |candidate| {
        candidate.payload.as_deref().map_or(256, str::len).max(1)
    });
    let prepare_started = Instant::now();
    let raw_paths = group
        .iter()
        .flat_map(|candidate| {
            [
                candidate.request_raw_path.clone(),
                candidate.response_raw_path.clone(),
            ]
        })
        .collect::<Vec<_>>();
    let ids = group
        .iter()
        .map(|candidate| candidate.id)
        .collect::<Vec<_>>();
    let materialized_rows = group
        .iter()
        .map(invocation_archive_candidate_to_hourly_source_record)
        .collect::<Vec<_>>();
    let Some(mut archive_outcome) = retention_prepared_batch_or_deferred(
        match archive_layout_for_dataset(config, spec.dataset) {
            ArchiveBatchLayout::LegacyMonth => {
                archive_rows_into_month_batch(pool, config, spec, group_key, &ids).await
            }
            ArchiveBatchLayout::SegmentV1 => {
                archive_rows_into_segment_batch(pool, config, spec, group_key, &ids).await
            }
        },
    )?
    else {
        return Ok(None);
    };
    set_archive_batch_coverage_from_local_rows(
        &mut archive_outcome,
        group.iter().map(|candidate| candidate.occurred_at.as_str()),
        None,
    )?;
    archive_outcome.archive_expires_at = Some(shanghai_archive_expiry_from_reference_timestamp(
        &format_utc_iso(Utc::now()),
        config.invocation_archive_ttl_days,
    )?);
    archive_outcome.summary_source_kind = SUMMARY_ARCHIVE_SOURCE_KIND_AUTHORITATIVE;
    Ok(Some(PreparedInvocationArchive {
        group,
        raw_paths,
        ids,
        materialized_rows,
        archive_outcome,
        prepare_elapsed: prepare_started.elapsed(),
    }))
}

struct CommittedInvocationArchive {
    row_count: usize,
    raw_paths: Vec<Option<String>>,
}

async fn commit_old_invocation_archive(
    pool: &Pool<Sqlite>,
    spec: ArchiveTableSpec,
    prepared: PreparedInvocationArchive,
    candidate_remaining_hint: usize,
) -> Result<Option<CommittedInvocationArchive>> {
    let Some(admission) = acquire_retention_write_admission("invocation_archive").await else {
        return Ok(None);
    };
    let execute_started = Instant::now();
    let mut tx = pool.begin().await?;
    materialize_unprojected_invocation_rows_tx(tx.as_mut(), &prepared.materialized_rows).await?;
    upsert_invocation_rollups(tx.as_mut(), &prepared.group).await?;
    stage_invocation_archive_batch_manifest(tx.as_mut(), &prepared.archive_outcome).await?;
    mark_archive_batch_historical_rollups_materialized_tx(
        tx.as_mut(),
        spec.dataset,
        &prepared.archive_outcome.file_path,
    )
    .await?;
    write_invocation_summary_snapshot_tx(
        tx.as_mut(),
        spec,
        &prepared.archive_outcome,
        &prepared.materialized_rows,
        &prepared.group,
    )
    .await?;
    finalize_invocation_archive_rows_tx(
        tx.as_mut(),
        spec,
        &prepared.archive_outcome,
        &prepared.ids,
        &prepared.materialized_rows,
    )
    .await?;
    let commit_started = Instant::now();
    tx.commit().await?;
    retention_record_commit!(
        "invocation_archive",
        admission.admission_mode(),
        prepared.group.len(),
        prepared
            .group
            .iter()
            .map(|candidate| candidate.payload.as_deref().map_or(256, str::len))
            .sum(),
        prepared.prepare_elapsed,
        admission.lock_wait(),
        commit_started.duration_since(execute_started),
        commit_started.elapsed(),
        admission.p1_waiter_count,
        candidate_remaining_hint,
    );
    drop(admission);
    Ok(Some(CommittedInvocationArchive {
        row_count: prepared.group.len(),
        raw_paths: prepared.raw_paths,
    }))
}

async fn materialize_unprojected_invocation_rows_tx(
    tx: &mut SqliteConnection,
    materialized_rows: &[InvocationHourlySourceRecord],
) -> Result<()> {
    let live_rollup_cursor =
        load_hourly_rollup_live_progress_tx(tx, HOURLY_ROLLUP_DATASET_INVOCATIONS).await?;
    let unprojected_rows = materialized_rows
        .iter()
        .filter(|row| row.id > live_rollup_cursor)
        .cloned()
        .collect::<Vec<_>>();
    if unprojected_rows.is_empty() {
        return Ok(());
    }
    upsert_invocation_hourly_rollups_tx(tx, &unprojected_rows, &INVOCATION_HOURLY_ROLLUP_TARGETS)
        .await?;
    let prefix_end = unprojected_rows
        .iter()
        .map(|row| row.id)
        .max()
        .expect("unprojected rows are non-empty");
    let prefix_row_count = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM codex_invocations WHERE id > ?1 AND id <= ?2",
    )
    .bind(live_rollup_cursor)
    .bind(prefix_end)
    .fetch_one(&mut *tx)
    .await?;
    if prefix_row_count == unprojected_rows.len() as i64 {
        save_hourly_rollup_live_progress_tx(tx, HOURLY_ROLLUP_DATASET_INVOCATIONS, prefix_end)
            .await?;
    }
    Ok(())
}

async fn write_invocation_summary_snapshot_tx(
    tx: &mut SqliteConnection,
    spec: ArchiveTableSpec,
    archive_outcome: &ArchiveBatchOutcome,
    materialized_rows: &[InvocationHourlySourceRecord],
    group: &[InvocationArchiveCandidate],
) -> Result<()> {
    let snapshot_archive_batch_id = load_archive_batch_id_for_file_tx(
        tx,
        spec.dataset,
        &archive_outcome.month_key,
        &archive_outcome.file_path,
    )
    .await?;
    let invoke_ids_by_row_id = group
        .iter()
        .map(|candidate| (candidate.id, candidate.invoke_id.clone()))
        .collect::<HashMap<_, _>>();
    let snapshot_page = SummaryArchiveSnapshotPage {
        archive_batch_id: snapshot_archive_batch_id,
        manifest_sha256: archive_outcome.sha256.clone(),
        page_index: 0,
        coverage_start: archive_outcome
            .coverage_start_at
            .clone()
            .ok_or_else(|| anyhow!("authoritative archive is missing Snapshot start coverage"))?,
        coverage_end: archive_outcome
            .coverage_end_at
            .clone()
            .ok_or_else(|| anyhow!("authoritative archive is missing Snapshot end coverage"))?,
        row_count: u32::try_from(materialized_rows.len())
            .context("Summary Archive Snapshot row count overflow")?,
        payload: encode_summary_archive_snapshot_v2_payload(
            materialized_rows,
            &invoke_ids_by_row_id,
        )?,
    };
    store_summary_archive_snapshot_page_v2_tx(tx, &snapshot_page).await?;
    if !summary_archive_snapshot_has_proof_tx(
        tx,
        snapshot_archive_batch_id,
        &archive_outcome.sha256,
    )
    .await?
    {
        bail!("retention Summary Snapshot V2 semantic proof validation failed");
    }
    store_summary_archive_snapshot_v2_final_proof_tx(
        tx,
        snapshot_archive_batch_id,
        &archive_outcome.sha256,
    )
    .await
}

async fn finalize_invocation_archive_rows_tx(
    tx: &mut SqliteConnection,
    spec: ArchiveTableSpec,
    archive_outcome: &ArchiveBatchOutcome,
    ids: &[i64],
    materialized_rows: &[InvocationHourlySourceRecord],
) -> Result<()> {
    delete_rows_by_ids(tx, spec.dataset, ids).await?;
    mark_retention_archived_hourly_rollup_targets_tx(tx, spec.dataset, materialized_rows, &[])
        .await?;
    for target in INVOCATION_HOURLY_ROLLUP_TARGETS {
        mark_hourly_rollup_archive_replayed_tx(
            tx,
            target,
            spec.dataset,
            &archive_outcome.file_path,
        )
        .await?;
    }
    finalize_invocation_archive_batch_publication_tx(tx, &archive_outcome.file_path).await
}

async fn dry_run_old_invocations(
    pool: &Pool<Sqlite>,
    config: &AppConfig,
    raw_path_fallback_root: Option<&Path>,
    cutoff: &str,
    spec: ArchiveTableSpec,
) -> Result<(usize, usize, usize)> {
    let candidates = sqlx::query_as::<_, InvocationArchiveCandidate>(
        r#"
        SELECT
            id,
            invoke_id,
            occurred_at,
            source,
            status,
            input_tokens,
            output_tokens,
            cache_input_tokens,
            reasoning_tokens,
            total_tokens,
            cost,
            first_token_ms,
            payload,
            request_raw_path,
            response_raw_path
        FROM codex_invocations
        WHERE occurred_at < ?1
        ORDER BY occurred_at ASC, id ASC
        "#,
    )
    .bind(cutoff)
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
            reason = DETAIL_PRUNE_REASON_MAX_AGE_ARCHIVED,
            "retention dry-run planned invocation archive batch"
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
    Ok((
        candidates.len(),
        by_group.len(),
        count_existing_proxy_raw_paths(&raw_paths, raw_path_fallback_root),
    ))
}
