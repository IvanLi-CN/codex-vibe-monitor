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

async fn replay_invocation_archive_rows_into_hourly_rollups_until_cursor_tx_with_budget(
    tx: &mut SqliteConnection,
    archive_pool: &Pool<Sqlite>,
    initial_cursor_id: i64,
    target_cursor_id: i64,
    pending_targets: &[&str],
    started_at: Instant,
    max_elapsed: Option<Duration>,
) -> Result<HistoricalRollupArchiveReplayResult> {
    let mut start_after_id = initial_cursor_id.max(0);
    let target_cursor_id = target_cursor_id.max(0);
    if start_after_id >= target_cursor_id {
        return Ok(HistoricalRollupArchiveReplayResult {
            outcome: HistoricalRollupArchiveReplayOutcome::Completed,
            cursor_id: start_after_id,
        });
    }

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
        let bounded_rows = rows
            .into_iter()
            .take_while(|row| row.id <= target_cursor_id)
            .collect::<Vec<_>>();
        if bounded_rows.is_empty() {
            return Ok(HistoricalRollupArchiveReplayResult {
                outcome: HistoricalRollupArchiveReplayOutcome::Completed,
                cursor_id: start_after_id,
            });
        }

        upsert_invocation_hourly_rollups_tx(tx, &bounded_rows, pending_targets).await?;
        mark_invocation_hourly_rollup_buckets_materialized_tx(tx, &bounded_rows).await?;
        start_after_id = bounded_rows
            .last()
            .map(|row| row.id)
            .ok_or_else(|| anyhow!("missing invocation archive row id"))?;

        if start_after_id >= target_cursor_id || !has_more {
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

pub(crate) async fn replay_invocation_archive_files_into_hourly_rollups_tx_with_limits(
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
            HOURLY_ROLLUP_DATASET_INVOCATIONS,
            &archive_file.file_path,
        )
        .await?
        {
            // A completed archive with no immutable manifest identity is intentionally
            // unverified. Do not clear state or additively replay it until it becomes
            // verifiable or a caller can perform a proven full rebuild.
            summary.blocked_batches += 1;
            continue;
        }
        if invocation_archive_has_unverified_replay_marker_tx(tx, &archive_file.file_path).await? {
            // A nullable or blank marker is not evidence that this archive's contributions are
            // represented by the current manifest. Keep it quarantined until an explicit,
            // proven rebuild can replace the unknown state.
            summary.blocked_batches += 1;
            continue;
        }
        let has_stale_replay_marker =
            invocation_archive_has_stale_replay_marker_tx(tx, &archive_file.file_path).await?;
        let materialized_summary_proof_missing =
            invocation_archive_has_materialized_rollups_tx(tx, &archive_file.file_path).await?
                && invocation_archive_is_missing_summary_projection_proof_tx(
                    tx,
                    &archive_file.file_path,
                )
                .await?;
        if has_stale_replay_marker || materialized_summary_proof_missing {
            // A stale marker or missing Summary proof on a materialized archive means a prior
            // contribution may remain. Reset the verified overlap closure before inspecting
            // pending targets so an incremental replay cannot double count old rows.
            let Some(_) = reopen_replaced_materialized_invocation_archive_tx(
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
        let mut pending_targets = Vec::new();
        let mut blocked_targets = Vec::new();
        for target in [
            HOURLY_ROLLUP_TARGET_INVOCATIONS,
            HOURLY_ROLLUP_TARGET_INVOCATION_FAILURES,
            HOURLY_ROLLUP_TARGET_PROXY_PERF,
            HOURLY_ROLLUP_TARGET_PROMPT_CACHE,
            HOURLY_ROLLUP_TARGET_PROMPT_CACHE_UPSTREAM_ACCOUNTS,
            HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE,
            HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE_BREAKDOWN,
            HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_STATS_HOURLY,
            HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_ACTIVITY_V2,
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
        let account_activity_v2_pending =
            pending_targets.contains(&HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_ACTIVITY_V2);
        pending_targets
            .retain(|target| *target != HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_ACTIVITY_V2);
        if !account_activity_v2_pending
            && can_shortcut_legacy_materialized_upstream_account_targets(&pending_targets)
        {
            mark_materialized_upstream_account_archive_replayed_tx(tx, &archive_file.file_path)
                .await?;
            mark_archive_batch_historical_rollups_materialized_tx(
                tx,
                HOURLY_ROLLUP_DATASET_INVOCATIONS,
                &archive_file.file_path,
            )
            .await?;
            summary.changed_batches += 1;
            continue;
        }
        if pending_targets.is_empty() && !account_activity_v2_pending {
            mark_archive_batch_historical_rollups_materialized_tx(
                tx,
                HOURLY_ROLLUP_DATASET_INVOCATIONS,
                &archive_file.file_path,
            )
            .await?;
            summary.changed_batches += 1;
            continue;
        }
        let replay_cursor = load_hourly_rollup_archive_progress_tx(
            tx,
            HOURLY_ROLLUP_DATASET_INVOCATIONS,
            &archive_file.file_path,
        )
        .await?;
        let usage_breakdown_pending =
            pending_targets.contains(&HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE_BREAKDOWN);
        let mut usage_breakdown_cursor = if usage_breakdown_pending {
            load_hourly_rollup_archive_progress_tx(
                tx,
                INVOCATION_USAGE_BREAKDOWN_ARCHIVE_PROGRESS_DATASET,
                &archive_file.file_path,
            )
            .await?
        } else {
            0
        };

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
            delete_hourly_rollup_archive_progress_tx(
                tx,
                INVOCATION_USAGE_BREAKDOWN_ARCHIVE_PROGRESS_DATASET,
                &archive_file.file_path,
            )
            .await?;
            delete_hourly_rollup_archive_progress_tx(
                tx,
                INVOCATION_ACCOUNT_ACTIVITY_V2_ARCHIVE_PROGRESS_DATASET,
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
        let has_pruned_success_details =
            invocation_archive_has_pruned_success_details_in_db(&archive_pool).await?;
        let mut changed_in_prior_stage = false;
        if has_pruned_success_details {
            let mut replayable_targets = Vec::with_capacity(pending_targets.len());
            let mut structured_rollup_targets = Vec::new();
            for target in pending_targets {
                if invocation_archive_target_needs_full_payload(target) {
                    blocked_targets.push(target);
                } else {
                    if target == HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE_BREAKDOWN {
                        structured_rollup_targets.push(target);
                    }
                    replayable_targets.push(target);
                }
            }
            pending_targets = replayable_targets;
            if !structured_rollup_targets.is_empty() {
                tracing::info!(
                    dataset = HOURLY_ROLLUP_DATASET_INVOCATIONS,
                    file_path = archive_file.file_path,
                    legacy_pruned_payload_mode =
                        LEGACY_PRUNED_PAYLOAD_MODE_STRUCTURED_ROLLUP_UNKNOWN_REASONING,
                    archive_replay_target = ?structured_rollup_targets,
                    pending_target_count = pending_targets.len(),
                    blocked_target_count = blocked_targets.len(),
                    materialized_batch_count = summary.materialized_batches,
                    "legacy archive batch contains pruned success details; replaying structured hourly rollup targets with unknown reasoning fallback"
                );
            }
        }

        if account_activity_v2_pending {
            let account_activity_v2_cursor = load_hourly_rollup_archive_progress_tx(
                tx,
                INVOCATION_ACCOUNT_ACTIVITY_V2_ARCHIVE_PROGRESS_DATASET,
                &archive_file.file_path,
            )
            .await?;
            let replay = replay_invocation_archive_rows_into_hourly_rollups_tx_with_budget(
                tx,
                &archive_pool,
                account_activity_v2_cursor,
                &[HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_ACTIVITY_V2],
                started_at,
                max_elapsed,
            )
            .await?;
            if replay.outcome == HistoricalRollupArchiveReplayOutcome::HitBudget {
                let replay_progressed =
                    historical_rollup_replay_made_progress(replay, account_activity_v2_cursor);
                if replay.cursor_id > account_activity_v2_cursor {
                    save_hourly_rollup_archive_progress_tx(
                        tx,
                        INVOCATION_ACCOUNT_ACTIVITY_V2_ARCHIVE_PROGRESS_DATASET,
                        &archive_file.file_path,
                        replay.cursor_id,
                    )
                    .await?;
                }
                archive_pool.close().await;
                summary.budget_consumed_batches += 1;
                summary.hit_budget = true;
                if replay_progressed {
                    summary.changed_batches += 1;
                }
                std::mem::forget(temp_cleanup);
                break;
            }
            delete_hourly_rollup_archive_progress_tx(
                tx,
                INVOCATION_ACCOUNT_ACTIVITY_V2_ARCHIVE_PROGRESS_DATASET,
                &archive_file.file_path,
            )
            .await?;
            mark_hourly_rollup_archive_replayed_tx(
                tx,
                HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_ACTIVITY_V2,
                HOURLY_ROLLUP_DATASET_INVOCATIONS,
                &archive_file.file_path,
            )
            .await?;
            changed_in_prior_stage = true;
            tracing::debug!(
                archive_replay_target = HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_ACTIVITY_V2,
                file_path = archive_file.file_path,
                materialized_batch_count = summary.materialized_batches,
                blocked_target_count = blocked_targets.len(),
                pending_target_count = pending_targets.len(),
                "materialized account activity v2 archive rollup"
            );
        }

        if pending_targets.is_empty() {
            archive_pool.close().await;
            drop(temp_cleanup);
            summary.budget_consumed_batches += 1;
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
                delete_hourly_rollup_archive_progress_tx(
                    tx,
                    HOURLY_ROLLUP_DATASET_INVOCATIONS,
                    &archive_file.file_path,
                )
                .await?;
                warn!(
                    dataset = HOURLY_ROLLUP_DATASET_INVOCATIONS,
                    file_path = archive_file.file_path,
                    legacy_pruned_payload_mode =
                        LEGACY_PRUNED_PAYLOAD_MODE_BLOCKED_PAYLOAD_REQUIRED,
                    archive_replay_target = HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_ACTIVITY_V2,
                    pending_target_count = 0_usize,
                    blocked_target_count = blocked_targets.len(),
                    blocked_targets = ?blocked_targets,
                    "legacy archive account activity replay completed; keyed targets remain blocked"
                );
            }
            if account_activity_v2_pending || blocked_targets.is_empty() {
                summary.changed_batches += 1;
            }
            continue;
        }

        let coverage_updated =
            archive_file.coverage_start_at.is_none() || archive_file.coverage_end_at.is_none();
        if coverage_updated {
            let bounds = load_archive_coverage_bounds(&archive_pool, "codex_invocations").await?;
            update_archive_batch_coverage_bounds_tx(
                tx,
                archive_file.id,
                bounds.coverage_start_at.as_deref(),
                bounds.coverage_end_at.as_deref(),
            )
            .await?;
        }

        if usage_breakdown_pending && usage_breakdown_cursor < replay_cursor {
            let catch_up_outcome =
                replay_invocation_archive_rows_into_hourly_rollups_until_cursor_tx_with_budget(
                    tx,
                    &archive_pool,
                    usage_breakdown_cursor,
                    replay_cursor,
                    &[HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE_BREAKDOWN],
                    started_at,
                    max_elapsed,
                )
                .await?;
            let catch_up_progressed =
                historical_rollup_replay_made_progress(catch_up_outcome, usage_breakdown_cursor);
            if catch_up_progressed {
                save_hourly_rollup_archive_progress_tx(
                    tx,
                    INVOCATION_USAGE_BREAKDOWN_ARCHIVE_PROGRESS_DATASET,
                    &archive_file.file_path,
                    catch_up_outcome.cursor_id,
                )
                .await?;
                usage_breakdown_cursor = catch_up_outcome.cursor_id;
                changed_in_prior_stage = true;
            }
            if catch_up_outcome.outcome == HistoricalRollupArchiveReplayOutcome::HitBudget
                && usage_breakdown_cursor < replay_cursor
            {
                archive_pool.close().await;
                summary.budget_consumed_batches += 1;
                summary.hit_budget = true;
                if historical_rollup_candidate_changed(
                    changed_in_prior_stage,
                    false,
                    coverage_updated,
                ) {
                    summary.changed_batches += 1;
                }
                std::mem::forget(temp_cleanup);
                break;
            }
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
            let replay_progressed =
                historical_rollup_replay_made_progress(replay_outcome, replay_cursor);
            summary.budget_consumed_batches += 1;
            summary.hit_budget = true;
            if replay_progressed {
                save_hourly_rollup_archive_progress_tx(
                    tx,
                    HOURLY_ROLLUP_DATASET_INVOCATIONS,
                    &archive_file.file_path,
                    replay_outcome.cursor_id,
                )
                .await?;
            }
            if usage_breakdown_pending && replay_outcome.cursor_id > usage_breakdown_cursor {
                save_hourly_rollup_archive_progress_tx(
                    tx,
                    INVOCATION_USAGE_BREAKDOWN_ARCHIVE_PROGRESS_DATASET,
                    &archive_file.file_path,
                    replay_outcome.cursor_id,
                )
                .await?;
            }
            if historical_rollup_candidate_changed(
                changed_in_prior_stage,
                replay_progressed,
                coverage_updated,
            ) {
                summary.changed_batches += 1;
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
        if usage_breakdown_pending {
            delete_hourly_rollup_archive_progress_tx(
                tx,
                INVOCATION_USAGE_BREAKDOWN_ARCHIVE_PROGRESS_DATASET,
                &archive_file.file_path,
            )
            .await?;
        }
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
                legacy_pruned_payload_mode = LEGACY_PRUNED_PAYLOAD_MODE_BLOCKED_PAYLOAD_REQUIRED,
                archive_replay_target = "mixed",
                pending_target_count = 0_usize,
                blocked_target_count = blocked_targets.len(),
                materialized_batch_count = summary.materialized_batches,
                blocked_targets = ?blocked_targets,
                "legacy archive batch contains pruned success details; keeping historical rollup materialization pending for keyed conversation targets"
            );
        }
        summary.changed_batches += 1;
    }

    summary.remaining_skip_batches = skip_remaining;
    Ok(summary)
}

pub(crate) async fn replay_invocation_archives_into_hourly_rollups_tx_with_limits(
    tx: &mut SqliteConnection,
    started_at: Instant,
    max_archive_batches: Option<u64>,
    max_elapsed: Option<Duration>,
    skip_archive_batches: usize,
) -> Result<HistoricalRollupArchiveReplaySummary> {
    let archive_files = sqlx::query_as::<_, ArchiveBatchFileRow>(
        r#"
        SELECT id, file_path, coverage_start_at, coverage_end_at
        FROM archive_batches
        WHERE dataset = 'codex_invocations'
          AND status = ?1
          AND COALESCE(summary_source_kind, 'unknown') <> 'live_mirror'
          AND historical_rollups_materialized_at IS NULL
        ORDER BY month_key ASC, created_at ASC, id ASC
        "#,
    )
    .bind(ARCHIVE_STATUS_COMPLETED)
    .fetch_all(&mut *tx)
    .await?;
    replay_invocation_archive_files_into_hourly_rollups_tx_with_limits(
        tx,
        started_at,
        max_archive_batches,
        max_elapsed,
        skip_archive_batches,
        archive_files,
    )
    .await
}

pub(crate) async fn replay_invocation_usage_breakdown_archives_into_hourly_rollups_tx_with_limits(
    tx: &mut SqliteConnection,
    started_at: Instant,
    max_archive_batches: Option<u64>,
    max_elapsed: Option<Duration>,
    skip_archive_batches: usize,
) -> Result<HistoricalRollupArchiveReplaySummary> {
    let archive_files = load_invocation_archive_files_missing_rollup_target(
        &mut *tx,
        HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE_BREAKDOWN,
    )
    .await?;
    replay_invocation_archive_files_into_hourly_rollups_tx_with_limits(
        tx,
        started_at,
        max_archive_batches,
        max_elapsed,
        skip_archive_batches,
        archive_files,
    )
    .await
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
    let archive_files = sqlx::query_as::<_, ArchiveBatchFileRow>(
        r#"
        SELECT batches.id, batches.file_path, batches.coverage_start_at, batches.coverage_end_at
        FROM archive_batches AS batches
        WHERE batches.dataset = 'forward_proxy_attempts'
          AND batches.status = ?1
          AND (
                batches.historical_rollups_materialized_at IS NULL
                OR EXISTS (
                    SELECT 1
                    FROM hourly_rollup_archive_replay AS replay
                    WHERE replay.target = ?2
                      AND replay.dataset = batches.dataset
                      AND replay.file_path = batches.file_path
                      AND batches.sha256 IS NOT NULL
                      AND TRIM(batches.sha256) <> ''
                      AND (
                            replay.archive_sha256 IS NULL
                            OR TRIM(replay.archive_sha256) = ''
                            OR replay.archive_sha256 <> batches.sha256
                      )
                )
          )
        ORDER BY month_key ASC, created_at ASC, id ASC
        "#,
    )
    .bind(ARCHIVE_STATUS_COMPLETED)
    .bind(HOURLY_ROLLUP_TARGET_FORWARD_PROXY_ATTEMPTS)
    .fetch_all(&mut *tx)
    .await?;

    replay_forward_proxy_archive_files_into_hourly_rollups_tx_with_limits(
        tx,
        started_at,
        max_archive_batches,
        max_elapsed,
        skip_archive_batches,
        archive_files,
    )
    .await
}
