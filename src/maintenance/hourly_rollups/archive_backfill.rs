use super::live_repair::{
    archive_batch_has_completed_manifest_sha_tx,
    pool_upstream_node_health_archive_has_stale_replay_marker_tx,
    reset_replaced_pool_upstream_node_health_archive_state_tx,
};
use super::*;

pub(crate) async fn backfill_pool_upstream_node_health_archives_for_files(
    pool: &Pool<Sqlite>,
    archive_files: Vec<ArchiveBatchFileRow>,
    max_archive_batches: Option<u64>,
    max_elapsed: Option<Duration>,
) -> Result<PoolUpstreamNodeHealthArchiveBackfillSummary> {
    let mut committed_scanned_batches = 0_u64;
    let mut committed_materialized_batches = 0_u64;
    let result = async {
        let started_at = Instant::now();
        let mut replay_started_any_pending_batch = false;

        let mut summary = PoolUpstreamNodeHealthArchiveBackfillSummary::default();
        for archive_file in archive_files {
            let archive_path = PathBuf::from(&archive_file.file_path);
            let temp_path = pool_upstream_node_health_archive_temp_path(&archive_path);
            if archive_path.parent().is_none_or(|parent| !parent.exists())
                || !archive_path.is_file()
            {
                // A source that disappeared after a budgeted pass cannot provide a reusable
                // SHA-bound copy. Remove the old copy before leaving it pending.
                remove_temp_sqlite_artifacts(&temp_path);
                summary.scanned_batches += 1;
                committed_scanned_batches = summary.scanned_batches;
                continue;
            }
            let _archive_lock = retention_archive_file_lock_async(&archive_path).await?;
            let Some(expected_sha256) = archive_file.sha256.as_deref() else {
                remove_temp_sqlite_artifacts(&temp_path);
                summary.scanned_batches += 1;
                committed_scanned_batches = summary.scanned_batches;
                continue;
            };
            if sha256_hex_file_with_deadline(
                &archive_path,
                super::super::retention::retention_archive_file_operation_deadline(),
            )
            .ok()
            .as_deref()
                != Some(expected_sha256)
            {
                remove_temp_sqlite_artifacts(&temp_path);
                summary.scanned_batches += 1;
                committed_scanned_batches = summary.scanned_batches;
                continue;
            }
            let mut tx = pool.begin().await?;
            if !archive_batch_has_completed_manifest_sha_tx(
                tx.as_mut(),
                "pool_upstream_request_attempts",
                &archive_file.file_path,
            )
            .await?
            {
                // Keep a source without immutable manifest proof pending. Any reusable copy from a
                // prior manifest must be removed because the copy cannot be bound to this row.
                remove_temp_sqlite_artifacts(&temp_path);
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

            // Do not hold the SQLite transaction while the archive file phase waits for or runs
            // native file work. The replay transaction is reopened after the source is prepared.
            tx.commit().await?;

            replay_started_any_pending_batch = true;
            let mut temp_cleanup = TempSqliteCleanup::new(temp_path.clone());
            let archive_pool =
                open_historical_rollup_archive_pool(&archive_path, &temp_path, expected_sha256)
                    .await?;
            {
                let mut archive_conn = archive_pool.acquire().await?;
                ensure_pool_upstream_request_attempts_archive_schema_in_place(&mut archive_conn)
                    .await?;
            }
            let mut tx = pool.begin().await?;
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
                temp_cleanup.keep_complete_copy();
                committed_scanned_batches = summary.scanned_batches;
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
            committed_scanned_batches = summary.scanned_batches;
            committed_materialized_batches = summary.materialized_batches;
        }

        summary.pending_batches = pending_pool_upstream_node_health_archive_batches(pool).await?;

        Ok::<PoolUpstreamNodeHealthArchiveBackfillSummary, anyhow::Error>(summary)
    }
    .await;
    result.map_err(|error| {
        anyhow::Error::new(crate::BackfillPartialFailure {
            source: error,
            next_cursor_id: 0,
            scanned: committed_scanned_batches,
            updated: committed_materialized_batches,
        })
    })
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
    let mut committed_scanned_batches = 0_u64;
    let mut committed_materialized_batches = 0_u64;
    let result = async {
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
            committed_scanned_batches = summary.scanned_batches;
            committed_materialized_batches = summary.materialized_batches;
        }

        summary.pending_batches =
            pending_pool_upstream_node_health_hourly_archive_batches(pool).await?;
        Ok::<PoolUpstreamNodeHealthHourlyArchiveBackfillSummary, anyhow::Error>(summary)
    }
    .await;
    result.map_err(|error| {
        anyhow::Error::new(crate::BackfillPartialFailure {
            source: error,
            next_cursor_id: 0,
            scanned: committed_scanned_batches,
            updated: committed_materialized_batches,
        })
    })
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
