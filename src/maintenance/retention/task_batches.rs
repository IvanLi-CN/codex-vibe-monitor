use super::*;
use crate::observability::contention::RetentionPhase;
use sqlx::SqliteConnection;

struct ArchiveSourceIdentityProof {
    cleanup: TempSqliteCleanup,
    _work_owner_file: crate::maintenance::archive::TaskWorkOwner,
    connection: Option<SqliteConnection>,
    spec: ArchiveTableSpec,
}

impl ArchiveSourceIdentityProof {
    async fn load(path: &Path, spec: ArchiveTableSpec) -> Result<Self> {
        // The writer/publication path already proves the returned artifact digest. The first
        // source admission revalidates that digest under the archive fence; this local copy only
        // supplies per-chunk identity data and must not replace that final recheck.
        let proof_path = PathBuf::from(format!(
            "{}.task-{}.sqlite",
            path.display(),
            retention_temp_suffix()
        ));
        let work_owner_file =
            crate::maintenance::archive::create_retention_identity_work_file(path, &proof_path)?;
        let cleanup = TempSqliteCleanup::new(proof_path.clone());
        inflate_gzip_sqlite_file(path, &proof_path)?;
        let connection = open_archive_sqlite_connection(&proof_path).await?;
        Ok(Self {
            cleanup,
            _work_owner_file: work_owner_file,
            connection: Some(connection),
            spec,
        })
    }

    async fn identity(&mut self, ids: &[i64]) -> Result<String> {
        let connection = self
            .connection
            .as_mut()
            .context("archive source identity proof connection is closed")?;
        if self.spec.dataset == "codex_invocations" {
            invocation_archive_source_identity_sha256(
                connection,
                InvocationArchiveIdentityDatabase::Main,
                ids,
            )
            .await
        } else {
            archive_table_source_identity_sha256(connection, self.spec, "main", ids).await
        }
    }

    async fn close(mut self) -> Result<()> {
        if let Some(connection) = self.connection.take() {
            connection.close().await?;
        }
        let _ = &self.cleanup;
        Ok(())
    }
}

struct RetentionBatchCommit {
    raw_paths: Vec<Option<String>>,
    raw_reference_check_elapsed: Option<Duration>,
    commit_started: Instant,
    prompt_cache_keys: Vec<String>,
}

struct RetentionSqliteTransaction<'a>(&'a mut SqliteConnection);

impl RetentionSqliteTransaction<'_> {
    fn as_mut(&mut self) -> &mut SqliteConnection {
        self.0
    }
}

