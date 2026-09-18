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
        let Some(plan) = prepare_invocation_archive_replay(tx, &archive_file, &mut summary).await?
        else {
            continue;
        };
        if replay_invocation_archive_file_with_limits(
            tx,
            started_at,
            max_elapsed,
            &archive_file,
            plan,
            &mut summary,
        )
        .await?
        {
            break;
        }
    }
    summary.remaining_skip_batches = skip_remaining;
    Ok(summary)
}

struct InvocationArchiveReplayPlan {
    pending_targets: Vec<&'static str>,
    blocked_targets: Vec<&'static str>,
    account_activity_v2_pending: bool,
    replay_cursor: i64,
    usage_breakdown_cursor: i64,
}

async fn prepare_invocation_archive_replay(
    tx: &mut SqliteConnection,
    archive_file: &ArchiveBatchFileRow,
    summary: &mut HistoricalRollupArchiveReplaySummary,
) -> Result<Option<InvocationArchiveReplayPlan>> {
    if !archive_batch_has_completed_manifest_sha_tx(
        tx,
        HOURLY_ROLLUP_DATASET_INVOCATIONS,
        &archive_file.file_path,
    )
    .await?
        || invocation_archive_has_unverified_replay_marker_tx(tx, &archive_file.file_path).await?
    {
        summary.blocked_batches += 1;
        return Ok(None);
    }
    if invocation_archive_needs_overlap_reset(tx, archive_file).await?
        && reopen_replaced_materialized_invocation_archive_tx(
            tx,
            &archive_file.file_path,
            archive_file.coverage_start_at.as_deref(),
            archive_file.coverage_end_at.as_deref(),
        )
        .await?
        .is_none()
    {
        summary.blocked_batches += 1;
        return Ok(None);
    }
    let (pending_targets, blocked_targets) =
        load_invocation_archive_pending_targets(tx, &archive_file.file_path).await?;
    let account_activity_v2_pending =
        pending_targets.contains(&HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_ACTIVITY_V2);
    let pending_targets = pending_targets
        .into_iter()
        .filter(|target| *target != HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_ACTIVITY_V2)
        .collect::<Vec<_>>();
    if !account_activity_v2_pending
        && can_shortcut_legacy_materialized_upstream_account_targets(&pending_targets)
    {
        mark_materialized_upstream_account_archive_replayed_tx(tx, &archive_file.file_path).await?;
        mark_archive_batch_historical_rollups_materialized_tx(
            tx,
            HOURLY_ROLLUP_DATASET_INVOCATIONS,
            &archive_file.file_path,
        )
        .await?;
        summary.changed_batches += 1;
        return Ok(None);
    }
    if pending_targets.is_empty() && !account_activity_v2_pending {
        mark_archive_batch_historical_rollups_materialized_tx(
            tx,
            HOURLY_ROLLUP_DATASET_INVOCATIONS,
            &archive_file.file_path,
        )
        .await?;
        summary.changed_batches += 1;
        return Ok(None);
    }
    let replay_cursor = load_hourly_rollup_archive_progress_tx(
        tx,
        HOURLY_ROLLUP_DATASET_INVOCATIONS,
        &archive_file.file_path,
    )
    .await?;
    let usage_breakdown_cursor =
        if pending_targets.contains(&HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE_BREAKDOWN) {
            load_hourly_rollup_archive_progress_tx(
                tx,
                INVOCATION_USAGE_BREAKDOWN_ARCHIVE_PROGRESS_DATASET,
                &archive_file.file_path,
            )
            .await?
        } else {
            0
        };
    if !Path::new(&archive_file.file_path).exists() {
        delete_missing_invocation_archive_progress(tx, &archive_file.file_path).await?;
        return Ok(None);
    }
    Ok(Some(InvocationArchiveReplayPlan {
        pending_targets,
        blocked_targets,
        account_activity_v2_pending,
        replay_cursor,
        usage_breakdown_cursor,
    }))
}

async fn invocation_archive_needs_overlap_reset(
    tx: &mut SqliteConnection,
    archive_file: &ArchiveBatchFileRow,
) -> Result<bool> {
    let stale = invocation_archive_has_stale_replay_marker_tx(tx, &archive_file.file_path).await?;
    let missing_summary_proof =
        invocation_archive_has_materialized_rollups_tx(tx, &archive_file.file_path).await?
            && invocation_archive_is_missing_summary_projection_proof_tx(
                tx,
                &archive_file.file_path,
            )
            .await?;
    Ok(stale || missing_summary_proof)
}

async fn load_invocation_archive_pending_targets(
    tx: &mut SqliteConnection,
    file_path: &str,
) -> Result<(Vec<&'static str>, Vec<&'static str>)> {
    let mut pending_targets = Vec::new();
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
            file_path,
        )
        .await?
        {
            pending_targets.push(target);
        }
    }
    Ok((pending_targets, Vec::new()))
}

