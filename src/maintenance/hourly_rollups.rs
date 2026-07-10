use super::*;

pub(crate) async fn sync_hourly_rollups_from_live_tables(pool: &Pool<Sqlite>) -> Result<()> {
    loop {
        let updated = replay_live_invocation_hourly_rollups(pool).await?;
        if updated == 0 {
            break;
        }
    }
    loop {
        let updated = replay_live_forward_proxy_attempt_hourly_rollups(pool).await?;
        if updated == 0 {
            break;
        }
    }
    Ok(())
}

pub(crate) async fn mark_materialized_upstream_account_archive_replayed_tx(
    tx: &mut SqliteConnection,
    file_path: &str,
) -> Result<()> {
    for target in [
        HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE,
        HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_STATS_HOURLY,
        HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_STATS_MINUTE,
    ] {
        mark_hourly_rollup_archive_replayed_tx(
            tx,
            target,
            HOURLY_ROLLUP_DATASET_INVOCATIONS,
            file_path,
        )
        .await?;
    }
    Ok(())
}

pub(crate) async fn load_materialized_invocation_archives_missing_upstream_account_markers_tx(
    tx: &mut SqliteConnection,
) -> Result<Vec<String>> {
    sqlx::query_scalar(
        r#"
        SELECT batches.file_path
        FROM archive_batches AS batches
        WHERE batches.dataset = 'codex_invocations'
          AND batches.status = ?1
          AND batches.historical_rollups_materialized_at IS NOT NULL
          AND (
                NOT EXISTS (
                    SELECT 1
                    FROM hourly_rollup_archive_replay AS replay
                    WHERE replay.target = ?2
                      AND replay.dataset = batches.dataset
                      AND replay.file_path = batches.file_path
                )
                OR NOT EXISTS (
                    SELECT 1
                    FROM hourly_rollup_archive_replay AS replay
                    WHERE replay.target = ?3
                      AND replay.dataset = batches.dataset
                      AND replay.file_path = batches.file_path
                )
                OR NOT EXISTS (
                    SELECT 1
                    FROM hourly_rollup_archive_replay AS replay
                    WHERE replay.target = ?4
                      AND replay.dataset = batches.dataset
                      AND replay.file_path = batches.file_path
                )
          )
        ORDER BY batches.month_key ASC, batches.created_at ASC, batches.id ASC
        "#,
    )
    .bind(ARCHIVE_STATUS_COMPLETED)
    .bind(HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE)
    .bind(HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_STATS_HOURLY)
    .bind(HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_STATS_MINUTE)
    .fetch_all(&mut *tx)
    .await
    .map_err(Into::into)
}

pub(crate) async fn repair_materialized_upstream_account_archive_markers(
    pool: &Pool<Sqlite>,
) -> Result<usize> {
    let mut tx = pool.begin().await?;
    let file_paths =
        load_materialized_invocation_archives_missing_upstream_account_markers_tx(tx.as_mut())
            .await?;
    for file_path in &file_paths {
        mark_materialized_upstream_account_archive_replayed_tx(tx.as_mut(), file_path).await?;
    }
    tx.commit().await?;
    Ok(file_paths.len())
}

