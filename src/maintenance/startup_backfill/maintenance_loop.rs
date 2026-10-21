use super::*;

pub(super) fn startup_backfill_wake_decision(wake_reason: &str) -> WakeDecision {
    match wake_reason {
        "operator_disabled" | "managed_task_disabled" => WakeDecision::Disabled,
        reason if reason.contains("stale") || reason.contains("retry") => WakeDecision::Stale,
        _ => WakeDecision::New,
    }
}

#[derive(Debug, Default)]
pub(crate) struct StartupBackfillMaintenancePass {
    pub(crate) ran_actionable_task: bool,
    pub(crate) had_failure: bool,
    pub(crate) deferred: bool,
    pub(crate) detail: Option<String>,
}

pub(crate) async fn defer_startup_backfill_task(
    state: &AppState,
    task: StartupBackfillTask,
    delay: Duration,
    wake_reason: &'static str,
) -> Result<()> {
    let task_name = startup_backfill_task_progress_key(state, task).await;
    let progress = load_startup_backfill_progress(&state.pool, &task_name).await?;
    let retry_after = Utc::now() + ChronoDuration::from_std(delay).unwrap_or_default();
    let retry_after = format_utc_iso(retry_after);
    save_startup_backfill_progress(
        &state.pool,
        &task_name,
        StartupBackfillProgressUpdate {
            cursor_id: progress.cursor_id,
            scanned: progress.last_scanned,
            updated: progress.last_updated,
            zero_update_streak: progress.zero_update_streak,
            next_run_after: &retry_after,
            status: &progress.last_status,
            suspension_reason: progress.suspension_reason.as_deref(),
        },
    )
    .await?;
    let retry_at = parse_to_utc_datetime(&retry_after).unwrap_or_else(Utc::now);
    STARTUP_BACKFILL_SCHEDULER.record_next_due(task, retry_at);
    info!(
        task = task.log_label(),
        next_retry_after = %retry_after,
        wake_reason,
        "startup backfill task retry scheduled"
    );
    Ok(())
}

pub(crate) fn coverage_repair_retry_delay(retry_generation: u32) -> Duration {
    let index = retry_generation.saturating_sub(1) as usize;
    Duration::from_secs(
        COVERAGE_REPAIR_RETRY_DELAYS_SECS[index.min(COVERAGE_REPAIR_RETRY_DELAYS_SECS.len() - 1)],
    )
}

pub(crate) async fn defer_startup_backfill_coverage_repair(
    state: &AppState,
) -> Result<DateTime<Utc>> {
    let task = StartupBackfillTask::AccountActivityV2Coverage;
    let task_name = startup_backfill_task_progress_key(state, task).await;
    let progress = load_startup_backfill_progress(&state.pool, &task_name).await?;
    let retry_generation = progress.zero_update_streak.saturating_add(1);
    let delay = coverage_repair_retry_delay(retry_generation);
    let retry_after = Utc::now() + ChronoDuration::from_std(delay).unwrap_or_default();
    let retry_after = format_utc_iso(retry_after);
    save_startup_backfill_progress(
        &state.pool,
        &task_name,
        StartupBackfillProgressUpdate {
            cursor_id: progress.cursor_id,
            scanned: progress.last_scanned,
            updated: progress.last_updated,
            zero_update_streak: retry_generation,
            next_run_after: &retry_after,
            status: &progress.last_status,
            suspension_reason: progress.suspension_reason.as_deref(),
        },
    )
    .await?;
    let retry_at = parse_to_utc_datetime(&retry_after).unwrap_or_else(Utc::now);
    STARTUP_BACKFILL_SCHEDULER.record_next_due(task, retry_at);
    let backoff_stage = match delay.as_secs() {
        0..=15 => "15s",
        16..=60 => "1m",
        61..=300 => "5m",
        _ => "15m",
    };
    info!(
        task = task.log_label(),
        next_retry_after = %retry_after,
        retry_generation,
        backoff_stage,
        wake_reason = "coverage_repair_retry",
        "startup backfill coverage repair retry scheduled"
    );
    Ok(retry_at)
}