async fn delete_missing_invocation_archive_progress(
    tx: &mut SqliteConnection,
    file_path: &str,
) -> Result<()> {
    warn!(
        dataset = HOURLY_ROLLUP_DATASET_INVOCATIONS,
        file_path, "skipping missing archive batch during historical rollup materialization"
    );
    for dataset in [
        HOURLY_ROLLUP_DATASET_INVOCATIONS,
        INVOCATION_USAGE_BREAKDOWN_ARCHIVE_PROGRESS_DATASET,
        INVOCATION_ACCOUNT_ACTIVITY_V2_ARCHIVE_PROGRESS_DATASET,
    ] {
        delete_hourly_rollup_archive_progress_tx(tx, dataset, file_path).await?;
    }
    Ok(())
}

async fn replay_invocation_archive_file_with_limits(
    tx: &mut SqliteConnection,
    started_at: Instant,
    max_elapsed: Option<Duration>,
    archive_file: &ArchiveBatchFileRow,
    mut plan: InvocationArchiveReplayPlan,
    summary: &mut HistoricalRollupArchiveReplaySummary,
) -> Result<bool> {
    let archive_path = PathBuf::from(&archive_file.file_path);
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
        return Ok(true);
    };
    filter_pruned_invocation_targets(&archive_pool, archive_file, &mut plan, summary).await?;
    let activity_stage = if plan.account_activity_v2_pending {
        Some(
            replay_invocation_archive_activity_stage(
                tx,
                &archive_pool,
                archive_file,
                &plan,
                started_at,
                max_elapsed,
            )
            .await?,
        )
    } else {
        None
    };
    if activity_stage.is_some_and(|stage| stage.hit_budget) {
        let stage = activity_stage.expect("activity stage must exist when budget is hit");
        archive_pool.close().await;
        summary.budget_consumed_batches += 1;
        summary.hit_budget = true;
        if stage.changed {
            summary.changed_batches += 1;
        }
        std::mem::forget(temp_cleanup);
        return Ok(true);
    }
    let changed_in_prior_stage = activity_stage.is_some_and(|stage| stage.changed);
    if plan.pending_targets.is_empty() {
        archive_pool.close().await;
        drop(temp_cleanup);
        finish_empty_invocation_archive(tx, archive_file, &plan, summary).await?;
        return Ok(false);
    }
    let stage = replay_invocation_archive_rollup_stage(
        tx,
        &archive_pool,
        archive_file,
        &plan,
        changed_in_prior_stage,
        started_at,
        max_elapsed,
    )
    .await?;
    archive_pool.close().await;
    apply_invocation_archive_rollup_stage(tx, archive_file, plan, stage, temp_cleanup, summary)
        .await
}

async fn filter_pruned_invocation_targets(
    archive_pool: &Pool<Sqlite>,
    archive_file: &ArchiveBatchFileRow,
    plan: &mut InvocationArchiveReplayPlan,
    summary: &HistoricalRollupArchiveReplaySummary,
) -> Result<()> {
    if !invocation_archive_has_pruned_success_details_in_db(archive_pool).await? {
        return Ok(());
    }
    let mut replayable_targets = Vec::with_capacity(plan.pending_targets.len());
    let mut structured_rollup_targets = Vec::new();
    for target in plan.pending_targets.drain(..) {
        if invocation_archive_target_needs_full_payload(target) {
            plan.blocked_targets.push(target);
        } else {
            if target == HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE_BREAKDOWN {
                structured_rollup_targets.push(target);
            }
            replayable_targets.push(target);
        }
    }
    plan.pending_targets = replayable_targets;
    if !structured_rollup_targets.is_empty() {
        tracing::info!(
            dataset = HOURLY_ROLLUP_DATASET_INVOCATIONS,
            file_path = archive_file.file_path,
            legacy_pruned_payload_mode = LEGACY_PRUNED_PAYLOAD_MODE_STRUCTURED_ROLLUP_UNKNOWN_REASONING,
            archive_replay_target = ?structured_rollup_targets,
            pending_target_count = plan.pending_targets.len(),
            blocked_target_count = plan.blocked_targets.len(),
            materialized_batch_count = summary.materialized_batches,
            "legacy archive batch contains pruned success details; replaying structured hourly rollup targets with unknown reasoning fallback"
        );
    }
    Ok(())
}

#[derive(Debug, Clone, Copy)]
struct InvocationArchiveActivityStage {
    hit_budget: bool,
    changed: bool,
}

