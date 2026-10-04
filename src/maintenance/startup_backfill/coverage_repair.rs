use super::*;
use std::future::Future;

pub(super) async fn run_startup_backfill_coverage_repair_if_due(
    state: &Arc<AppState>,
    gate: &crate::db_pressure::DbPressureGate,
    observation_parent_task_key: Option<&'static str>,
    managed_run_id: Option<i64>,
) -> Result<StartupBackfillTaskRunOutcome> {
    run_startup_backfill_coverage_repair_if_due_with_observation(
        state,
        gate,
        observation_parent_task_key,
        managed_run_id,
        || repair_active_account_activity_v2_coverage(&state.pool),
    )
    .await
}

#[cfg(test)]
pub(crate) async fn run_startup_backfill_coverage_repair_if_due_with_repair<Repair, RepairFuture>(
    state: &Arc<AppState>,
    gate: &crate::db_pressure::DbPressureGate,
    repair: Repair,
) -> Result<StartupBackfillTaskRunOutcome>
where
    Repair: FnOnce() -> RepairFuture,
    RepairFuture: Future<Output = Result<ActiveAccountActivityV2RepairOutcome>>,
{
    run_startup_backfill_coverage_repair_if_due_with_observation(state, gate, None, None, repair)
        .await
}

async fn run_startup_backfill_coverage_repair_if_due_with_observation<Repair, RepairFuture>(
    state: &Arc<AppState>,
    gate: &crate::db_pressure::DbPressureGate,
    observation_parent_task_key: Option<&'static str>,
    managed_run_id: Option<i64>,
    repair: Repair,
) -> Result<StartupBackfillTaskRunOutcome>
where
    Repair: FnOnce() -> RepairFuture,
    RepairFuture: Future<Output = Result<ActiveAccountActivityV2RepairOutcome>>,
{
    let task = StartupBackfillTask::AccountActivityV2Coverage;

    // Coverage repair shares the hourly-rollup synchronization lock. Only admit work when the
    // lock is immediately available; waiting while holding P2 could deadlock another maintenance
    // path that already owns the lock and is waiting for coordinator admission.
    let _hourly_rollup_guard = match state.hourly_rollup_sync_lock.try_lock() {
        Ok(guard) => guard,
        Err(_) => {
            return Ok(startup_backfill_pressure_defer_outcome(
                task,
                gate,
                crate::db_pressure::DbPressureDenyReason::BackgroundBusy,
            ));
        }
    };

    // Coverage repair owns this one permit. The hourly-rollup convenience wrapper also acquires
    // the global gate, so calling it while the startup path holds a permit would always defer in
    // production. Keep admission before progress access, then call the underlying repair once.
    let _permit = match gate.try_begin_background("startup_backfill_account_activity_v2_coverage") {
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
    let progress = match load_startup_backfill_progress(&state.pool, &task_name).await {
        Ok(progress) => progress,
        Err(err) => {
            if let Some(outcome) =
                startup_backfill_pressure_error_defer_outcome_if_recorded(task, gate, &err)
            {
                return Ok(outcome);
            }
            return Err(err);
        }
    };
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

    let observation = managed_run_id
        .and_then(crate::TaskExecutionObservation::for_managed_run)
        .unwrap_or_else(|| {
            let parent_task_key = observation_parent_task_key.unwrap_or("startup_backfill");
            crate::TaskExecutionObservation::begin(
                parent_task_key,
                &crate::maintenance_store::task_title_for_observation(parent_task_key),
                "event_or_due",
                crate::maintenance_store::task_execution_class(parent_task_key),
                "resource_wait",
            )
        });
    let child_key = "startup_backfill.account_activity_v2_coverage";
    let child_title = crate::maintenance_store::task_title_for_observation(child_key);
    observation.set_child(child_key, &child_title);
    let _child_observation_guard = TaskObservationChildGuard(observation.clone());
    let mut workload_observation = observation.begin_subtask_workload(child_key);

    let coordinator = crate::proxy_sqlite_write_coordinator::proxy_sqlite_write_coordinator();
    let repair_outcome = tokio::select! {
        biased;
        _ = coordinator.wait_for_p2_preemption() => {
            drop(write_permit);
            workload_observation
                .finish_with_status_and_reason("skipped", Some("background_busy"));
            if managed_run_id.is_none() {
                observation.finish_with_status_and_reason("skipped", Some("background_busy"));
            }
            return Ok(startup_backfill_pressure_defer_outcome(
                task,
                gate,
                crate::db_pressure::DbPressureDenyReason::BackgroundBusy,
            ));
        }
        outcome = repair() => outcome,
    };
    let repair_outcome = match repair_outcome {
        Ok(outcome) => {
            workload_observation.set_processed_work(
                i64::try_from(outcome.repaired_bucket_count).unwrap_or(i64::MAX),
            );
            outcome
        }
        Err(err) => {
            warn!(
                task = task.log_label(),
                error = %err,
                wake_reason = "active_window_coverage_check",
                "startup backfill account activity v2 coverage repair failed"
            );
            let next_due = match defer_startup_backfill_coverage_repair(state.as_ref()).await {
                Ok(next_due) => next_due,
                Err(persist_err) => {
                    // A retry-progress lock is still a pressure defer, even though the durable
                    // retry deadline could not be written. Prefer that error so two locks close
                    // the gate once, then leave re-dispatch to the in-memory eligibility wake.
                    if let Some(outcome) = startup_backfill_pressure_error_defer_outcome_if_recorded(
                        task,
                        gate,
                        &persist_err,
                    ) {
                        return Ok(outcome);
                    }
                    if let Some(outcome) =
                        startup_backfill_pressure_error_defer_outcome_if_recorded(task, gate, &err)
                    {
                        return Ok(outcome);
                    }
                    return Err(persist_err);
                }
            };
            if let Some(outcome) =
                startup_backfill_pressure_error_defer_outcome_if_recorded(task, gate, &err)
            {
                workload_observation.finish_with_status("failed");
                return Ok(outcome);
            }
            workload_observation.finish_with_status("failed");
            return Ok(StartupBackfillTaskRunOutcome {
                actionable: false,
                failed: true,
                deferred: false,
                completed: true,
                next_due,
            });
        }
    };

    match repair_outcome {
        outcome if outcome.repaired_bucket_count > 0 => {
            let next_due =
                match record_startup_backfill_coverage_repair_progress(state.as_ref(), outcome)
                    .await
                {
                    Ok(next_due) => next_due,
                    Err(err) => {
                        if let Some(outcome) =
                            startup_backfill_pressure_error_defer_outcome_if_recorded(
                                task, gate, &err,
                            )
                        {
                            return Ok(outcome);
                        }
                        return Err(err);
                    }
                };
            workload_observation.finish_with_status("success");
            if managed_run_id.is_none() {
                observation.finish_with_status("success");
            }
            Ok(StartupBackfillTaskRunOutcome {
                actionable: true,
                failed: false,
                deferred: false,
                completed: true,
                next_due,
            })
        }
        outcome if outcome.priority_bucket_count > 0 => {
            let next_due = match defer_startup_backfill_coverage_repair(state.as_ref()).await {
                Ok(next_due) => next_due,
                Err(err) => {
                    if let Some(outcome) =
                        startup_backfill_pressure_error_defer_outcome_if_recorded(task, gate, &err)
                    {
                        return Ok(outcome);
                    }
                    return Err(err);
                }
            };
            workload_observation
                .finish_with_status_and_reason("skipped", Some("priority_bucket_available"));
            if managed_run_id.is_none() {
                observation
                    .finish_with_status_and_reason("skipped", Some("priority_bucket_available"));
            }
            Ok(StartupBackfillTaskRunOutcome {
                actionable: false,
                failed: false,
                deferred: true,
                completed: true,
                next_due,
            })
        }
        _ => {
            let task_name = startup_backfill_task_progress_key(state.as_ref(), task).await;
            let progress = match load_startup_backfill_progress(&state.pool, &task_name).await {
                Ok(progress) => progress,
                Err(err) => {
                    if let Some(outcome) =
                        startup_backfill_pressure_error_defer_outcome_if_recorded(task, gate, &err)
                    {
                        return Ok(outcome);
                    }
                    return Err(err);
                }
            };
            let next_retry_after = format_utc_iso(
                Utc::now() + ChronoDuration::seconds(STARTUP_BACKFILL_IDLE_INTERVAL_SECS as i64),
            );
            let save_result = save_startup_backfill_progress(
                &state.pool,
                &task_name,
                StartupBackfillProgressUpdate {
                    cursor_id: progress.cursor_id,
                    scanned: progress.last_scanned,
                    updated: progress.last_updated,
                    zero_update_streak: 0,
                    next_run_after: &next_retry_after,
                    status: STARTUP_BACKFILL_STATUS_IDLE,
                    suspension_reason: None,
                },
            )
            .await;
            if let Err(err) = save_result {
                if let Some(outcome) =
                    startup_backfill_pressure_error_defer_outcome_if_recorded(task, gate, &err)
                {
                    return Ok(outcome);
                }
                return Err(err);
            }
            let next_due = parse_to_utc_datetime(&next_retry_after).unwrap_or_else(Utc::now);
            STARTUP_BACKFILL_SCHEDULER.record_next_due(task, next_due);
            workload_observation.finish_with_status("success");
            if managed_run_id.is_none() {
                observation.finish_with_status("success");
            }
            debug!(
                task = task.log_label(),
                next_retry_after = %next_retry_after,
                "account activity v2 coverage repair is idle"
            );
            Ok(StartupBackfillTaskRunOutcome {
                actionable: false,
                failed: false,
                deferred: false,
                completed: true,
                next_due,
            })
        }
    }
}
