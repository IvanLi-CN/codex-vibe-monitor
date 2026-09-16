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
        .bind(&cutoff)
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
        return Ok((
            candidates.len(),
            by_group.len(),
            count_existing_proxy_raw_paths(&raw_paths, raw_path_fallback_root),
        ));
    }

    let mut rows_archived = 0usize;
    let mut archive_batches = 0usize;
    let mut raw_files_removed = 0usize;

    loop {
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
        let mut by_group: BTreeMap<String, Vec<InvocationArchiveCandidate>> = BTreeMap::new();
        for candidate in candidates {
            let group_key = invocation_archive_group_key(config, &candidate.occurred_at)?;
            by_group.entry(group_key).or_default().push(candidate);
        }

        for (group_key, group) in by_group {
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
                        archive_rows_into_month_batch(pool, config, spec, &group_key, &ids).await
                    }
                    ArchiveBatchLayout::SegmentV1 => {
                        archive_rows_into_segment_batch(pool, config, spec, &group_key, &ids).await
                    }
                },
            )?
            else {
                return Ok((rows_archived, archive_batches, raw_files_removed));
            };
            set_archive_batch_coverage_from_local_rows(
                &mut archive_outcome,
                group.iter().map(|candidate| candidate.occurred_at.as_str()),
                None,
            )?;
            archive_outcome.archive_expires_at =
                Some(shanghai_archive_expiry_from_reference_timestamp(
                    &format_utc_iso(Utc::now()),
                    config.invocation_archive_ttl_days,
                )?);
            archive_outcome.summary_source_kind = SUMMARY_ARCHIVE_SOURCE_KIND_AUTHORITATIVE;
            let prepare_elapsed = prepare_started.elapsed();
            let Some(admission) = acquire_retention_write_admission("invocation_archive").await
            else {
                return Ok((rows_archived, archive_batches, raw_files_removed));
            };
            let execute_started = Instant::now();
            let mut tx = pool.begin().await?;
            // P2 normally advances this cursor before retention. Rows beyond it would be
            // deleted before the regular replay can observe them, so materialize just those
            // rows in this same archive transaction before claiming the archive is covered.
            let live_rollup_cursor =
                load_hourly_rollup_live_progress_tx(tx.as_mut(), HOURLY_ROLLUP_DATASET_INVOCATIONS)
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
                let prefix_row_count = sqlx::query_scalar::<_, i64>(
                    "SELECT COUNT(*) FROM codex_invocations WHERE id > ?1 AND id <= ?2",
                )
                .bind(live_rollup_cursor)
                .bind(prefix_end)
                .fetch_one(tx.as_mut())
                .await?;
                if prefix_row_count == unprojected_rows.len() as i64 {
                    save_hourly_rollup_live_progress_tx(
                        tx.as_mut(),
                        HOURLY_ROLLUP_DATASET_INVOCATIONS,
                        prefix_end,
                    )
                    .await?;
                }
            }
            upsert_invocation_rollups(tx.as_mut(), &group).await?;
            stage_invocation_archive_batch_manifest(tx.as_mut(), &archive_outcome).await?;
            mark_archive_batch_historical_rollups_materialized_tx(
                tx.as_mut(),
                spec.dataset,
                &archive_outcome.file_path,
            )
            .await?;
            // Keep a normalized, raw-free Summary source page in the same transaction as the
            // authoritative archive publication. Cleanup can therefore retire the file only
            // after this exact page and its manifest identity are durable.
            let snapshot_archive_batch_id = load_archive_batch_id_for_file_tx(
                tx.as_mut(),
                spec.dataset,
                &archive_outcome.month_key,
                &archive_outcome.file_path,
            )
            .await?;
            let invoke_ids_by_row_id = group
                .iter()
                .map(|candidate| (candidate.id, candidate.invoke_id.clone()))
                .collect::<HashMap<_, _>>();
            let snapshot_payload = encode_summary_archive_snapshot_v2_payload(
                &materialized_rows,
                &invoke_ids_by_row_id,
            )?;
            let snapshot_page = SummaryArchiveSnapshotPage {
                archive_batch_id: snapshot_archive_batch_id,
                manifest_sha256: archive_outcome.sha256.clone(),
                page_index: 0,
                coverage_start: archive_outcome.coverage_start_at.clone().ok_or_else(|| {
                    anyhow!("authoritative archive is missing Snapshot start coverage")
                })?,
                coverage_end: archive_outcome.coverage_end_at.clone().ok_or_else(|| {
                    anyhow!("authoritative archive is missing Snapshot end coverage")
                })?,
                row_count: u32::try_from(materialized_rows.len())
                    .context("Summary Archive Snapshot row count overflow")?,
                payload: snapshot_payload,
            };
            store_summary_archive_snapshot_page_v2_tx(tx.as_mut(), &snapshot_page).await?;
            // The page is written as part of the authoritative archive transaction.  Mark the
            // V2 proof in that same transaction so cleanup can never observe a page without its
            // semantic authority.
            if !summary_archive_snapshot_has_proof_tx(
                tx.as_mut(),
                snapshot_archive_batch_id,
                &archive_outcome.sha256,
            )
            .await?
            {
                bail!("retention Summary Snapshot V2 semantic proof validation failed");
            }
            store_summary_archive_snapshot_v2_final_proof_tx(
                tx.as_mut(),
                snapshot_archive_batch_id,
                &archive_outcome.sha256,
            )
            .await?;
            delete_rows_by_ids(tx.as_mut(), spec.dataset, &ids).await?;
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
            finalize_invocation_archive_batch_publication_tx(
                tx.as_mut(),
                &archive_outcome.file_path,
            )
            .await?;
            let commit_started = Instant::now();
            tx.commit().await?;
            retention_record_commit!(
                "invocation_archive",
                admission.admission_mode(),
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
                admission.lock_wait(),
                commit_started.duration_since(execute_started),
                commit_started.elapsed(),
                admission.p1_waiter_count,
                candidate_remaining_hint,
            );
            drop(admission);
            rows_archived += group.len();
            archive_batches += 1;
            let raw_paths = filter_unreferenced_proxy_raw_paths(pool, &raw_paths).await?;
            raw_files_removed += delete_proxy_raw_paths(&raw_paths, raw_path_fallback_root)?;
        }
    }

    Ok((rows_archived, archive_batches, raw_files_removed))
}

