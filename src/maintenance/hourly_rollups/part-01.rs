async fn reopen_replaced_materialized_forward_proxy_archive_tx(
    tx: &mut SqliteConnection,
    file_path: &str,
    coverage_start_at: Option<&str>,
    coverage_end_at: Option<&str>,
) -> Result<Option<Vec<String>>> {
    let (Some(coverage_start_at), Some(coverage_end_at)) = (coverage_start_at, coverage_end_at)
    else {
        return Ok(None);
    };
    let mut bucket_start_epochs =
        forward_proxy_archive_bucket_start_epochs_from_bounds(coverage_start_at, coverage_end_at)?;
    let mut reopened_file_paths = vec![file_path.to_string()];
    let mut reopened_file_path_set = HashSet::from([file_path.to_string()]);

    // Rebuild the transitive overlap closure so rows from a retained peer cannot survive the
    // replacement clear and then be added again by a later replay.
    loop {
        let overlapping_archives =
            load_completed_forward_proxy_archives_overlapping_buckets_tx(tx, &bucket_start_epochs)
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
            bucket_start_epochs.extend(forward_proxy_archive_bucket_start_epochs_from_bounds(
                &overlapping_archive.coverage_start_at,
                &overlapping_archive.coverage_end_at,
            )?);
            expanded = true;
        }
        if !expanded {
            break;
        }
    }

    clear_forward_proxy_rollup_rows_for_bucket_epochs_tx(tx, &bucket_start_epochs).await?;
    reset_forward_proxy_archive_replay_state_tx(tx, &reopened_file_paths).await?;
    Ok(Some(reopened_file_paths))
}