pub(crate) const HISTORICAL_ROLLUP_ARCHIVE_REPLAY_BATCH_SIZE: i64 = BACKFILL_BATCH_SIZE;
#[cfg(test)]
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

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct HistoricalRollupArchiveReplaySummary {
    pub(crate) scanned_batches: u64,
    pub(crate) skipped_batches: u64,
    pub(crate) remaining_skip_batches: usize,
    pub(crate) budget_consumed_batches: u64,
    pub(crate) blocked_batches: u64,
    pub(crate) materialized_batches: u64,
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

#[cfg(test)]
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
    format!(
        r#"
        SELECT
            id,
            occurred_at,
            source,
            status,
            detail_level,
            {input_tokens},
            {output_tokens},
            {cache_input_tokens},
            total_tokens,
            cost,
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

pub(crate) async fn invocation_archive_has_pruned_success_details_in_db(
    archive_pool: &Pool<Sqlite>,
) -> Result<bool> {
    let success_like_sql = invocation_status_is_success_like_sql("status", "error_message");
    let query = format!(
        r#"
        SELECT EXISTS(
            SELECT 1
            FROM codex_invocations
            WHERE detail_level != ?1
              AND {success_like_sql}
              AND COALESCE(NULLIF(LOWER(TRIM(COALESCE(failure_class, ''))), ''), 'none') = 'none'
            LIMIT 1
        )
        "#
    );
    let exists = sqlx::query_scalar::<_, i64>(&query)
        .bind(DETAIL_LEVEL_FULL)
        .fetch_one(archive_pool)
        .await?;
    Ok(exists != 0)
}

pub(crate) async fn replay_invocation_archive_rows_into_hourly_rollups_tx_with_budget(
    tx: &mut SqliteConnection,
    archive_pool: &Pool<Sqlite>,
    initial_cursor_id: i64,
    pending_targets: &[&str],
    started_at: Instant,
    max_elapsed: Option<Duration>,
) -> Result<HistoricalRollupArchiveReplayResult> {
    let mut start_after_id = initial_cursor_id.max(0);
    let archive_columns = load_sqlite_table_columns(archive_pool, "codex_invocations").await?;
    let query_sql = build_invocation_archive_rows_chunk_query(&archive_columns);
    loop {
        if historical_rollup_elapsed_budget_reached(started_at, max_elapsed) {
            return Ok(HistoricalRollupArchiveReplayResult {
                outcome: HistoricalRollupArchiveReplayOutcome::HitBudget,
                cursor_id: start_after_id,
            });
        }

        let (rows, has_more) =
            load_invocation_archive_rows_chunk(archive_pool, &query_sql, start_after_id).await?;
        if rows.is_empty() {
            return Ok(HistoricalRollupArchiveReplayResult {
                outcome: HistoricalRollupArchiveReplayOutcome::Completed,
                cursor_id: start_after_id,
            });
        }

        upsert_invocation_hourly_rollups_tx(tx, &rows, pending_targets).await?;
        mark_invocation_hourly_rollup_buckets_materialized_tx(tx, &rows).await?;
        start_after_id = rows
            .last()
            .map(|row| row.id)
            .ok_or_else(|| anyhow!("missing invocation archive row id"))?;

        if !has_more {
            return Ok(HistoricalRollupArchiveReplayResult {
                outcome: HistoricalRollupArchiveReplayOutcome::Completed,
                cursor_id: start_after_id,
            });
        }

        if historical_rollup_elapsed_budget_reached(started_at, max_elapsed) {
            return Ok(HistoricalRollupArchiveReplayResult {
                outcome: HistoricalRollupArchiveReplayOutcome::HitBudget,
                cursor_id: start_after_id,
            });
        }
    }
}

pub(crate) async fn replay_forward_proxy_archive_rows_into_hourly_rollups_tx_with_budget(
    tx: &mut SqliteConnection,
    archive_pool: &Pool<Sqlite>,
    initial_cursor_id: i64,
    started_at: Instant,
    max_elapsed: Option<Duration>,
) -> Result<HistoricalRollupArchiveReplayResult> {
    let mut start_after_id = initial_cursor_id.max(0);
    loop {
        if historical_rollup_elapsed_budget_reached(started_at, max_elapsed) {
            return Ok(HistoricalRollupArchiveReplayResult {
                outcome: HistoricalRollupArchiveReplayOutcome::HitBudget,
                cursor_id: start_after_id,
            });
        }

        let (rows, has_more) =
            load_forward_proxy_archive_rows_chunk(archive_pool, start_after_id).await?;
        if rows.is_empty() {
            return Ok(HistoricalRollupArchiveReplayResult {
                outcome: HistoricalRollupArchiveReplayOutcome::Completed,
                cursor_id: start_after_id,
            });
        }

        upsert_forward_proxy_attempt_hourly_rollups_tx(tx, &rows).await?;
        mark_forward_proxy_hourly_rollup_buckets_materialized_tx(tx, &rows).await?;
        start_after_id = rows
            .last()
            .map(|row| row.id)
            .ok_or_else(|| anyhow!("missing forward proxy archive row id"))?;

        if !has_more {
            return Ok(HistoricalRollupArchiveReplayResult {
                outcome: HistoricalRollupArchiveReplayOutcome::Completed,
                cursor_id: start_after_id,
            });
        }

        if historical_rollup_elapsed_budget_reached(started_at, max_elapsed) {
            return Ok(HistoricalRollupArchiveReplayResult {
                outcome: HistoricalRollupArchiveReplayOutcome::HitBudget,
                cursor_id: start_after_id,
            });
        }
    }
}

pub(crate) async fn replay_pool_upstream_node_health_archive_rows_tx_with_budget(
    tx: &mut SqliteConnection,
    archive_pool: &Pool<Sqlite>,
    archive_file_path: &str,
    initial_cursor_id: i64,
    started_at: Instant,
    max_elapsed: Option<Duration>,
) -> Result<(HistoricalRollupArchiveReplayResult, u64)> {
    let mut start_after_id = initial_cursor_id.max(0);
    let mut cached_rows = 0_u64;
    loop {
        if historical_rollup_elapsed_budget_reached(started_at, max_elapsed) {
            return Ok((
                HistoricalRollupArchiveReplayResult {
                    outcome: HistoricalRollupArchiveReplayOutcome::HitBudget,
                    cursor_id: start_after_id,
                },
                cached_rows,
            ));
        }

        let (rows, has_more) =
            load_pool_upstream_node_health_archive_rows_chunk(archive_pool, start_after_id).await?;
        if rows.is_empty() {
            return Ok((
                HistoricalRollupArchiveReplayResult {
                    outcome: HistoricalRollupArchiveReplayOutcome::Completed,
                    cursor_id: start_after_id,
                },
                cached_rows,
            ));
        }

        upsert_pool_upstream_node_health_archive_rows_tx(tx, archive_file_path, &rows).await?;
        cached_rows += rows.len() as u64;
        start_after_id = rows
            .last()
            .map(|row| row.archived_row_id)
            .ok_or_else(|| anyhow!("missing pool upstream node health archive row id"))?;

        if !has_more {
            return Ok((
                HistoricalRollupArchiveReplayResult {
                    outcome: HistoricalRollupArchiveReplayOutcome::Completed,
                    cursor_id: start_after_id,
                },
                cached_rows,
            ));
        }

        if historical_rollup_elapsed_budget_reached(started_at, max_elapsed) {
            return Ok((
                HistoricalRollupArchiveReplayResult {
                    outcome: HistoricalRollupArchiveReplayOutcome::HitBudget,
                    cursor_id: start_after_id,
                },
                cached_rows,
            ));
        }
    }
}

pub(crate) async fn replay_invocation_archives_into_hourly_rollups_tx_with_limits(
    tx: &mut SqliteConnection,
    started_at: Instant,
    max_archive_batches: Option<u64>,
    max_elapsed: Option<Duration>,
    skip_archive_batches: usize,
) -> Result<HistoricalRollupArchiveReplaySummary> {
    let mut summary = HistoricalRollupArchiveReplaySummary::default();
    let mut skip_remaining = skip_archive_batches;
    let archive_files = sqlx::query_as::<_, ArchiveBatchFileRow>(
        r#"
        SELECT id, file_path, coverage_start_at, coverage_end_at
        FROM archive_batches
        WHERE dataset = 'codex_invocations'
          AND status = ?1
          AND historical_rollups_materialized_at IS NULL
        ORDER BY month_key ASC, created_at ASC, id ASC
        "#,
    )
    .bind(ARCHIVE_STATUS_COMPLETED)
    .fetch_all(&mut *tx)
    .await?;

    for archive_file in archive_files {
        if skip_remaining > 0 {
            skip_remaining -= 1;
            summary.scanned_batches += 1;
            summary.skipped_batches += 1;
            continue;
        }
        if historical_rollup_materialization_budget_reached(
            started_at,
            summary.budget_consumed_batches,
            max_archive_batches,
            max_elapsed,
        ) {
            break;
        }
        summary.scanned_batches += 1;
        let mut pending_targets = Vec::new();
        let mut blocked_targets = Vec::new();
        for target in [
            HOURLY_ROLLUP_TARGET_INVOCATIONS,
            HOURLY_ROLLUP_TARGET_INVOCATION_FAILURES,
            HOURLY_ROLLUP_TARGET_PROXY_PERF,
            HOURLY_ROLLUP_TARGET_PROMPT_CACHE,
            HOURLY_ROLLUP_TARGET_PROMPT_CACHE_UPSTREAM_ACCOUNTS,
            HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE,
            HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_STATS_HOURLY,
            HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_STATS_MINUTE,
            HOURLY_ROLLUP_TARGET_STICKY_KEYS,
        ] {
            if !hourly_rollup_archive_replayed_tx(
                tx,
                target,
                HOURLY_ROLLUP_DATASET_INVOCATIONS,
                &archive_file.file_path,
            )
            .await?
            {
                pending_targets.push(target);
            }
        }
        if pending_targets.as_slice()
            == [
                HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE,
                HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_STATS_HOURLY,
                HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_STATS_MINUTE,
            ]
            || pending_targets.as_slice() == [HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE]
        {
            mark_materialized_upstream_account_archive_replayed_tx(tx, &archive_file.file_path)
                .await?;
            mark_archive_batch_historical_rollups_materialized_tx(
                tx,
                HOURLY_ROLLUP_DATASET_INVOCATIONS,
                &archive_file.file_path,
            )
            .await?;
            continue;
        }
        if pending_targets.is_empty() {
            mark_archive_batch_historical_rollups_materialized_tx(
                tx,
                HOURLY_ROLLUP_DATASET_INVOCATIONS,
                &archive_file.file_path,
            )
            .await?;
            continue;
        }
        let replay_cursor = load_hourly_rollup_archive_progress_tx(
            tx,
            HOURLY_ROLLUP_DATASET_INVOCATIONS,
            &archive_file.file_path,
        )
        .await?;

        let archive_path = PathBuf::from(&archive_file.file_path);
        if !archive_path.exists() {
            warn!(
                dataset = HOURLY_ROLLUP_DATASET_INVOCATIONS,
                file_path = archive_file.file_path,
                "skipping missing archive batch during historical rollup materialization"
            );
            delete_hourly_rollup_archive_progress_tx(
                tx,
                HOURLY_ROLLUP_DATASET_INVOCATIONS,
                &archive_file.file_path,
            )
            .await?;
            continue;
        }
        let temp_path = PathBuf::from(format!(
            "{}.{}.sqlite",
            archive_path.display(),
            retention_temp_suffix()
        ));
        let temp_cleanup = TempSqliteCleanup(temp_path.clone());
        let archive_pool = open_historical_rollup_archive_pool(&archive_path, &temp_path).await?;
        let has_pruned_success_details =
            invocation_archive_has_pruned_success_details_in_db(&archive_pool).await?;
        if has_pruned_success_details {
            let mut replayable_targets = Vec::with_capacity(pending_targets.len());
            for target in pending_targets {
                if invocation_archive_target_needs_full_payload(target) {
                    blocked_targets.push(target);
                } else {
                    replayable_targets.push(target);
                }
            }
            pending_targets = replayable_targets;
        }

        if pending_targets.is_empty() && blocked_targets.is_empty() {
            archive_pool.close().await;
            drop(temp_cleanup);
            mark_archive_batch_historical_rollups_materialized_tx(
                tx,
                HOURLY_ROLLUP_DATASET_INVOCATIONS,
                &archive_file.file_path,
            )
            .await?;
            continue;
        }
        if pending_targets.is_empty() {
            archive_pool.close().await;
            drop(temp_cleanup);
            delete_hourly_rollup_archive_progress_tx(
                tx,
                HOURLY_ROLLUP_DATASET_INVOCATIONS,
                &archive_file.file_path,
            )
            .await?;
            summary.blocked_batches += 1;
            summary.budget_consumed_batches += 1;
            warn!(
                dataset = HOURLY_ROLLUP_DATASET_INVOCATIONS,
                file_path = archive_file.file_path,
                blocked_targets = ?blocked_targets,
                "legacy archive batch contains pruned success details; keeping historical rollup materialization pending for keyed conversation targets"
            );
            continue;
        }

        if archive_file.coverage_start_at.is_none() || archive_file.coverage_end_at.is_none() {
            let bounds = load_archive_coverage_bounds(&archive_pool, "codex_invocations").await?;
            update_archive_batch_coverage_bounds_tx(
                tx,
                archive_file.id,
                bounds.coverage_start_at.as_deref(),
                bounds.coverage_end_at.as_deref(),
            )
            .await?;
        }

        let replay_outcome = replay_invocation_archive_rows_into_hourly_rollups_tx_with_budget(
            tx,
            &archive_pool,
            replay_cursor,
            &pending_targets,
            started_at,
            max_elapsed,
        )
        .await?;
        archive_pool.close().await;
        if replay_outcome.outcome == HistoricalRollupArchiveReplayOutcome::HitBudget {
            summary.budget_consumed_batches += 1;
            if replay_outcome.cursor_id > replay_cursor {
                save_hourly_rollup_archive_progress_tx(
                    tx,
                    HOURLY_ROLLUP_DATASET_INVOCATIONS,
                    &archive_file.file_path,
                    replay_outcome.cursor_id,
                )
                .await?;
            }
            std::mem::forget(temp_cleanup);
            break;
        }
        drop(temp_cleanup);
        summary.budget_consumed_batches += 1;
        delete_hourly_rollup_archive_progress_tx(
            tx,
            HOURLY_ROLLUP_DATASET_INVOCATIONS,
            &archive_file.file_path,
        )
        .await?;
        for target in pending_targets {
            mark_hourly_rollup_archive_replayed_tx(
                tx,
                target,
                HOURLY_ROLLUP_DATASET_INVOCATIONS,
                &archive_file.file_path,
            )
            .await?;
        }
        if blocked_targets.is_empty() {
            mark_archive_batch_historical_rollups_materialized_tx(
                tx,
                HOURLY_ROLLUP_DATASET_INVOCATIONS,
                &archive_file.file_path,
            )
            .await?;
            summary.materialized_batches += 1;
        } else {
            summary.blocked_batches += 1;
            warn!(
                dataset = HOURLY_ROLLUP_DATASET_INVOCATIONS,
                file_path = archive_file.file_path,
                blocked_targets = ?blocked_targets,
                "legacy archive batch contains pruned success details; keeping historical rollup materialization pending for keyed conversation targets"
            );
        }
    }

    summary.remaining_skip_batches = skip_remaining;
    Ok(summary)
}

pub(crate) async fn replay_invocation_archives_into_hourly_rollups_tx(
    tx: &mut SqliteConnection,
) -> Result<HistoricalRollupArchiveReplaySummary> {
    replay_invocation_archives_into_hourly_rollups_tx_with_limits(tx, Instant::now(), None, None, 0)
        .await
}

pub(crate) async fn replay_forward_proxy_archives_into_hourly_rollups_tx_with_limits(
    tx: &mut SqliteConnection,
    started_at: Instant,
    max_archive_batches: Option<u64>,
    max_elapsed: Option<Duration>,
    skip_archive_batches: usize,
) -> Result<HistoricalRollupArchiveReplaySummary> {
    let mut summary = HistoricalRollupArchiveReplaySummary::default();
    let mut skip_remaining = skip_archive_batches;
    let archive_files = sqlx::query_as::<_, ArchiveBatchFileRow>(
        r#"
        SELECT id, file_path, coverage_start_at, coverage_end_at
        FROM archive_batches
        WHERE dataset = 'forward_proxy_attempts'
          AND status = ?1
          AND historical_rollups_materialized_at IS NULL
        ORDER BY month_key ASC, created_at ASC, id ASC
        "#,
    )
    .bind(ARCHIVE_STATUS_COMPLETED)
    .fetch_all(&mut *tx)
    .await?;

    for archive_file in archive_files {
        if skip_remaining > 0 {
            skip_remaining -= 1;
            summary.scanned_batches += 1;
            summary.skipped_batches += 1;
            continue;
        }
        if historical_rollup_materialization_budget_reached(
            started_at,
            summary.budget_consumed_batches,
            max_archive_batches,
            max_elapsed,
        ) {
            break;
        }
        summary.scanned_batches += 1;
        if hourly_rollup_archive_replayed_tx(
            tx,
            HOURLY_ROLLUP_TARGET_FORWARD_PROXY_ATTEMPTS,
            HOURLY_ROLLUP_DATASET_FORWARD_PROXY_ATTEMPTS,
            &archive_file.file_path,
        )
        .await?
        {
            mark_archive_batch_historical_rollups_materialized_tx(
                tx,
                HOURLY_ROLLUP_DATASET_FORWARD_PROXY_ATTEMPTS,
                &archive_file.file_path,
            )
            .await?;
            continue;
        }
        let replay_cursor = load_hourly_rollup_archive_progress_tx(
            tx,
            HOURLY_ROLLUP_DATASET_FORWARD_PROXY_ATTEMPTS,
            &archive_file.file_path,
        )
        .await?;

        let archive_path = PathBuf::from(&archive_file.file_path);
        if !archive_path.exists() {
            warn!(
                dataset = HOURLY_ROLLUP_DATASET_FORWARD_PROXY_ATTEMPTS,
                file_path = archive_file.file_path,
                "skipping missing archive batch during historical rollup materialization"
            );
            delete_hourly_rollup_archive_progress_tx(
                tx,
                HOURLY_ROLLUP_DATASET_FORWARD_PROXY_ATTEMPTS,
                &archive_file.file_path,
            )
            .await?;
            continue;
        }
        let temp_path = PathBuf::from(format!(
            "{}.{}.sqlite",
            archive_path.display(),
            retention_temp_suffix()
        ));
        let temp_cleanup = TempSqliteCleanup(temp_path.clone());
        let archive_pool = open_historical_rollup_archive_pool(&archive_path, &temp_path).await?;

        if archive_file.coverage_start_at.is_none() || archive_file.coverage_end_at.is_none() {
            let bounds =
                load_archive_coverage_bounds(&archive_pool, "forward_proxy_attempts").await?;
            update_archive_batch_coverage_bounds_tx(
                tx,
                archive_file.id,
                bounds.coverage_start_at.as_deref(),
                bounds.coverage_end_at.as_deref(),
            )
            .await?;
        }

        let replay_outcome = replay_forward_proxy_archive_rows_into_hourly_rollups_tx_with_budget(
            tx,
            &archive_pool,
            replay_cursor,
            started_at,
            max_elapsed,
        )
        .await?;
        archive_pool.close().await;
        if replay_outcome.outcome == HistoricalRollupArchiveReplayOutcome::HitBudget {
            summary.budget_consumed_batches += 1;
            if replay_outcome.cursor_id > replay_cursor {
                save_hourly_rollup_archive_progress_tx(
                    tx,
                    HOURLY_ROLLUP_DATASET_FORWARD_PROXY_ATTEMPTS,
                    &archive_file.file_path,
                    replay_outcome.cursor_id,
                )
                .await?;
            }
            std::mem::forget(temp_cleanup);
            break;
        }
        drop(temp_cleanup);
        summary.budget_consumed_batches += 1;
        delete_hourly_rollup_archive_progress_tx(
            tx,
            HOURLY_ROLLUP_DATASET_FORWARD_PROXY_ATTEMPTS,
            &archive_file.file_path,
        )
        .await?;
        mark_archive_batch_historical_rollups_materialized_tx(
            tx,
            HOURLY_ROLLUP_DATASET_FORWARD_PROXY_ATTEMPTS,
            &archive_file.file_path,
        )
        .await?;
        mark_hourly_rollup_archive_replayed_tx(
            tx,
            HOURLY_ROLLUP_TARGET_FORWARD_PROXY_ATTEMPTS,
            HOURLY_ROLLUP_DATASET_FORWARD_PROXY_ATTEMPTS,
            &archive_file.file_path,
        )
        .await?;
        summary.materialized_batches += 1;
    }

    summary.remaining_skip_batches = skip_remaining;
    Ok(summary)
}

pub(crate) async fn backfill_pool_upstream_node_health_archives_for_files(
    pool: &Pool<Sqlite>,
    archive_files: Vec<ArchiveBatchFileRow>,
    max_archive_batches: Option<u64>,
    max_elapsed: Option<Duration>,
) -> Result<PoolUpstreamNodeHealthArchiveBackfillSummary> {
    let started_at = Instant::now();
    let mut replay_started_any_pending_batch = false;

    let mut summary = PoolUpstreamNodeHealthArchiveBackfillSummary::default();
    for archive_file in archive_files {
        let mut tx = pool.begin().await?;
        if hourly_rollup_archive_replayed_tx(
            tx.as_mut(),
            POOL_UPSTREAM_NODE_HEALTH_ARCHIVE_REPLAY_TARGET,
            "pool_upstream_request_attempts",
            &archive_file.file_path,
        )
        .await?
        {
            tx.commit().await?;
            continue;
        }

        if max_archive_batches.is_some_and(|limit| summary.materialized_batches >= limit)
            || (replay_started_any_pending_batch
                && historical_rollup_elapsed_budget_reached(started_at, max_elapsed))
        {
            summary.hit_budget = true;
            tx.commit().await?;
            break;
        }

        summary.scanned_batches += 1;
        let replay_cursor = load_hourly_rollup_archive_progress_tx(
            tx.as_mut(),
            "pool_upstream_request_attempts",
            &archive_file.file_path,
        )
        .await?;

        let archive_path = PathBuf::from(&archive_file.file_path);
        if !archive_path.exists() {
            warn!(
                dataset = "pool_upstream_request_attempts",
                file_path = archive_file.file_path,
                "pool upstream node health cache backfill marking missing archive batch as replayed"
            );
            delete_pool_upstream_node_health_archive_rows_for_file_tx(
                tx.as_mut(),
                &archive_file.file_path,
            )
            .await?;
            delete_hourly_rollup_archive_progress_tx(
                tx.as_mut(),
                "pool_upstream_request_attempts",
                &archive_file.file_path,
            )
            .await?;
            mark_hourly_rollup_archive_replayed_tx(
                tx.as_mut(),
                POOL_UPSTREAM_NODE_HEALTH_ARCHIVE_REPLAY_TARGET,
                "pool_upstream_request_attempts",
                &archive_file.file_path,
            )
            .await?;
            tx.commit().await?;
            continue;
        }

        replay_started_any_pending_batch = true;
        let temp_path = pool_upstream_node_health_archive_temp_path(&archive_path);
        let temp_cleanup = TempSqliteCleanup(temp_path.clone());
        let archive_pool = open_historical_rollup_archive_pool(&archive_path, &temp_path).await?;
        {
            let mut archive_conn = archive_pool.acquire().await?;
            ensure_pool_upstream_request_attempts_archive_schema_in_place(&mut archive_conn)
                .await?;
        }
        let (replay_outcome, cached_rows) =
            replay_pool_upstream_node_health_archive_rows_tx_with_budget(
                tx.as_mut(),
                &archive_pool,
                &archive_file.file_path,
                replay_cursor,
                started_at,
                max_elapsed,
            )
            .await?;
        archive_pool.close().await;
        summary.cached_rows += cached_rows;
        if replay_outcome.outcome == HistoricalRollupArchiveReplayOutcome::HitBudget {
            if replay_outcome.cursor_id > replay_cursor {
                save_hourly_rollup_archive_progress_tx(
                    tx.as_mut(),
                    "pool_upstream_request_attempts",
                    &archive_file.file_path,
                    replay_outcome.cursor_id,
                )
                .await?;
            }
            tx.commit().await?;
            std::mem::forget(temp_cleanup);
            summary.hit_budget = true;
            break;
        }
        drop(temp_cleanup);
        delete_hourly_rollup_archive_progress_tx(
            tx.as_mut(),
            "pool_upstream_request_attempts",
            &archive_file.file_path,
        )
        .await?;
        mark_hourly_rollup_archive_replayed_tx(
            tx.as_mut(),
            POOL_UPSTREAM_NODE_HEALTH_ARCHIVE_REPLAY_TARGET,
            "pool_upstream_request_attempts",
            &archive_file.file_path,
        )
        .await?;
        tx.commit().await?;
        summary.materialized_batches += 1;
    }

    summary.pending_batches = pending_pool_upstream_node_health_archive_batches(pool).await?;

    Ok(summary)
}

pub(crate) async fn backfill_pool_upstream_node_health_archives(
    pool: &Pool<Sqlite>,
    max_archive_batches: Option<u64>,
    max_elapsed: Option<Duration>,
) -> Result<PoolUpstreamNodeHealthArchiveBackfillSummary> {
    let archive_files =
        load_pending_pool_upstream_node_health_archive_files(pool, None, None).await?;
    backfill_pool_upstream_node_health_archives_for_files(
        pool,
        archive_files,
        max_archive_batches,
        max_elapsed,
    )
    .await
}

pub(crate) async fn backfill_pool_upstream_node_health_hourly_archives_for_files(
    pool: &Pool<Sqlite>,
    archive_files: Vec<ArchiveBatchFileRow>,
    max_archive_batches: Option<u64>,
    max_elapsed: Option<Duration>,
) -> Result<PoolUpstreamNodeHealthHourlyArchiveBackfillSummary> {
    let started_at = Instant::now();
    let mut summary = PoolUpstreamNodeHealthHourlyArchiveBackfillSummary::default();

    for archive_file in archive_files {
        if max_archive_batches.is_some_and(|limit| summary.materialized_batches >= limit)
            || historical_rollup_elapsed_budget_reached(started_at, max_elapsed)
        {
            summary.hit_budget = true;
            break;
        }

        summary.scanned_batches += 1;
        let mut tx = pool.begin().await?;
        if hourly_rollup_archive_replayed_tx(
            tx.as_mut(),
            POOL_UPSTREAM_NODE_HEALTH_HOURLY_ARCHIVE_REPLAY_TARGET,
            "pool_upstream_request_attempts",
            &archive_file.file_path,
        )
        .await?
        {
            tx.commit().await?;
            continue;
        }

        if !hourly_rollup_archive_replayed_tx(
            tx.as_mut(),
            POOL_UPSTREAM_NODE_HEALTH_ARCHIVE_REPLAY_TARGET,
            "pool_upstream_request_attempts",
            &archive_file.file_path,
        )
        .await?
        {
            tx.commit().await?;
            continue;
        }

        let materialized_rows =
            refresh_pool_upstream_node_health_hourly_archive_rows_from_cache_tx(
                tx.as_mut(),
                archive_file.id,
                &archive_file.file_path,
            )
            .await?;
        mark_hourly_rollup_archive_replayed_tx(
            tx.as_mut(),
            POOL_UPSTREAM_NODE_HEALTH_HOURLY_ARCHIVE_REPLAY_TARGET,
            "pool_upstream_request_attempts",
            &archive_file.file_path,
        )
        .await?;
        tx.commit().await?;
        summary.materialized_batches += 1;
        summary.materialized_rows += materialized_rows;
    }

    summary.pending_batches =
        pending_pool_upstream_node_health_hourly_archive_batches(pool).await?;
    Ok(summary)
}

pub(crate) async fn backfill_pool_upstream_node_health_hourly_archives(
    pool: &Pool<Sqlite>,
    max_archive_batches: Option<u64>,
    max_elapsed: Option<Duration>,
) -> Result<PoolUpstreamNodeHealthHourlyArchiveBackfillSummary> {
    let archive_files = load_pending_pool_upstream_node_health_hourly_archive_files(pool).await?;
    backfill_pool_upstream_node_health_hourly_archives_for_files(
        pool,
        archive_files,
        max_archive_batches,
        max_elapsed,
    )
    .await
}

#[cfg(test)]
mod hourly_rollup_budget_tests {
    use super::*;
    use std::{
        env,
        time::{SystemTime, UNIX_EPOCH},
    };

    fn budgeted_inflate_test_dir(prefix: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time before unix epoch")
            .as_nanos();
        let path = env::temp_dir().join(format!(
            "codex-vibe-monitor-{prefix}-{}-{unique}",
            std::process::id()
        ));
        fs::create_dir_all(&path).expect("create budgeted inflate temp dir");
        path
    }

    #[test]
    fn historical_rollup_elapsed_budget_reached_respects_unbounded_mode() {
        assert!(!historical_rollup_elapsed_budget_reached(
            Instant::now(),
            None
        ));
    }

    #[test]
    fn inflate_gzip_sqlite_file_with_budget_stops_mid_inflate_when_elapsed_budget_is_exhausted() {
        let temp_dir = budgeted_inflate_test_dir("historical-rollup-budgeted-inflate");
        let source_path = temp_dir.join("archive.sqlite.gz");
        let destination_path = temp_dir.join("archive.sqlite");
        let payload = vec![b'a'; HISTORICAL_ROLLUP_ARCHIVE_INFLATE_BUFFER_BYTES * 4];

        {
            let output = fs::File::create(&source_path).expect("create gzip source");
            let mut encoder = GzEncoder::new(io::BufWriter::new(output), Compression::default());
            encoder.write_all(&payload).expect("write gzip payload");
            let mut writer = encoder.finish().expect("finish gzip payload");
            writer.flush().expect("flush gzip payload");
        }

        let started_at = Instant::now() - Duration::from_millis(25);
        let completed = inflate_gzip_sqlite_file_with_budget(
            &source_path,
            &destination_path,
            started_at,
            Some(Duration::from_millis(1)),
        )
        .expect("inflate with budget");

        assert!(!completed, "expired elapsed budget should stop inflate");
        let written = fs::metadata(&destination_path)
            .expect("inflated temp file should exist")
            .len() as usize;
        assert!(
            written > 0,
            "budgeted inflate should still write at least one chunk"
        );
        assert!(
            written < payload.len(),
            "expired elapsed budget should stop before the whole sqlite copy completes"
        );

        let _ = fs::remove_dir_all(&temp_dir);
    }
}

pub(crate) async fn replay_forward_proxy_archives_into_hourly_rollups_tx(
    tx: &mut SqliteConnection,
) -> Result<HistoricalRollupArchiveReplaySummary> {
    replay_forward_proxy_archives_into_hourly_rollups_tx_with_limits(
        tx,
        Instant::now(),
        None,
        None,
        0,
    )
    .await
}

pub(crate) async fn bootstrap_hourly_rollups(pool: &Pool<Sqlite>) -> Result<()> {
    sync_hourly_rollups_from_live_tables(pool).await?;
    repair_materialized_upstream_account_archive_markers(pool).await?;
    let account_stats_hourly_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM upstream_account_stats_hourly")
            .fetch_one(pool)
            .await?;
    let account_stats_minute_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM upstream_account_stats_minute")
            .fetch_one(pool)
            .await?;
    if account_stats_hourly_count == 0 || account_stats_minute_count == 0 {
        rebuild_upstream_account_stats_rollups_from_sources(pool).await?;
        repair_materialized_upstream_account_archive_markers(pool).await?;
    }
    Ok(())
}

pub(crate) async fn refresh_hourly_rollups_for_read_surfaces(pool: &Pool<Sqlite>) -> Result<()> {
    sync_hourly_rollups_from_live_tables(pool).await?;
    ensure_invocation_summary_rollups_ready_best_effort(pool).await?;
    Ok(())
}

pub(crate) async fn ensure_hourly_rollups_caught_up(state: &AppState) -> Result<()> {
    let _guard = state.hourly_rollup_sync_lock.lock().await;
    sync_hourly_rollups_from_live_tables(&state.pool).await
}

pub(crate) async fn refresh_hourly_rollups_for_read_surfaces_best_effort(
    pool: &Pool<Sqlite>,
    hourly_rollup_sync_lock: &Mutex<()>,
    reason: &'static str,
) {
    let gate = crate::db_pressure::global_db_pressure_gate();
    let _permit = match gate.try_begin_background("hourly_rollup_refresh") {
        Ok(permit) => permit,
        Err(deny_reason) => {
            warn!(
                reason,
                deny_reason = %deny_reason,
                "background hourly rollup refresh skipped because database pressure gate is closed"
            );
            return;
        }
    };
    let _guard = hourly_rollup_sync_lock.lock().await;

    if let Err(err) = refresh_hourly_rollups_for_read_surfaces(pool).await {
        gate.record_error("hourly_rollup_refresh", &err);
        warn!(
            error = %err,
            reason,
            "background hourly rollup refresh failed; keeping existing rollups for read surfaces"
        );
    }
}

pub(crate) async fn delete_rows_by_ids(
    tx: &mut sqlx::SqliteConnection,
    table: &str,
    ids: &[i64],
) -> Result<()> {
    if ids.is_empty() {
        return Ok(());
    }
    let mut query = QueryBuilder::<Sqlite>::new(format!("DELETE FROM {table} WHERE id IN ("));
    {
        let mut separated = query.separated(", ");
        for id in ids {
            separated.push_bind(id);
        }
    }
    query.push(")");
    query.build().execute(&mut *tx).await?;
    Ok(())
}

pub(crate) async fn sweep_orphan_proxy_raw_files(
    pool: &Pool<Sqlite>,
    config: &AppConfig,
    raw_path_fallback_root: Option<&Path>,
    dry_run: bool,
) -> Result<usize> {
    let raw_dir = config.resolved_proxy_raw_dir();
    if !raw_dir.exists() {
        return Ok(0);
    }

    let referenced = sqlx::query_scalar::<_, String>(
        r#"
        SELECT path
        FROM (
            SELECT request_raw_path AS path
            FROM codex_invocations
            WHERE request_raw_path IS NOT NULL
            UNION
            SELECT response_raw_path AS path
            FROM codex_invocations
            WHERE response_raw_path IS NOT NULL
        )
        WHERE path IS NOT NULL
        "#,
    )
    .fetch_all(pool)
    .await?;

    let mut referenced_paths = HashSet::new();
    for path in referenced {
        for candidate in resolved_raw_path_candidates(&path, raw_path_fallback_root) {
            referenced_paths.insert(candidate);
        }
    }

    let min_file_age = Duration::from_secs(DEFAULT_ORPHAN_SWEEP_MIN_AGE_SECS);
    let mut removed = 0usize;
    for entry in fs::read_dir(&raw_dir)
        .with_context(|| format!("failed to read raw payload directory {}", raw_dir.display()))?
    {
        let entry = entry?;
        let path = entry.path();
        if !entry.file_type()?.is_file() {
            continue;
        }
        let age = match entry.metadata().and_then(|metadata| metadata.modified()) {
            Ok(modified) => modified.elapsed().unwrap_or_default(),
            Err(err) => {
                warn!(path = %path.display(), error = %err, "failed to inspect orphan raw payload file age");
                continue;
            }
        };
        if age < min_file_age {
            continue;
        }
        let normalized = normalize_path_for_compare(&path);
        if referenced_paths.contains(&normalized) {
            continue;
        }
        if dry_run {
            removed += 1;
            continue;
        }
        match fs::remove_file(&path) {
            Ok(_) => removed += 1,
            Err(err) if err.kind() == io::ErrorKind::NotFound => {}
            Err(err) => {
                warn!(path = %path.display(), error = %err, "failed to remove orphan raw payload file");
            }
        }
    }

    Ok(removed)
}

#[path = "hourly_rollup_archive_support.rs"]
mod hourly_rollup_archive_support;
pub(crate) use hourly_rollup_archive_support::*;

pub(crate) fn build_health_routes(router: Router<Arc<AppState>>) -> Router<Arc<AppState>> {
    router
        .route("/health", get(health_check))
        .route("/api/version", get(get_versions))
}

pub(crate) fn build_settings_routes(router: Router<Arc<AppState>>) -> Router<Arc<AppState>> {
    router
        .route("/api/settings", get(get_settings))
        .route(
            "/api/settings/external-api-keys",
            get(list_external_api_keys).post(create_external_api_key),
        )
        .route(
            "/api/settings/external-api-keys/:id/rotate",
            post(rotate_external_api_key),
        )
        .route(
            "/api/settings/external-api-keys/:id/disable",
            post(disable_external_api_key),
        )
        .route(
            "/api/settings/proxy-models",
            any(removed_proxy_model_settings_endpoint),
        )
        .route("/api/settings/proxy", put(put_proxy_settings))
        .route(
            "/api/settings/forward-proxy",
            put(put_forward_proxy_settings),
        )
        .route(
            "/api/settings/forward-proxy/validate",
            post(post_forward_proxy_candidate_validation),
        )
        .route(
            "/api/settings/forward-proxy/refresh-subscriptions",
            post(post_forward_proxy_refresh_subscriptions),
        )
        .route(
            "/api/settings/forward-proxy/nodes/:proxy_key/test-stream",
            get(stream_forward_proxy_node_latency_test),
        )
        .route(
            "/api/settings/forward-proxy/nodes/test-stream",
            get(stream_forward_proxy_nodes_latency_test),
        )
        .route("/api/settings/pricing", put(put_pricing_settings))
}

pub(crate) fn build_invocation_routes(router: Router<Arc<AppState>>) -> Router<Arc<AppState>> {
    router
        .route("/api/invocations", get(list_invocations))
        .route("/api/invocations/locate", get(locate_invocation))
        .route(
            "/api/invocations/:invoke_id/pool-attempts",
            get(fetch_invocation_pool_attempts),
        )
        .route(
            "/api/invocations/:id/detail",
            get(fetch_invocation_record_detail),
        )
        .route(
            "/api/invocations/:id/response-body",
            get(fetch_invocation_response_body),
        )
        .route("/api/invocations/summary", get(fetch_invocation_summary))
        .route(
            "/api/invocations/suggestions",
            get(fetch_invocation_suggestions),
        )
        .route(
            "/api/invocations/new-count",
            get(fetch_invocation_new_records_count),
        )
}

pub(crate) fn build_stats_routes(router: Router<Arc<AppState>>) -> Router<Arc<AppState>> {
    router
        .route("/api/stats", get(fetch_stats))
        .route("/api/stats/summary", get(fetch_summary))
        .route(
            "/api/stats/dashboard-activity",
            get(fetch_dashboard_activity),
        )
        .route(
            "/api/stats/upstream-account-activity",
            get(fetch_upstream_account_activity),
        )
        .route(
            "/api/stats/forward-proxy",
            get(fetch_forward_proxy_live_stats),
        )
        .route(
            "/api/stats/forward-proxy/timeseries",
            get(fetch_forward_proxy_timeseries),
        )
        .route("/api/stats/timeseries", get(fetch_timeseries))
        .route(
            "/api/stats/parallel-work",
            get(fetch_parallel_work_stats_cached),
        )
        .route("/api/stats/perf", get(fetch_perf_stats))
        .route("/api/stats/errors", get(fetch_error_distribution))
        .route("/api/stats/failures/summary", get(fetch_failure_summary))
        .route("/api/stats/errors/others", get(fetch_other_errors))
        .route(
            "/api/stats/prompt-cache-conversations",
            get(fetch_prompt_cache_conversations),
        )
        .route(
            "/api/stats/prompt-cache-conversation-bindings/*encodedPromptCacheKey",
            get(get_prompt_cache_conversation_binding)
                .patch(patch_prompt_cache_conversation_binding),
        )
        .route("/api/quota/latest", get(latest_quota_snapshot))
}

pub(crate) fn build_system_routes(router: Router<Arc<AppState>>) -> Router<Arc<AppState>> {
    router
        .route("/api/system/status", get(fetch_system_status))
        .route("/api/system/tasks", get(list_system_task_runs))
}

pub(crate) fn build_pool_routes(router: Router<Arc<AppState>>) -> Router<Arc<AppState>> {
    router
        .route(
            "/api/pool/routing-settings",
            get(get_pool_routing_settings).put(update_pool_routing_settings),
        )
        .route("/api/pool/tags", get(list_tags))
        .route(
            "/api/pool/forward-proxy-binding-nodes",
            get(list_forward_proxy_binding_nodes),
        )
        .route(
            "/api/pool/upstream-accounts",
            get(list_upstream_accounts_from_uri).post(bulk_update_upstream_accounts),
        )
        .route(
            "/api/pool/upstream-account-events",
            get(list_upstream_account_action_events),
        )
        .route(
            "/api/pool/upstream-accounts/window-usage",
            post(get_upstream_account_window_usage),
        )
        .route(
            "/api/pool/upstream-accounts/bulk-sync-jobs",
            post(create_bulk_upstream_account_sync_job),
        )
        .route(
            "/api/pool/upstream-accounts/bulk-sync-jobs/:jobId/events",
            get(stream_bulk_upstream_account_sync_job_events),
        )
        .route(
            "/api/pool/upstream-accounts/bulk-sync-jobs/:jobId",
            get(get_bulk_upstream_account_sync_job).delete(cancel_bulk_upstream_account_sync_job),
        )
        .route(
            "/api/pool/upstream-account-groups/*groupName",
            put(update_upstream_account_group).delete(delete_upstream_account_group),
        )
        .route(
            "/api/pool/upstream-accounts/:id/sticky-keys",
            get(get_upstream_account_sticky_keys),
        )
        .route(
            "/api/pool/upstream-accounts/:id",
            get(get_upstream_account)
                .patch(update_upstream_account)
                .delete(delete_upstream_account),
        )
        .route(
            "/api/pool/upstream-accounts/:id/sync",
            post(sync_upstream_account),
        )
        .route(
            "/api/pool/upstream-accounts/:id/oauth/relogin",
            post(relogin_upstream_account),
        )
        .route(
            "/api/pool/upstream-accounts/api-keys",
            post(create_api_key_account),
        )
        .route(
            "/api/pool/upstream-accounts/oauth/login-sessions",
            post(create_oauth_login_session),
        )
        .route(
            "/api/pool/upstream-accounts/oauth/imports/validate",
            post(validate_imported_oauth_accounts)
                .layer(DefaultBodyLimit::max(IMPORTED_OAUTH_ROUTE_MAX_BODY_BYTES)),
        )
        .route(
            "/api/pool/upstream-accounts/oauth/imports/validation-jobs",
            post(create_imported_oauth_validation_job)
                .layer(DefaultBodyLimit::max(IMPORTED_OAUTH_ROUTE_MAX_BODY_BYTES)),
        )
        .route(
            "/api/pool/upstream-accounts/oauth/imports/validation-jobs/:jobId/events",
            get(stream_imported_oauth_validation_job_events),
        )
        .route(
            "/api/pool/upstream-accounts/oauth/imports/validation-jobs/:jobId",
            delete(cancel_imported_oauth_validation_job),
        )
        .route(
            "/api/pool/upstream-accounts/oauth/imports",
            post(import_validated_oauth_accounts)
                .layer(DefaultBodyLimit::max(IMPORTED_OAUTH_ROUTE_MAX_BODY_BYTES)),
        )
        .route(
            "/api/pool/upstream-accounts/oauth/mailbox-sessions",
            post(create_oauth_mailbox_session),
        )
        .route(
            "/api/pool/upstream-accounts/oauth/mailbox-sessions/status",
            post(get_oauth_mailbox_session_status),
        )
        .route(
            "/api/pool/upstream-accounts/oauth/mailbox-sessions/:sessionId",
            delete(delete_oauth_mailbox_session),
        )
        .route(
            "/api/pool/upstream-accounts/oauth/login-sessions/:loginId",
            get(get_oauth_login_session).patch(update_oauth_login_session),
        )
        .route(
            "/api/pool/upstream-accounts/oauth/login-sessions/:loginId/complete",
            post(complete_oauth_login_session),
        )
        .route(
            "/api/pool/upstream-accounts/oauth/login-sessions/:loginId/confirm-identity-overwrite",
            post(confirm_oauth_login_session_identity_overwrite),
        )
        .route(
            "/api/pool/upstream-accounts/oauth/callback",
            get(oauth_callback),
        )
}

pub(crate) fn build_event_routes(router: Router<Arc<AppState>>) -> Router<Arc<AppState>> {
    router.route("/events", get(sse_stream))
}

pub(crate) fn build_external_routes(router: Router<Arc<AppState>>) -> Router<Arc<AppState>> {
    router
        .route(
            "/api/external/v1/upstream-accounts/oauth/:sourceAccountId",
            put(external_upsert_oauth_upstream_account_route)
                .patch(external_patch_oauth_upstream_account_route),
        )
        .route(
            "/api/external/v1/upstream-accounts/oauth/:sourceAccountId/relogin",
            post(external_relogin_oauth_upstream_account_route),
        )
}

pub(crate) fn build_proxy_routes(router: Router<Arc<AppState>>) -> Router<Arc<AppState>> {
    router.route("/v1/*path", any(proxy_openai_v1_with_connect_info))
}

pub(crate) fn build_app_router(state: Arc<AppState>) -> Router {
    build_proxy_routes(build_event_routes(build_external_routes(
        build_pool_routes(build_system_routes(build_stats_routes(
            build_invocation_routes(build_settings_routes(build_health_routes(Router::new()))),
        ))),
    )))
    .with_state(state)
}

pub(crate) const SOCIAL_PREVIEW_RELATIVE_ATTR: &str = "content=\"/social-preview.png\"";
pub(crate) const SOCIAL_PREVIEW_PATH: &str = "/social-preview.png";

pub(crate) fn inject_absolute_social_preview_urls(
    index_html: String,
    headers: &HeaderMap,
    configured_public_origin: Option<&str>,
) -> String {
    let Some(origin) = request_public_origin(headers, configured_public_origin) else {
        return index_html;
    };
    let absolute_attr = format!("content=\"{origin}{SOCIAL_PREVIEW_PATH}\"");
    index_html.replace(SOCIAL_PREVIEW_RELATIVE_ATTR, &absolute_attr)
}

pub(crate) async fn render_spa_index_response(
    state: Arc<AppState>,
    headers: &HeaderMap,
) -> Response {
    let Some(static_dir) = state.config.static_dir.as_ref() else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let index_file = static_dir.join("index.html");
    let index_html = match tokio::fs::read_to_string(&index_file).await {
        Ok(contents) => contents,
        Err(err) => {
            error!(path = %index_file.display(), ?err, "failed to read static index.html");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    Html(inject_absolute_social_preview_urls(
        index_html,
        headers,
        state.config.public_origin.as_deref(),
    ))
    .into_response()
}

pub(crate) async fn spawn_http_server(
    state: Arc<AppState>,
) -> Result<(SocketAddr, JoinHandle<()>)> {
    let cors_layer = build_cors_layer(&state.config);
    let mut router = build_app_router(state.clone())
        .layer(TraceLayer::new_for_http())
        .layer(cors_layer);

    // Optionally attach headers in the future; standard EventSource cannot read headers

    if let Some(static_dir) = state.config.static_dir.clone() {
        let index_file = static_dir.join("index.html");
        if index_file.exists() {
            let index_state = state.clone();
            let spa_index_service = service_fn(move |request: Request<Body>| {
                let state = index_state.clone();
                let headers = request.headers().clone();
                async move { Ok::<_, Infallible>(render_spa_index_response(state, &headers).await) }
            });
            let index_html_state = state.clone();
            let spa_index_html_service = service_fn(move |request: Request<Body>| {
                let state = index_html_state.clone();
                let headers = request.headers().clone();
                async move { Ok::<_, Infallible>(render_spa_index_response(state, &headers).await) }
            });
            let fallback_state = state.clone();
            let spa_fallback = service_fn(move |request: Request<Body>| {
                let state = fallback_state.clone();
                let headers = request.headers().clone();
                async move { Ok::<_, Infallible>(render_spa_index_response(state, &headers).await) }
            });
            let spa_service = ServeDir::new(static_dir).not_found_service(spa_fallback);
            router = router
                .route_service("/", spa_index_service)
                .route_service("/index.html", spa_index_html_service)
                .fallback_service(spa_service);
        } else {
            warn!(
                path = %index_file.display(),
                "static index.html not found; SPA fallback disabled"
            );
        }
    }

    let listener = TcpListener::bind(&state.config.http_bind).await?;
    let addr = listener.local_addr()?;
    info!(%addr, "http server listening");

    let shutdown = state.shutdown.clone();
    let handle = tokio::spawn(async move {
        if let Err(err) = serve_router_with_graceful_shutdown(listener, router, async move {
            shutdown.cancelled().await
        })
        .await
        {
            error!(?err, "http server exited with error");
        }
    });

    Ok((addr, handle))
}

#[cfg(test)]
mod social_preview_tests {
    use super::*;
    use axum::http::{HeaderValue, header};

    #[test]
    fn inject_absolute_social_preview_urls_rewrites_both_meta_tags() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::HOST,
            HeaderValue::from_static("monitor.example.com"),
        );
        headers.insert(
            header::HeaderName::from_static("x-forwarded-proto"),
            HeaderValue::from_static("https"),
        );
        let html = r#"
            <meta property="og:image" content="/social-preview.png" />
            <meta name="twitter:image" content="/social-preview.png" />
        "#
        .to_string();

        let rewritten = inject_absolute_social_preview_urls(html, &headers, None);

        assert!(rewritten.contains(r#"content="https://monitor.example.com/social-preview.png""#));
        assert_eq!(
            rewritten
                .matches("https://monitor.example.com/social-preview.png")
                .count(),
            2
        );
    }

    #[test]
    fn inject_absolute_social_preview_urls_prefers_configured_public_origin() {
        let mut headers = HeaderMap::new();
        headers.insert(header::HOST, HeaderValue::from_static("127.0.0.1:8080"));
        headers.insert(
            header::HeaderName::from_static("x-forwarded-host"),
            HeaderValue::from_static("monitor.example.com"),
        );
        let html = r#"
            <meta property="og:image" content="/social-preview.png" />
            <meta name="twitter:image" content="/social-preview.png" />
        "#
        .to_string();

        let rewritten = inject_absolute_social_preview_urls(
            html,
            &headers,
            Some("https://preview.example.com"),
        );

        assert!(rewritten.contains(r#"content="https://preview.example.com/social-preview.png""#));
    }
}

pub(crate) fn spawn_shutdown_signal_listener(cancel: CancellationToken) -> JoinHandle<()> {
    tokio::spawn(async move {
        shutdown_listener().await;
        cancel.cancel();
        info!("shutdown signal received; beginning graceful shutdown");
    })
}

pub(crate) async fn shutdown_listener() {
    // Wait for Ctrl+C or SIGTERM (unix)
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        let mut sigterm =
            signal(SignalKind::terminate()).expect("failed to install SIGTERM handler");
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {},
            _ = sigterm.recv() => {},
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}

pub(crate) struct SummaryPublish {
    pub(crate) window: String,
    pub(crate) summary: StatsResponse,
}

pub(crate) struct SummaryBroadcastSpec {
    window: &'static str,
    duration: Option<ChronoDuration>,
}

pub(crate) fn summary_broadcast_specs() -> Vec<SummaryBroadcastSpec> {
    vec![
        SummaryBroadcastSpec {
            window: "all",
            duration: None,
        },
        SummaryBroadcastSpec {
            window: "30m",
            duration: Some(ChronoDuration::minutes(30)),
        },
        SummaryBroadcastSpec {
            window: "1h",
            duration: Some(ChronoDuration::hours(1)),
        },
        SummaryBroadcastSpec {
            window: "1d",
            duration: Some(ChronoDuration::days(1)),
        },
        SummaryBroadcastSpec {
            window: "1mo",
            duration: Some(ChronoDuration::days(30)),
        },
    ]
}

pub(crate) async fn collect_summary_snapshots(
    pool: &Pool<Sqlite>,
    invocation_max_days: u64,
) -> Result<Vec<SummaryPublish>> {
    let mut summaries = Vec::new();
    let mut cached_all: Option<StatsResponse> = None;
    let now = Utc::now();
    let source_scope = resolve_default_source_scope(pool).await?;
    let in_progress_started_at = Instant::now();
    let in_progress_conversation_count =
        query_in_progress_prompt_cache_conversation_count(pool, source_scope, None).await?;
    let in_progress_elapsed_ms = in_progress_started_at.elapsed().as_millis() as u64;
    if in_progress_elapsed_ms >= 150 {
        warn!(
            endpoint = "summary_publish",
            window = "current",
            ?source_scope,
            upstream_account_id = Option::<i64>::None,
            selected_key_count = 0_i64,
            row_count = in_progress_conversation_count,
            cache_hit_or_miss = "maintenance_live_distinct_count",
            elapsed_ms = in_progress_elapsed_ms,
            "summary publish in-progress distinct-count exceeded slow-path threshold"
        );
    } else {
        debug!(
            endpoint = "summary_publish",
            window = "current",
            ?source_scope,
            upstream_account_id = Option::<i64>::None,
            selected_key_count = 0_i64,
            row_count = in_progress_conversation_count,
            cache_hit_or_miss = "maintenance_live_distinct_count",
            elapsed_ms = in_progress_elapsed_ms,
            "summary publish in-progress distinct-count completed"
        );
    }

    for spec in summary_broadcast_specs() {
        let mut summary = match spec.duration {
            None => {
                if let Some(existing) = &cached_all {
                    existing.clone()
                } else {
                    let stats = query_combined_totals(pool, StatsFilter::All, source_scope)
                        .await?
                        .into_response();
                    cached_all = Some(stats.clone());
                    stats
                }
            }
            Some(duration) => {
                let start = now - duration;
                query_hourly_backed_summary_since_with_config(
                    pool,
                    invocation_max_days,
                    start,
                    source_scope,
                )
                .await
                .map_err(|err| anyhow!("{err:?}"))?
                .into_response()
            }
        };
        summary.in_progress_conversation_count = Some(in_progress_conversation_count);

        summaries.push(SummaryPublish {
            window: spec.window.to_string(),
            summary,
        });
    }

    Ok(summaries)
}

pub(crate) fn codex_invocations_create_sql(table_name: &str) -> String {
    format!(
        r#"
        CREATE TABLE IF NOT EXISTS {table_name} (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            invoke_id TEXT NOT NULL,
            occurred_at TEXT NOT NULL,
            source TEXT NOT NULL DEFAULT 'xy',
            model TEXT,
            input_tokens INTEGER,
            output_tokens INTEGER,
            cache_input_tokens INTEGER,
            reasoning_tokens INTEGER,
            total_tokens INTEGER,
            cost REAL,
            cost_input REAL,
            cost_cache_write REAL,
            cost_cache_read REAL,
            cost_output REAL,
            cost_reasoning REAL,
            status TEXT,
            error_message TEXT,
            failure_kind TEXT,
            failure_class TEXT,
            is_actionable INTEGER NOT NULL DEFAULT 0,
            payload TEXT,
            raw_response TEXT NOT NULL,
            cost_estimated INTEGER NOT NULL DEFAULT 0,
            price_version TEXT,
            request_raw_path TEXT,
            request_raw_codec TEXT NOT NULL DEFAULT 'identity',
            request_raw_size INTEGER,
            request_raw_truncated INTEGER NOT NULL DEFAULT 0,
            request_raw_truncated_reason TEXT,
            response_raw_path TEXT,
            response_raw_codec TEXT NOT NULL DEFAULT 'identity',
            response_raw_size INTEGER,
            response_raw_truncated INTEGER NOT NULL DEFAULT 0,
            response_raw_truncated_reason TEXT,
            detail_level TEXT NOT NULL DEFAULT 'full',
            detail_pruned_at TEXT,
            detail_prune_reason TEXT,
            t_total_ms REAL,
            t_req_read_ms REAL,
            t_req_parse_ms REAL,
            t_upstream_connect_ms REAL,
            t_upstream_ttfb_ms REAL,
            t_upstream_stream_ms REAL,
            t_resp_parse_ms REAL,
            t_persist_ms REAL,
            created_at TEXT NOT NULL DEFAULT (STRFTIME('%Y-%m-%dT%H:%M:%fZ', 'now')),
            UNIQUE(invoke_id, occurred_at)
        )
        "#,
        table_name = table_name,
    )
}

pub(crate) async fn load_sqlite_table_columns(
    pool: &Pool<Sqlite>,
    table_name: &str,
) -> Result<HashSet<String>> {
    let pragma = format!("PRAGMA table_info('{table_name}')");
    let columns = sqlx::query(&pragma)
        .fetch_all(pool)
        .await
        .with_context(|| format!("failed to inspect {table_name} schema"))?
        .into_iter()
        .filter_map(|row| row.try_get::<String, _>("name").ok())
        .collect::<HashSet<_>>();
    Ok(columns)
}

pub(crate) async fn load_sqlite_table_columns_from_connection(
    conn: &mut SqliteConnection,
    schema_name: Option<&str>,
    table_name: &str,
) -> Result<HashSet<String>> {
    let pragma = schema_name.map_or_else(
        || format!("PRAGMA table_info('{table_name}')"),
        |schema_name| format!("PRAGMA {schema_name}.table_info('{table_name}')"),
    );
    let columns = sqlx::query(&pragma)
        .fetch_all(&mut *conn)
        .await
        .with_context(|| format!("failed to inspect {table_name} schema"))?
        .into_iter()
        .filter_map(|row| row.try_get::<String, _>("name").ok())
        .collect::<HashSet<_>>();
    Ok(columns)
}

pub(crate) async fn ensure_pool_upstream_request_attempts_archive_schema(
    conn: &mut SqliteConnection,
) -> Result<()> {
    let archive_columns = load_sqlite_table_columns_from_connection(
        conn,
        Some("archive_db"),
        "pool_upstream_request_attempts",
    )
    .await?;
    for (column, ty) in [
        ("upstream_route_key", "TEXT"),
        ("phase", "TEXT"),
        ("downstream_http_status", "INTEGER"),
        ("downstream_error_message", "TEXT"),
        ("compact_support_status", "TEXT"),
        ("compact_support_reason", "TEXT"),
        ("group_name_snapshot", "TEXT"),
        ("proxy_binding_key_snapshot", "TEXT"),
    ] {
        if !archive_columns.contains(column) {
            let statement = format!(
                "ALTER TABLE archive_db.pool_upstream_request_attempts ADD COLUMN {column} {ty}"
            );
            sqlx::query(&statement)
                .execute(&mut *conn)
                .await
                .with_context(|| {
                    format!(
                        "failed to add archive_db.pool_upstream_request_attempts column {column}"
                    )
                })?;
        }
    }
    Ok(())
}

pub(crate) async fn ensure_pool_upstream_request_attempts_archive_schema_in_place(
    conn: &mut SqliteConnection,
) -> Result<()> {
    let archive_columns =
        load_sqlite_table_columns_from_connection(conn, None, "pool_upstream_request_attempts")
            .await?;
    for (column, ty) in [
        ("upstream_route_key", "TEXT"),
        ("phase", "TEXT"),
        ("downstream_http_status", "INTEGER"),
        ("downstream_error_message", "TEXT"),
        ("compact_support_status", "TEXT"),
        ("compact_support_reason", "TEXT"),
        ("group_name_snapshot", "TEXT"),
        ("proxy_binding_key_snapshot", "TEXT"),
    ] {
        if !archive_columns.contains(column) {
            let statement =
                format!("ALTER TABLE pool_upstream_request_attempts ADD COLUMN {column} {ty}");
            sqlx::query(&statement)
                .execute(&mut *conn)
                .await
                .with_context(|| {
                    format!(
                        "failed to add in-place pool_upstream_request_attempts archive column {column}"
                    )
                })?;
        }
    }
    Ok(())
}

pub(crate) async fn ensure_codex_invocations_archive_schema(
    conn: &mut SqliteConnection,
) -> Result<()> {
    let archive_columns =
        load_sqlite_table_columns_from_connection(conn, Some("archive_db"), "codex_invocations")
            .await?;
    for (column, ty) in [
        ("request_raw_codec", "TEXT NOT NULL DEFAULT 'identity'"),
        ("response_raw_codec", "TEXT NOT NULL DEFAULT 'identity'"),
        ("cost_input", "REAL"),
        ("cost_cache_write", "REAL"),
        ("cost_cache_read", "REAL"),
        ("cost_output", "REAL"),
        ("cost_reasoning", "REAL"),
    ] {
        if !archive_columns.contains(column) {
            let statement =
                format!("ALTER TABLE archive_db.codex_invocations ADD COLUMN {column} {ty}");
            sqlx::query(&statement)
                .execute(&mut *conn)
                .await
                .with_context(|| {
                    format!("failed to add archive_db.codex_invocations column {column}")
                })?;
        }
    }
    sqlx::query(
        r#"
        UPDATE archive_db.codex_invocations
        SET request_raw_codec = CASE
                WHEN request_raw_path IS NOT NULL AND request_raw_path LIKE '%.gz' THEN 'gzip'
                ELSE 'identity'
            END
        WHERE COALESCE(TRIM(request_raw_codec), '') = ''
           OR (request_raw_codec = 'identity' AND request_raw_path LIKE '%.gz')
        "#,
    )
    .execute(&mut *conn)
    .await
    .context("failed to backfill archive_db.codex_invocations request_raw_codec")?;
    sqlx::query(
        r#"
        UPDATE archive_db.codex_invocations
        SET response_raw_codec = CASE
                WHEN response_raw_path IS NOT NULL AND response_raw_path LIKE '%.gz' THEN 'gzip'
                ELSE 'identity'
            END
        WHERE COALESCE(TRIM(response_raw_codec), '') = ''
           OR (response_raw_codec = 'identity' AND response_raw_path LIKE '%.gz')
        "#,
    )
    .execute(&mut *conn)
    .await
    .context("failed to backfill archive_db.codex_invocations response_raw_codec")?;
    Ok(())
}

pub(crate) async fn migrate_codex_invocations_drop_raw_expires_at(
    pool: &Pool<Sqlite>,
    existing: &HashSet<String>,
) -> Result<()> {
    const TEMP_TABLE: &str = "codex_invocations_drop_raw_expires_at_new";

    let mut tx = pool.begin().await?;
    let drop_temp_sql = format!("DROP TABLE IF EXISTS {TEMP_TABLE}");
    sqlx::query(&drop_temp_sql)
        .execute(tx.as_mut())
        .await
        .context("failed to clear stale codex_invocations migration temp table")?;
    let create_temp_sql = codex_invocations_create_sql(TEMP_TABLE);
    sqlx::query(&create_temp_sql)
        .execute(tx.as_mut())
        .await
        .context("failed to create codex_invocations migration temp table")?;

    let temp_pragma_sql = format!("PRAGMA table_info('{TEMP_TABLE}')");
    let new_columns: Vec<String> = sqlx::query(&temp_pragma_sql)
        .fetch_all(tx.as_mut())
        .await
        .context("failed to inspect codex_invocations migration temp schema")?
        .into_iter()
        .filter_map(|row| row.try_get::<String, _>("name").ok())
        .collect();
    let copy_columns: Vec<String> = new_columns
        .into_iter()
        .filter(|column| existing.contains(column))
        .collect();
    if copy_columns.is_empty() {
        bail!("codex_invocations migration found no shared columns to copy");
    }

    let copy_columns_csv = copy_columns.join(", ");
    let copy_sql = format!(
        "INSERT INTO {TEMP_TABLE} ({copy_columns_csv}) SELECT {copy_columns_csv} FROM codex_invocations"
    );
    sqlx::query(&copy_sql)
        .execute(tx.as_mut())
        .await
        .context("failed to copy codex_invocations rows into migration temp table")?;
    sqlx::query("DROP TABLE codex_invocations")
        .execute(tx.as_mut())
        .await
        .context("failed to drop legacy codex_invocations table during migration")?;
    let rename_sql = format!("ALTER TABLE {TEMP_TABLE} RENAME TO codex_invocations");
    sqlx::query(&rename_sql)
        .execute(tx.as_mut())
        .await
        .context("failed to swap migrated codex_invocations table into place")?;
    tx.commit().await?;
    Ok(())
}