async fn replay_invocation_archive_activity_stage(
    tx: &mut SqliteConnection,
    archive_pool: &Pool<Sqlite>,
    archive_file: &ArchiveBatchFileRow,
    plan: &InvocationArchiveReplayPlan,
    started_at: Instant,
    max_elapsed: Option<Duration>,
) -> Result<InvocationArchiveActivityStage> {
    let cursor = load_hourly_rollup_archive_progress_tx(
        tx,
        INVOCATION_ACCOUNT_ACTIVITY_V2_ARCHIVE_PROGRESS_DATASET,
        &archive_file.file_path,
    )
    .await?;
    let replay = replay_invocation_archive_rows_into_hourly_rollups_tx_with_budget(
        tx,
        archive_pool,
        cursor,
        &[HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_ACTIVITY_V2],
        started_at,
        max_elapsed,
    )
    .await?;
    let progressed = historical_rollup_replay_made_progress(replay, cursor);
    if replay.outcome == HistoricalRollupArchiveReplayOutcome::HitBudget {
        if progressed {
            save_hourly_rollup_archive_progress_tx(
                tx,
                INVOCATION_ACCOUNT_ACTIVITY_V2_ARCHIVE_PROGRESS_DATASET,
                &archive_file.file_path,
                replay.cursor_id,
            )
            .await?;
        }
        return Ok(InvocationArchiveActivityStage {
            hit_budget: true,
            changed: progressed,
        });
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
    tracing::debug!(
        archive_replay_target = HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_ACTIVITY_V2,
        file_path = archive_file.file_path,
        materialized_batch_count = 0_u64,
        blocked_target_count = plan.blocked_targets.len(),
        pending_target_count = plan.pending_targets.len(),
        "materialized account activity v2 archive rollup"
    );
    Ok(InvocationArchiveActivityStage {
        hit_budget: false,
        changed: true,
    })
}

async fn finish_empty_invocation_archive(
    tx: &mut SqliteConnection,
    archive_file: &ArchiveBatchFileRow,
    plan: &InvocationArchiveReplayPlan,
    summary: &mut HistoricalRollupArchiveReplaySummary,
) -> Result<()> {
    summary.budget_consumed_batches += 1;
    if plan.blocked_targets.is_empty() {
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
            legacy_pruned_payload_mode = LEGACY_PRUNED_PAYLOAD_MODE_BLOCKED_PAYLOAD_REQUIRED,
            archive_replay_target = HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_ACTIVITY_V2,
            pending_target_count = 0_usize,
            blocked_target_count = plan.blocked_targets.len(),
            blocked_targets = ?plan.blocked_targets,
            "legacy archive account activity replay completed; keyed targets remain blocked"
        );
    }
    if plan.account_activity_v2_pending || plan.blocked_targets.is_empty() {
        summary.changed_batches += 1;
    }
    Ok(())
}

enum InvocationArchiveRollupStage {
    HitBudget {
        replay: HistoricalRollupArchiveReplayResult,
        initial_cursor: i64,
        usage_breakdown_cursor: i64,
        changed_in_prior_stage: bool,
        coverage_updated: bool,
        catch_up_only: bool,
    },
    Completed,
}

async fn replay_invocation_archive_rollup_stage(
    tx: &mut SqliteConnection,
    archive_pool: &Pool<Sqlite>,
    archive_file: &ArchiveBatchFileRow,
    plan: &InvocationArchiveReplayPlan,
    changed_in_prior_stage: bool,
    started_at: Instant,
    max_elapsed: Option<Duration>,
) -> Result<InvocationArchiveRollupStage> {
    let coverage_updated =
        archive_file.coverage_start_at.is_none() || archive_file.coverage_end_at.is_none();
    if coverage_updated {
        let bounds = load_archive_coverage_bounds(archive_pool, "codex_invocations").await?;
        update_archive_batch_coverage_bounds_tx(
            tx,
            archive_file.id,
            bounds.coverage_start_at.as_deref(),
            bounds.coverage_end_at.as_deref(),
        )
        .await?;
    }
    let mut usage_breakdown_cursor = plan.usage_breakdown_cursor;
    if plan
        .pending_targets
        .contains(&HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE_BREAKDOWN)
        && usage_breakdown_cursor < plan.replay_cursor
    {
        let catch_up =
            replay_invocation_archive_rows_into_hourly_rollups_until_cursor_tx_with_budget(
                tx,
                archive_pool,
                usage_breakdown_cursor,
                plan.replay_cursor,
                &[HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE_BREAKDOWN],
                started_at,
                max_elapsed,
            )
            .await?;
        let progressed = historical_rollup_replay_made_progress(catch_up, usage_breakdown_cursor);
        if progressed {
            save_hourly_rollup_archive_progress_tx(
                tx,
                INVOCATION_USAGE_BREAKDOWN_ARCHIVE_PROGRESS_DATASET,
                &archive_file.file_path,
                catch_up.cursor_id,
            )
            .await?;
            usage_breakdown_cursor = catch_up.cursor_id;
        }
        if catch_up.outcome == HistoricalRollupArchiveReplayOutcome::HitBudget
            && usage_breakdown_cursor < plan.replay_cursor
        {
            return Ok(InvocationArchiveRollupStage::HitBudget {
                replay: catch_up,
                initial_cursor: plan.usage_breakdown_cursor,
                usage_breakdown_cursor,
                changed_in_prior_stage,
                coverage_updated,
                catch_up_only: true,
            });
        }
    }
    let replay = replay_invocation_archive_rows_into_hourly_rollups_tx_with_budget(
        tx,
        archive_pool,
        plan.replay_cursor,
        &plan.pending_targets,
        started_at,
        max_elapsed,
    )
    .await?;
    if replay.outcome == HistoricalRollupArchiveReplayOutcome::HitBudget {
        return Ok(InvocationArchiveRollupStage::HitBudget {
            replay,
            initial_cursor: plan.replay_cursor,
            usage_breakdown_cursor,
            changed_in_prior_stage,
            coverage_updated,
            catch_up_only: false,
        });
    }
    Ok(InvocationArchiveRollupStage::Completed)
}

