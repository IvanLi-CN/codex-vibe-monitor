pub(crate) fn startup_backfill_task_enabled(state: &AppState, task: StartupBackfillTask) -> bool {
    match task {
        StartupBackfillTask::ProxyUsage => state.config.proxy_usage_backfill_on_startup,
        _ => true,
    }
}

enum StartupBackfillExecution {
    Completed(Result<(StartupBackfillRunState, String)>),
    Deferred(StartupBackfillTaskRunOutcome),
}

pub(crate) async fn run_startup_backfill_task_if_due(
    state: &Arc<AppState>,
    task: StartupBackfillTask,
) -> Result<bool> {
    run_startup_backfill_task_if_due_outcome(
        state,
        task,
        crate::db_pressure::global_db_pressure_gate(),
    )
    .await
    .map(|outcome| outcome.actionable)
}

pub(crate) async fn run_startup_backfill_task_if_due_with_gate(
    state: &Arc<AppState>,
    task: StartupBackfillTask,
    gate: &crate::db_pressure::DbPressureGate,
) -> Result<bool> {
    run_startup_backfill_task_if_due_outcome(state, task, gate)
        .await
        .map(|outcome| outcome.actionable)
}

async fn run_startup_backfill_task_if_due_outcome(
    state: &Arc<AppState>,
    task: StartupBackfillTask,
    gate: &crate::db_pressure::DbPressureGate,
) -> Result<StartupBackfillTaskRunOutcome> {
    if !startup_backfill_task_enabled(state.as_ref(), task) {
        return Ok(startup_backfill_disabled_outcome(task));
    }
    if task == StartupBackfillTask::AccountActivityV2Coverage {
        return run_startup_backfill_coverage_repair_if_due(state, gate).await;
    }
    if startup_backfill_waits_for_summary_projection(state, task).await {
        return Ok(startup_backfill_projection_wait_outcome());
    }
    run_startup_backfill_task_if_due_with_admission(state, task, gate).await
}

fn startup_backfill_disabled_outcome(task: StartupBackfillTask) -> StartupBackfillTaskRunOutcome {
    debug!(
        task = task.log_label(),
        "startup backfill task is disabled by config"
    );
    StartupBackfillTaskRunOutcome {
        actionable: false,
        failed: false,
        deferred: false,
        completed: true,
        next_due: Utc::now() + ChronoDuration::days(1),
    }
}

async fn startup_backfill_waits_for_summary_projection(
    state: &Arc<AppState>,
    task: StartupBackfillTask,
) -> bool {
    task == StartupBackfillTask::LegacyDetailMirrors
        && state.subscription_hub.summary_projection().await.is_none()
}

fn startup_backfill_projection_wait_outcome() -> StartupBackfillTaskRunOutcome {
    StartupBackfillTaskRunOutcome {
        actionable: false,
        failed: false,
        deferred: false,
        completed: false,
        next_due: Utc::now()
            + ChronoDuration::seconds(STARTUP_BACKFILL_ACTIVE_INTERVAL_SECS as i64),
    }
}

async fn run_startup_backfill_task_if_due_with_admission(
    state: &Arc<AppState>,
    task: StartupBackfillTask,
    gate: &crate::db_pressure::DbPressureGate,
) -> Result<StartupBackfillTaskRunOutcome> {
    // Pool health archive backfill shares the hourly-rollup synchronization lock. Acquire that
    // lock before P2 admission so another maintenance path cannot hold the lock while waiting
    // for the coordinator and deadlock an already-admitted P2 task.
    let _hourly_rollup_guard = if task == StartupBackfillTask::PoolUpstreamNodeHealthArchives {
        Some(state.hourly_rollup_sync_lock.lock().await)
    } else {
        None
    };

    // This admission is deliberately before progress lookup: a pressure defer must remain a
    // scheduler-only decision, not turn into a SQLite read, progress write, or task-run audit.
    let _permit = match gate.try_begin_background("startup_backfill") {
        Ok(permit) => permit,
        Err(reason) => return Ok(startup_backfill_pressure_defer_outcome(task, gate, reason)),
    };
    let Some(write_permit) =
        crate::proxy_sqlite_write_coordinator::proxy_sqlite_write_coordinator()
            .try_acquire(crate::proxy_sqlite_write_coordinator::ProxySqliteWriteClass::P2Derived)
    else {
        return Ok(startup_backfill_pressure_defer_outcome(
            task,
            gate,
            crate::db_pressure::DbPressureDenyReason::BackgroundBusy,
        ));
    };
    run_startup_backfill_task_after_admission(state, task, gate, write_permit).await
}