pub(crate) async fn record_startup_backfill_coverage_repair_progress(
    state: &AppState,
    outcome: ActiveAccountActivityV2RepairOutcome,
) -> Result<DateTime<Utc>> {
    if outcome.repaired_bucket_count == 0 {
        return Ok(Utc::now());
    }

    let task = StartupBackfillTask::AccountActivityV2Coverage;
    let task_name = startup_backfill_task_progress_key(state, task).await;
    let progress = load_startup_backfill_progress(&state.pool, &task_name).await?;
    let retry_after = format_utc_iso(
        Utc::now() + ChronoDuration::seconds(STARTUP_BACKFILL_ACTIVE_INTERVAL_SECS as i64),
    );
    save_startup_backfill_progress(
        &state.pool,
        &task_name,
        StartupBackfillProgressUpdate {
            cursor_id: progress.cursor_id,
            scanned: progress.last_scanned,
            updated: progress.last_updated,
            zero_update_streak: 0,
            next_run_after: &retry_after,
            status: STARTUP_BACKFILL_STATUS_OK,
            suspension_reason: None,
        },
    )
    .await?;
    let retry_at = parse_to_utc_datetime(&retry_after).unwrap_or_else(Utc::now);
    STARTUP_BACKFILL_SCHEDULER.record_next_due(task, retry_at);
    info!(
        task = task.log_label(),
        coverage_priority_bucket_count = outcome.priority_bucket_count,
        repaired_bucket_count = outcome.repaired_bucket_count,
        next_retry_after = %retry_after,
        retry_generation = 0_u32,
        backoff_stage = "15s",
        wake_reason = "coverage_repair_progress",
        "startup backfill coverage repair progress reset its retry backoff"
    );
    Ok(retry_at)
}

pub(crate) async fn wake_startup_backfill_coverage_repair(
    pool: &Pool<Sqlite>,
    wake_reason: &'static str,
) -> Result<u64> {
    let task = StartupBackfillTask::AccountActivityV2Coverage;
    let ticket = STARTUP_BACKFILL_SCHEDULER
        .record_notification(task, startup_backfill_wake_decision(wake_reason));
    wake_startup_backfill_coverage_repair_inner(
        startup_backfill_progress_pool(pool),
        wake_reason,
        ticket,
    )
    .await
}

pub(super) async fn wake_startup_backfill_coverage_repair_inner(
    pool: Option<&Pool<Sqlite>>,
    wake_reason: &'static str,
    ticket: Option<WakeObservationTicket>,
) -> Result<u64> {
    let task = StartupBackfillTask::AccountActivityV2Coverage;
    let Some(pool) = pool else {
        STARTUP_BACKFILL_SCHEDULER.wake(task);
        return Ok(0);
    };
    let task_name = task.name();
    let progress = load_startup_backfill_progress(pool, task_name).await?;
    let deadline_preserved = !progress.is_due(Utc::now())
        && (progress.zero_update_streak > 0 || progress.last_status == STARTUP_BACKFILL_STATUS_OK);
    if deadline_preserved {
        STARTUP_BACKFILL_SCHEDULER.record_next_due(task, startup_backfill_progress_due(&progress));
        info!(
            task = task.log_label(),
            wake_reason,
            deadline_preserved,
            retry_generation = progress.zero_update_streak,
            "kept account activity v2 coverage repair on its active follow-up deadline"
        );
        return Ok(progress.wake_generation);
    }

    observe_wake_attempt(task, async {
        crate::maintenance_store::observe_store_write(
            crate::observability::contention::StoreOperation::Wake,
            async {
                sqlx::query(
                    r#"
        INSERT INTO startup_backfill_progress (
            task_name,
            cursor_id,
            next_run_after,
            zero_update_streak,
            last_started_at,
            last_finished_at,
            last_scanned,
            last_updated,
            last_status,
            suspension_reason,
            next_probe_at,
            wake_generation
        )
        VALUES (?1, 0, NULL, 0, NULL, NULL, 0, 0, ?2, NULL, NULL, 1)
        ON CONFLICT(task_name) DO UPDATE SET
            next_run_after = CASE
                WHEN startup_backfill_progress.zero_update_streak > 0
                    THEN startup_backfill_progress.next_run_after
                ELSE NULL
            END,
            last_status = CASE
                WHEN startup_backfill_progress.zero_update_streak > 0
                    THEN startup_backfill_progress.last_status
                ELSE ?2
            END,
            wake_generation = startup_backfill_progress.wake_generation + 1
                    "#,
                )
                .bind(task_name)
                .bind(STARTUP_BACKFILL_STATUS_IDLE)
                .execute(pool)
                .await?;
                Ok::<_, anyhow::Error>(())
            },
        )
        .await?;
        Ok::<((), bool), anyhow::Error>(((), true))
    })
    .await?;

    if let Some(ticket) = ticket {
        confirm_scheduler_wake_observation(&STARTUP_BACKFILL_SCHEDULER, ticket);
    }
    let progress = load_startup_backfill_progress(pool, task_name).await?;
    STARTUP_BACKFILL_SCHEDULER.wake(task);
    info!(
        task = task.log_label(),
        wake_reason,
        deadline_preserved,
        retry_generation = progress.zero_update_streak,
        "woke account activity v2 coverage repair"
    );
    Ok(progress.wake_generation)
}

