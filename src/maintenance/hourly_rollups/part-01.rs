async fn invocation_archive_is_missing_summary_projection_proof_tx(
    tx: &mut SqliteConnection,
    file_path: &str,
) -> Result<bool> {
    for target in SUMMARY_PROJECTION_ARCHIVE_REPLAY_TARGETS {
        if !hourly_rollup_archive_replayed_tx(
            tx,
            target,
            HOURLY_ROLLUP_DATASET_INVOCATIONS,
            file_path,
        )
        .await?
        {
            return Ok(true);
        }
    }
    Ok(false)
}

async fn reopen_materialized_invocation_archive_usage_breakdown_backfill_tx(
    tx: &mut SqliteConnection,
    file_path: &str,
    coverage_start_at: Option<&str>,
    coverage_end_at: Option<&str>,
) -> Result<Option<Vec<String>>> {
    let mut reopened_file_paths = vec![file_path.to_string()];
    if let (Some(coverage_start_at), Some(coverage_end_at)) = (coverage_start_at, coverage_end_at) {
        let mut bucket_start_epochs = crate::stats::archive_bucket_start_epochs_from_bounds(
            None,
            Some(coverage_start_at),
            Some(coverage_end_at),
        )?;
        let mut reopened_file_path_set = HashSet::from([file_path.to_string()]);

        loop {
            let overlapping_archives =
                load_completed_invocation_archives_overlapping_usage_breakdown_buckets_tx(
                    tx,
                    &bucket_start_epochs,
                )
                .await?;
            let mut expanded = false;
            for overlapping_archive in overlapping_archives {
                if overlapping_archive
                    .sha256
                    .as_deref()
                    .is_none_or(|sha256| sha256.trim().is_empty())
                {
                    return Ok(None);
                }
                if !reopened_file_path_set.insert(overlapping_archive.file_path.clone()) {
                    continue;
                }
                reopened_file_paths.push(overlapping_archive.file_path);
                bucket_start_epochs.extend(crate::stats::archive_bucket_start_epochs_from_bounds(
                    None,
                    Some(&overlapping_archive.coverage_start_at),
                    Some(&overlapping_archive.coverage_end_at),
                )?);
                expanded = true;
            }
            if !expanded {
                break;
            }
        }
        clear_usage_breakdown_rollup_rows_for_bucket_epochs_tx(tx, &bucket_start_epochs).await?;
        let mut bucket_start_epochs = bucket_start_epochs.into_iter().collect::<Vec<_>>();
        bucket_start_epochs.sort_unstable();
        let retained_live_rows =
            load_live_invocation_hourly_rows_for_bucket_epochs_tx(tx, &bucket_start_epochs).await?;
        upsert_invocation_hourly_rollups_tx(
            tx,
            &retained_live_rows,
            &[HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE_BREAKDOWN],
        )
        .await?;
    }
    for reopened_file_path in &reopened_file_paths {
        reset_invocation_archive_usage_breakdown_backfill_state_tx(tx, reopened_file_path).await?;
    }
    Ok(Some(reopened_file_paths))
}

pub(crate) async fn repair_materialized_invocation_archive_usage_breakdown_backfill_state(
    pool: &Pool<Sqlite>,
) -> Result<usize> {
    let mut tx = pool.begin().await?;
    let archive_rows =
        load_materialized_invocation_archives_for_usage_breakdown_repair_tx(tx.as_mut()).await?;
    let mut touched_batches = 0usize;
    let mut reopened_file_paths = HashSet::new();

    for (file_path, coverage_start_at, coverage_end_at) in archive_rows {
        if reopened_file_paths.contains(&file_path) {
            continue;
        }
        if !archive_batch_has_completed_manifest_sha_tx(
            tx.as_mut(),
            HOURLY_ROLLUP_DATASET_INVOCATIONS,
            &file_path,
        )
        .await?
        {
            continue;
        }
        if invocation_archive_has_unverified_replay_marker_tx(tx.as_mut(), &file_path).await? {
            continue;
        }
        let has_stale_replay_marker =
            invocation_archive_has_stale_replay_marker_tx(tx.as_mut(), &file_path).await?;
        let breakdown_replayed = hourly_rollup_archive_replayed_tx(
            tx.as_mut(),
            HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE_BREAKDOWN,
            HOURLY_ROLLUP_DATASET_INVOCATIONS,
            &file_path,
        )
        .await?;
        if !has_stale_replay_marker && breakdown_replayed {
            continue;
        }
        let reopened = if has_stale_replay_marker {
            reopen_replaced_materialized_invocation_archive_tx(
                tx.as_mut(),
                &file_path,
                coverage_start_at.as_deref(),
                coverage_end_at.as_deref(),
            )
            .await?
        } else {
            reopen_materialized_invocation_archive_usage_breakdown_backfill_tx(
                tx.as_mut(),
                &file_path,
                coverage_start_at.as_deref(),
                coverage_end_at.as_deref(),
            )
            .await?
        };
        let Some(reopened) = reopened else {
            continue;
        };
        for reopened_file_path in reopened {
            if reopened_file_paths.insert(reopened_file_path) {
                touched_batches += 1;
            }
        }
    }

    tx.commit().await?;
    Ok(touched_batches)
}

