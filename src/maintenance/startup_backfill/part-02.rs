async fn run_startup_backfill_maintenance_pass_with_gate_inner(
    state: Arc<AppState>,
    cancel: &CancellationToken,
    selected_tasks: Option<&[StartupBackfillTask]>,
    gate: &crate::db_pressure::DbPressureGate,
) -> StartupBackfillMaintenancePass {
    let mut had_failure = false;
    let mut ran_actionable_task = false;
    let mut had_deferred_task = false;
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

    if (ran_actionable_task || had_failure) && !cancel.is_cancelled() {
        // The audit row is non-critical bookkeeping. Do not hold a pressure slot while the
        // P2 admission waits: if cancellation wins, leave the row for the next pass. The durable
        // task/progress rows above remain the source of truth.
        let gate = crate::db_pressure::global_db_pressure_gate();
        match gate.try_begin_background("startup_backfill_audit") {
            Ok(pressure_permit) => {
                // Do not hold the background slot while waiting for the process-wide write
                // coordinator. P1/interactive writes can still preempt this P2 audit waiter.
                drop(pressure_permit);
                let task_run = begin_startup_backfill_audit(&state, cancel).await;
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
    }
}

pub(crate) fn startup_backfill_task_enabled(state: &AppState, task: StartupBackfillTask) -> bool {
    match task {
        StartupBackfillTask::ProxyUsage => state.config.proxy_usage_backfill_on_startup,
        _ => true,
    }
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
        debug!(
            task = task.log_label(),
            "startup backfill task is disabled by config"
        );
        return Ok(StartupBackfillTaskRunOutcome {
            actionable: false,
            failed: false,
            deferred: false,
            completed: true,
            next_due: Utc::now() + ChronoDuration::days(1),
        });
    }

    if task == StartupBackfillTask::AccountActivityV2Coverage {
        return run_startup_backfill_coverage_repair_if_due(state, gate).await;
    }

    // Legacy-mirror identity reads can decompress large archives. Keep that raw work out of
    // cold Bootstrap entirely; the generic durable cursor starts after a Projection publishes.
    if task == StartupBackfillTask::LegacyDetailMirrors
        && state.subscription_hub.summary_projection().await.is_none()
    {
        return Ok(StartupBackfillTaskRunOutcome {
            actionable: false,
            failed: false,
            deferred: false,
            completed: false,
            next_due: Utc::now()
                + ChronoDuration::seconds(STARTUP_BACKFILL_ACTIVE_INTERVAL_SECS as i64),
        });
    }

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
    let write_permit = match crate::proxy_sqlite_write_coordinator::proxy_sqlite_write_coordinator()
        .try_acquire(crate::proxy_sqlite_write_coordinator::ProxySqliteWriteClass::P2Derived)
    {
        Some(permit) => permit,
        None => {
            return Ok(startup_backfill_pressure_defer_outcome(
                task,
                gate,
                crate::db_pressure::DbPressureDenyReason::BackgroundBusy,
            ));
        }
    };

    let task_name = startup_backfill_task_progress_key(state.as_ref(), task).await;
    let progress = load_startup_backfill_progress(&state.pool, &task_name)
        .await
        .inspect_err(|err| {
            record_startup_backfill_pressure_error(gate, err);
        })?;
    let now = Utc::now();
    if !progress.is_due(now) {
        debug!(
            task = task.log_label(),
            task_name = %progress.task_name,
            next_run_after = progress.next_run_after.as_deref().unwrap_or("-"),
            last_status = %progress.last_status,
            last_started_at = progress.last_started_at.as_deref().unwrap_or("-"),
            last_finished_at = progress.last_finished_at.as_deref().unwrap_or("-"),
            last_scanned = progress.last_scanned,
            last_updated = progress.last_updated,
            "startup backfill task is not due"
        );
        return Ok(StartupBackfillTaskRunOutcome {
            actionable: false,
            failed: false,
            deferred: false,
            completed: false,
            next_due: startup_backfill_progress_due(&progress),
        });
    }

    mark_startup_backfill_running(&state.pool, &task_name, progress.cursor_id)
        .await
        .inspect_err(|err| {
            record_startup_backfill_pressure_error(gate, err);
        })?;

    let started_at = Instant::now();
    let coordinator = crate::proxy_sqlite_write_coordinator::proxy_sqlite_write_coordinator();
    // Backfill implementations combine bounded SQL batches with file reads/decompression. If
    // an interactive writer arrives while this P2 task is active, cancel the in-flight future so
    // its transaction rolls back and release admission immediately for the higher-priority write.
    let task_result = tokio::select! {
        biased;
        _ = coordinator.wait_for_p2_preemption() => {
            drop(write_permit);
            return persist_startup_backfill_pressure_defer(
                state,
                task,
                &task_name,
                &progress,
                gate,
                crate::db_pressure::DbPressureDenyReason::BackgroundBusy,
            )
            .await;
        }
        result = run_startup_backfill_task(
            state,
            task,
            progress.cursor_id,
            progress.zero_update_streak,
            progress.last_status == STARTUP_BACKFILL_STATUS_SOURCE_UNAVAILABLE,
        ) => result,
    };
    let outcome = match task_result {
        Ok((run, detail)) => {
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
                &task_name,
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
                backoff_stage = match startup_backfill_next_delay(&run, zero_update_streak).as_secs() {
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
            StartupBackfillTaskRunOutcome {
                actionable: startup_backfill_run_is_actionable(&run),
                failed: false,
                deferred: false,
                completed: true,
                next_due: parse_to_utc_datetime(&next_run_after).unwrap_or_else(Utc::now),
            }
        }
        Err(err) => {
            let next_due = match persist_startup_backfill_task_failure(
                state, task, &task_name, &progress, started_at, &err,
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
            // Keep the permit until the failure state is durable and any relevant cooldown is
            // visible. Releasing it first would let another background task enter SQLite.
            record_startup_backfill_pressure_error(gate, &err);
            StartupBackfillTaskRunOutcome {
                actionable: false,
                failed: true,
                deferred: false,
                completed: true,
                next_due,
            }
        }
    };

    Ok(outcome)
}

pub(crate) async fn persist_startup_backfill_task_failure(
    state: &Arc<AppState>,
    task: StartupBackfillTask,
    task_name: &str,
    progress: &StartupBackfillProgress,
    started_at: Instant,
    err: &anyhow::Error,
) -> Result<DateTime<Utc>> {
    let failure_kind = startup_backfill_failure_kind(err);
    let retry_after = format_utc_iso(
        Utc::now() + ChronoDuration::seconds(STARTUP_BACKFILL_ACTIVE_INTERVAL_SECS as i64),
    );
    save_startup_backfill_progress(
        &state.pool,
        task_name,
        StartupBackfillProgressUpdate {
            cursor_id: progress.cursor_id,
            scanned: 0,
            updated: 0,
            zero_update_streak: progress.zero_update_streak,
            next_run_after: &retry_after,
            status: STARTUP_BACKFILL_STATUS_FAILED,
            suspension_reason: None,
        },
    )
    .await?;
    warn!(
        task = task.log_label(),
        task_name,
        cursor_id = progress.cursor_id,
        elapsed_ms = started_at.elapsed().as_millis() as u64,
        next_run_after = %retry_after,
        failure_kind = failure_kind.telemetry_reason(),
        retry_kind = "bounded_operation_backoff",
        error = %err,
        "startup backfill pass failed"
    );
    Ok(parse_to_utc_datetime(&retry_after).unwrap_or_else(Utc::now))
}

fn startup_backfill_run_is_actionable(run: &StartupBackfillRunState) -> bool {
    run.updated > 0 || (run.hit_scan_limit && !run.force_idle)
}