pub(super) fn startup_backfill_hourly_rollup_refresh_scope() -> HourlyRollupRefreshScope {
    // The dedicated coverage task owns the active-window planner and its retry
    // deadline. Generic backfill refreshes must never re-enter that planner.
    HourlyRollupRefreshScope::SkipActiveAccountActivityV2CoverageRepair
}

pub(crate) async fn run_startup_backfill_maintenance_pass(
    state: Arc<AppState>,
    cancel: &CancellationToken,
    selected_tasks: Option<&[StartupBackfillTask]>,
) -> StartupBackfillMaintenancePass {
    run_startup_backfill_maintenance_pass_with_gate(
        state,
        cancel,
        selected_tasks,
        crate::db_pressure::global_db_pressure_gate(),
    )
    .await
}

pub(crate) async fn run_startup_backfill_maintenance_pass_with_gate(
    state: Arc<AppState>,
    cancel: &CancellationToken,
    selected_tasks: Option<&[StartupBackfillTask]>,
    gate: &crate::db_pressure::DbPressureGate,
) -> StartupBackfillMaintenancePass {
    let Some(_execution_lease) =
        crate::maintenance_store::try_acquire_task_execution("startup_backfill")
    else {
        if let Some(tasks) = selected_tasks {
            STARTUP_BACKFILL_SCHEDULER.requeue_after_dispatch_gap(
                tasks,
                Utc::now() + ChronoDuration::seconds(STARTUP_BACKFILL_ACTIVE_INTERVAL_SECS as i64),
            );
        }
        return StartupBackfillMaintenancePass::default();
    };
    let pass =
        run_startup_backfill_maintenance_pass_with_gate_held(state, cancel, selected_tasks, gate)
            .await;
    if cancel.is_cancelled()
        && let Some(tasks) = selected_tasks
    {
        STARTUP_BACKFILL_SCHEDULER.requeue_after_dispatch_gap(
            tasks,
            Utc::now() + ChronoDuration::seconds(STARTUP_BACKFILL_ACTIVE_INTERVAL_SECS as i64),
        );
    }
    pass
}

pub(super) async fn run_startup_backfill_maintenance_pass_with_gate_held(
    state: Arc<AppState>,
    cancel: &CancellationToken,
    selected_tasks: Option<&[StartupBackfillTask]>,
    gate: &crate::db_pressure::DbPressureGate,
) -> StartupBackfillMaintenancePass {
    run_startup_backfill_maintenance_pass_with_gate_inner(
        state,
        cancel,
        selected_tasks,
        gate,
        observation_parent_task_key(selected_tasks),
        None,
    )
    .await
}

pub(crate) async fn run_startup_backfill_maintenance_pass_managed(
    state: Arc<AppState>,
    cancel: &CancellationToken,
    selected_tasks: Option<&[StartupBackfillTask]>,
    observation_parent_task_key: Option<&'static str>,
    managed_run_id: Option<i64>,
) -> StartupBackfillMaintenancePass {
    // The managed root owns legacy backfills. Prompt-cache materialization has
    // its own managed task and must only enter through an explicit selection.
    let owned_tasks = selected_tasks
        .map(|tasks| tasks.to_vec())
        .unwrap_or_else(|| legacy_startup_backfill_tasks(None));
    run_startup_backfill_maintenance_pass_with_gate_inner(
        state,
        cancel,
        Some(&owned_tasks),
        crate::db_pressure::global_db_pressure_gate(),
        observation_parent_task_key,
        managed_run_id,
    )
    .await
}