pub(crate) const HISTORICAL_ROLLUP_ARCHIVE_REPLAY_BATCH_SIZE: i64 = BACKFILL_BATCH_SIZE;
pub(crate) const HISTORICAL_ROLLUP_ARCHIVE_INFLATE_BUFFER_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HistoricalRollupArchiveReplayOutcome {
    Completed,
    HitBudget,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct HistoricalRollupArchiveReplayResult {
    outcome: HistoricalRollupArchiveReplayOutcome,
    cursor_id: i64,
}

fn historical_rollup_replay_made_progress(
    replay: HistoricalRollupArchiveReplayResult,
    initial_cursor_id: i64,
) -> bool {
    replay.cursor_id > initial_cursor_id
}

fn historical_rollup_candidate_changed(
    changed_in_prior_stage: bool,
    replay_progressed: bool,
    coverage_updated: bool,
) -> bool {
    changed_in_prior_stage || replay_progressed || coverage_updated
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct HistoricalRollupArchiveReplaySummary {
    pub(crate) scanned_batches: u64,
    pub(crate) skipped_batches: u64,
    pub(crate) remaining_skip_batches: usize,
    pub(crate) budget_consumed_batches: u64,
    pub(crate) blocked_batches: u64,
    pub(crate) materialized_batches: u64,
    pub(crate) changed_batches: u64,
    pub(crate) hit_budget: bool,
    pub(crate) advance_cursor_after_unstarted_replay: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct PoolUpstreamNodeHealthArchiveBackfillSummary {
    pub(crate) scanned_batches: u64,
    pub(crate) materialized_batches: u64,
    pub(crate) cached_rows: u64,
    pub(crate) pending_batches: u64,
    pub(crate) hit_budget: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct PoolUpstreamNodeHealthHourlyArchiveBackfillSummary {
    pub(crate) scanned_batches: u64,
    pub(crate) materialized_batches: u64,
    pub(crate) materialized_rows: u64,
    pub(crate) pending_batches: u64,
    pub(crate) hit_budget: bool,
}

#[derive(Debug, FromRow)]
pub(crate) struct HistoricalRollupArchiveCoverageBoundsRow {
    coverage_start_at: Option<String>,
    coverage_end_at: Option<String>,
}

pub(crate) fn historical_rollup_elapsed_budget_reached(
    started_at: Instant,
    max_elapsed: Option<Duration>,
) -> bool {
    max_elapsed.is_some_and(|limit| started_at.elapsed() >= limit)
}

pub(crate) fn historical_rollup_materialization_budget_reached(
    started_at: Instant,
    replayed: u64,
    max_archive_batches: Option<u64>,
    max_elapsed: Option<Duration>,
) -> bool {
    max_archive_batches.is_some_and(|limit| replayed >= limit)
        || historical_rollup_elapsed_budget_reached(started_at, max_elapsed)
}

pub(crate) fn inflate_gzip_sqlite_file_with_budget(
    source: &Path,
    destination: &Path,
    started_at: Instant,
    max_elapsed: Option<Duration>,
) -> Result<bool> {
    let input = fs::File::open(source)
        .with_context(|| format!("failed to open archive batch {}", source.display()))?;
    let mut decoder = GzDecoder::new(input);
    let output = fs::File::create(destination)
        .with_context(|| format!("failed to create temp archive db {}", destination.display()))?;
    let mut writer = io::BufWriter::new(output);
    let mut buffer = vec![0_u8; HISTORICAL_ROLLUP_ARCHIVE_INFLATE_BUFFER_BYTES];

    loop {
        let read = decoder.read(&mut buffer).with_context(|| {
            format!(
                "failed to decompress archive batch {} into {}",
                source.display(),
                destination.display()
            )
        })?;
        if read == 0 {
            break;
        }
        writer.write_all(&buffer[..read]).with_context(|| {
            format!(
                "failed to decompress archive batch {} into {}",
                source.display(),
                destination.display()
            )
        })?;
        if historical_rollup_elapsed_budget_reached(started_at, max_elapsed) {
            writer.flush()?;
            return Ok(false);
        }
    }

    writer.flush()?;
    Ok(true)
}

pub(crate) async fn open_historical_rollup_archive_pool(
    archive_path: &Path,
    temp_path: &Path,
) -> Result<Pool<Sqlite>> {
    let current_signature = historical_rollup_archive_source_signature(archive_path)?;
    let stale_temp = !temp_path.exists()
        || load_historical_rollup_temp_source_signature(temp_path).as_deref()
            != Some(current_signature.as_str());
    if stale_temp {
        remove_temp_sqlite_artifacts(temp_path);
        inflate_gzip_sqlite_file(archive_path, temp_path)?;
        persist_historical_rollup_temp_source_signature(temp_path, &current_signature)?;
    }

    let connect = || async {
        SqlitePoolOptions::new()
            .max_connections(1)
            .connect(&sqlite_url_for_path(temp_path))
            .await
    };

    match connect().await {
        Ok(pool) => Ok(pool),
        Err(first_err) => {
            remove_temp_sqlite_artifacts(temp_path);
            inflate_gzip_sqlite_file(archive_path, temp_path)?;
            persist_historical_rollup_temp_source_signature(temp_path, &current_signature)?;
            connect().await.with_context(|| {
                format!(
                    "failed to reopen archive batch {} after resetting temp db (initial error: {first_err})",
                    archive_path.display()
                )
            })
        }
    }
}

async fn open_historical_rollup_archive_pool_with_budget(
    archive_path: &Path,
    temp_path: &Path,
    started_at: Instant,
    max_elapsed: Option<Duration>,
) -> Result<Option<Pool<Sqlite>>> {
    if historical_rollup_elapsed_budget_reached(started_at, max_elapsed) {
        return Ok(None);
    }
    let current_signature = historical_rollup_archive_source_signature(archive_path)?;
    let stale_temp = !temp_path.exists()
        || load_historical_rollup_temp_source_signature(temp_path).as_deref()
            != Some(current_signature.as_str());
    if stale_temp {
        remove_temp_sqlite_artifacts(temp_path);
        if !inflate_gzip_sqlite_file_with_budget(archive_path, temp_path, started_at, max_elapsed)?
        {
            remove_temp_sqlite_artifacts(temp_path);
            return Ok(None);
        }
        persist_historical_rollup_temp_source_signature(temp_path, &current_signature)?;
    }

    let connect = || async {
        SqlitePoolOptions::new()
            .max_connections(1)
            .connect(&sqlite_url_for_path(temp_path))
            .await
    };

    match connect().await {
        Ok(pool) => Ok(Some(pool)),
        Err(first_err) => {
            remove_temp_sqlite_artifacts(temp_path);
            if !inflate_gzip_sqlite_file_with_budget(
                archive_path,
                temp_path,
                started_at,
                max_elapsed,
            )? {
                remove_temp_sqlite_artifacts(temp_path);
                return Ok(None);
            }
            persist_historical_rollup_temp_source_signature(temp_path, &current_signature)?;
            connect().await.map(Some).with_context(|| {
                format!(
                    "failed to reopen archive batch {} after resetting temp db (initial error: {first_err})",
                    archive_path.display()
                )
            })
        }
    }
}

pub(crate) fn pool_upstream_node_health_archive_temp_path(archive_path: &Path) -> PathBuf {
    PathBuf::from(format!(
        "{}.pool-upstream-node-health.sqlite",
        archive_path.display()
    ))
}

pub(crate) fn historical_rollup_archive_source_signature(path: &Path) -> Result<String> {
    let metadata = fs::metadata(path)
        .with_context(|| format!("failed to inspect archive batch {}", path.display()))?;
    let modified_ns = metadata
        .modified()
        .ok()
        .and_then(|value| value.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|value| value.as_nanos())
        .unwrap_or_default();
    Ok(format!("{}:{modified_ns}", metadata.len()))
}

pub(crate) fn load_historical_rollup_temp_source_signature(temp_path: &Path) -> Option<String> {
    fs::read_to_string(temp_sqlite_source_meta_path(temp_path))
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

pub(crate) fn persist_historical_rollup_temp_source_signature(
    temp_path: &Path,
    signature: &str,
) -> Result<()> {
    fs::write(temp_sqlite_source_meta_path(temp_path), signature).with_context(|| {
        format!(
            "failed to persist archive temp source signature for {}",
            temp_path.display()
        )
    })
}

pub(crate) async fn load_pending_pool_upstream_node_health_archive_files(
    pool: &Pool<Sqlite>,
    range_start_at: Option<&str>,
    range_end_at: Option<&str>,
) -> Result<Vec<ArchiveBatchFileRow>> {
    let mut query = QueryBuilder::<Sqlite>::new(
        r#"
        SELECT id, file_path, coverage_start_at, coverage_end_at
        FROM archive_batches AS batches
        WHERE batches.dataset = 'pool_upstream_request_attempts'
          AND batches.status = "#,
    );
    query.push_bind(ARCHIVE_STATUS_COMPLETED);
    query.push(
        r#"
          AND NOT EXISTS (
                SELECT 1
                FROM hourly_rollup_archive_replay AS replay
                WHERE replay.target = "#,
    );
    query.push_bind(POOL_UPSTREAM_NODE_HEALTH_ARCHIVE_REPLAY_TARGET);
    query.push(
        r#"
                  AND replay.dataset = batches.dataset
                  AND replay.file_path = batches.file_path
                  AND batches.sha256 IS NOT NULL
                  AND TRIM(batches.sha256) <> ''
                  AND replay.archive_sha256 = batches.sha256
          )
        "#,
    );
    if let (Some(range_start_at), Some(range_end_at)) = (range_start_at, range_end_at) {
        query.push(
            r#"
          AND (
                batches.coverage_start_at IS NULL
                OR batches.coverage_end_at IS NULL
                OR (batches.coverage_end_at >= "#,
        );
        query.push_bind(range_start_at);
        query.push(" AND batches.coverage_start_at < ");
        query.push_bind(range_end_at);
        query.push(")\n          )");
    }
    query.push("\nORDER BY month_key ASC, created_at ASC, id ASC");
    query
        .build_query_as::<ArchiveBatchFileRow>()
        .fetch_all(pool)
        .await
        .context("failed to list pending pool upstream node health archive batches")
}

pub(crate) async fn pending_pool_upstream_node_health_archive_batches(
    pool: &Pool<Sqlite>,
) -> Result<u64> {
    Ok(
        load_pending_pool_upstream_node_health_archive_files(pool, None, None)
            .await?
            .len() as u64,
    )
}

pub(crate) async fn load_pending_pool_upstream_node_health_hourly_archive_files(
    pool: &Pool<Sqlite>,
) -> Result<Vec<ArchiveBatchFileRow>> {
    let mut query = QueryBuilder::<Sqlite>::new(
        r#"
        SELECT id, file_path, coverage_start_at, coverage_end_at
        FROM archive_batches AS batches
        WHERE batches.dataset = 'pool_upstream_request_attempts'
          AND batches.status = "#,
    );
    query.push_bind(ARCHIVE_STATUS_COMPLETED);
    query.push(
        r#"
          AND NOT EXISTS (
                SELECT 1
                FROM hourly_rollup_archive_replay AS replay
                WHERE replay.target = "#,
    );
    query.push_bind(POOL_UPSTREAM_NODE_HEALTH_HOURLY_ARCHIVE_REPLAY_TARGET);
    query.push(
        r#"
                  AND replay.dataset = batches.dataset
                  AND replay.file_path = batches.file_path
                  AND batches.sha256 IS NOT NULL
                  AND TRIM(batches.sha256) <> ''
                  AND replay.archive_sha256 = batches.sha256
          )
        ORDER BY month_key ASC, created_at ASC, id ASC
        "#,
    );
    query
        .build_query_as::<ArchiveBatchFileRow>()
        .fetch_all(pool)
        .await
        .context("failed to list pending pool upstream node health hourly archive batches")
}

pub(crate) async fn pending_pool_upstream_node_health_hourly_archive_batches(
    pool: &Pool<Sqlite>,
) -> Result<u64> {
    Ok(
        load_pending_pool_upstream_node_health_hourly_archive_files(pool)
            .await?
            .len() as u64,
    )
}

pub(crate) async fn load_invocation_archive_files_missing_rollup_target(
    executor: impl sqlx::Executor<'_, Database = Sqlite>,
    target: &str,
) -> Result<Vec<ArchiveBatchFileRow>> {
    let archive_files = sqlx::query_as::<_, ArchiveBatchFileRow>(
        r#"
        SELECT id, file_path, coverage_start_at, coverage_end_at
        FROM archive_batches AS batches
        WHERE batches.dataset = 'codex_invocations'
          AND batches.status = ?1
          AND COALESCE(batches.summary_source_kind, 'unknown') <> 'live_mirror'
          AND NOT EXISTS (
                SELECT 1
                FROM hourly_rollup_archive_replay AS replay
                WHERE replay.target = ?2
                  AND replay.dataset = batches.dataset
                  AND replay.file_path = batches.file_path
                  AND replay.archive_sha256 = batches.sha256
          )
        ORDER BY month_key ASC, created_at ASC, id ASC
        "#,
    )
    .bind(ARCHIVE_STATUS_COMPLETED)
    .bind(target)
    .fetch_all(executor)
    .await
    .context("failed to list invocation archive batches missing historical rollup target")?;
    Ok(archive_files
        .into_iter()
        .filter(|archive_file| Path::new(&archive_file.file_path).exists())
        .collect())
}
