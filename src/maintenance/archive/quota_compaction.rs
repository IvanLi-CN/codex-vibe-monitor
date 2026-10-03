use super::*;

pub(crate) async fn compact_old_quota_snapshots(
    pool: &Pool<Sqlite>,
    config: &AppConfig,
    dry_run: bool,
) -> Result<(usize, usize)> {
    let cutoff = shanghai_utc_cutoff_string(config.quota_snapshot_full_days);
    let spec = archive_table_spec("codex_quota_snapshots");

    if dry_run {
        let batch_counts = sqlx::query_as::<_, DryRunBatchCount>(
            r#"
            WITH ranked AS (
                SELECT
                    captured_at,
                    ROW_NUMBER() OVER (
                        PARTITION BY strftime('%Y-%m-%d', datetime(captured_at, '+8 hours'))
                        ORDER BY captured_at DESC, id DESC
                    ) AS row_num
                FROM codex_quota_snapshots
                WHERE captured_at < ?1
            )
            SELECT strftime('%Y-%m', datetime(captured_at, '+8 hours')) AS month_key,
                   COUNT(*) AS row_count
            FROM ranked
            WHERE row_num > 1
            GROUP BY 1
            ORDER BY 1
            "#,
        )
        .bind(&cutoff)
        .fetch_all(pool)
        .await?;
        for batch in &batch_counts {
            info!(
                dataset = spec.dataset,
                month_key = %batch.month_key,
                rows = batch.row_count,
                "retention dry-run planned quota compaction batch"
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
        if !super::super::retention::batch_plan::archive_batch_can_start() {
            break;
        }
        let candidate_limit = super::super::retention::batch_plan::archive_candidate_limit(config);
        let candidates = sqlx::query_as::<_, TimestampedArchiveCandidate>(
            r#"
            WITH ranked AS (
                SELECT
                    id,
                    captured_at AS timestamp_value,
                    ROW_NUMBER() OVER (
                        PARTITION BY strftime('%Y-%m-%d', datetime(captured_at, '+8 hours'))
                        ORDER BY captured_at DESC, id DESC
                    ) AS row_num
                FROM codex_quota_snapshots
                WHERE captured_at < ?1
            )
            SELECT id, timestamp_value
            FROM ranked
            WHERE row_num > 1
            ORDER BY timestamp_value ASC, id ASC
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
        let mut by_month: BTreeMap<String, Vec<TimestampedArchiveCandidate>> = BTreeMap::new();
        for candidate in candidates {
            let month_key = shanghai_month_key_from_utc_naive(&candidate.timestamp_value)?;
            by_month.entry(month_key).or_default().push(candidate);
        }

        for (month_key, group) in by_month {
            let candidate_ids = group.iter().map(|row| row.id).collect::<Vec<_>>();
            let sizes =
                super::super::retention::archive_source_row_sizes(pool, spec, &candidate_ids)
                    .await?;
            let group = super::super::retention::batch_plan::select_archive_batch(group, |row| {
                sizes.get(&row.id).copied().unwrap_or(512)
            });
            let mut observation = super::super::retention::batch_plan::BatchObservation::begin(
                spec.dataset,
                &month_key,
                group.len(),
            );
            let prepare_started = Instant::now();
            let ids = group
                .iter()
                .map(|candidate| candidate.id)
                .collect::<Vec<_>>();
            let mut connection = pool.acquire().await?;
            let identity = super::super::retention::archive_table_source_identity_sha256(
                &mut connection,
                spec,
                "main",
                &ids,
            )
            .await?;
            let mut chunk_identities = Vec::new();
            for chunk in ids.chunks(super::super::retention::RETENTION_WRITE_MAX_ROWS) {
                chunk_identities.push(
                    super::super::retention::archive_table_source_identity_sha256(
                        &mut connection,
                        spec,
                        "main",
                        chunk,
                    )
                    .await?,
                );
            }
            drop(connection);
            let Some(mut archive_outcome) =
                super::super::retention::retention_prepared_batch_or_deferred(
                    archive_rows_into_month_batch(pool, config, spec, &month_key, &ids).await,
                )?
            else {
                return Ok((rows_archived, archive_batches));
            };
            if archive_outcome.source_identity_sha256.as_deref() != Some(identity.as_str()) {
                bail!("quota task-local archive identity changed during preparation");
            }
            set_archive_batch_coverage_from_utc_rows(
                &mut archive_outcome,
                group
                    .iter()
                    .map(|candidate| candidate.timestamp_value.as_str()),
            )?;
            let prepare_elapsed = prepare_started.elapsed();
            observation.file_prepared(prepare_elapsed);
            let _archive_lock = super::super::retention::retention_archive_file_lock(Path::new(
                &archive_outcome.file_path,
            ))?;
            let actual_sha256 = match sha256_hex_file(Path::new(&archive_outcome.file_path)) {
                Ok(value) => value,
                Err(_) => return Ok((rows_archived, archive_batches)),
            };
            if actual_sha256 != archive_outcome.sha256 {
                return Ok((rows_archived, archive_batches));
            }
            let mut committed_rows = 0;
            for (group, identity) in group
                .chunks(super::super::retention::RETENTION_WRITE_MAX_ROWS)
                .zip(chunk_identities)
            {
                let ids = group.iter().map(|row| row.id).collect::<Vec<_>>();
                let Some(admission) =
                    super::super::retention::acquire_retention_write_admission("quota_compaction")
                        .await
                else {
                    return Ok((rows_archived, archive_batches));
                };
                let execute_started = Instant::now();
                let mut tx = pool.begin().await?;
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
                    tx.rollback().await?;
                    drop(admission);
                    return Ok((rows_archived, archive_batches));
                }
                if super::super::retention::archive_table_source_identity_sha256(
                    tx.as_mut(),
                    spec,
                    "main",
                    &ids,
                )
                .await?
                    != identity
                {
                    bail!("quota task-local source changed before conversion");
                }
                upsert_archive_batch_manifest(tx.as_mut(), &archive_outcome).await?;
                delete_rows_by_ids(tx.as_mut(), spec.dataset, &ids).await?;
                let commit_started = Instant::now();
                tx.commit().await?;
                super::super::retention::retention_record_commit!(
                    "quota_compaction",
                    admission.admission_mode(),
                    group.len(),
                    group.len().saturating_mul(256),
                    prepare_elapsed,
                    admission.lock_wait(),
                    commit_started.duration_since(execute_started),
                    commit_started.elapsed(),
                    admission.p1_waiter_count(),
                    candidate_remaining_hint,
                );
                observation.committed(group.len(), admission.lock_wait());
                drop(admission);
                rows_archived += group.len();
                committed_rows += group.len();
            }
            if committed_rows > 0 {
                archive_batches += 1;
            }
        }
        if super::super::retention::retention_run_remaining_budget().is_some() {
            break;
        }
    }

    Ok((rows_archived, archive_batches))
}