async fn begin_startup_backfill_audit(
    state: &Arc<AppState>,
    cancel: &CancellationToken,
    task_kind: SystemTaskKind,
) -> Result<Option<SystemTaskRunHandle>> {
    tokio::select! {
        biased;
        _ = cancel.cancelled() => Ok(None),
        result = begin_system_task_run_admitted(
            state.as_ref(),
            crate::proxy_sqlite_write_coordinator::ProxySqliteWriteClass::P2Derived,
            task_kind,
            "event_or_due",
            Some("startup backfill maintenance changed data or failed".to_string()),
        ) => result.map(Some),
    }
}

async fn run_startup_backfill_maintenance_pass_with_gate_inner(
    state: Arc<AppState>,
    cancel: &CancellationToken,
    selected_tasks: Option<&[StartupBackfillTask]>,
    gate: &crate::db_pressure::DbPressureGate,
    observation_parent_task_key: Option<&'static str>,
    managed_run_id: Option<i64>,
) -> StartupBackfillMaintenancePass {
    let mut had_failure = false;
    let mut ran_actionable_task = false;
    let mut had_deferred_task = false;
    let mut detail = None;
    let tasks = match selected_tasks {
        Some(tasks) => tasks,
        None => StartupBackfillTask::ordered_tasks(),
    };
    for task in tasks {
        if cancel.is_cancelled() {
            info!(
                task = task.log_label(),
                "startup backfill maintenance stopped at a task boundary because shutdown is in progress"
            );
            break;
        }
        if *task == StartupBackfillTask::ProxyUsage && !state.config.proxy_usage_backfill_on_startup
        {
            debug!(
                task = task.log_label(),
                "startup backfill task is disabled by config"
            );
            STARTUP_BACKFILL_SCHEDULER.clear_next_due(*task);
            continue;
        }
        let task_result = tokio::select! {
            biased;
            _ = cancel.cancelled() => break,
            result = run_startup_backfill_task_if_due_outcome(
                &state,
                *task,
                gate,
                observation_parent_task_key,
                managed_run_id,
            ) => result,
        };
        let apply_task_result = || match task_result {
            Ok((outcome, task_detail)) => {
                if !crate::maintenance::maintenance_execution_is_manual() {
                    STARTUP_BACKFILL_SCHEDULER.record_next_due(*task, outcome.next_due);
                }
                ran_actionable_task |= outcome.actionable;
                had_failure |= outcome.failed;
                had_deferred_task |= outcome.deferred;
                if task_detail.is_some() {
                    detail = task_detail;
                }
                if outcome.completed {
                    STARTUP_BACKFILL_SCHEDULER.record_task_result_with_actionable(
                        *task,
                        outcome.failed,
                        outcome.deferred,
                        outcome.actionable,
                    );
                }
            }
            Err(err) => {
                had_failure = true;
                STARTUP_BACKFILL_SCHEDULER.record_task_result(*task, true, false);
                if !crate::maintenance::maintenance_execution_is_manual() {
                    STARTUP_BACKFILL_SCHEDULER.record_next_due(
                        *task,
                        Utc::now()
                            + ChronoDuration::seconds(STARTUP_BACKFILL_ACTIVE_INTERVAL_SECS as i64),
                    );
                }
                warn!(task = task.log_label(), error = %err, "startup backfill supervisor pass failed");
            }
        };
        apply_task_result();
    }

    if ran_actionable_task {
        had_deferred_task |= refresh_hourly_rollups_for_read_surfaces_best_effort(
            &state.pool,
            state.hourly_rollup_sync_lock.as_ref(),
            cancel,
            "startup backfill maintenance pass",
            startup_backfill_hourly_rollup_refresh_scope(),
        )
        .await;
        if !cancel.is_cancelled() {
            let _guard = tokio::select! {
                biased;
                _ = cancel.cancelled() => {
                    return StartupBackfillMaintenancePass {
                        ran_actionable_task,
                        had_failure,
                        deferred: had_deferred_task,
                        detail,
                    };
                }
                guard = state.hourly_rollup_sync_lock.lock() => guard,
            };
            let gate = crate::db_pressure::global_db_pressure_gate();
            if let Ok(_pressure_permit) =
                gate.try_begin_background("parallel_work_rollup_maintenance")
            {
                if let Some(_write_permit) =
                    crate::proxy_sqlite_write_coordinator::proxy_sqlite_write_coordinator()
                        .try_acquire(
                            crate::proxy_sqlite_write_coordinator::ProxySqliteWriteClass::P2Derived,
                        )
                {
                    let live_start_epoch =
                        shanghai_retention_cutoff(state.config.invocation_max_days).timestamp();
                    let coordinator =
                        crate::proxy_sqlite_write_coordinator::proxy_sqlite_write_coordinator();
                    let maintenance_deferred = tokio::select! {
                        _ = cancel.cancelled() => {
                            debug!("parallel-work rollup maintenance cancelled during SQLite work");
                            true
                        }
                        _ = coordinator.wait_for_p2_preemption() => {
                            debug!("parallel-work rollup maintenance yielded to higher-priority SQLite writes");
                            true
                        }
                        result = maintain_parallel_work_rollups(&state.pool, Some(live_start_epoch)) => {
                            if let Err(err) = result {
                                had_failure = true;
                                gate.record_error("parallel_work_rollup_maintenance", &err);
                                warn!(error = %err, "parallel-work rollup maintenance pass failed");
                            }
                            false
                        }
                    };
                    had_deferred_task |= maintenance_deferred;
                } else {
                    had_deferred_task = true;
                    debug!(
                        defer_reason = "coordinator_priority",
                        "parallel-work rollup maintenance deferred before SQLite access"
                    );
                }
            } else {
                had_deferred_task = true;
                debug!(
                    defer_reason = "database_pressure",
                    "parallel-work rollup maintenance deferred before SQLite access"
                );
            }
        }
    }

    if should_record_audit(
        managed_run_id,
        ran_actionable_task,
        had_failure,
        detail.is_some(),
        cancel.is_cancelled(),
    ) {
        // The audit row is non-critical bookkeeping. Do not hold a pressure slot while the
        // P2 admission waits: if cancellation wins, leave the row for the next pass. The durable
        // task/progress rows above remain the source of truth.
        let gate = crate::db_pressure::global_db_pressure_gate();
        match gate.try_begin_background("startup_backfill_audit") {
            Ok(pressure_permit) => {
                // Do not hold the background slot while waiting for the process-wide write
                // coordinator. P1/interactive writes can still preempt this P2 audit waiter.
                drop(pressure_permit);
                let task_run =
                    begin_startup_backfill_audit(&state, cancel, audit_task_kind(selected_tasks))
                        .await;
                match task_run {
                    Ok(Some(run)) => {
                        let audit_status = if had_failure {
                            SystemTaskStatus::Failed
                        } else if had_deferred_task {
                            SystemTaskStatus::Skipped
                        } else {
                            SystemTaskStatus::Success
                        };
                        let audit_summary = if had_failure {
                            "startup backfill maintenance pass completed with failures"
                        } else if had_deferred_task {
                            "startup backfill maintenance pass deferred work for a later run"
                        } else {
                            "startup backfill maintenance pass completed"
                        };
                        if !finish_system_task_run_reliably(
                            state.as_ref(),
                            Some(cancel),
                            &run,
                            audit_status,
                            Some(audit_summary.to_string()),
                            detail.clone(),
                        )
                        .await
                        {
                            warn!(
                                task_kind = run.task_kind.as_str(),
                                trigger_kind = %run.trigger_kind,
                                "failed to durably finalize startup backfill maintenance audit"
                            );
                        }
                    }
                    Ok(None) => {
                        debug!(
                            defer_reason = "cancelled",
                            "startup backfill maintenance audit cancelled before SQLite access"
                        );
                    }
                    Err(err) => {
                        warn!(
                            error = %err,
                            "failed to record startup backfill maintenance audit start"
                        );
                    }
                }
            }
            Err(reason) => {
                debug!(
                    defer_reason = %reason,
                    "startup backfill maintenance audit deferred before SQLite access"
                );
            }
        }
    } else if !had_deferred_task {
        // Idle passes deliberately do not write system_task_runs, avoiding an audit workload
        // that would itself wake persistence maintenance.
        STARTUP_BACKFILL_SCHEDULER.record_noop_suppressed();
    }

    StartupBackfillMaintenancePass {
        ran_actionable_task,
        had_failure,
        deferred: had_deferred_task,
        detail,
    }
}