fn take_retention_batch<T>(
    pending: &mut Vec<T>,
    config: &AppConfig,
    operation: &'static str,
    estimated_bytes: impl Fn(&T) -> usize,
) -> Vec<T> {
    let candidate_count = pending
        .len()
        .min(retention_micro_batch_limit(config, operation));
    let indices = (0..candidate_count).collect::<Vec<_>>();
    let selected = take_retention_micro_batch(indices, |index| estimated_bytes(&pending[*index]));
    let count = selected.len();
    pending.drain(..count).collect()
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
        if retention_run_budget_expired() {
            break;
        }
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
        let candidate_limit = archive_candidate_limit(config);
        let candidates_query = sqlx::query_as::<_, InvocationDetailPruneCandidate>(&sql)
            .bind(DETAIL_LEVEL_FULL)
            .bind(&prune_cutoff)
            .bind(&archive_cutoff)
            .bind(candidate_limit as i64)
            .fetch_all(pool);
        let candidates = if let Some(remaining) = retention_run_remaining_budget() {
            match tokio::time::timeout(remaining, candidates_query).await {
                Ok(result) => result?,
                Err(_) => break,
            }
        } else {
            candidates_query.await?
        };

        if candidates.is_empty() {
            break;
        }

        let candidate_remaining_hint = usize::from(candidates.len() >= candidate_limit);
        let mut by_group: BTreeMap<String, Vec<InvocationDetailPruneCandidate>> = BTreeMap::new();
        for candidate in candidates {
            if retention_run_budget_expired() {
                break;
            }
            let group_key = invocation_archive_group_key(config, &candidate.occurred_at)?;
            by_group.entry(group_key).or_default().push(candidate);
        }

        for (group_key, group) in by_group {
            if retention_run_budget_expired() {
                break;
            }
            let group = select_archive_batch(group, |candidate| {
                candidate.estimated_write_bytes.max(1) as usize
            });
            let mut batch_observation = batch_plan::BatchObservation::begin(
                "codex_invocation_details",
                &group_key,
                group.len(),
            );
            let prepare_started = Instant::now();
            let ids = group
                .iter()
                .map(|candidate| candidate.id)
                .collect::<Vec<_>>();
            if ids.is_empty() || retention_run_budget_expired() {
                break;
            }
            let Some(mut source_connection) =
                acquire_retention_pool_connection(pool, "invocation_detail_identity").await?
            else {
                return Ok((rows_pruned, archive_batches, raw_files_removed));
            };
            let source_verify_phase = batch_observation.phase(RetentionPhase::SourceVerify);
            let source_identity_started = Instant::now();
            let source_identity_query = invocation_archive_source_identity_sha256(
                &mut source_connection,
                InvocationArchiveIdentityDatabase::Main,
                &ids,
            );
            let source_identity_sha256 = if let Some(remaining) = retention_run_remaining_budget() {
                match tokio::time::timeout(remaining, source_identity_query).await {
                    Ok(result) => result?,
                    Err(_) => break,
                }
            } else {
                source_identity_query
                    .await
                    .context("read task-local invocation source identity")?
            };
            batch_observation.source_identity_finished(source_identity_started.elapsed());
            drop(source_connection);
            drop(source_verify_phase);
            if retention_run_budget_expired() {
                break;
            }
            let mut descriptor = retention_prepared_archive_descriptor(
                config,
                spec.dataset,
                &group_key,
                &load_invocation_archive_candidates_by_ids(pool, &ids).await?,
                source_identity_sha256,
                RETENTION_RECOVERY_PUBLICATION_DETAIL_PRUNE,
            )?;
            descriptor.file_path = retention_live_mirror_archive_path(
                config,
                &group_key,
                &ids,
                &descriptor.source_identity_sha256,
            )?
            .to_string_lossy()
            .to_string();
            descriptor.prepared_key = format!(
                "{}:{}:{}",
                descriptor.dataset, descriptor.file_path, descriptor.source_identity_sha256
            );
            let prepare_phase = batch_observation.phase(RetentionPhase::Prepare);
            let archive_result = match archive_layout_for_dataset(config, spec.dataset) {
                ArchiveBatchLayout::LegacyMonth => {
                    archive_rows_into_month_batch_at_path(
                        pool,
                        spec,
                        &group_key,
                        &ids,
                        PathBuf::from(&descriptor.file_path),
                    )
                    .await
                }
                ArchiveBatchLayout::SegmentV1 => {
                    archive_rows_into_segment_batch_at_path(
                        pool,
                        config,
                        spec,
                        &group_key,
                        &ids,
                        PathBuf::from(&descriptor.file_path),
                    )
                    .await
                }
            };
            let Some(mut archive_outcome) = retention_prepared_batch_or_deferred(archive_result)?
            else {
                return Ok((rows_pruned, archive_batches, raw_files_removed));
            };
            if archive_outcome.source_identity_sha256.as_deref()
                != Some(descriptor.source_identity_sha256.as_str())
            {
                bail!("retention task-local archive source identity verification failed");
            }
            set_archive_batch_coverage_from_local_rows(
                &mut archive_outcome,
                group.iter().map(|candidate| candidate.occurred_at.as_str()),
                Some(config.invocation_archive_ttl_days),
            )?;
            archive_outcome.summary_source_kind = SUMMARY_ARCHIVE_SOURCE_KIND_LIVE_MIRROR;
            let pruned_at = format_naive(Utc::now().with_timezone(&Shanghai).naive_local());
            let prepare_elapsed = prepare_started.elapsed();
            batch_observation.file_prepared(prepare_elapsed);
            drop(prepare_phase);
            let mut archive_identity_proof =
                ArchiveSourceIdentityProof::load(Path::new(&archive_outcome.file_path), spec)
                    .await?;
            #[cfg(test)]
            batch_admission::pause_after_archive_artifact_verified().await;
            let mut committed_rows = 0;
            let mut pending_group = group;
            while !pending_group.is_empty() {
                let group = take_retention_batch(
                    &mut pending_group,
                    config,
                    "invocation_detail_prune",
                    |candidate| candidate.estimated_write_bytes.max(1) as usize,
                );
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
                let chunk_identity = archive_identity_proof
                    .identity(&ids)
                    .await
                    .context("read task-local invocation source chunk identity")?;
                let admission_wait_phase = batch_observation.phase(RetentionPhase::AdmissionWait);
                let Some((source_connection, admission, archive_lock)) =
                    acquire_retention_batch_write_connection_with_archive_fence_observed(
                        pool,
                        "invocation_detail_prune",
                        Path::new(&archive_outcome.file_path),
                        Some(&archive_outcome.sha256),
                        || Some(batch_observation.phase(RetentionPhase::FileVerify)),
                    )
                    .await?
                else {
                    drop(admission_wait_phase);
                    return Ok((rows_pruned, archive_batches, raw_files_removed));
                };
                let admission_mode = admission.admission_mode();
                let lock_wait = admission.lock_wait();
                let p1_waiter_count = admission.p1_waiter_count();
                let mut transaction_resources =
                    RetentionTransactionResources::new(source_connection, admission, archive_lock);
                drop(admission_wait_phase);
                let transaction_phase = batch_observation.phase(RetentionPhase::Transaction);
                let execute_started = Instant::now();
                transaction_resources.mark_transaction_started();
                let begin_result = sqlx::query("BEGIN")
                    .execute(transaction_resources.connection_mut())
                    .await;
                if let Err(error) = begin_result {
                    let cleanup_error = transaction_resources.rollback_and_close().await;
                    drop(transaction_phase);
                    let mut error: anyhow::Error = error.into();
                    if let Err(cleanup_error) = cleanup_error {
                        error = error.context(format!(
                            "failed to rollback and close source connection after begin failure: {cleanup_error}"
                        ));
                    }
                    return Err(error);
                }
                #[cfg(test)]
                batch_admission::pause_after_transaction_begin().await;
                let transaction_result: Result<RetentionBatchCommit> = async {
                    let mut tx = RetentionSqliteTransaction(transaction_resources.connection_mut());
                            upsert_archive_batch_manifest(tx.as_mut(), &archive_outcome).await?;
                            mark_archive_batch_historical_rollups_materialized_tx(
                                tx.as_mut(),
                                spec.dataset,
                                &archive_outcome.file_path,
                            )
                            .await?;
                            let source_verify_phase =
                                batch_observation.phase(RetentionPhase::SourceVerify);
                            if invocation_archive_source_identity_sha256(
                                tx.as_mut(),
                                InvocationArchiveIdentityDatabase::Main,
                                &ids,
                            )
                            .await?
                                != chunk_identity
                            {
                                bail!("retention task-local detail source changed before conversion");
                            }
                            drop(source_verify_phase);
                            let prompt_cache_keys =
                                load_prompt_cache_keys_for_invocation_ids_tx(tx.as_mut(), &ids)
                                    .await?;
                            let prompt_cache_key_refs = prompt_cache_keys
                                .iter()
                                .map(String::as_str)
                                .collect::<Vec<_>>();
                            if !prompt_cache_key_refs.is_empty() {
                                mark_prompt_cache_conversation_stats_stale_on_connection(
                                    tx.as_mut(),
                                )
                                .await?;
                            }
                            let mut query = QueryBuilder::<Sqlite>::new(
                                "UPDATE codex_invocations SET payload = CASE WHEN json_valid(payload) AND (json_extract(payload, '$.upstreamAccountId') IS NOT NULL OR json_extract(payload, '$.requestModel') IS NOT NULL OR json_extract(payload, '$.responseModel') IS NOT NULL OR json_extract(payload, '$.reasoningEffort') IS NOT NULL OR json_extract(payload, '$.requestCompressionAlgorithm') IS NOT NULL) THEN json_patch(json_patch(json_patch(json_patch(json_patch('{}', CASE WHEN json_extract(payload, '$.upstreamAccountId') IS NOT NULL THEN json_object('upstreamAccountId', json_extract(payload, '$.upstreamAccountId')) ELSE '{}' END), CASE WHEN json_extract(payload, '$.requestModel') IS NOT NULL THEN json_object('requestModel', json_extract(payload, '$.requestModel')) ELSE '{}' END), CASE WHEN json_extract(payload, '$.responseModel') IS NOT NULL THEN json_object('responseModel', json_extract(payload, '$.responseModel')) ELSE '{}' END), CASE WHEN json_extract(payload, '$.reasoningEffort') IS NOT NULL THEN json_object('reasoningEffort', json_extract(payload, '$.reasoningEffort')) ELSE '{}' END), CASE WHEN json_extract(payload, '$.requestCompressionAlgorithm') IS NOT NULL THEN json_object('requestCompressionAlgorithm', json_extract(payload, '$.requestCompressionAlgorithm')) ELSE '{}' END) ELSE NULL END, raw_response = '', request_raw_path = NULL, request_raw_codec = 'identity', request_raw_size = NULL, request_raw_truncated = 0, request_raw_truncated_reason = NULL, response_raw_path = NULL, response_raw_codec = 'identity', response_raw_size = NULL, response_raw_truncated = 0, response_raw_truncated_reason = NULL, detail_level = ",
                            );
                            query
                                .push_bind(DETAIL_LEVEL_STRUCTURED_ONLY)
                                .push(", detail_pruned_at = ")
                                .push_bind(&pruned_at)
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
                            let delete_phase = batch_observation.phase(RetentionPhase::Delete);
                            query.build().execute(tx.as_mut()).await?;
                            drop(delete_phase);
                            // UPDATE triggers persist the affected keys. Statistics are refreshed by the
                            // materialization owner after this retention transaction commits.
                            if let Some(latest) = group
                                .iter()
                                .map(|candidate| candidate.occurred_at.as_str())
                                .max()
                            {
                                record_parallel_work_unrecoverable_detail_tx(tx.as_mut(), latest)
                                    .await?;
                            }
                            let raw_reference_check_started = Instant::now();
                            let had_raw_reference_candidates = raw_paths.iter().any(Option::is_some);
                            let raw_paths = filter_unreferenced_proxy_raw_paths(
                                tx.as_mut(),
                                &raw_paths,
                                raw_path_fallback_root,
                            )
                            .await?;
                            let raw_reference_check_elapsed = had_raw_reference_candidates
                                .then(|| raw_reference_check_started.elapsed());
                            let commit_started = Instant::now();
                    Ok(RetentionBatchCommit {
                        raw_paths,
                        raw_reference_check_elapsed,
                        commit_started,
                        prompt_cache_keys: Vec::new(),
                    })
                }
                .await;
                let commit = match transaction_result {
                    Ok(commit) => commit,
                    Err(error) => {
                        let cleanup_error = transaction_resources.rollback_and_close().await;
                        drop(transaction_phase);
                        let mut error = error;
                        if let Err(cleanup_error) = cleanup_error {
                            error = error.context(format!(
                                "retention transaction rollback and close failed: {cleanup_error}"
                            ));
                        }
                        return Err(error);
                    }
                };
                let commit_phase = batch_observation.phase(RetentionPhase::Commit);
                let commit_result = sqlx::query("COMMIT")
                    .execute(transaction_resources.connection_mut())
                    .await;
                drop(commit_phase);
                if let Err(error) = commit_result {
                    let cleanup_error = transaction_resources.rollback_and_close().await;
                    drop(transaction_phase);
                    let mut error: anyhow::Error = error.into();
                    if let Err(cleanup_error) = cleanup_error {
                        error = error.context(format!(
                            "retention transaction rollback and close failed after commit failure: {cleanup_error}"
                        ));
                    }
                    return Err(error);
                }
                transaction_resources.mark_transaction_finished();
                let raw_paths = commit.raw_paths;
                let breached = retention_record_commit_with_reference_check!(
                    "invocation_detail_prune",
                    admission_mode,
                    group.len(),
                    group
                        .iter()
                        .map(|candidate| candidate.estimated_write_bytes.max(1) as usize)
                        .sum(),
                    prepare_elapsed,
                    lock_wait,
                    commit.commit_started.duration_since(execute_started),
                    commit.commit_started.elapsed(),
                    commit.raw_reference_check_elapsed,
                    p1_waiter_count,
                    candidate_remaining_hint,
                );
                rows_pruned += group.len();
                committed_rows += group.len();
                batch_observation.committed(group.len(), lock_wait);
                drop(transaction_phase);
                let cleanup_error = transaction_resources.return_to_pool().await;
                if let Err(cleanup_error) = cleanup_error {
                    return Err(anyhow!(
                        "failed to return retention transaction connection to pool: {cleanup_error}"
                    ));
                }
                let cleanup_phase = batch_observation.phase(RetentionPhase::Cleanup);
                if !raw_paths.is_empty() {
                    mark_retention_raw_inventory_reset_intent(pool).await?;
                }
                raw_files_removed += delete_proxy_raw_paths(&raw_paths, raw_path_fallback_root)?;
                drop(cleanup_phase);
                if breached {
                    tokio::task::yield_now().await;
                }
            }
            archive_identity_proof.close().await?;
            if committed_rows > 0 {
                archive_batches += 1;
            }
            batch_observation.complete();
        }
        if retention_run_remaining_budget().is_some() {
            break;
        }
    }

    Ok((rows_pruned, archive_batches, raw_files_removed))
}