async fn apply_invocation_archive_rollup_stage(
    tx: &mut SqliteConnection,
    archive_file: &ArchiveBatchFileRow,
    plan: InvocationArchiveReplayPlan,
    stage: InvocationArchiveRollupStage,
    temp_cleanup: TempSqliteCleanup,
    summary: &mut HistoricalRollupArchiveReplaySummary,
) -> Result<bool> {
    match stage {
        InvocationArchiveRollupStage::HitBudget {
            replay,
            initial_cursor,
            usage_breakdown_cursor,
            changed_in_prior_stage,
            coverage_updated,
            catch_up_only,
        } => {
            let replay_progressed = historical_rollup_replay_made_progress(replay, initial_cursor);
            if !catch_up_only && replay_progressed {
                save_hourly_rollup_archive_progress_tx(
                    tx,
                    HOURLY_ROLLUP_DATASET_INVOCATIONS,
                    &archive_file.file_path,
                    replay.cursor_id,
                )
                .await?;
            }
            if !catch_up_only
                && plan
                    .pending_targets
                    .contains(&HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE_BREAKDOWN)
                && replay.cursor_id > usage_breakdown_cursor
            {
                save_hourly_rollup_archive_progress_tx(
                    tx,
                    INVOCATION_USAGE_BREAKDOWN_ARCHIVE_PROGRESS_DATASET,
                    &archive_file.file_path,
                    replay.cursor_id,
                )
                .await?;
            }
            summary.budget_consumed_batches += 1;
            summary.hit_budget = true;
            if historical_rollup_candidate_changed(
                changed_in_prior_stage,
                replay_progressed,
                coverage_updated,
            ) {
                summary.changed_batches += 1;
            }
            std::mem::forget(temp_cleanup);
            Ok(true)
        }
        InvocationArchiveRollupStage::Completed => {
            drop(temp_cleanup);
            complete_invocation_archive_rollup_stage(tx, archive_file, plan, summary).await
        }
    }
}

async fn complete_invocation_archive_rollup_stage(
    tx: &mut SqliteConnection,
    archive_file: &ArchiveBatchFileRow,
    plan: InvocationArchiveReplayPlan,
    summary: &mut HistoricalRollupArchiveReplaySummary,
) -> Result<bool> {
    summary.budget_consumed_batches += 1;
    delete_hourly_rollup_archive_progress_tx(
        tx,
        HOURLY_ROLLUP_DATASET_INVOCATIONS,
        &archive_file.file_path,
    )
    .await?;
    if plan
        .pending_targets
        .contains(&HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE_BREAKDOWN)
    {
        delete_hourly_rollup_archive_progress_tx(
            tx,
            INVOCATION_USAGE_BREAKDOWN_ARCHIVE_PROGRESS_DATASET,
            &archive_file.file_path,
        )
        .await?;
    }
    for target in plan.pending_targets {
        mark_hourly_rollup_archive_replayed_tx(
            tx,
            target,
            HOURLY_ROLLUP_DATASET_INVOCATIONS,
            &archive_file.file_path,
        )
        .await?;
    }
    if plan.blocked_targets.is_empty() {
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
            blocked_target_count = plan.blocked_targets.len(),
            materialized_batch_count = summary.materialized_batches,
            blocked_targets = ?plan.blocked_targets,
            "legacy archive batch contains pruned success details; keeping historical rollup materialization pending for keyed conversation targets"
        );
    }
    summary.changed_batches += 1;
    Ok(false)
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