pub(crate) async fn run_startup_persistent_prep_best_effort(
    state: &Arc<AppState>,
    prep_cli: &CliArgs,
) -> bool {
    if !should_run_startup_persistent_prep(prep_cli) {
        return true;
    }

    // The individual manifest and expiry writers acquire the maintenance permit themselves.
    // Holding the global background slot here would make those nested admissions reject every
    // startup pass before they can do any work.
    let defer_generation = super::retention::retention_defer_generation();
    match run_startup_persistent_prep_inner(&state.pool, &state.config, prep_cli, false).await {
        Ok(summary) => {
            let deferred = super::retention::retention_defer_generation() != defer_generation;
            info!(
                refreshed_manifest_batches = summary.refreshed_manifest_batches,
                refreshed_manifest_account_rows = summary.refreshed_manifest_account_rows,
                missing_manifest_files = summary.missing_manifest_files,
                backfilled_archive_expiries = summary.backfilled_archive_expiries,
                bootstrapped_hourly_rollups = summary.bootstrapped_hourly_rollups,
                pending_manifest_batches = summary.pending_manifest_batches,
                deferred,
                "startup background prep finished"
            );
            !deferred && summary.pending_manifest_batches == 0
        }
        Err(err) => {
            let pressure_error = crate::db_pressure::global_db_pressure_gate()
                .record_error("startup_persistent_prep", &err);
            warn!(error = %err, retry_soon = pressure_error, "startup background prep failed");
            !pressure_error
        }
    }
}