#[cfg(test)]
pub(crate) async fn archive_old_invocations(
    pool: &Pool<Sqlite>,
    config: &AppConfig,
    raw_path_fallback_root: Option<&Path>,
    dry_run: bool,
) -> Result<(usize, usize, usize, std::collections::HashSet<String>)> {
    let cutoff = shanghai_local_cutoff_string(config.invocation_max_days);
    archive_old_invocations_with_source_max(
        pool,
        config,
        raw_path_fallback_root,
        dry_run,
        &cutoff,
        None,
    )
    .await
}

pub(super) async fn archive_old_invocations_with_source_max(
    pool: &Pool<Sqlite>,
    config: &AppConfig,
    raw_path_fallback_root: Option<&Path>,
    dry_run: bool,
    cutoff: &str,
    source_max_invocation_id: Option<i64>,
) -> Result<(usize, usize, usize, std::collections::HashSet<String>)> {
    let spec = archive_table_spec("codex_invocations");

    if dry_run {
        let candidates_query = sqlx::query_as::<_, InvocationArchiveCandidate>(
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
              AND (?2 IS NULL OR id <= ?2)
            ORDER BY occurred_at ASC, id ASC
            "#,
        )
        .bind(cutoff)
        .bind(source_max_invocation_id)
        .fetch_all(pool);
        let candidates = if let Some(remaining) = retention_run_remaining_budget() {
            match tokio::time::timeout(remaining, candidates_query).await {
                Ok(result) => result?,
                Err(_) => return Ok((0, 0, 0, std::collections::HashSet::new())),
            }
        } else {
            candidates_query.await?
        };

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
        return Ok((
            candidates.len(),
            by_group.len(),
            count_existing_proxy_raw_paths(&raw_paths, raw_path_fallback_root),
            std::collections::HashSet::new(),
        ));
    }

    let mut rows_archived = 0usize;
    let mut archive_batches = 0usize;
    let mut raw_files_removed = 0usize;
    let mut prompt_cache_keys = std::collections::HashSet::new();
    let mut discovered_ids = HashSet::new();

    loop {
        if retention_run_budget_expired() {
            break;
        }
        if !archive_batch_can_start() {
            break;
        }
        let candidate_limit = archive_candidate_limit(config);
        let candidates_query = sqlx::query_as::<_, InvocationArchiveCandidate>(
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
              AND (?2 IS NULL OR id <= ?2)
            ORDER BY occurred_at ASC, id ASC
            LIMIT ?3
            "#,
        )
        .bind(cutoff)
        .bind(source_max_invocation_id)
        .bind(candidate_limit as i64)
        .fetch_all(pool);
        let candidates = if let Some(remaining) = retention_run_remaining_budget() {
            match tokio::time::timeout(remaining, candidates_query).await {
                Ok(result) => result?,
                Err(_) => break,
            }
        } else {
            candidates_query.await?
        };

        if candidates.is_empty() {
            break;
        }

        discovered_ids.extend(candidates.iter().map(|candidate| candidate.id));
        workload::record_discovered_count(discovered_ids.len());

        let candidate_remaining_hint = usize::from(candidates.len() >= candidate_limit);
        let mut by_group: BTreeMap<String, Vec<InvocationArchiveCandidate>> = BTreeMap::new();
        for candidate in candidates {
            if retention_run_budget_expired() {
                break;
            }
            let group_key = shanghai_month_key_from_local_naive(&candidate.occurred_at)?;
            by_group.entry(group_key).or_default().push(candidate);
        }

        for (group_key, group) in by_group {
            if retention_run_budget_expired() {
                break;
            }
            let candidate_ids = group.iter().map(|row| row.id).collect::<Vec<_>>();
            let row_sizes = archive_source_row_sizes(pool, spec, &candidate_ids).await?;
            let group = select_archive_batch(group, |candidate| {
                row_sizes.get(&candidate.id).copied().unwrap_or(512)
            });
            let ids = group
                .iter()
                .map(|candidate| candidate.id)
                .collect::<Vec<_>>();
            if ids.is_empty() || retention_run_budget_expired() {
                break;
            }
            let mut batch_observation =
                batch_plan::BatchObservation::begin(spec.dataset, &group_key, group.len());
            let Some(mut source_connection) =
                acquire_retention_pool_connection(pool, "invocation_archive_identity").await?
            else {
                return Ok((
                    rows_archived,
                    archive_batches,
                    raw_files_removed,
                    prompt_cache_keys,
                ));
            };
            let source_verify_phase = batch_observation.phase(RetentionPhase::SourceVerify);
            let source_identity_started = Instant::now();
            let source_identity_query = invocation_archive_source_identity_sha256(
                &mut source_connection,
                InvocationArchiveIdentityDatabase::Main,
                &ids,
            );
            let source_identity_sha256 = if let Some(remaining) = retention_run_remaining_budget() {
                match tokio::time::timeout(remaining, source_identity_query).await {
                    Ok(result) => result?,
                    Err(_) => {
                        batch_observation
                            .source_identity_finished(source_identity_started.elapsed());
                        break;
                    }
                }
            } else {
                source_identity_query
                    .await
                    .context("read task-local source identity")?
            };
            batch_observation.source_identity_finished(source_identity_started.elapsed());
            drop(source_connection);
            drop(source_verify_phase);
            if retention_run_budget_expired() {
                break;
            }
            let prepare_started = Instant::now();
            let mut snapshot_pages = Vec::new();
            let prepare_phase = batch_observation.phase(RetentionPhase::Prepare);
            let archive_future = archive_rows_into_task_month_batch(
                pool,
                config,
                spec,
                &group_key,
                &ids,
                &mut snapshot_pages,
            );
            let archive_writer_started = Instant::now();
            let archive_result = if let Some(remaining) = retention_run_remaining_budget() {
                match tokio::time::timeout(remaining, archive_future).await {
                    Ok(result) => result,
                    Err(_) => {
                        batch_observation.archive_writer_finished(archive_writer_started.elapsed());
                        break;
                    }
                }
            } else {
                archive_future.await
            };
            batch_observation.archive_writer_finished(archive_writer_started.elapsed());
            let Some(mut archive_outcome) = (match archive_result {
                Ok(outcome) => Some(outcome),
                Err(error) if is_retention_write_deferred(&error) => {
                    retention_recovery_record_deferred("preparing");
                    return Ok((
                        rows_archived,
                        archive_batches,
                        raw_files_removed,
                        prompt_cache_keys,
                    ));
                }
                Err(error) => return Err(error.context("prepare task-local invocation month file")),
            }) else {
                return Ok((
                    rows_archived,
                    archive_batches,
                    raw_files_removed,
                    prompt_cache_keys,
                ));
            };
            if archive_outcome.source_identity_sha256.as_deref()
                != Some(source_identity_sha256.as_str())
            {
                bail!("retention task-local archive source identity verification failed");
            }
            set_archive_batch_coverage_from_local_rows(
                &mut archive_outcome,
                group.iter().map(|candidate| candidate.occurred_at.as_str()),
                None,
            )?;
            archive_outcome.coverage_start_at = snapshot_pages
                .first()
                .map(|page| page.coverage_start.clone());
            archive_outcome.coverage_end_at =
                snapshot_pages.last().map(|page| page.coverage_end.clone());
            archive_outcome.archive_expires_at =
                Some(shanghai_archive_expiry_from_reference_timestamp(
                    &format_utc_iso(Utc::now()),
                    config.invocation_archive_ttl_days,
                )?);
            archive_outcome.summary_source_kind = SUMMARY_ARCHIVE_SOURCE_KIND_AUTHORITATIVE;
            let prepare_elapsed = prepare_started.elapsed();
            batch_observation.file_prepared(prepare_elapsed);
            drop(prepare_phase);
            let mut archive_identity_proof =
                ArchiveSourceIdentityProof::load(Path::new(&archive_outcome.file_path), spec)
                    .await?;
            let summary_proof_started = Instant::now();
            let proof_result =
                batch_plan::prepare_summary_proof(pool, &archive_outcome, snapshot_pages)
                    .await
                    .context("prepare task-local monthly Summary proof");
            batch_observation.summary_proof_finished(summary_proof_started.elapsed());
            let Some(proof) = proof_result? else {
                return Ok((
                    rows_archived,
                    archive_batches,
                    raw_files_removed,
                    prompt_cache_keys,
                ));
            };
            #[cfg(test)]
            batch_admission::pause_after_archive_artifact_verified().await;
            let mut committed_rows = 0;
            let mut chunk_index = 0usize;
            let mut pending_group = group;
            while !pending_group.is_empty() {
                let group = take_retention_batch(
                    &mut pending_group,
                    config,
                    "invocation_archive",
                    |candidate| {
                        candidate
                            .payload
                            .as_deref()
                            .map_or(256, |payload| payload.len())
                    },
                );
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
                let materialized_rows = group
                    .iter()
                    .map(invocation_archive_candidate_to_hourly_source_record)
                    .collect::<Vec<_>>();
                let chunk_identity = archive_identity_proof
                    .identity(&ids)
                    .await
                    .context("read task-local invocation source chunk identity")?;
                let admission_wait_phase = batch_observation.phase(RetentionPhase::AdmissionWait);
                let Some((source_connection, admission, archive_lock)) =
                    acquire_retention_batch_write_connection_with_archive_fence_observed(
                        pool,
                        "invocation_archive",
                        Path::new(&archive_outcome.file_path),
                        Some(&archive_outcome.sha256),
                        || Some(batch_observation.phase(RetentionPhase::FileVerify)),
                    )
                    .await?
                else {
                    drop(admission_wait_phase);
                    return Ok((
                        rows_archived,
                        archive_batches,
                        raw_files_removed,
                        prompt_cache_keys,
                    ));
                };
                let admission_mode = admission.admission_mode();
                let lock_wait = admission.lock_wait();
                let p1_waiter_count = admission.p1_waiter_count();
                let mut transaction_resources =
                    RetentionTransactionResources::new(source_connection, admission, archive_lock);
                #[cfg(test)]
                transaction_resources.observe_return_for_test("invocation_archive");
                drop(admission_wait_phase);
                let transaction_phase = batch_observation.phase(RetentionPhase::Transaction);
                let execute_started = Instant::now();
                transaction_resources.mark_transaction_started();
                let begin_result = sqlx::query("BEGIN")
                    .execute(transaction_resources.connection_mut())
                    .await;
                if let Err(error) = begin_result {
                    let cleanup_error = transaction_resources.rollback_and_close().await;
                    drop(transaction_phase);
                    let mut error: anyhow::Error = error.into();
                    if let Err(cleanup_error) = cleanup_error {
                        error = error.context(format!(
                            "failed to rollback and close source connection after begin failure: {cleanup_error}"
                        ));
                    }
                    return Err(error);
                }
                #[cfg(test)]
                batch_admission::pause_after_transaction_begin().await;
                let transaction_result: Result<RetentionBatchCommit> = async {
                    let mut tx = RetentionSqliteTransaction(transaction_resources.connection_mut());
                    // P2 normally advances this cursor before retention. Rows beyond it would be
                    // deleted before the regular replay can observe them, so materialize just those
                    // rows in this same archive transaction before claiming the archive is covered.
                    let live_rollup_cursor = load_hourly_rollup_live_progress_tx(
                        tx.as_mut(),
                        HOURLY_ROLLUP_DATASET_INVOCATIONS,
                    )
                    .await?;
                    let unprojected_rows = materialized_rows
                        .iter()
                        .filter(|row| row.id > live_rollup_cursor)
                        .cloned()
                        .collect::<Vec<_>>();
                    if !unprojected_rows.is_empty() {
                        upsert_invocation_hourly_rollups_tx(
                            tx.as_mut(),
                            &unprojected_rows,
                            &INVOCATION_HOURLY_ROLLUP_TARGETS,
                        )
                        .await?;

                        // The all-time reader can safely read a raw tail only after a
                        // contiguous live prefix. Do not leap over newer retained rows
                        // that happened to receive lower IDs than this archive batch.
                        let prefix_end = unprojected_rows
                            .iter()
                            .map(|row| row.id)
                            .max()
                            .expect("unprojected rows are non-empty");
                        let unprojected_ids = unprojected_rows
                            .iter()
                            .map(|row| row.id)
                            .collect::<Vec<_>>();
                        // Stop at the first uncovered ID instead of counting a million-row prefix
                        // while holding the writer permit when old timestamps have newer IDs.
                        let has_uncovered_prefix: bool = sqlx::query_scalar(
                        "SELECT EXISTS(SELECT 1 FROM codex_invocations WHERE id > ?1 AND id <= ?2
                         AND id NOT IN (SELECT value FROM json_each(?3)))",
                    )
                    .bind(live_rollup_cursor)
                    .bind(prefix_end)
                    .bind(serde_json::to_string(&unprojected_ids)?)
                    .fetch_one(tx.as_mut())
                    .await?;
                        if !has_uncovered_prefix {
                            save_hourly_rollup_live_progress_tx(
                                tx.as_mut(),
                                HOURLY_ROLLUP_DATASET_INVOCATIONS,
                                prefix_end,
                            )
                            .await?;
                        }
                    }
                    upsert_invocation_rollups(tx.as_mut(), &group).await?;
                    // The file fence is released between chunks. Recheck pages and manifest in
                    // the source transaction after any admission wait or concurrent rewrite.
                    store_verified_summary_archive_snapshot_tx(tx.as_mut(), &proof).await?;
                    if chunk_index == 0 {
                        mark_archive_batch_historical_rollups_materialized_tx(
                            tx.as_mut(),
                            spec.dataset,
                            &archive_outcome.file_path,
                        )
                        .await?;
                    }
                    let source_verify_phase = batch_observation.phase(RetentionPhase::SourceVerify);
                    if invocation_archive_source_identity_sha256(
                        tx.as_mut(),
                        InvocationArchiveIdentityDatabase::Main,
                        &ids,
                    )
                    .await?
                        != chunk_identity
                    {
                        bail!("retention task-local archive source changed before conversion");
                    }
                    drop(source_verify_phase);
                    let ids_json = serde_json::to_string(&ids)?;
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
                        mark_prompt_cache_conversation_stats_stale_on_connection(tx.as_mut())
                            .await?;
                    }
                    let delete_phase = batch_observation.phase(RetentionPhase::Delete);
                    delete_rows_by_ids(tx.as_mut(), spec.dataset, &ids).await?;
                    drop(delete_phase);
                    // DELETE triggers persist the affected keys. Statistics are refreshed asynchronously
                    // by the materialization owner after archive publication.
                    mark_retention_archived_hourly_rollup_targets_tx(
                        tx.as_mut(),
                        spec.dataset,
                        &materialized_rows,
                        &[],
                    )
                    .await?;
                    for target in INVOCATION_HOURLY_ROLLUP_TARGETS {
                        mark_hourly_rollup_archive_replayed_tx(
                            tx.as_mut(),
                            target,
                            spec.dataset,
                            &archive_outcome.file_path,
                        )
                        .await?;
                    }
                    if chunk_index == 0 {
                        finalize_invocation_archive_batch_publication_tx(
                            tx.as_mut(),
                            &archive_outcome.file_path,
                        )
                        .await?;
                    }
                    let raw_reference_check_started = Instant::now();
                    let had_raw_reference_candidates = raw_paths.iter().any(Option::is_some);
                    let raw_paths = filter_unreferenced_proxy_raw_paths(
                        tx.as_mut(),
                        &raw_paths,
                        raw_path_fallback_root,
                    )
                    .await?;
                    let raw_reference_check_elapsed =
                        had_raw_reference_candidates.then(|| raw_reference_check_started.elapsed());
                    let commit_started = Instant::now();
                    Ok(RetentionBatchCommit {
                        raw_paths,
                        raw_reference_check_elapsed,
                        commit_started,
                        prompt_cache_keys: archived_prompt_cache_keys,
                    })
                }
                .await;
                let commit = match transaction_result {
                    Ok(commit) => commit,
                    Err(error) => {
                        let cleanup_error = transaction_resources.rollback_and_close().await;
                        drop(transaction_phase);
                        let mut error = error;
                        if let Err(cleanup_error) = cleanup_error {
                            error = error.context(format!(
                                "retention transaction rollback and close failed: {cleanup_error}"
                            ));
                        }
                        return Err(error);
                    }
                };
                let commit_phase = batch_observation.phase(RetentionPhase::Commit);
                let commit_result = sqlx::query("COMMIT")
                    .execute(transaction_resources.connection_mut())
                    .await;
                drop(commit_phase);
                if let Err(error) = commit_result {
                    let cleanup_error = transaction_resources.rollback_and_close().await;
                    drop(transaction_phase);
                    let mut error: anyhow::Error = error.into();
                    if let Err(cleanup_error) = cleanup_error {
                        error = error.context(format!(
                            "retention transaction rollback and close failed after commit failure: {cleanup_error}"
                        ));
                    }
                    return Err(error);
                }
                transaction_resources.mark_transaction_finished();
                prompt_cache_keys.extend(commit.prompt_cache_keys);
                let raw_paths = commit.raw_paths;
                workload::record_processed_count(group.len());
                let breached = retention_record_commit_with_reference_check!(
                    "invocation_archive",
                    admission_mode,
                    group.len(),
                    group
                        .iter()
                        .map(|candidate| {
                            candidate
                                .payload
                                .as_deref()
                                .map_or(256, |payload| payload.len())
                        })
                        .sum(),
                    prepare_elapsed,
                    lock_wait,
                    commit.commit_started.duration_since(execute_started),
                    commit.commit_started.elapsed(),
                    commit.raw_reference_check_elapsed,
                    p1_waiter_count,
                    candidate_remaining_hint,
                );
                rows_archived += group.len();
                committed_rows += group.len();
                batch_observation.committed(group.len(), lock_wait);
                drop(transaction_phase);
                let cleanup_error = transaction_resources.return_to_pool().await;
                if let Err(cleanup_error) = cleanup_error {
                    return Err(anyhow!(
                        "failed to return retention transaction connection to pool: {cleanup_error}"
                    ));
                }
                let cleanup_phase = batch_observation.phase(RetentionPhase::Cleanup);
                if !raw_paths.is_empty() {
                    mark_retention_raw_inventory_reset_intent(pool).await?;
                }
                retention_recovery_record_progress();
                raw_files_removed += delete_proxy_raw_paths(&raw_paths, raw_path_fallback_root)?;
                drop(cleanup_phase);
                if breached {
                    tokio::task::yield_now().await;
                }
                #[cfg(test)]
                batch_admission::pause_after_first_chunk(committed_rows).await;
                chunk_index += 1;
            }
            archive_identity_proof.close().await?;
            if committed_rows > 0 {
                archive_batches += 1;
            }
            batch_observation.complete();
        }
        // Normal task scope is fixed at selection. The next scheduled run reselects live
        // rows; it never resumes this run's file/cursor. Direct fixture helpers may drain.
        if retention_run_remaining_budget().is_some() {
            break;
        }
    }

    Ok((
        rows_archived,
        archive_batches,
        raw_files_removed,
        prompt_cache_keys,
    ))
}