pub(crate) async fn archive_timestamped_dataset(
    pool: &Pool<Sqlite>,
    config: &AppConfig,
    spec: ArchiveTableSpec,
    select_sql: &str,
    cutoff: String,
    dry_run: bool,
) -> Result<(usize, usize)> {
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
        ));
    }

    let mut rows_archived = 0usize;
    let mut archive_batches = 0usize;

    loop {
        let candidate_limit = retention_candidate_limit(config, "timestamped_archive");
        let candidates = sqlx::query_as::<_, TimestampedArchiveCandidate>(select_sql)
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
            let month_key =
                archive_timestamped_dataset_month_key(spec.dataset, &candidate.timestamp_value)?;
            by_month.entry(month_key).or_default().push(candidate);
        }

        for (month_key, group) in by_month {
            let group = take_retention_micro_batch(group, |_| 256);
            let prepare_started = Instant::now();
            let ids = group
                .iter()
                .map(|candidate| candidate.id)
                .collect::<Vec<_>>();
            let pool_attempt_raw_paths = if spec.dataset == "pool_upstream_request_attempts" {
                let placeholders = std::iter::repeat_n("?", ids.len())
                    .collect::<Vec<_>>()
                    .join(",");
                let query = format!(
                    "SELECT response_raw_path FROM pool_upstream_request_attempts WHERE id IN ({placeholders})"
                );
                let mut query_builder = sqlx::query_scalar::<_, Option<String>>(&query);
                for id in &ids {
                    query_builder = query_builder.bind(id);
                }
                query_builder
                    .fetch_all(pool)
                    .await?
                    .into_iter()
                    .collect::<Vec<_>>()
            } else {
                Vec::new()
            };
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
            let Some(mut archive_outcome) = retention_prepared_batch_or_deferred(
                archive_rows_into_month_batch(pool, config, spec, &month_key, &ids).await,
            )?
            else {
                return Ok((rows_archived, archive_batches));
            };
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
            let Some(admission) = acquire_retention_write_admission("timestamped_archive").await
            else {
                return Ok((rows_archived, archive_batches));
            };
            let execute_started = Instant::now();
            let mut tx = pool.begin().await?;
            upsert_archive_batch_manifest(tx.as_mut(), &archive_outcome).await?;
            if spec.dataset == "pool_upstream_request_attempts" {
                let archive_batch_id = load_archive_batch_id_for_file_tx(
                    tx.as_mut(),
                    spec.dataset,
                    &archive_outcome.month_key,
                    &archive_outcome.file_path,
                )
                .await?;
                let archive_file_contains_only_new_rows =
                    archive_outcome.row_count == ids.len() as i64;
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
                cache_pool_upstream_node_health_archive_rows_from_live_ids_tx(
                    tx.as_mut(),
                    &archive_outcome.file_path,
                    &ids,
                )
                .await?;
                refresh_pool_upstream_node_health_hourly_archive_rows_from_cache_tx(
                    tx.as_mut(),
                    archive_batch_id,
                    &archive_outcome.file_path,
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
            delete_rows_by_ids(tx.as_mut(), spec.dataset, &ids).await?;
            mark_retention_archived_hourly_rollup_targets_tx(
                tx.as_mut(),
                spec.dataset,
                &[],
                &materialized_forward_proxy_rows,
            )
            .await?;
            let commit_started = Instant::now();
            tx.commit().await?;
            retention_record_commit!(
                "timestamped_archive",
                admission.admission_mode(),
                group.len(),
                group.len().saturating_mul(256),
                prepare_elapsed,
                admission.lock_wait(),
                commit_started.duration_since(execute_started),
                commit_started.elapsed(),
                admission.p1_waiter_count,
                candidate_remaining_hint,
            );
            drop(admission);
            rows_archived += group.len();
            archive_batches += 1;
            if spec.dataset == "pool_upstream_request_attempts" {
                let raw_paths =
                    filter_unreferenced_proxy_raw_paths(pool, &pool_attempt_raw_paths).await?;
                let _ = delete_proxy_raw_paths(&raw_paths, config.database_path.parent())?;
            }
        }
    }

    Ok((rows_archived, archive_batches))
}