async fn run_startup_persistent_prep_best_effort_cancellable(
    state: &Arc<AppState>,
    prep_cli: &CliArgs,
    cancel: &CancellationToken,
) -> Option<bool> {
    tokio::select! {
        biased;
        _ = cancel.cancelled() => None,
        result = run_startup_persistent_prep_best_effort(state, prep_cli) => Some(result),
    }
}

pub(crate) async fn run_pressure_eligible_startup_backfill_tasks(
    state: Arc<AppState>,
    cancel: &CancellationToken,
    gate: &crate::db_pressure::DbPressureGate,
) {
    if cancel.is_cancelled() {
        return;
    }
    let Some(_execution_lease) =
        crate::maintenance_store::try_acquire_task_execution("startup_backfill")
    else {
        return;
    };
    let startup_backfill_root_skipped =
        crate::maintenance_store::legacy_worker_should_skip("startup_backfill").await;
    let prompt_cache_task = [StartupBackfillTask::PromptCacheConversationsMaterialization];
    let legacy_allowed_tasks = legacy_startup_backfill_tasks(None);
    let legacy_tasks = if startup_backfill_root_skipped {
        Vec::new()
    } else {
        STARTUP_BACKFILL_SCHEDULER.take_pressure_deferred_tasks_matching(
            Utc::now(),
            Some(legacy_allowed_tasks.as_slice()),
        )
    };
    // The prompt task has an independent owner even when the legacy root is enabled.
    let prompt_tasks = STARTUP_BACKFILL_SCHEDULER
        .take_pressure_deferred_tasks_matching(Utc::now(), Some(prompt_cache_task.as_slice()));
    let all_tasks = legacy_tasks
        .iter()
        .chain(prompt_tasks.iter())
        .copied()
        .collect::<Vec<_>>();
    if all_tasks.is_empty() {
        return;
    }
    debug!(
        task_count = legacy_tasks.len() + prompt_tasks.len(),
        "pressure eligibility changed; dispatching deferred startup backfill tasks"
    );
    if !legacy_tasks.is_empty() {
        run_background_startup_backfill_pass_with_root_state_held(
            state.clone(),
            cancel,
            Some(&legacy_tasks),
            gate,
            startup_backfill_root_skipped,
        )
        .await;
    }
    if !prompt_tasks.is_empty() {
        run_background_startup_backfill_pass_with_root_state_held(
            state,
            cancel,
            Some(&prompt_tasks),
            gate,
            true,
        )
        .await;
    }
    if cancel.is_cancelled() {
        STARTUP_BACKFILL_SCHEDULER.requeue_after_dispatch_gap(
            &all_tasks,
            Utc::now() + ChronoDuration::seconds(STARTUP_BACKFILL_ACTIVE_INTERVAL_SECS as i64),
        );
    }
}