async fn reopen_replaced_materialized_invocation_archive_tx(
    tx: &mut SqliteConnection,
    file_path: &str,
    coverage_start_at: Option<&str>,
    coverage_end_at: Option<&str>,
) -> Result<Option<Vec<String>>> {
    let (Some(coverage_start_at), Some(coverage_end_at)) = (coverage_start_at, coverage_end_at)
    else {
        return Ok(None);
    };
    let mut bucket_start_epochs = crate::stats::archive_bucket_start_epochs_from_bounds(
        None,
        Some(coverage_start_at),
        Some(coverage_end_at),
    )?;
    let mut reopened_file_paths = Vec::new();
    let mut reopened_file_path_set = HashSet::new();

    // Resetting an overlapping archive requires clearing its entire coverage before it can be
    // replayed. Keep expanding the overlap set until every reopened archive is represented.
    loop {
        let overlapping_archives =
            load_completed_invocation_archives_overlapping_usage_breakdown_buckets_tx(
                tx,
                &bucket_start_epochs,
            )
            .await?;
        let mut expanded = false;
        for overlapping_archive in overlapping_archives {
            let Some(expected_sha256) = overlapping_archive
                .sha256
                .as_deref()
                .filter(|sha256| !sha256.trim().is_empty())
            else {
                // An unverifiable overlap cannot be replayed after its rows are cleared.
                // Leave the complete closure quarantined rather than partially rebuilding it.
                return Ok(None);
            };
            if !reopened_file_path_set.insert(overlapping_archive.file_path.clone()) {
                continue;
            }
            let actual_sha256 = match crate::maintenance::sha256_hex_file(Path::new(
                &overlapping_archive.file_path,
            )) {
                Ok(value) => value,
                Err(_) => return Ok(None),
            };
            if actual_sha256 != expected_sha256 {
                return Ok(None);
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
    if !reopened_file_path_set.contains(file_path) {
        return Ok(None);
    }
    clear_invocation_rollup_rows_for_bucket_epochs_tx(tx, &bucket_start_epochs).await?;
    reset_invocation_archive_replay_state_tx(tx, &reopened_file_paths).await?;
    Ok(Some(reopened_file_paths))
}

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

pub(crate) fn legacy_compatible_archive_select_expr(
    archive_columns: &HashSet<String>,
    column_name: &str,
) -> String {
    if archive_columns.contains(column_name) {
        column_name.to_string()
    } else {
        format!("NULL AS {column_name}")
    }
}

pub(crate) fn build_invocation_archive_rows_chunk_query(
    archive_columns: &HashSet<String>,
) -> String {
    let input_tokens = legacy_compatible_archive_select_expr(archive_columns, "input_tokens");
    let output_tokens = legacy_compatible_archive_select_expr(archive_columns, "output_tokens");
    let cache_input_tokens =
        legacy_compatible_archive_select_expr(archive_columns, "cache_input_tokens");
    let model = legacy_compatible_archive_select_expr(archive_columns, "model");
    let cost_input = legacy_compatible_archive_select_expr(archive_columns, "cost_input");
    let cost_cache_write =
        legacy_compatible_archive_select_expr(archive_columns, "cost_cache_write");
    let cost_cache_read = legacy_compatible_archive_select_expr(archive_columns, "cost_cache_read");
    let cost_output = legacy_compatible_archive_select_expr(archive_columns, "cost_output");
    let cost_reasoning = legacy_compatible_archive_select_expr(archive_columns, "cost_reasoning");
    let first_token_ms = legacy_compatible_archive_select_expr(archive_columns, "first_token_ms");
    format!(
        r#"
        SELECT
            id,
            occurred_at,
            source,
            status,
            detail_level,
            {model},
            {input_tokens},
            {output_tokens},
            {cache_input_tokens},
            total_tokens,
            cost,
            {cost_input},
            {cost_cache_write},
            {cost_cache_read},
            {cost_output},
            {cost_reasoning},
            error_message,
            failure_kind,
            failure_class,
            is_actionable,
            payload,
            t_total_ms,
            t_req_read_ms,
            t_req_parse_ms,
            t_upstream_connect_ms,
            t_upstream_ttfb_ms,
            {first_token_ms},
            t_upstream_stream_ms,
            t_resp_parse_ms,
            t_persist_ms
        FROM codex_invocations
        WHERE id > ?1
        ORDER BY id ASC
        LIMIT ?2
        "#
    )
}

/// Summary Snapshot V2 has a chronological proof contract, so its archive pager must use the
/// same deterministic key as the proof validator. Keep the generic historical-rollup pager
/// above ID-ordered because its checkpoint schema and replay semantics are independent.
pub(crate) fn build_invocation_archive_rows_time_chunk_query(
    archive_columns: &HashSet<String>,
) -> String {
    build_invocation_archive_rows_chunk_query(archive_columns).replace(
        "WHERE id > ?1\n        ORDER BY id ASC\n        LIMIT ?2",
        "WHERE julianday(occurred_at) IS NOT NULL\n          AND (?1 IS NULL\n               OR julianday(occurred_at) > julianday(?1)\n               OR (julianday(occurred_at) = julianday(?1) AND id > ?2))\n        ORDER BY julianday(occurred_at) ASC, id ASC\n        LIMIT ?3",
    )
}

pub(crate) async fn load_invocation_archive_rows_time_chunk(
    archive_pool: &Pool<Sqlite>,
    query_sql: &str,
    cursor_occurred_at: Option<&str>,
    cursor_id: i64,
) -> Result<(Vec<InvocationHourlySourceRecord>, bool)> {
    let mut rows = sqlx::query_as::<_, InvocationHourlySourceRecord>(query_sql)
        .bind(cursor_occurred_at)
        .bind(cursor_id.max(0))
        .bind(HISTORICAL_ROLLUP_ARCHIVE_REPLAY_BATCH_SIZE + 1)
        .fetch_all(archive_pool)
        .await?;
    let has_more = rows.len() > HISTORICAL_ROLLUP_ARCHIVE_REPLAY_BATCH_SIZE as usize;
    if has_more {
        rows.truncate(HISTORICAL_ROLLUP_ARCHIVE_REPLAY_BATCH_SIZE as usize);
    }
    Ok((rows, has_more))
}

pub(crate) async fn load_invocation_archive_rows_chunk(
    archive_pool: &Pool<Sqlite>,
    query_sql: &str,
    start_after_id: i64,
) -> Result<(Vec<InvocationHourlySourceRecord>, bool)> {
    let mut rows = sqlx::query_as::<_, InvocationHourlySourceRecord>(query_sql)
        .bind(start_after_id)
        .bind(HISTORICAL_ROLLUP_ARCHIVE_REPLAY_BATCH_SIZE + 1)
        .fetch_all(archive_pool)
        .await?;
    let has_more = rows.len() > HISTORICAL_ROLLUP_ARCHIVE_REPLAY_BATCH_SIZE as usize;
    if has_more {
        rows.truncate(HISTORICAL_ROLLUP_ARCHIVE_REPLAY_BATCH_SIZE as usize);
    }
    Ok((rows, has_more))
}

pub(crate) async fn load_forward_proxy_archive_rows_chunk(
    archive_pool: &Pool<Sqlite>,
    start_after_id: i64,
) -> Result<(Vec<ForwardProxyAttemptHourlySourceRecord>, bool)> {
    let mut rows = sqlx::query_as::<_, ForwardProxyAttemptHourlySourceRecord>(
        r#"
        SELECT
            id,
            proxy_key,
            occurred_at,
            is_success,
            latency_ms
        FROM forward_proxy_attempts
        WHERE id > ?1
        ORDER BY id ASC
        LIMIT ?2
        "#,
    )
    .bind(start_after_id)
    .bind(HISTORICAL_ROLLUP_ARCHIVE_REPLAY_BATCH_SIZE + 1)
    .fetch_all(archive_pool)
    .await?;
    let has_more = rows.len() > HISTORICAL_ROLLUP_ARCHIVE_REPLAY_BATCH_SIZE as usize;
    if has_more {
        rows.truncate(HISTORICAL_ROLLUP_ARCHIVE_REPLAY_BATCH_SIZE as usize);
    }
    Ok((rows, has_more))
}

pub(crate) async fn load_archive_coverage_bounds(
    archive_pool: &Pool<Sqlite>,
    table_name: &str,
) -> Result<HistoricalRollupArchiveCoverageBoundsRow> {
    Ok(
        sqlx::query_as::<_, HistoricalRollupArchiveCoverageBoundsRow>(&format!(
            r#"
        SELECT
            MIN(occurred_at) AS coverage_start_at,
            MAX(occurred_at) AS coverage_end_at
        FROM {table_name}
        "#
        ))
        .fetch_one(archive_pool)
        .await?,
    )
}