pub(crate) async fn archive_timestamped_dataset(
    pool: &Pool<Sqlite>,
    config: &AppConfig,
    spec: ArchiveTableSpec,
    select_sql: &str,
    cutoff: String,
    dry_run: bool,
) -> Result<(usize, usize, usize)> {
    if dry_run {
        let dry_run_sql = match spec.dataset {
            "forward_proxy_attempts" => {
                r#"
                SELECT strftime('%Y-%m', datetime(occurred_at, '+8 hours')) AS month_key,
                       COUNT(*) AS row_count
                FROM forward_proxy_attempts
                WHERE occurred_at < ?1
                GROUP BY 1
                ORDER BY 1
                "#
            }
            "pool_upstream_request_attempts" => {
                r#"
                SELECT strftime('%Y-%m', occurred_at) AS month_key,
                       COUNT(*) AS row_count
                FROM pool_upstream_request_attempts
                WHERE occurred_at < ?1
                GROUP BY 1
                ORDER BY 1
                "#
            }
            other => bail!("unsupported dry-run archive dataset: {other}"),
        };
        let batch_counts = sqlx::query_as::<_, DryRunBatchCount>(dry_run_sql)
            .bind(&cutoff)
            .fetch_all(pool)
            .await?;
        for batch in &batch_counts {
            info!(
                dataset = spec.dataset,
                month_key = %batch.month_key,
                rows = batch.row_count,
                "retention dry-run planned archive batch"
            );
        }
        return Ok((
            batch_counts
                .iter()
                .map(|batch| batch.row_count as usize)
                .sum(),
            batch_counts.len(),
            0,
        ));
    }

    let mut rows_archived = 0usize;
    let mut archive_batches = 0usize;
    let mut raw_files_removed = 0usize;

    loop {
        if retention_run_budget_expired() {
            break;
        }
        if !archive_batch_can_start() {
            break;
        }
        let candidate_limit = archive_candidate_limit(config);
        let candidates_query = sqlx::query_as::<_, TimestampedArchiveCandidate>(select_sql)
            .bind(&cutoff)
            .bind(candidate_limit as i64)
            .fetch_all(pool);
        let candidates = if let Some(remaining) = retention_run_remaining_budget() {
            match tokio::time::timeout(remaining, candidates_query).await {
                Ok(result) => result?,
                Err(_) => break,
            }
        } else {
            candidates_query.await?
        };

        if candidates.is_empty() {
            break;
        }

        let candidate_remaining_hint = usize::from(candidates.len() >= candidate_limit);
        let mut by_month: BTreeMap<String, Vec<TimestampedArchiveCandidate>> = BTreeMap::new();
        for candidate in candidates {
            if retention_run_budget_expired() {
                break;
            }
            let month_key =
                archive_timestamped_dataset_month_key(spec.dataset, &candidate.timestamp_value)?;
            by_month.entry(month_key).or_default().push(candidate);
        }

        for (month_key, group) in by_month {
            if retention_run_budget_expired() {
                break;
            }
            let candidate_ids = group.iter().map(|row| row.id).collect::<Vec<_>>();
            let row_sizes = archive_source_row_sizes(pool, spec, &candidate_ids).await?;
            let group = select_archive_batch(group, |candidate| {
                row_sizes.get(&candidate.id).copied().unwrap_or(512)
            });
            let mut batch_observation =
                batch_plan::BatchObservation::begin(spec.dataset, &month_key, group.len());
            let prepare_started = Instant::now();
            let ids = group
                .iter()
                .map(|candidate| candidate.id)
                .collect::<Vec<_>>();
            if ids.is_empty() || retention_run_budget_expired() {
                break;
            }
            let recreated_pool_upstream_month_archive = if spec.dataset
                == "pool_upstream_request_attempts"
            {
                let archive_file_path = archive_batch_file_path(config, spec.dataset, &month_key)?
                    .to_string_lossy()
                    .to_string();
                pool_upstream_month_archive_reappeared_after_cleanup(pool, &archive_file_path)
                    .await?
            } else {
                false
            };
            let Some(mut source_connection) =
                acquire_retention_pool_connection(pool, "timestamped_archive_identity").await?
            else {
                return Ok((rows_archived, archive_batches, raw_files_removed));
            };
            let source_verify_phase = batch_observation.phase(RetentionPhase::SourceVerify);
            let source_identity_started = Instant::now();
            let source_identity_query =
                archive_table_source_identity_sha256(&mut source_connection, spec, "main", &ids);
            let source_identity = if let Some(remaining) = retention_run_remaining_budget() {
                match tokio::time::timeout(remaining, source_identity_query).await {
                    Ok(result) => result?,
                    Err(_) => {
                        batch_observation
                            .source_identity_finished(source_identity_started.elapsed());
                        break;
                    }
                }
            } else {
                source_identity_query.await?
            };
            batch_observation.source_identity_finished(source_identity_started.elapsed());
            drop(source_connection);
            drop(source_verify_phase);
            let prepare_phase = batch_observation.phase(RetentionPhase::Prepare);
            let archive_writer_started = Instant::now();
            let archive_future =
                archive_rows_into_month_batch(pool, config, spec, &month_key, &ids);
            let archive_result = if let Some(remaining) = retention_run_remaining_budget() {
                match tokio::time::timeout(remaining, archive_future).await {
                    Ok(result) => result,
                    Err(_) => {
                        batch_observation.archive_writer_finished(archive_writer_started.elapsed());
                        break;
                    }
                }
            } else {
                archive_future.await
            };
            batch_observation.archive_writer_finished(archive_writer_started.elapsed());
            let Some(mut archive_outcome) = retention_prepared_batch_or_deferred(archive_result)?
            else {
                return Ok((rows_archived, archive_batches, raw_files_removed));
            };
            if archive_outcome.source_identity_sha256.as_deref() != Some(source_identity.as_str()) {
                bail!("timestamped task-local archive source identity verification failed");
            }
            if spec.dataset == "pool_upstream_request_attempts" {
                set_archive_batch_coverage_from_local_rows(
                    &mut archive_outcome,
                    group
                        .iter()
                        .map(|candidate| candidate.timestamp_value.as_str()),
                    Some(config.pool_upstream_request_attempts_archive_ttl_days),
                )?;
                if recreated_pool_upstream_month_archive
                    && archive_outcome.row_count == ids.len() as i64
                {
                    archive_outcome.archive_expires_at =
                        Some(shanghai_archive_expiry_from_reference_timestamp(
                            &format_utc_iso(Utc::now()),
                            config.pool_upstream_request_attempts_archive_ttl_days,
                        )?);
                }
            } else {
                set_archive_batch_coverage_from_utc_rows(
                    &mut archive_outcome,
                    group
                        .iter()
                        .map(|candidate| candidate.timestamp_value.as_str()),
                )?;
            }
            let prepare_elapsed = prepare_started.elapsed();
            batch_observation.file_prepared(prepare_elapsed);
            drop(prepare_phase);
            let mut archive_identity_proof =
                ArchiveSourceIdentityProof::load(Path::new(&archive_outcome.file_path), spec)
                    .await?;
            #[cfg(test)]
            batch_admission::pause_after_archive_artifact_verified().await;
            let archive_file_contains_only_new_rows = archive_outcome.row_count == ids.len() as i64;
            let mut committed_rows = 0;
            let mut pending_group = group;
            while !pending_group.is_empty() {
                let group = take_retention_batch(
                    &mut pending_group,
                    config,
                    "timestamped_archive",
                    |candidate| row_sizes.get(&candidate.id).copied().unwrap_or(512),
                );
                let ids = group
                    .iter()
                    .map(|candidate| candidate.id)
                    .collect::<Vec<_>>();
                let pool_attempt_raw_paths = if spec.dataset == "pool_upstream_request_attempts" {
                    sqlx::query_scalar::<_, Option<String>>("SELECT response_raw_path FROM pool_upstream_request_attempts WHERE id IN (SELECT value FROM json_each(?1))")
                        .bind(serde_json::to_string(&ids)?).fetch_all(pool).await?
                } else {
                    Vec::new()
                };
                let materialized_forward_proxy_rows = if spec.dataset == "forward_proxy_attempts" {
                    group
                        .iter()
                        .map(|candidate| ForwardProxyAttemptHourlySourceRecord {
                            id: candidate.id,
                            proxy_key: String::new(),
                            occurred_at: candidate.timestamp_value.clone(),
                            is_success: 0,
                            latency_ms: None,
                        })
                        .collect::<Vec<_>>()
                } else {
                    Vec::new()
                };
                let chunk_identity = archive_identity_proof
                    .identity(&ids)
                    .await
                    .context("read task-local timestamped archive chunk identity")?;
                let admission_wait_phase = batch_observation.phase(RetentionPhase::AdmissionWait);
                let Some((source_connection, admission, archive_lock)) =
                    acquire_retention_batch_write_connection_with_archive_fence_observed(
                        pool,
                        "timestamped_archive",
                        Path::new(&archive_outcome.file_path),
                        Some(&archive_outcome.sha256),
                        || Some(batch_observation.phase(RetentionPhase::FileVerify)),
                    )
                    .await?
                else {
                    drop(admission_wait_phase);
                    return Ok((rows_archived, archive_batches, raw_files_removed));
                };
                let admission_mode = admission.admission_mode();
                let lock_wait = admission.lock_wait();
                let p1_waiter_count = admission.p1_waiter_count();
                let mut transaction_resources =
                    RetentionTransactionResources::new(source_connection, admission, archive_lock);
                drop(admission_wait_phase);
                let transaction_phase = batch_observation.phase(RetentionPhase::Transaction);
                let execute_started = Instant::now();
                transaction_resources.mark_transaction_started();
                let begin_result = sqlx::query("BEGIN")
                    .execute(transaction_resources.connection_mut())
                    .await;
                if let Err(error) = begin_result {
                    let cleanup_error = transaction_resources.rollback_and_close().await;
                    drop(transaction_phase);
                    let mut error: anyhow::Error = error.into();
                    if let Err(cleanup_error) = cleanup_error {
                        error = error.context(format!(
                            "failed to rollback and close source connection after begin failure: {cleanup_error}"
                        ));
                    }
                    return Err(error);
                }
                #[cfg(test)]
                batch_admission::pause_after_transaction_begin().await;
                let transaction_result: Result<Option<RetentionBatchCommit>> = async {
                    let mut tx = RetentionSqliteTransaction(transaction_resources.connection_mut());
                let cleanup_state = sqlx::query_scalar::<_, Option<String>>(
                "SELECT cleanup_state FROM archive_batches WHERE dataset = ?1 AND month_key = ?2 AND file_path = ?3",
            )
            .bind(spec.dataset)
            .bind(&archive_outcome.month_key)
            .bind(&archive_outcome.file_path)
            .fetch_optional(tx.as_mut())
            .await?
            .flatten();
                if cleanup_state
                    .as_deref()
                    .is_some_and(|state| state != ARCHIVE_CLEANUP_STATE_ACTIVE)
                {
                    return Ok(None);
                }
                let source_verify_phase = batch_observation.phase(RetentionPhase::SourceVerify);
                if archive_table_source_identity_sha256(tx.as_mut(), spec, "main", &ids).await?
                    != chunk_identity
                {
                    bail!("timestamped task-local archive source changed before conversion");
                }
                drop(source_verify_phase);
                upsert_archive_batch_manifest(tx.as_mut(), &archive_outcome).await?;
                if spec.dataset == "pool_upstream_request_attempts" {
                    let archive_batch_id = load_archive_batch_id_for_file_tx(
                        tx.as_mut(),
                        spec.dataset,
                        &archive_outcome.month_key,
                        &archive_outcome.file_path,
                    )
                    .await?;
                    let node_health_archive_already_replayed = hourly_rollup_archive_replayed_tx(
                        tx.as_mut(),
                        POOL_UPSTREAM_NODE_HEALTH_ARCHIVE_REPLAY_TARGET,
                        spec.dataset,
                        &archive_outcome.file_path,
                    )
                    .await?;
                    let node_health_hourly_archive_already_replayed =
                        hourly_rollup_archive_replayed_tx(
                            tx.as_mut(),
                            POOL_UPSTREAM_NODE_HEALTH_HOURLY_ARCHIVE_REPLAY_TARGET,
                            spec.dataset,
                            &archive_outcome.file_path,
                        )
                        .await?;
                    cache_pool_node_health_batch_chunk_tx(
                        tx.as_mut(),
                        archive_batch_id,
                        &archive_outcome.file_path,
                        &ids,
                        committed_rows == 0,
                    )
                    .await?;
                    if archive_file_contains_only_new_rows
                        || node_health_hourly_archive_already_replayed
                    {
                        mark_hourly_rollup_archive_replayed_tx(
                            tx.as_mut(),
                            POOL_UPSTREAM_NODE_HEALTH_HOURLY_ARCHIVE_REPLAY_TARGET,
                            spec.dataset,
                            &archive_outcome.file_path,
                        )
                        .await?;
                    } else {
                        sqlx::query(
                            r#"
                        DELETE FROM hourly_rollup_archive_replay
                        WHERE target = ?1
                          AND dataset = ?2
                          AND file_path = ?3
                        "#,
                        )
                        .bind(POOL_UPSTREAM_NODE_HEALTH_HOURLY_ARCHIVE_REPLAY_TARGET)
                        .bind(spec.dataset)
                        .bind(&archive_outcome.file_path)
                        .execute(tx.as_mut())
                        .await?;
                    }
                    if archive_file_contains_only_new_rows || node_health_archive_already_replayed {
                        mark_hourly_rollup_archive_replayed_tx(
                            tx.as_mut(),
                            POOL_UPSTREAM_NODE_HEALTH_ARCHIVE_REPLAY_TARGET,
                            spec.dataset,
                            &archive_outcome.file_path,
                        )
                        .await?;
                        mark_archive_batch_historical_rollups_materialized_tx(
                            tx.as_mut(),
                            spec.dataset,
                            &archive_outcome.file_path,
                        )
                        .await?;
                    } else {
                        sqlx::query(
                            r#"
                        DELETE FROM hourly_rollup_archive_replay
                        WHERE target = ?1
                          AND dataset = ?2
                          AND file_path = ?3
                        "#,
                        )
                        .bind(POOL_UPSTREAM_NODE_HEALTH_ARCHIVE_REPLAY_TARGET)
                        .bind(spec.dataset)
                        .bind(&archive_outcome.file_path)
                        .execute(tx.as_mut())
                        .await?;
                        sqlx::query(
                            r#"
                        UPDATE archive_batches
                        SET historical_rollups_materialized_at = NULL
                        WHERE dataset = ?1
                          AND file_path = ?2
                        "#,
                        )
                        .bind(spec.dataset)
                        .bind(&archive_outcome.file_path)
                        .execute(tx.as_mut())
                        .await?;
                    }
                } else {
                    mark_archive_batch_historical_rollups_materialized_tx(
                        tx.as_mut(),
                        spec.dataset,
                        &archive_outcome.file_path,
                    )
                    .await?;
                }
                let delete_phase = batch_observation.phase(RetentionPhase::Delete);
                delete_rows_by_ids(tx.as_mut(), spec.dataset, &ids).await?;
                drop(delete_phase);
                mark_retention_archived_hourly_rollup_targets_tx(
                    tx.as_mut(),
                    spec.dataset,
                    &[],
                    &materialized_forward_proxy_rows,
                )
                .await?;
                let (raw_paths, raw_reference_check_elapsed) = if spec.dataset
                    == "pool_upstream_request_attempts"
                {
                    let raw_reference_check_started = Instant::now();
                    let had_raw_reference_candidates =
                        pool_attempt_raw_paths.iter().any(Option::is_some);
                    let raw_paths = filter_unreferenced_proxy_raw_paths(
                        tx.as_mut(),
                        &pool_attempt_raw_paths,
                        config.database_path.parent(),
                    )
                    .await?;
                    (
                        raw_paths,
                        had_raw_reference_candidates.then(|| raw_reference_check_started.elapsed()),
                    )
                } else {
                    (Vec::new(), None)
                };
                let commit_started = Instant::now();
                Ok(Some(RetentionBatchCommit {
                    raw_paths,
                    raw_reference_check_elapsed,
                    commit_started,
                    prompt_cache_keys: Vec::new(),
                }))
                }
                .await;
                let commit = match transaction_result {
                    Ok(commit) => commit,
                    Err(error) => {
                        let cleanup_error = transaction_resources.rollback_and_close().await;
                        drop(transaction_phase);
                        let mut error = error;
                        if let Err(cleanup_error) = cleanup_error {
                            error = error.context(format!(
                                "retention transaction rollback and close failed: {cleanup_error}"
                            ));
                        }
                        return Err(error);
                    }
                };
                let commit_phase = batch_observation.phase(RetentionPhase::Commit);
                let commit_result = sqlx::query("COMMIT")
                    .execute(transaction_resources.connection_mut())
                    .await;
                drop(commit_phase);
                if let Err(error) = commit_result {
                    let cleanup_error = transaction_resources.rollback_and_close().await;
                    drop(transaction_phase);
                    let mut error: anyhow::Error = error.into();
                    if let Err(cleanup_error) = cleanup_error {
                        error = error.context(format!(
                            "retention transaction rollback and close failed after commit failure: {cleanup_error}"
                        ));
                    }
                    return Err(error);
                }
                transaction_resources.mark_transaction_finished();
                let Some(commit) = commit else {
                    drop(transaction_phase);
                    let cleanup_error = transaction_resources.return_to_pool().await;
                    if let Err(cleanup_error) = cleanup_error {
                        return Err(anyhow!(
                            "failed to return retention transaction connection to pool: {cleanup_error}"
                        ));
                    }
                    return Ok((rows_archived, archive_batches, raw_files_removed));
                };
                let raw_paths = commit.raw_paths;
                let breached = retention_record_commit_with_reference_check!(
                    "timestamped_archive",
                    admission_mode,
                    group.len(),
                    group.len().saturating_mul(256),
                    prepare_elapsed,
                    lock_wait,
                    commit.commit_started.duration_since(execute_started),
                    commit.commit_started.elapsed(),
                    commit.raw_reference_check_elapsed,
                    p1_waiter_count,
                    candidate_remaining_hint,
                );
                batch_observation.committed(group.len(), lock_wait);
                drop(transaction_phase);
                let cleanup_error = transaction_resources.return_to_pool().await;
                if let Err(cleanup_error) = cleanup_error {
                    return Err(anyhow!(
                        "failed to return retention transaction connection to pool: {cleanup_error}"
                    ));
                }
                let cleanup_phase = batch_observation.phase(RetentionPhase::Cleanup);
                if spec.dataset == "pool_upstream_request_attempts" {
                    if !raw_paths.is_empty() {
                        mark_retention_raw_inventory_reset_intent(pool).await?;
                    }
                    rows_archived += group.len();
                    committed_rows += group.len();
                    raw_files_removed +=
                        delete_proxy_raw_paths(&raw_paths, config.database_path.parent())?;
                } else {
                    rows_archived += group.len();
                    committed_rows += group.len();
                }
                drop(cleanup_phase);
                if breached {
                    tokio::task::yield_now().await;
                }
                #[cfg(test)]
                batch_admission::pause_after_first_chunk(committed_rows).await;
            }
            archive_identity_proof.close().await?;
            if committed_rows > 0 {
                archive_batches += 1;
            }
            batch_observation.complete();
        }
        if retention_run_remaining_budget().is_some() {
            break;
        }
    }

    Ok((rows_archived, archive_batches, raw_files_removed))
}