async fn run_background_startup_backfill_pass_with_gate(
    state: Arc<AppState>,
    cancel: &CancellationToken,
    selected_tasks: Option<&[StartupBackfillTask]>,
    gate: &crate::db_pressure::DbPressureGate,
) -> StartupBackfillMaintenancePass {
    let startup_backfill_root_skipped =
        crate::maintenance_store::legacy_worker_should_skip("startup_backfill").await;
    let retry_tasks =
        startup_backfill_tasks_for_retry(selected_tasks, startup_backfill_root_skipped);
    let Some(_execution_lease) =
        crate::maintenance_store::try_acquire_task_execution("startup_backfill")
    else {
        STARTUP_BACKFILL_SCHEDULER.requeue_after_dispatch_gap(
            &retry_tasks,
            Utc::now() + ChronoDuration::seconds(STARTUP_BACKFILL_ACTIVE_INTERVAL_SECS as i64),
        );
        return StartupBackfillMaintenancePass::default();
    };
    let pass = run_background_startup_backfill_pass_with_root_state_held(
        state,
        cancel,
        selected_tasks,
        gate,
        startup_backfill_root_skipped,
    )
    .await;
    if cancel.is_cancelled() {
        STARTUP_BACKFILL_SCHEDULER.requeue_after_dispatch_gap(
            &retry_tasks,
            Utc::now() + ChronoDuration::seconds(STARTUP_BACKFILL_ACTIVE_INTERVAL_SECS as i64),
        );
    }
    pass
}

async fn run_background_startup_backfill_pass_with_root_state_held(
    state: Arc<AppState>,
    cancel: &CancellationToken,
    selected_tasks: Option<&[StartupBackfillTask]>,
    gate: &crate::db_pressure::DbPressureGate,
    startup_backfill_root_skipped: bool,
) -> StartupBackfillMaintenancePass {
    run_background_startup_backfill_pass_with_root_state(
        state,
        cancel,
        selected_tasks,
        gate,
        startup_backfill_root_skipped,
    )
    .await
}

