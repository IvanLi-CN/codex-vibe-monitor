async fn sync_hourly_rollups_from_live_tables(pool: &Pool<Sqlite>) -> Result<()> {
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

const HISTORICAL_ROLLUP_ARCHIVE_REPLAY_BATCH_SIZE: i64 = BACKFILL_BATCH_SIZE;
const HISTORICAL_ROLLUP_ARCHIVE_INFLATE_BUFFER_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HistoricalRollupArchiveReplayOutcome {
    Completed,
    HitBudget,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct HistoricalRollupArchiveReplayResult {
    outcome: HistoricalRollupArchiveReplayOutcome,
    cursor_id: i64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct HistoricalRollupArchiveReplaySummary {
    scanned_batches: u64,
    skipped_batches: u64,
    remaining_skip_batches: usize,
    budget_consumed_batches: u64,
    blocked_batches: u64,
    materialized_batches: u64,
}

#[derive(Debug, FromRow)]
struct HistoricalRollupArchiveCoverageBoundsRow {
    coverage_start_at: Option<String>,
    coverage_end_at: Option<String>,
}

fn historical_rollup_elapsed_budget_reached(
    started_at: Instant,
    max_elapsed: Option<Duration>,
) -> bool {
    max_elapsed.is_some_and(|limit| started_at.elapsed() >= limit)
}

fn historical_rollup_materialization_budget_reached(
    started_at: Instant,
    replayed: u64,
    max_archive_batches: Option<u64>,
    max_elapsed: Option<Duration>,
) -> bool {
    max_archive_batches.is_some_and(|limit| replayed >= limit)
        || historical_rollup_elapsed_budget_reached(started_at, max_elapsed)
}

#[cfg(test)]
fn inflate_gzip_sqlite_file_with_budget(
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

async fn open_historical_rollup_archive_pool(
    archive_path: &Path,
    temp_path: &Path,
) -> Result<Pool<Sqlite>> {
    if !temp_path.exists() {
        inflate_gzip_sqlite_file(archive_path, temp_path)?;
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
            let _ = fs::remove_file(temp_path);
            inflate_gzip_sqlite_file(archive_path, temp_path)?;
            connect().await.with_context(|| {
                format!(
                    "failed to reopen archive batch {} after resetting temp db (initial error: {first_err})",
                    archive_path.display()
                )
            })
        }
    }
}

async fn load_invocation_archive_rows_chunk(
    archive_pool: &Pool<Sqlite>,
    start_after_id: i64,
) -> Result<(Vec<InvocationHourlySourceRecord>, bool)> {
    let mut rows = sqlx::query_as::<_, InvocationHourlySourceRecord>(
        r#"
        SELECT
            id,
            occurred_at,
            source,
            status,
            detail_level,
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

async fn load_forward_proxy_archive_rows_chunk(
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

async fn load_archive_coverage_bounds(
    archive_pool: &Pool<Sqlite>,
    table_name: &str,
) -> Result<HistoricalRollupArchiveCoverageBoundsRow> {
    Ok(sqlx::query_as::<_, HistoricalRollupArchiveCoverageBoundsRow>(&format!(
        r#"
        SELECT
            MIN(occurred_at) AS coverage_start_at,
            MAX(occurred_at) AS coverage_end_at
        FROM {table_name}
        "#
    ))
    .fetch_one(archive_pool)
    .await?)
}

async fn invocation_archive_has_pruned_success_details_in_db(
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

async fn replay_invocation_archive_rows_into_hourly_rollups_tx_with_budget(
    tx: &mut SqliteConnection,
    archive_pool: &Pool<Sqlite>,
    initial_cursor_id: i64,
    pending_targets: &[&str],
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

        let (rows, has_more) = load_invocation_archive_rows_chunk(archive_pool, start_after_id).await?;
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

async fn replay_forward_proxy_archive_rows_into_hourly_rollups_tx_with_budget(
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

        let (rows, has_more) = load_forward_proxy_archive_rows_chunk(archive_pool, start_after_id).await?;
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

async fn replay_invocation_archives_into_hourly_rollups_tx_with_limits(
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

async fn replay_invocation_archives_into_hourly_rollups_tx(
    tx: &mut SqliteConnection,
) -> Result<HistoricalRollupArchiveReplaySummary> {
    replay_invocation_archives_into_hourly_rollups_tx_with_limits(
        tx,
        Instant::now(),
        None,
        None,
        0,
    )
    .await
}

async fn replay_forward_proxy_archives_into_hourly_rollups_tx_with_limits(
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
            let bounds = load_archive_coverage_bounds(&archive_pool, "forward_proxy_attempts").await?;
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
        assert!(!historical_rollup_elapsed_budget_reached(Instant::now(), None));
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
        assert!(written > 0, "budgeted inflate should still write at least one chunk");
        assert!(
            written < payload.len(),
            "expired elapsed budget should stop before the whole sqlite copy completes"
        );

        let _ = fs::remove_dir_all(&temp_dir);
    }
}

async fn replay_forward_proxy_archives_into_hourly_rollups_tx(
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

async fn bootstrap_hourly_rollups(pool: &Pool<Sqlite>) -> Result<()> {
    sync_hourly_rollups_from_live_tables(pool).await?;
    Ok(())
}

async fn refresh_hourly_rollups_for_read_surfaces(pool: &Pool<Sqlite>) -> Result<()> {
    sync_hourly_rollups_from_live_tables(pool).await?;
    ensure_invocation_summary_rollups_ready_best_effort(pool).await?;
    Ok(())
}

async fn ensure_hourly_rollups_caught_up(state: &AppState) -> Result<()> {
    let _guard = state.hourly_rollup_sync_lock.lock().await;
    sync_hourly_rollups_from_live_tables(&state.pool).await
}

pub(crate) async fn refresh_hourly_rollups_for_read_surfaces_best_effort(
    pool: &Pool<Sqlite>,
    hourly_rollup_sync_lock: &Mutex<()>,
    reason: &'static str,
) {
    let _guard = hourly_rollup_sync_lock.lock().await;
    if let Err(err) = refresh_hourly_rollups_for_read_surfaces(pool).await {
        warn!(
            error = %err,
            reason,
            "background hourly rollup refresh failed; keeping existing rollups for read surfaces"
        );
    }
}

async fn delete_rows_by_ids(
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

async fn sweep_orphan_proxy_raw_files(
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

async fn schedule_poll(
    state: Arc<AppState>,
    cancel: &CancellationToken,
) -> Result<Option<JoinHandle<()>>> {
    let permit = tokio::select! {
        _ = cancel.cancelled() => return Ok(None),
        permit = state.semaphore.clone().acquire_owned() => {
            permit.context("failed to acquire scheduler permit")?
        }
    };
    if cancel.is_cancelled() {
        return Ok(None);
    }

    let in_flight = state
        .config
        .max_parallel_polls
        .saturating_sub(state.semaphore.available_permits());
    let force_new_connection = in_flight > state.config.shared_connection_parallelism;
    let state_clone = state.clone();

    let handle = tokio::spawn(async move {
        let collect_broadcast_state = state_clone.broadcaster.receiver_count() > 0;
        let fut = fetch_and_store(&state_clone, force_new_connection, collect_broadcast_state);
        match timeout(state_clone.config.request_timeout, fut).await {
            Ok(Ok(publish)) => {
                let PublishResult {
                    mut summaries,
                    mut quota_snapshot,
                    collected_broadcast_state,
                } = publish;

                let receiver_count = state_clone.broadcaster.receiver_count();
                if should_collect_late_broadcast_state(receiver_count, collected_broadcast_state) {
                    match collect_broadcast_state_snapshots(
                        &state_clone.pool,
                        state_clone.config.crs_stats.as_ref(),
                        state_clone.config.invocation_max_days,
                    )
                    .await
                    {
                        Ok((latest_summaries, latest_quota_snapshot)) => {
                            summaries = latest_summaries;
                            quota_snapshot = latest_quota_snapshot;
                        }
                        Err(err) => {
                            warn!(?err, "failed to collect late-subscriber broadcast state");
                        }
                    }
                }

                for summary in summaries {
                    if let Err(err) = broadcast_summary_if_changed(
                        &state_clone.broadcaster,
                        state_clone.broadcast_state_cache.as_ref(),
                        &summary.window,
                        summary.summary,
                    )
                    .await
                    {
                        warn!(?err, "failed to broadcast summary payload");
                    }
                }

                if let Some(snapshot) = quota_snapshot
                    && let Err(err) = broadcast_quota_if_changed(
                        &state_clone.broadcaster,
                        state_clone.broadcast_state_cache.as_ref(),
                        snapshot,
                    )
                    .await
                {
                    warn!(?err, "failed to broadcast quota snapshot");
                }
            }
            Ok(Err(err)) => {
                warn!(?err, "poll execution failed");
            }
            Err(_) => {
                warn!("scheduler fetch timed out");
            }
        }

        drop(permit);
    });

    Ok(Some(handle))
}

fn build_health_routes(router: Router<Arc<AppState>>) -> Router<Arc<AppState>> {
    router
        .route("/health", get(health_check))
        .route("/api/version", get(get_versions))
}

fn build_settings_routes(router: Router<Arc<AppState>>) -> Router<Arc<AppState>> {
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
        .route(
            "/api/settings/proxy",
            any(removed_proxy_model_settings_endpoint),
        )
        .route(
            "/api/settings/forward-proxy",
            put(put_forward_proxy_settings),
        )
        .route(
            "/api/settings/forward-proxy/validate",
            post(post_forward_proxy_candidate_validation),
        )
        .route("/api/settings/pricing", put(put_pricing_settings))
}

fn build_invocation_routes(router: Router<Arc<AppState>>) -> Router<Arc<AppState>> {
    router
        .route("/api/invocations", get(list_invocations))
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

fn build_stats_routes(router: Router<Arc<AppState>>) -> Router<Arc<AppState>> {
    router
        .route("/api/stats", get(fetch_stats))
        .route("/api/stats/summary", get(fetch_summary))
        .route(
            "/api/stats/forward-proxy",
            get(fetch_forward_proxy_live_stats),
        )
        .route(
            "/api/stats/forward-proxy/timeseries",
            get(fetch_forward_proxy_timeseries),
        )
        .route("/api/stats/timeseries", get(fetch_timeseries))
        .route("/api/stats/parallel-work", get(fetch_parallel_work_stats))
        .route("/api/stats/perf", get(fetch_perf_stats))
        .route("/api/stats/errors", get(fetch_error_distribution))
        .route("/api/stats/failures/summary", get(fetch_failure_summary))
        .route("/api/stats/errors/others", get(fetch_other_errors))
        .route(
            "/api/stats/prompt-cache-conversations",
            get(fetch_prompt_cache_conversations),
        )
        .route("/api/quota/latest", get(latest_quota_snapshot))
}

fn build_pool_routes(router: Router<Arc<AppState>>) -> Router<Arc<AppState>> {
    router
        .route(
            "/api/pool/routing-settings",
            get(get_pool_routing_settings).put(update_pool_routing_settings),
        )
        .route("/api/pool/tags", get(list_tags).post(create_tag))
        .route(
            "/api/pool/tags/:id",
            get(get_tag).patch(update_tag).delete(delete_tag),
        )
        .route(
            "/api/pool/forward-proxy-binding-nodes",
            get(list_forward_proxy_binding_nodes),
        )
        .route(
            "/api/pool/upstream-accounts",
            get(list_upstream_accounts_from_uri).post(bulk_update_upstream_accounts),
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
            put(update_upstream_account_group),
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
            "/api/pool/upstream-accounts/oauth/callback",
            get(oauth_callback),
        )
}

fn build_event_routes(router: Router<Arc<AppState>>) -> Router<Arc<AppState>> {
    router.route("/events", get(sse_stream))
}

fn build_external_routes(router: Router<Arc<AppState>>) -> Router<Arc<AppState>> {
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

fn build_proxy_routes(router: Router<Arc<AppState>>) -> Router<Arc<AppState>> {
    router.route("/v1/*path", any(proxy_openai_v1_with_connect_info))
}

async fn spawn_http_server(state: Arc<AppState>) -> Result<(SocketAddr, JoinHandle<()>)> {
    let cors_layer = build_cors_layer(&state.config);
    let mut router = build_proxy_routes(build_event_routes(build_external_routes(
        build_pool_routes(build_stats_routes(build_invocation_routes(build_settings_routes(
            build_health_routes(Router::new()),
        )))),
    )))
    .with_state(state.clone())
    .layer(TraceLayer::new_for_http())
    .layer(cors_layer);

    // Optionally attach headers in the future; standard EventSource cannot read headers

    if let Some(static_dir) = state.config.static_dir.clone() {
        let index_file = static_dir.join("index.html");
        if index_file.exists() {
            let spa_service =
                ServeDir::new(static_dir).not_found_service(ServeFile::new(index_file));
            router = router.fallback_service(spa_service);
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
        if let Err(err) = axum::serve(
            listener,
            router.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .with_graceful_shutdown(async move { shutdown.cancelled().await })
        .await
        {
            error!(?err, "http server exited with error");
        }
    });

    Ok((addr, handle))
}

fn spawn_shutdown_signal_listener(cancel: CancellationToken) -> JoinHandle<()> {
    tokio::spawn(async move {
        shutdown_listener().await;
        cancel.cancel();
        info!("shutdown signal received; beginning graceful shutdown");
    })
}

async fn shutdown_listener() {
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

struct PublishResult {
    summaries: Vec<SummaryPublish>,
    quota_snapshot: Option<QuotaSnapshotResponse>,
    collected_broadcast_state: bool,
}

struct SummaryPublish {
    window: String,
    summary: StatsResponse,
}

fn should_collect_late_broadcast_state(
    receiver_count: usize,
    collected_broadcast_state: bool,
) -> bool {
    receiver_count > 0 && !collected_broadcast_state
}

async fn collect_broadcast_state_snapshots(
    pool: &Pool<Sqlite>,
    relay: Option<&CrsStatsConfig>,
    invocation_max_days: u64,
) -> Result<(Vec<SummaryPublish>, Option<QuotaSnapshotResponse>)> {
    Ok((
        collect_summary_snapshots(pool, relay, invocation_max_days).await?,
        QuotaSnapshotResponse::fetch_latest(pool).await?,
    ))
}

async fn fetch_and_store(
    state: &AppState,
    force_new_connection: bool,
    collect_broadcast_state: bool,
) -> Result<PublishResult> {
    let client = state
        .http_clients
        .client_for_parallelism(force_new_connection)?;
    let relay_config = state.config.crs_stats.clone();

    if let Some(relay) = relay_config.as_ref()
        && should_poll_crs_stats(&state.pool, relay).await?
    {
        match fetch_crs_stats(&client, relay).await {
            Ok(payload) => {
                if let Err(err) = persist_crs_stats(&state.pool, relay, payload).await {
                    warn!(?err, "failed to persist crs stats");
                }
            }
            Err(err) => {
                warn!(?err, "failed to fetch crs stats");
            }
        }
    }

    let (summaries, quota_payload) = if collect_broadcast_state {
        collect_broadcast_state_snapshots(
            &state.pool,
            relay_config.as_ref(),
            state.config.invocation_max_days,
        )
        .await?
    } else {
        (Vec::new(), None)
    };

    Ok(PublishResult {
        summaries,
        quota_snapshot: quota_payload,
        collected_broadcast_state: collect_broadcast_state,
    })
}

struct SummaryBroadcastSpec {
    window: &'static str,
    duration: Option<ChronoDuration>,
}

fn summary_broadcast_specs() -> Vec<SummaryBroadcastSpec> {
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

async fn collect_summary_snapshots(
    pool: &Pool<Sqlite>,
    relay: Option<&CrsStatsConfig>,
    invocation_max_days: u64,
) -> Result<Vec<SummaryPublish>> {
    let mut summaries = Vec::new();
    let mut cached_all: Option<StatsResponse> = None;
    let now = Utc::now();
    let source_scope = resolve_default_source_scope(pool).await?;

    for spec in summary_broadcast_specs() {
        let summary = match spec.duration {
            None => {
                if let Some(existing) = &cached_all {
                    existing.clone()
                } else {
                    let stats = query_combined_totals(pool, relay, StatsFilter::All, source_scope)
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
                    relay,
                    invocation_max_days,
                    start,
                    source_scope,
                )
                .await
                .map_err(|err| anyhow!("{err:?}"))?
                .into_response()
            }
        };

        summaries.push(SummaryPublish {
            window: spec.window.to_string(),
            summary,
        });
    }

    Ok(summaries)
}

async fn should_poll_crs_stats(pool: &Pool<Sqlite>, relay: &CrsStatsConfig) -> Result<bool> {
    let last_epoch = sqlx::query_scalar::<_, i64>(
        r#"
        SELECT captured_at_epoch
        FROM stats_source_snapshots
        WHERE source = ?1 AND period = ?2 AND model IS NULL
        ORDER BY captured_at_epoch DESC
        LIMIT 1
        "#,
    )
    .bind(SOURCE_CRS)
    .bind(&relay.period)
    .fetch_optional(pool)
    .await?;

    let now_epoch = Utc::now().timestamp();
    Ok(match last_epoch {
        Some(last) => now_epoch.saturating_sub(last) >= relay.poll_interval.as_secs() as i64,
        None => true,
    })
}

async fn fetch_crs_stats(client: &Client, relay: &CrsStatsConfig) -> Result<CrsStatsResponse> {
    let url = relay
        .base_url
        .join("apiStats/api/user-model-stats")
        .context("failed to join crs stats endpoint")?;
    let payload = json!({
        "apiId": relay.api_id,
        "period": relay.period,
    });

    let response = client
        .post(url)
        .json(&payload)
        .send()
        .await
        .context("failed to send crs stats request")?
        .error_for_status()
        .context("crs stats request returned error status")?;

    let payload: CrsStatsResponse = response
        .json()
        .await
        .context("failed to decode crs stats JSON")?;

    if !payload.success {
        return Err(anyhow!("crs stats responded with success=false"));
    }

    Ok(payload)
}

fn aggregate_crs_totals(models: &[CrsModelStats]) -> CrsTotals {
    let mut totals = CrsTotals::default();
    for model in models {
        totals.total_count += model.requests;
        totals.total_tokens += model.all_tokens;
        totals.total_cost += model.costs.total;
        totals.input_tokens += model.input_tokens;
        totals.output_tokens += model.output_tokens;
        totals.cache_create_tokens += model.cache_create_tokens;
        totals.cache_read_tokens += model.cache_read_tokens;
        totals.cost_input += model.costs.input;
        totals.cost_output += model.costs.output;
        totals.cost_cache_write += model.costs.cache_write;
        totals.cost_cache_read += model.costs.cache_read;
    }
    totals
}

#[derive(Debug, FromRow)]
struct CrsMaxRow {
    max_requests: Option<i64>,
    max_all_tokens: Option<i64>,
    max_cost_total: Option<f64>,
}

fn compute_crs_delta(
    stats_date: &str,
    now_utc: DateTime<Utc>,
    totals: CrsTotals,
    prev: CrsMaxRow,
) -> StatsTotals {
    let max_requests = prev.max_requests.unwrap_or(0);
    let max_tokens = prev.max_all_tokens.unwrap_or(0);
    let max_cost = prev.max_cost_total.unwrap_or(0.0);

    if totals.total_count < max_requests {
        if totals.total_count == 0 {
            let local = now_utc.with_timezone(&Shanghai);
            error!(
                stats_date,
                now = %local.to_rfc3339(),
                current = totals.total_count,
                previous_max = max_requests,
                "crs stats reset to zero outside day boundary"
            );
        } else {
            warn!(
                stats_date,
                current = totals.total_count,
                previous_max = max_requests,
                "crs stats total decreased; keeping daily max"
            );
        }
    }

    let delta_count = if totals.total_count > max_requests {
        totals.total_count - max_requests
    } else {
        0
    };
    let delta_tokens = if totals.total_tokens > max_tokens {
        totals.total_tokens - max_tokens
    } else {
        0
    };
    let delta_cost = if totals.total_cost > max_cost {
        totals.total_cost - max_cost
    } else {
        0.0
    };

    StatsTotals {
        total_count: delta_count,
        success_count: delta_count,
        failure_count: 0,
        total_tokens: delta_tokens,
        total_cost: delta_cost,
    }
}

async fn persist_crs_stats(
    pool: &Pool<Sqlite>,
    relay: &CrsStatsConfig,
    payload: CrsStatsResponse,
) -> Result<Option<StatsTotals>> {
    let now_utc = Utc::now();
    let captured_at = format_naive(now_utc.naive_utc());
    let captured_at_epoch = now_utc.timestamp();
    let stats_date = now_utc
        .with_timezone(&Shanghai)
        .date_naive()
        .format("%Y-%m-%d")
        .to_string();

    let period = if payload.period.is_empty() {
        relay.period.clone()
    } else {
        payload.period.clone()
    };

    if period != relay.period {
        warn!(
            expected = %relay.period,
            actual = %period,
            "crs stats period mismatch; using response period"
        );
    }

    let totals = aggregate_crs_totals(&payload.data);
    let raw_response = serde_json::to_string(&payload)?;

    let mut tx = pool.begin().await?;
    let prev = sqlx::query_as::<_, CrsMaxRow>(
        r#"
        SELECT
            MAX(requests) AS max_requests,
            MAX(all_tokens) AS max_all_tokens,
            MAX(cost_total) AS max_cost_total
        FROM stats_source_snapshots
        WHERE source = ?1 AND period = ?2 AND stats_date = ?3 AND model IS NULL
        "#,
    )
    .bind(SOURCE_CRS)
    .bind(&period)
    .bind(&stats_date)
    .fetch_one(&mut *tx)
    .await?;

    for model in &payload.data {
        sqlx::query(
            r#"
            INSERT OR IGNORE INTO stats_source_snapshots (
                source,
                period,
                stats_date,
                model,
                requests,
                input_tokens,
                output_tokens,
                cache_create_tokens,
                cache_read_tokens,
                all_tokens,
                cost_input,
                cost_output,
                cost_cache_write,
                cost_cache_read,
                cost_total,
                raw_response,
                captured_at,
                captured_at_epoch
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18)
            "#,
        )
        .bind(SOURCE_CRS)
        .bind(&period)
        .bind(&stats_date)
        .bind(&model.model)
        .bind(model.requests)
        .bind(model.input_tokens)
        .bind(model.output_tokens)
        .bind(model.cache_create_tokens)
        .bind(model.cache_read_tokens)
        .bind(model.all_tokens)
        .bind(model.costs.input)
        .bind(model.costs.output)
        .bind(model.costs.cache_write)
        .bind(model.costs.cache_read)
        .bind(model.costs.total)
        .bind(Option::<String>::None)
        .bind(&captured_at)
        .bind(captured_at_epoch)
        .execute(&mut *tx)
        .await?;
    }

    sqlx::query(
        r#"
        INSERT OR IGNORE INTO stats_source_snapshots (
            source,
            period,
            stats_date,
            model,
            requests,
            input_tokens,
            output_tokens,
            cache_create_tokens,
            cache_read_tokens,
            all_tokens,
            cost_input,
            cost_output,
            cost_cache_write,
            cost_cache_read,
            cost_total,
            raw_response,
            captured_at,
            captured_at_epoch
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18)
        "#,
    )
    .bind(SOURCE_CRS)
    .bind(&period)
    .bind(&stats_date)
    .bind(Option::<String>::None)
    .bind(totals.total_count)
    .bind(totals.input_tokens)
    .bind(totals.output_tokens)
    .bind(totals.cache_create_tokens)
    .bind(totals.cache_read_tokens)
    .bind(totals.total_tokens)
    .bind(totals.cost_input)
    .bind(totals.cost_output)
    .bind(totals.cost_cache_write)
    .bind(totals.cost_cache_read)
    .bind(totals.total_cost)
    .bind(raw_response)
    .bind(&captured_at)
    .bind(captured_at_epoch)
    .execute(&mut *tx)
    .await?;

    let delta = compute_crs_delta(&stats_date, now_utc, totals, prev);
    let has_delta = delta.total_count > 0 || delta.total_tokens > 0 || delta.total_cost > 0.0;
    if has_delta {
        sqlx::query(
            r#"
            INSERT OR IGNORE INTO stats_source_deltas (
                source,
                period,
                stats_date,
                captured_at,
                captured_at_epoch,
                total_count,
                success_count,
                failure_count,
                total_tokens,
                total_cost
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
            "#,
        )
        .bind(SOURCE_CRS)
        .bind(&period)
        .bind(&stats_date)
        .bind(&captured_at)
        .bind(captured_at_epoch)
        .bind(delta.total_count)
        .bind(delta.success_count)
        .bind(delta.failure_count)
        .bind(delta.total_tokens)
        .bind(delta.total_cost)
        .execute(&mut *tx)
        .await?;
    }

    tx.commit().await?;

    Ok(if has_delta { Some(delta) } else { None })
}

fn codex_invocations_create_sql(table_name: &str) -> String {
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

async fn load_sqlite_table_columns(
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

async fn load_sqlite_table_columns_from_connection(
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

async fn ensure_pool_upstream_request_attempts_archive_schema(
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

async fn ensure_codex_invocations_archive_schema(conn: &mut SqliteConnection) -> Result<()> {
    let archive_columns =
        load_sqlite_table_columns_from_connection(conn, Some("archive_db"), "codex_invocations")
            .await?;
    for (column, ty) in [
        ("request_raw_codec", "TEXT NOT NULL DEFAULT 'identity'"),
        ("response_raw_codec", "TEXT NOT NULL DEFAULT 'identity'"),
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

async fn migrate_codex_invocations_drop_raw_expires_at(
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
