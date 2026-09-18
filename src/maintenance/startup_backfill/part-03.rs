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
    run_startup_backfill_maintenance_pass_with_gate_inner(state, cancel, selected_tasks, gate).await
}

async fn begin_startup_backfill_audit(
    state: &Arc<AppState>,
    cancel: &CancellationToken,
) -> Result<Option<SystemTaskRunHandle>> {
    tokio::select! {
        biased;
        _ = cancel.cancelled() => Ok(None),
        result = begin_system_task_run_admitted(
            state.as_ref(),
            crate::proxy_sqlite_write_coordinator::ProxySqliteWriteClass::P2Derived,
            SystemTaskKind::StartupBackfill,
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
) -> StartupBackfillMaintenancePass {
    let tasks = match selected_tasks {
        Some(tasks) => tasks,
        None => StartupBackfillTask::ordered_tasks(),
    };
    let (mut had_failure, ran_actionable_task, mut had_deferred_task) =
        run_startup_backfill_tasks(&state, cancel, tasks, gate).await;

    if ran_actionable_task
        && run_parallel_work_maintenance_after_backfill(
            &state,
            cancel,
            &mut had_failure,
            &mut had_deferred_task,
        )
        .await
    {
        return StartupBackfillMaintenancePass {
            ran_actionable_task,
            had_failure,
        };
    }

    if (ran_actionable_task || had_failure) && !cancel.is_cancelled() {
        record_startup_backfill_audit(&state, cancel, had_failure, had_deferred_task).await;
    } else if !had_deferred_task {
        // Idle passes deliberately do not write system_task_runs, avoiding an audit workload
        // that would itself wake persistence maintenance.
        STARTUP_BACKFILL_SCHEDULER.record_noop_suppressed();
    }

    StartupBackfillMaintenancePass {
        ran_actionable_task,
        had_failure,
    }
}

async fn run_startup_backfill_tasks(
    state: &Arc<AppState>,
    cancel: &CancellationToken,
    tasks: &[StartupBackfillTask],
    gate: &crate::db_pressure::DbPressureGate,
) -> (bool, bool, bool) {
    let mut had_failure = false;
    let mut ran_actionable_task = false;
    let mut had_deferred_task = false;
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
            result = run_startup_backfill_task_if_due_outcome(&state, *task, gate) => result,
        };
        match task_result {
            Ok(outcome) => {
                STARTUP_BACKFILL_SCHEDULER.record_next_due(*task, outcome.next_due);
                ran_actionable_task |= outcome.actionable;
                had_failure |= outcome.failed;
                had_deferred_task |= outcome.deferred;
                if outcome.completed {
                    STARTUP_BACKFILL_SCHEDULER.record_task_result(
                        *task,
                        outcome.failed,
                        outcome.deferred,
                    );
                }
            }
            Err(err) => {
                had_failure = true;
                STARTUP_BACKFILL_SCHEDULER.record_task_result(*task, true, false);
                STARTUP_BACKFILL_SCHEDULER.record_next_due(
                    *task,
                    Utc::now()
                        + ChronoDuration::seconds(STARTUP_BACKFILL_ACTIVE_INTERVAL_SECS as i64),
                );
                warn!(task = task.log_label(), error = %err, "startup backfill supervisor pass failed");
            }
        }
    }
    (had_failure, ran_actionable_task, had_deferred_task)
}

async fn run_parallel_work_maintenance_after_backfill(
    state: &Arc<AppState>,
    cancel: &CancellationToken,
    had_failure: &mut bool,
    had_deferred_task: &mut bool,
) -> bool {
    *had_deferred_task |= refresh_hourly_rollups_for_read_surfaces_best_effort(
        &state.pool,
        state.hourly_rollup_sync_lock.as_ref(),
        cancel,
        "startup backfill maintenance pass",
        startup_backfill_hourly_rollup_refresh_scope(),
    )
    .await;
    if cancel.is_cancelled() {
        return false;
    }
    let _guard = tokio::select! {
        biased;
        _ = cancel.cancelled() => return true,
        guard = state.hourly_rollup_sync_lock.lock() => guard,
    };
    let pressure_gate = crate::db_pressure::global_db_pressure_gate();
    if let Ok(_pressure_permit) =
        pressure_gate.try_begin_background("parallel_work_rollup_maintenance")
    {
        let Some(_write_permit) =
            crate::proxy_sqlite_write_coordinator::proxy_sqlite_write_coordinator().try_acquire(
                crate::proxy_sqlite_write_coordinator::ProxySqliteWriteClass::P2Derived,
            )
        else {
            *had_deferred_task = true;
            debug!(
                defer_reason = "coordinator_priority",
                "parallel-work rollup maintenance deferred before SQLite access"
            );
            return false;
        };
        let live_start_epoch =
            shanghai_retention_cutoff(state.config.invocation_max_days).timestamp();
        let coordinator = crate::proxy_sqlite_write_coordinator::proxy_sqlite_write_coordinator();
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
                    *had_failure = true;
                    pressure_gate.record_error("parallel_work_rollup_maintenance", &err);
                    warn!(error = %err, "parallel-work rollup maintenance pass failed");
                }
                false
            }
        };
        *had_deferred_task |= maintenance_deferred;
    } else {
        *had_deferred_task = true;
        debug!(
            defer_reason = "database_pressure",
            "parallel-work rollup maintenance deferred before SQLite access"
        );
    }
    false
}

async fn record_startup_backfill_audit(
    state: &Arc<AppState>,
    cancel: &CancellationToken,
    had_failure: bool,
    had_deferred_task: bool,
) {
    let pressure_gate = crate::db_pressure::global_db_pressure_gate();
    let Ok(pressure_permit) = pressure_gate.try_begin_background("startup_backfill_audit") else {
        debug!(
            defer_reason = "database_pressure",
            "startup backfill maintenance audit deferred before SQLite access"
        );
        return;
    };
    drop(pressure_permit);
    let task_run = begin_startup_backfill_audit(state, cancel).await;
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
                None,
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