pub(crate) fn spawn_startup_backfill_maintenance(
    state: Arc<AppState>,
    cancel: CancellationToken,
) -> JoinHandle<()> {
    crate::observability::contention::spawn_performance(state.observability.clone(), async move {
        if cancel.is_cancelled() {
            info!("startup backfill maintenance skipped because shutdown is already in progress");
            return;
        }
        let prep_cli = CliArgs::default();
        let Some(prep_pending) =
            run_startup_persistent_prep_best_effort_cancellable(&state, &prep_cli, &cancel).await
        else {
            return;
        };
        let mut startup_prep_pending = prep_pending;
        let mut startup_prep_retry_at = startup_prep_pending
            .then(|| Instant::now() + Duration::from_secs(STARTUP_BACKFILL_ACTIVE_INTERVAL_SECS));
        let mut observed_generation = STARTUP_BACKFILL_SCHEDULER.generation();
        run_background_startup_backfill_pass_with_gate(
            state.clone(),
            &cancel,
            None,
            crate::db_pressure::global_db_pressure_gate(),
        )
        .await;
        // Register before either P2 supervisor is scheduled so long-term pruning cannot
        // reclaim a terminal event ahead of the minute projection consumer.
        state
            .terminal_projection_hub
            .activate_timeseries_consumer(0);
        let mut long_term_projection_handle = None;
        let mut timeseries_minute_projection_handle = None;
        if !crate::maintenance_store::legacy_worker_should_skip("long_term_projection").await {
            long_term_projection_handle = Some(spawn_long_term_projection_supervisor(
                state.clone(),
                cancel.clone(),
            ));
        }
        if !crate::maintenance_store::legacy_worker_should_skip("timeseries_minute_projection")
            .await
        {
            timeseries_minute_projection_handle = Some(
                spawn_timeseries_minute_projection_supervisor(state.clone(), cancel.clone()),
            );
        }

        loop {
            if STARTUP_BACKFILL_SCHEDULER
                .next_wake_obligation_due()
                .is_some_and(|deadline| deadline <= Utc::now())
            {
                confirm_startup_backfill_wake_obligations(&cancel).await;
            }
            let gate = crate::db_pressure::global_db_pressure_gate();
            let observed_pressure_eligibility = gate.eligibility_generation();
            let next_due = STARTUP_BACKFILL_SCHEDULER
                .next_due()
                .into_iter()
                .chain(STARTUP_BACKFILL_SCHEDULER.next_wake_obligation_due())
                .min();
            let mut wait_for = startup_backfill_wait_duration(next_due);
            if let Some(retry_at) = startup_prep_retry_at {
                wait_for = wait_for.min(retry_at.saturating_duration_since(Instant::now()));
            }

            tokio::select! {
                _ = cancel.cancelled() => {
                    info!("startup backfill maintenance received shutdown");
                    break;
                }
                _ = STARTUP_BACKFILL_SCHEDULER.wait_for_wake(observed_generation) => {
                    observed_generation = STARTUP_BACKFILL_SCHEDULER.generation();
                    confirm_startup_backfill_wake_obligations(&cancel).await;
                    let tasks = STARTUP_BACKFILL_SCHEDULER.drain_woken_tasks();
                    if !tasks.is_empty() {
                        run_background_startup_backfill_pass_with_gate(
                            state.clone(),
                            &cancel,
                            Some(&tasks),
                            crate::db_pressure::global_db_pressure_gate(),
                        )
                        .await;
                    }
                }
                _ = gate.wait_for_eligibility_change(observed_pressure_eligibility) => {
                    run_pressure_eligible_startup_backfill_tasks(state.clone(), &cancel, gate).await;
                }
                _ = sleep(wait_for) => {
                    observed_generation = STARTUP_BACKFILL_SCHEDULER.generation();
                    confirm_startup_backfill_wake_obligations(&cancel).await;
                    let woken_tasks = STARTUP_BACKFILL_SCHEDULER.drain_woken_tasks();
                    if !woken_tasks.is_empty() {
                        run_background_startup_backfill_pass_with_gate(
                            state.clone(),
                            &cancel,
                            Some(&woken_tasks),
                            crate::db_pressure::global_db_pressure_gate(),
                        )
                        .await;
                    }
                    if startup_prep_pending
                        && startup_prep_retry_at.is_none_or(|retry_at| retry_at <= Instant::now())
                    {
                        let Some(prep_pending) =
                            run_startup_persistent_prep_best_effort_cancellable(&state, &prep_cli, &cancel)
                                .await
                        else {
                            break;
                        };
                        startup_prep_pending = prep_pending;
                        startup_prep_retry_at = startup_prep_pending.then(|| {
                            Instant::now()
                                + Duration::from_secs(STARTUP_BACKFILL_ACTIVE_INTERVAL_SECS)
                        });
                    }
                    let due_tasks = STARTUP_BACKFILL_SCHEDULER.drain_due_tasks(Utc::now());
                    if !due_tasks.is_empty() {
                        run_background_startup_backfill_pass_with_gate(
                            state.clone(),
                            &cancel,
                            Some(&due_tasks),
                            crate::db_pressure::global_db_pressure_gate(),
                        )
                        .await;
                    }
                }
            }
        }
        if let Some(handle) = long_term_projection_handle
            && let Err(error) = handle.await
        {
            warn!(error = %error, "long-term projection supervisor terminated unexpectedly");
        }
        if let Some(handle) = timeseries_minute_projection_handle
            && let Err(error) = handle.await
        {
            warn!(error = %error, "timeseries minute projection supervisor terminated unexpectedly");
        }
    })
}