pub(crate) fn archive_timestamped_dataset_month_key(
    dataset: &str,
    timestamp_value: &str,
) -> Result<String> {
    match dataset {
        "pool_upstream_request_attempts" => shanghai_month_key_from_local_naive(timestamp_value),
        _ => shanghai_month_key_from_utc_naive(timestamp_value),
    }
}

pub(crate) fn set_archive_batch_coverage_from_local_rows<'a>(
    batch: &mut ArchiveBatchOutcome,
    rows: impl Iterator<Item = &'a str>,
    archive_ttl_days: Option<u64>,
) -> Result<()> {
    let values = rows.collect::<Vec<_>>();
    if values.is_empty() {
        return Ok(());
    }
    let mut sorted = values.into_iter().map(str::to_string).collect::<Vec<_>>();
    sorted.sort();
    batch.coverage_start_at = sorted.first().cloned();
    batch.coverage_end_at = sorted.last().cloned();
    batch.archive_expires_at = match (batch.coverage_end_at.as_deref(), archive_ttl_days) {
        (Some(coverage_end_at), Some(ttl_days)) => Some(
            shanghai_archive_expiry_from_local_timestamp(coverage_end_at, ttl_days)?,
        ),
        _ => None,
    };
    Ok(())
}

pub(crate) async fn pool_upstream_month_archive_reappeared_after_cleanup(
    pool: &Pool<Sqlite>,
    archive_file_path: &str,
) -> Result<bool> {
    let existing_manifest_rows: i64 = sqlx::query_scalar(
        r#"
        SELECT COUNT(*)
        FROM archive_batches
        WHERE dataset = 'pool_upstream_request_attempts'
          AND file_path = ?1
        "#,
    )
    .bind(archive_file_path)
    .fetch_one(pool)
    .await?;
    if existing_manifest_rows > 0 {
        return Ok(false);
    }

    let existing_hourly_rows: i64 = sqlx::query_scalar(
        r#"
        SELECT COUNT(*)
        FROM pool_upstream_node_health_hourly_archive
        WHERE archive_file_path = ?1
        "#,
    )
    .bind(archive_file_path)
    .fetch_one(pool)
    .await?;

    Ok(existing_hourly_rows > 0)
}