async fn run_startup_backfill_task_after_admission(
    state: &Arc<AppState>,
    task: StartupBackfillTask,
    gate: &crate::db_pressure::DbPressureGate,
    write_permit: crate::proxy_sqlite_write_coordinator::ProxySqliteWritePermit,
) -> Result<StartupBackfillTaskRunOutcome> {
    let task_name = startup_backfill_task_progress_key(state.as_ref(), task).await;
    let progress = match load_startup_backfill_progress(&state.pool, &task_name).await {
        Ok(progress) => progress,
        Err(err) => {
            record_startup_backfill_pressure_error(gate, &err);
            return Err(err);
        }
    };
    if !progress.is_due(Utc::now()) {
        debug_startup_backfill_not_due(task, &progress);
        return Ok(startup_backfill_not_due_outcome(&progress));
    }
    mark_startup_backfill_running(&state.pool, &task_name, progress.cursor_id)
        .await
        .inspect_err(|err| {
            record_startup_backfill_pressure_error(gate, err);
        })?;

    let started_at = Instant::now();
    let task_result = run_startup_backfill_task_with_preemption(
        state,
        task,
        &task_name,
        &progress,
        gate,
        write_permit,
    )
    .await?;
    match task_result {
        StartupBackfillExecution::Deferred(outcome) => Ok(outcome),
        StartupBackfillExecution::Completed(Ok(run)) => {
            finish_startup_backfill_task_success(
                state, task, gate, &task_name, &progress, started_at, run,
            )
            .await
        }
        StartupBackfillExecution::Completed(Err(err)) => {
            finish_startup_backfill_task_failure(
                state, task, gate, &task_name, &progress, started_at, err,
            )
            .await
        }
    }
}

async fn run_startup_backfill_task_with_preemption(
    state: &Arc<AppState>,
    task: StartupBackfillTask,
    task_name: &str,
    progress: &StartupBackfillProgress,
    gate: &crate::db_pressure::DbPressureGate,
    write_permit: crate::proxy_sqlite_write_coordinator::ProxySqliteWritePermit,
) -> Result<StartupBackfillExecution> {
    let coordinator = crate::proxy_sqlite_write_coordinator::proxy_sqlite_write_coordinator();
    // Backfill implementations combine bounded SQL batches with file reads/decompression. If
    // an interactive writer arrives while this P2 task is active, cancel the in-flight future so
    // its transaction rolls back and release admission immediately for the higher-priority write.
    Ok(tokio::select! {
        biased;
        _ = coordinator.wait_for_p2_preemption() => {
            drop(write_permit);
            StartupBackfillExecution::Deferred(persist_startup_backfill_pressure_defer(
                state,
                task,
                task_name,
                progress,
                gate,
                crate::db_pressure::DbPressureDenyReason::BackgroundBusy,
            ).await? )
        }
        result = run_startup_backfill_task(
            state,
            task,
            progress.cursor_id,
            progress.zero_update_streak,
            progress.last_status == STARTUP_BACKFILL_STATUS_SOURCE_UNAVAILABLE,
        ) => StartupBackfillExecution::Completed(result),
    })
}

