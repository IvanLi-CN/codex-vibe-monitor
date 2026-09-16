pub(crate) async fn replay_forward_proxy_archive_files_into_hourly_rollups_tx_with_limits(
    tx: &mut SqliteConnection,
    started_at: Instant,
    max_archive_batches: Option<u64>,
    max_elapsed: Option<Duration>,
    skip_archive_batches: usize,
    archive_files: Vec<ArchiveBatchFileRow>,
) -> Result<HistoricalRollupArchiveReplaySummary> {
    let mut summary = HistoricalRollupArchiveReplaySummary::default();
    let mut skip_remaining = skip_archive_batches;

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
            summary.hit_budget = true;
            break;
        }
        summary.scanned_batches += 1;
        if !archive_batch_has_completed_manifest_sha_tx(
            tx,
            HOURLY_ROLLUP_DATASET_FORWARD_PROXY_ATTEMPTS,
            &archive_file.file_path,
        )
        .await?
        {
            summary.blocked_batches += 1;
            continue;
        }
        if forward_proxy_archive_has_stale_replay_marker_tx(tx, &archive_file.file_path).await? {
            let Some(_) = reopen_replaced_materialized_forward_proxy_archive_tx(
                tx,
                &archive_file.file_path,
                archive_file.coverage_start_at.as_deref(),
                archive_file.coverage_end_at.as_deref(),
            )
            .await?
            else {
                summary.blocked_batches += 1;
                continue;
            };
        }
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
            summary.changed_batches += 1;
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
        let Some(archive_pool) = open_historical_rollup_archive_pool_with_budget(
            &archive_path,
            &temp_path,
            started_at,
            max_elapsed,
        )
        .await?
        else {
            summary.hit_budget = true;
            summary.advance_cursor_after_unstarted_replay = true;
            break;
        };

        let coverage_updated =
            archive_file.coverage_start_at.is_none() || archive_file.coverage_end_at.is_none();
        if coverage_updated {
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
            let replay_progressed =
                historical_rollup_replay_made_progress(replay_outcome, replay_cursor);
            summary.budget_consumed_batches += 1;
            summary.hit_budget = true;
            if replay_progressed {
                save_hourly_rollup_archive_progress_tx(
                    tx,
                    HOURLY_ROLLUP_DATASET_FORWARD_PROXY_ATTEMPTS,
                    &archive_file.file_path,
                    replay_outcome.cursor_id,
                )
                .await?;
            }
            if replay_progressed || coverage_updated {
                summary.changed_batches += 1;
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
        summary.changed_batches += 1;
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
        if !archive_batch_has_completed_manifest_sha_tx(
            tx.as_mut(),
            "pool_upstream_request_attempts",
            &archive_file.file_path,
        )
        .await?
        {
            // Keep a source without immutable manifest proof pending and untouched. Marking it
            // replayed would only create repeated work while still failing strict readers.
            tx.commit().await?;
            continue;
        }
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
        if pool_upstream_node_health_archive_has_stale_replay_marker_tx(
            tx.as_mut(),
            &archive_file.file_path,
        )
        .await?
        {
            // The archive may have removed rows that were previously eligible for the cache.
            // Replace both cache layers from the new source before accepting SHA B.
            reset_replaced_pool_upstream_node_health_archive_state_tx(
                tx.as_mut(),
                archive_file.id,
                &archive_file.file_path,
            )
            .await?;
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
        if !archive_batch_has_completed_manifest_sha_tx(
            tx.as_mut(),
            "pool_upstream_request_attempts",
            &archive_file.file_path,
        )
        .await?
        {
            tx.commit().await?;
            continue;
        }
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
    fn runtime_startup_bootstrap_leaves_active_coverage_to_the_dedicated_task() {
        assert_eq!(
            runtime_startup_hourly_rollup_refresh_scope(),
            HourlyRollupRefreshScope::SkipActiveAccountActivityV2CoverageRepair
        );
    }

    #[tokio::test]
    async fn read_surface_refresh_cancels_while_waiting_for_sync_lock() {
        let pool = SqlitePoolOptions::new()
            .connect_lazy("sqlite::memory:")
            .expect("create lazy sqlite pool");
        let lock = Arc::new(Mutex::new(()));
        let _held_guard = lock.lock().await;
        let cancel = CancellationToken::new();
        let refresh_lock = Arc::clone(&lock);
        let refresh_cancel = cancel.clone();
        let refresh = tokio::spawn(async move {
            refresh_hourly_rollups_for_read_surfaces_best_effort(
                &pool,
                refresh_lock.as_ref(),
                &refresh_cancel,
                "cancellation regression test",
                HourlyRollupRefreshScope::Full,
            )
            .await
        });

        tokio::task::yield_now().await;
        cancel.cancel();
        assert!(refresh.await.expect("refresh task should finish"));
    }

    #[test]
    fn historical_rollup_elapsed_budget_reached_respects_unbounded_mode() {
        assert!(!historical_rollup_elapsed_budget_reached(
            Instant::now(),
            None
        ));
    }

    #[test]
    fn invocation_archive_replay_treats_missing_first_token_column_as_null() {
        let legacy_query = build_invocation_archive_rows_chunk_query(&HashSet::new());
        assert!(legacy_query.contains("NULL AS first_token_ms"));

        let modern_query = build_invocation_archive_rows_chunk_query(&HashSet::from([
            "first_token_ms".to_string(),
        ]));
        assert!(modern_query.contains("first_token_ms,"));
        assert!(!modern_query.contains("NULL AS first_token_ms"));
    }

    #[tokio::test]
    async fn replay_budget_exhaustion_before_the_first_row_has_no_progress() {
        let archive_pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("open archive pool");
        sqlx::query("CREATE TABLE forward_proxy_attempts (id INTEGER PRIMARY KEY)")
            .execute(&archive_pool)
            .await
            .expect("create forward proxy archive schema");
        let mut tx = SqliteConnection::connect("sqlite::memory:")
            .await
            .expect("open rollup transaction connection");

        let replay = replay_forward_proxy_archive_rows_into_hourly_rollups_tx_with_budget(
            &mut tx,
            &archive_pool,
            0,
            Instant::now() - Duration::from_millis(1),
            Some(Duration::ZERO),
        )
        .await
        .expect("stop before replaying archive rows");

        assert_eq!(
            replay.outcome,
            HistoricalRollupArchiveReplayOutcome::HitBudget
        );
        assert_eq!(replay.cursor_id, 0);
        assert!(!historical_rollup_replay_made_progress(replay, 0));
    }

    #[test]
    fn candidate_with_prior_replay_progress_remains_actionable_after_later_budget_exhaustion() {
        assert!(historical_rollup_candidate_changed(true, false, false));
        assert!(!historical_rollup_candidate_changed(false, false, false));
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
    bootstrap_hourly_rollups_with_parallel_work_coverage(pool, None).await
}

pub(crate) async fn bootstrap_hourly_rollups_with_parallel_work_coverage(
    pool: &Pool<Sqlite>,
    invocation_full_detail_days: Option<u64>,
) -> Result<()> {
    bootstrap_hourly_rollups_with_scope(
        pool,
        invocation_full_detail_days,
        HourlyRollupRefreshScope::Full,
    )
    .await
}

pub(crate) async fn bootstrap_hourly_rollups_for_runtime_startup(
    pool: &Pool<Sqlite>,
    invocation_full_detail_days: Option<u64>,
) -> Result<()> {
    bootstrap_hourly_rollups_with_scope(
        pool,
        invocation_full_detail_days,
        runtime_startup_hourly_rollup_refresh_scope(),
    )
    .await
}

async fn bootstrap_hourly_rollups_with_scope(
    pool: &Pool<Sqlite>,
    invocation_full_detail_days: Option<u64>,
    scope: HourlyRollupRefreshScope,
) -> Result<()> {
    let usage_breakdown_started_at = Instant::now();
    repair_live_invocation_usage_breakdown_rollups(pool).await?;
    info!(
        rollup_bootstrap_step = "usage_breakdown_repair",
        elapsed_ms = usage_breakdown_started_at.elapsed().as_millis() as u64,
        "hourly rollup bootstrap step completed"
    );

    let live_sync_started_at = Instant::now();
    sync_hourly_rollups_from_live_tables_with_scope(pool, invocation_full_detail_days, scope)
        .await?;
    info!(
        rollup_bootstrap_step = "live_rollup_sync",
        elapsed_ms = live_sync_started_at.elapsed().as_millis() as u64,
        "hourly rollup bootstrap step completed"
    );

    let archive_usage_breakdown_started_at = Instant::now();
    repair_materialized_invocation_archive_usage_breakdown_backfill_state(pool).await?;
    info!(
        rollup_bootstrap_step = "archive_usage_breakdown_repair",
        elapsed_ms = archive_usage_breakdown_started_at.elapsed().as_millis() as u64,
        "hourly rollup bootstrap step completed"
    );

    let archive_marker_started_at = Instant::now();
    repair_materialized_upstream_account_archive_markers(pool).await?;
    info!(
        rollup_bootstrap_step = "upstream_account_archive_marker_repair",
        elapsed_ms = archive_marker_started_at.elapsed().as_millis() as u64,
        "hourly rollup bootstrap step completed"
    );

    let account_stats_probe_started_at = Instant::now();
    let account_stats_hourly_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM upstream_account_stats_hourly")
            .fetch_one(pool)
            .await?;
    let account_stats_minute_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM upstream_account_stats_minute")
            .fetch_one(pool)
            .await?;
    info!(
        rollup_bootstrap_step = "upstream_account_stats_probe",
        elapsed_ms = account_stats_probe_started_at.elapsed().as_millis() as u64,
        account_stats_hourly_count,
        account_stats_minute_count,
        "hourly rollup bootstrap step completed"
    );
    if account_stats_hourly_count == 0 || account_stats_minute_count == 0 {
        let account_stats_rebuild_started_at = Instant::now();
        rebuild_upstream_account_stats_rollups_from_sources(pool).await?;
        repair_materialized_upstream_account_archive_markers(pool).await?;
        info!(
            rollup_bootstrap_step = "upstream_account_stats_rebuild",
            elapsed_ms = account_stats_rebuild_started_at.elapsed().as_millis() as u64,
            "hourly rollup bootstrap step completed"
        );
    }
    Ok(())
}

fn runtime_startup_hourly_rollup_refresh_scope() -> HourlyRollupRefreshScope {
    // The dedicated startup task owns active-window coverage and its backoff.
    // Bootstrap still catches up rollups, but must not run that planner twice.
    HourlyRollupRefreshScope::SkipActiveAccountActivityV2CoverageRepair
}

pub(crate) async fn refresh_hourly_rollups_for_read_surfaces(pool: &Pool<Sqlite>) -> Result<()> {
    refresh_hourly_rollups_for_read_surfaces_with_scope(pool, HourlyRollupRefreshScope::Full).await
}

async fn refresh_hourly_rollups_for_read_surfaces_with_scope(
    pool: &Pool<Sqlite>,
    scope: HourlyRollupRefreshScope,
) -> Result<()> {
    sync_hourly_rollups_from_live_tables_with_scope(pool, None, scope).await?;
    ensure_invocation_summary_rollups_ready_best_effort(pool).await?;
    Ok(())
}

pub(crate) async fn ensure_hourly_rollups_caught_up(state: &AppState) -> Result<()> {
    let _guard = state.hourly_rollup_sync_lock.lock().await;
    sync_hourly_rollups_from_live_tables_with_parallel_work_coverage(
        &state.pool,
        Some(state.config.invocation_max_days),
    )
    .await
}

pub(crate) async fn refresh_hourly_rollups_for_read_surfaces_best_effort(
    pool: &Pool<Sqlite>,
    hourly_rollup_sync_lock: &Mutex<()>,
    cancel: &CancellationToken,
    reason: &'static str,
    scope: HourlyRollupRefreshScope,
) -> bool {
    let gate = crate::db_pressure::global_db_pressure_gate();
    let _guard = tokio::select! {
        biased;
        _ = cancel.cancelled() => {
            debug!(reason, "background hourly rollup refresh cancelled while waiting for sync lock");
            return true;
        }
        guard = hourly_rollup_sync_lock.lock() => guard,
    };
    let _permit = match gate.try_begin_background("hourly_rollup_refresh") {
        Ok(permit) => permit,
        Err(deny_reason) => {
            warn!(
                reason,
                deny_reason = %deny_reason,
                "background hourly rollup refresh skipped because database pressure gate is closed"
            );
            return true;
        }
    };
    let _write_permit =
        match crate::proxy_sqlite_write_coordinator::proxy_sqlite_write_coordinator()
            .try_acquire(crate::proxy_sqlite_write_coordinator::ProxySqliteWriteClass::P2Derived)
        {
            Some(permit) => permit,
            None => {
                debug!(
                    reason,
                    defer_reason = "coordinator_priority",
                    "background hourly rollup refresh skipped before SQLite access"
                );
                return true;
            }
        };

    let coordinator = crate::proxy_sqlite_write_coordinator::proxy_sqlite_write_coordinator();
    let refresh_result = tokio::select! {
        _ = cancel.cancelled() => {
            debug!(reason, "background hourly rollup refresh cancelled during SQLite work");
            return true;
        }
        _ = coordinator.wait_for_p2_preemption() => {
            debug!(reason, "background hourly rollup refresh yielded to higher-priority SQLite writes");
            return true;
        }
        result = refresh_hourly_rollups_for_read_surfaces_with_scope(pool, scope) => result,
    };
    if let Err(err) = refresh_result {
        gate.record_error("hourly_rollup_refresh", &err);
        warn!(
            error = %err,
            reason,
            "background hourly rollup refresh failed; keeping existing rollups for read surfaces"
        );
    }
    false
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum ActiveAccountActivityV2RepairResult {
    Repaired(ActiveAccountActivityV2RepairOutcome),
    Deferred,
    Failed,
}