pub(crate) fn set_archive_batch_coverage_from_utc_rows<'a>(
    batch: &mut ArchiveBatchOutcome,
    rows: impl Iterator<Item = &'a str>,
) -> Result<()> {
    let values = rows.collect::<Vec<_>>();
    if values.is_empty() {
        return Ok(());
    }
    let mut sorted = values.into_iter().map(str::to_string).collect::<Vec<_>>();
    sorted.sort();
    batch.coverage_start_at = sorted.first().cloned();
    batch.coverage_end_at = sorted.last().cloned();
    batch.archive_expires_at = None;
    Ok(())
}

pub(crate) fn shanghai_archive_expiry_from_local_timestamp(
    value: &str,
    archive_ttl_days: u64,
) -> Result<String> {
    let local = parse_shanghai_local_naive(value)?;
    shanghai_archive_expiry_from_local_naive(local, archive_ttl_days)
}

pub(crate) fn shanghai_archive_expiry_from_reference_timestamp(
    value: &str,
    archive_ttl_days: u64,
) -> Result<String> {
    let local = match parse_to_utc_datetime(value) {
        Some(value) => value.with_timezone(&Shanghai).naive_local(),
        None => parse_shanghai_local_naive(value)?,
    };
    shanghai_archive_expiry_from_local_naive(local, archive_ttl_days)
}

pub(crate) fn shanghai_archive_expiry_from_local_naive(
    local: NaiveDateTime,
    archive_ttl_days: u64,
) -> Result<String> {
    let expiry = start_of_local_day(local_naive_to_utc(local, Shanghai), Shanghai)
        + ChronoDuration::days(archive_ttl_days as i64 + 1);
    Ok(format_naive(expiry.with_timezone(&Shanghai).naive_local()))
}

#[cfg(test)]
mod retention_write_budget_tests {
    use super::*;

    #[test]
    fn retention_write_budget_adapts_without_exceeding_hard_bounds() {
        let mut budget = RetentionWriteBudget::default();
        assert_eq!(budget.candidate_limit(1_000), RETENTION_WRITE_INITIAL_ROWS);

        assert!(budget.observe_commit(4, 4 * 256, Duration::from_millis(251)));
        assert_eq!(budget.candidate_limit(1_000), 2);

        assert!(budget.observe_commit(1, RETENTION_WRITE_MAX_BYTES + 1, Duration::ZERO));
        assert_eq!(budget.candidate_limit(1_000), 1);

        for _ in 0..100 {
            assert!(!budget.observe_commit(1, 256, Duration::from_millis(1)));
        }
        assert!(budget.candidate_limit(1_000) <= RETENTION_WRITE_MAX_ROWS);
    }

    #[test]
    fn retention_health_records_a_budget_breach_for_the_next_production_candidate() {
        const OPERATION: &str = "retention_test_adaptive_budget";
        let mut health = RetentionWriteHealthState::default();
        assert_eq!(
            retention_adaptive_candidate_limit_from_state(&mut health, 64, OPERATION),
            4
        );
        assert!(observe_retention_write_commit(
            &mut health,
            OPERATION,
            "normal",
            4,
            4 * 256,
            Duration::ZERO,
            Duration::ZERO,
            Duration::from_millis(251),
            Duration::ZERO,
            0,
            0,
        ));
        assert_eq!(
            retention_adaptive_candidate_limit_from_state(&mut health, 64, OPERATION),
            2
        );
        assert_eq!(health.snapshot.state, "degraded");
    }

    #[test]
    fn retention_micro_batch_keeps_a_single_oversized_row_losslessly() {
        let selected =
            take_retention_micro_batch(vec![2 * RETENTION_WRITE_MAX_BYTES, 128], |value| *value);
        assert_eq!(selected, vec![2 * RETENTION_WRITE_MAX_BYTES]);
    }

    #[test]
    fn system_task_run_retention_only_backs_off_for_pressure_cooldown() {
        assert!(
            system_task_run_retention_admission_requires_pressure_backoff(Some(
                crate::db_pressure::DbPressureDenyReason::PressureCooldown { remaining_ms: 1 }
            ))
        );
        assert!(
            !system_task_run_retention_admission_requires_pressure_backoff(Some(
                crate::db_pressure::DbPressureDenyReason::BackgroundBusy
            ))
        );
        assert!(!system_task_run_retention_admission_requires_pressure_backoff(None));
    }
}

#[derive(Debug, FromRow)]
pub(crate) struct ArchiveExpiryBackfillCandidate {
    pub(crate) id: i64,
    pub(crate) coverage_end_at: String,
}