async fn finish_startup_backfill_task_success(
    state: &Arc<AppState>,
    task: StartupBackfillTask,
    gate: &crate::db_pressure::DbPressureGate,
    task_name: &str,
    progress: &StartupBackfillProgress,
    started_at: Instant,
    result: (StartupBackfillRunState, String),
) -> Result<StartupBackfillTaskRunOutcome> {
    let (run, detail) = result;
    let zero_update_streak = if run.updated == 0 {
        progress.zero_update_streak.saturating_add(1)
    } else {
        0
    };
    let next_cursor_id = if task == StartupBackfillTask::HistoricalRollups {
        run.next_cursor_id
    } else {
        run.next_cursor_id.max(progress.cursor_id)
    };
    let next_run_after = startup_backfill_next_run_after(&run, zero_update_streak);
    save_startup_backfill_progress(
        &state.pool,
        task_name,
        StartupBackfillProgressUpdate {
            cursor_id: next_cursor_id,
            scanned: run.scanned,
            updated: run.updated,
            zero_update_streak,
            next_run_after: &next_run_after,
            status: if run.source_unavailable {
                STARTUP_BACKFILL_STATUS_SOURCE_UNAVAILABLE
            } else {
                STARTUP_BACKFILL_STATUS_OK
            },
            suspension_reason: run.source_unavailable.then_some("source_unavailable"),
        },
    )
    .await
    .inspect_err(|err| {
        record_startup_backfill_pressure_error(gate, err);
    })?;
    log_startup_backfill_success(
        task,
        task_name,
        &run,
        detail,
        started_at,
        next_cursor_id,
        zero_update_streak,
    );
    Ok(StartupBackfillTaskRunOutcome {
        actionable: startup_backfill_run_is_actionable(&run),
        failed: false,
        deferred: false,
        completed: true,
        next_due: parse_to_utc_datetime(&next_run_after).unwrap_or_else(Utc::now),
    })
}

fn log_startup_backfill_success(
    task: StartupBackfillTask,
    task_name: &str,
    run: &StartupBackfillRunState,
    detail: String,
    started_at: Instant,
    next_cursor_id: i64,
    zero_update_streak: u32,
) {
    let next_run_after = startup_backfill_next_run_after(run, zero_update_streak);
    info!(
        task = task.log_label(),
        task_name = %task_name,
        scanned = run.scanned,
        updated = run.updated,
        cursor_id = next_cursor_id,
        hit_scan_limit = run.hit_scan_limit,
        zero_update_streak,
        elapsed_ms = started_at.elapsed().as_millis() as u64,
        next_run_after = %next_run_after,
        actionable_backlog_count = u64::from(run.hit_scan_limit),
        blocked_backlog_count = u64::from(run.source_unavailable),
        suspension_reason = if run.source_unavailable { "source_unavailable" } else { "none" },
        probe_budget_exhausted = false,
        backoff_stage = match startup_backfill_next_delay(run, zero_update_streak).as_secs() {
            0..=15 => "15s",
            16..=60 => "1m",
            61..=300 => "5m",
            301..=900 => "15m",
            _ => "idle",
        },
        wake_reason = if run.updated > 0 { "progress" } else if run.source_unavailable { "daily_probe" } else if run.hit_scan_limit { "actionable_backlog" } else { "scheduled" },
        detail = %detail,
        samples = %startup_backfill_samples_text(&run.samples),
        "startup backfill pass finished"
    );
}

async fn finish_startup_backfill_task_failure(
    state: &Arc<AppState>,
    task: StartupBackfillTask,
    gate: &crate::db_pressure::DbPressureGate,
    task_name: &str,
    progress: &StartupBackfillProgress,
    started_at: Instant,
    err: anyhow::Error,
) -> Result<StartupBackfillTaskRunOutcome> {
    let next_due = match persist_startup_backfill_task_failure(
        state, task, task_name, progress, started_at, &err,
    )
    .await
    {
        Ok(next_due) => next_due,
        Err(persist_err) => {
            if !record_startup_backfill_pressure_error(gate, &persist_err) {
                record_startup_backfill_pressure_error(gate, &err);
            }
            return Err(persist_err);
        }
    };
    // Keep the permit until the failure state is durable and any relevant cooldown is visible.
    // Releasing it first would let another background task enter SQLite.
    record_startup_backfill_pressure_error(gate, &err);
    Ok(StartupBackfillTaskRunOutcome {
        actionable: false,
        failed: true,
        deferred: false,
        completed: true,
        next_due,
    })
}
