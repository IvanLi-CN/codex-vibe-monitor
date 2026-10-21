use super::*;

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
        None,
        None,
    )
    .await
    .map(|(outcome, _)| outcome.actionable)
}

pub(crate) async fn run_startup_backfill_task_if_due_with_gate(
    state: &Arc<AppState>,
    task: StartupBackfillTask,
    gate: &crate::db_pressure::DbPressureGate,
) -> Result<bool> {
    run_startup_backfill_task_if_due_outcome(state, task, gate, None, None)
        .await
        .map(|(outcome, _)| outcome.actionable)
}

pub(super) async fn run_startup_backfill_task_if_due_outcome(
    state: &Arc<AppState>,
    task: StartupBackfillTask,
    gate: &crate::db_pressure::DbPressureGate,
    observation_parent_task_key: Option<&'static str>,
    managed_run_id: Option<i64>,
) -> Result<(StartupBackfillTaskRunOutcome, Option<String>)> {
    let prompt_cache_store = crate::maintenance_store::global().map(|store| store.as_ref());
    run_startup_backfill_task_if_due_outcome_with_store(
        state,
        task,
        gate,
        prompt_cache_store,
        observation_parent_task_key,
        managed_run_id,
    )
    .await
}

#[cfg(test)]
pub(crate) async fn run_startup_backfill_task_if_due_with_store_for_test(
    state: &Arc<AppState>,
    task: StartupBackfillTask,
    gate: &crate::db_pressure::DbPressureGate,
    store: &crate::maintenance_store::MaintenanceStore,
) -> Result<StartupBackfillTaskRunOutcome> {
    run_startup_backfill_task_if_due_outcome_with_store(state, task, gate, Some(store), None, None)
        .await
        .map(|(outcome, _)| outcome)
}

async fn run_startup_backfill_task_if_due_outcome_with_store(
    state: &Arc<AppState>,
    task: StartupBackfillTask,
    gate: &crate::db_pressure::DbPressureGate,
    prompt_cache_store: Option<&crate::maintenance_store::MaintenanceStore>,
    observation_parent_task_key: Option<&'static str>,
    managed_run_id: Option<i64>,
) -> Result<(StartupBackfillTaskRunOutcome, Option<String>)> {
    if !startup_backfill_task_enabled(state.as_ref(), task) {
        debug!(
            task = task.log_label(),
            "startup backfill task is disabled by config"
        );
        return Ok((
            StartupBackfillTaskRunOutcome {
                actionable: false,
                failed: false,
                deferred: false,
                completed: true,
                next_due: Utc::now() + ChronoDuration::days(1),
            },
            None,
        ));
    }

    let prompt_cache_materialization_control = if task
        == StartupBackfillTask::PromptCacheConversationsMaterialization
    {
        let retry_at =
            Utc::now() + ChronoDuration::seconds(STARTUP_BACKFILL_ACTIVE_INTERVAL_SECS as i64);
        let Some(store) = prompt_cache_store else {
            STARTUP_BACKFILL_SCHEDULER.record_next_due(task, retry_at);
            warn!(
                task = task.log_label(),
                defer_reason = "maintenance_database_unavailable",
                next_eligibility = %retry_at,
                "prompt-cache materialization control is unavailable"
            );
            return Ok((
                StartupBackfillTaskRunOutcome {
                    actionable: false,
                    failed: false,
                    deferred: true,
                    completed: true,
                    next_due: retry_at,
                },
                None,
            ));
        };
        let Some(snapshot) = store.prompt_cache_materialization_control.snapshot() else {
            STARTUP_BACKFILL_SCHEDULER.record_next_due(task, retry_at);
            warn!(
                task = task.log_label(),
                defer_reason = "maintenance_database_unavailable",
                next_eligibility = %retry_at,
                "prompt-cache materialization control has not been initialized"
            );
            return Ok((
                StartupBackfillTaskRunOutcome {
                    actionable: false,
                    failed: false,
                    deferred: true,
                    completed: true,
                    next_due: retry_at,
                },
                None,
            ));
        };
        if !snapshot.enabled && !crate::maintenance::maintenance_execution_is_admitted() {
            store
                .prompt_cache_materialization_control
                .with_current_generation(snapshot.generation, || {
                    STARTUP_BACKFILL_SCHEDULER.clear_pending(task)
                });
            return Ok((
                StartupBackfillTaskRunOutcome {
                    actionable: false,
                    failed: false,
                    deferred: false,
                    completed: true,
                    next_due: Utc::now() + ChronoDuration::days(3650),
                },
                None,
            ));
        }
        Some((
            crate::maintenance_store::PromptCacheMaterializationControl::admitted_control(snapshot),
            snapshot.generation,
        ))
    } else {
        None
    };

    if task == StartupBackfillTask::AccountActivityV2Coverage {
        return run_startup_backfill_coverage_repair_if_due(
            state,
            gate,
            observation_parent_task_key,
            managed_run_id,
        )
        .await
        .map(|outcome| (outcome, None));
    }

    // Legacy-mirror identity reads can decompress large archives. Keep that raw work out of
    // cold Bootstrap entirely; the generic durable cursor starts after a Projection publishes.
    if task == StartupBackfillTask::LegacyDetailMirrors
        && state.subscription_hub.summary_projection().await.is_none()
    {
        return Ok((
            StartupBackfillTaskRunOutcome {
                actionable: false,
                failed: false,
                deferred: false,
                completed: false,
                next_due: Utc::now()
                    + ChronoDuration::seconds(STARTUP_BACKFILL_ACTIVE_INTERVAL_SECS as i64),
            },
            None,
        ));
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
        Err(reason) => {
            return Ok((
                startup_backfill_pressure_defer_outcome(task, gate, reason),
                None,
            ));
        }
    };
    let mut write_permit = Some(
        match crate::proxy_sqlite_write_coordinator::proxy_sqlite_write_coordinator()
            .try_acquire(crate::proxy_sqlite_write_coordinator::ProxySqliteWriteClass::P2Derived)
        {
            Some(permit) => permit,
            None => {
                return Ok((
                    if task == StartupBackfillTask::PromptCacheConversationsMaterialization {
                        startup_backfill_coordinator_defer_outcome(
                            &STARTUP_BACKFILL_SCHEDULER,
                            task,
                            gate,
                        )
                    } else {
                        startup_backfill_pressure_defer_outcome(
                            task,
                            gate,
                            crate::db_pressure::DbPressureDenyReason::BackgroundBusy,
                        )
                    },
                    None,
                ));
            }
        },
    );

    if prompt_cache_materialization_control.is_some() {
        drop(write_permit.take());
    }
    let task_name = startup_backfill_task_progress_key(state.as_ref(), task).await;
    let progress = load_startup_backfill_progress(&state.pool, &task_name)
        .await
        .inspect_err(|err| {
            record_startup_backfill_pressure_error(gate, err);
        })?;
    let now = Utc::now();
    if !(progress.is_due(now)
        || task == StartupBackfillTask::PromptCacheConversationsMaterialization
            && crate::maintenance::maintenance_execution_is_manual())
    {
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
        return Ok((
            StartupBackfillTaskRunOutcome {
                actionable: false,
                failed: false,
                deferred: false,
                completed: false,
                next_due: startup_backfill_progress_due(&progress),
            },
            None,
        ));
    }

    {
        let _start_guard = match prompt_cache_materialization_control.as_ref() {
            Some((control, generation)) => {
                let Some(guard) = control.lock_current_generation(*generation).await else {
                    return Ok((prompt_cache_stale_result_outcome(), None));
                };
                Some(guard)
            }
            None => None,
        };
        mark_startup_backfill_running(&state.pool, &task_name, progress.cursor_id)
            .await
            .inspect_err(|err| {
                record_startup_backfill_pressure_error(gate, err);
            })?;
    }

    if prompt_cache_materialization_control.is_some() {
        let Some(permit) = coordinator_for_prompt_cache_run() else {
            return Ok((
                startup_backfill_coordinator_defer_outcome(&STARTUP_BACKFILL_SCHEDULER, task, gate),
                None,
            ));
        };
        write_permit = Some(permit);
    }

    let observation_task_key = startup_backfill_observation_task_key(observation_parent_task_key);
    let observation = managed_run_id
        .and_then(crate::TaskExecutionObservation::for_managed_run)
        .unwrap_or_else(|| {
            crate::TaskExecutionObservation::begin(
                observation_task_key,
                &crate::maintenance_store::task_title_for_observation(observation_task_key),
                "event_or_due",
                crate::maintenance_store::task_execution_class("startup_backfill"),
                "resource_wait",
            )
        });
    let child_key = crate::maintenance_store::managed_startup_backfill_suffix(&task_name)
        .map(|suffix| format!("startup_backfill.{suffix}"))
        .unwrap_or_else(|| format!("startup_backfill.{}", task.log_label()));
    let child_title = crate::maintenance_store::task_title_for_observation(&child_key);
    let mut workload_observation = observation.begin_subtask_workload(&child_key);
    observation.set_child(&child_key, &child_title);
    let _child_observation_guard = TaskObservationChildGuard(observation.clone());

    let started_at = Instant::now();
    let prompt_cache_completed_before =
        if task == StartupBackfillTask::PromptCacheConversationsMaterialization {
            crate::prompt_cache_conversations::load_prompt_cache_conversation_migration_progress(
                &state.pool,
            )
            .await
            .ok()
            .and_then(|progress| progress.completed_keys)
            .unwrap_or_default()
        } else {
            0
        };
    let coordinator = crate::proxy_sqlite_write_coordinator::proxy_sqlite_write_coordinator();
    // Most backfills combine bounded SQL batches with file reads/decompression. If an interactive
    // writer arrives while one of those P2 tasks is active, cancel the in-flight future so its
    // transaction rolls back. Prompt-cache materialization is the exception: it observes the same
    // signal only after each committed micro-batch.
    let prompt_cache_should_yield = || coordinator.p2_should_yield();
    observation.set_phase("processing");
    let task_result = if task == StartupBackfillTask::PromptCacheConversationsMaterialization {
        // Prompt-cache materialization checks priority only between committed micro-batches. The
        // enclosing maintenance pass still cancels the task during process shutdown.
        super::retention::retention_try_archive_locks_scope(
            run_startup_backfill_task_with_pressure(
                state,
                task,
                progress.cursor_id,
                progress.zero_update_streak,
                progress.last_status == STARTUP_BACKFILL_STATUS_SOURCE_UNAVAILABLE,
                Some(&prompt_cache_should_yield),
                prompt_cache_materialization_control
                    .as_ref()
                    .map(|(control, generation)| (control, *generation)),
            ),
        )
        .await
    } else {
        tokio::select! {
            biased;
            _ = coordinator.wait_for_p2_preemption() => {
                drop(write_permit.take());
                workload_observation
                    .finish_with_status_and_reason("skipped", Some("background_busy"));
                return persist_startup_backfill_pressure_defer(
                    state,
                    task,
                    &task_name,
                    &progress,
                    gate,
                    crate::db_pressure::DbPressureDenyReason::BackgroundBusy,
                )
                .await
                .map(|outcome| (outcome, None));
            }
            result = super::retention::retention_try_archive_locks_scope(
                run_startup_backfill_task_with_pressure(
                    state,
                    task,
                    progress.cursor_id,
                    progress.zero_update_streak,
                    progress.last_status == STARTUP_BACKFILL_STATUS_SOURCE_UNAVAILABLE,
                    None,
                    None,
                )
            ) => result,
        }
    };
    // Network/business work has finished. Do not hold online write admission
    // while waiting for a control update or maintenance-database checkpoint.
    let result_guard = match prompt_cache_materialization_control.as_ref() {
        Some((control, generation)) => {
            drop(write_permit.take());
            control.lock_current_generation(*generation).await
        }
        None => None,
    };
    if prompt_cache_materialization_control.is_some() && result_guard.is_none() {
        let committed_work = if let Ok((run, _)) = task_result.as_ref() {
            run.updated
        } else {
            crate::prompt_cache_conversations::load_prompt_cache_conversation_migration_progress(
                &state.pool,
            )
            .await
            .ok()
            .and_then(|progress| progress.completed_keys)
            .unwrap_or_default()
            .saturating_sub(prompt_cache_completed_before)
            .try_into()
            .unwrap_or_default()
        };
        let committed_work = i64::try_from(committed_work).unwrap_or(i64::MAX);
        workload_observation.set_processed_work(committed_work);
        if observation_parent_task_key == Some("prompt_cache_materialization") {
            observation.set_processed_work(committed_work);
        }
        if managed_run_id.is_none() {
            observation.finish_with_status_and_reason("skipped", Some("stats_generation_changed"));
        }
        workload_observation
            .finish_with_status_and_reason("skipped", Some("stats_generation_changed"));
        return Ok((prompt_cache_stale_result_outcome(), None));
    }
    let outcome = match task_result {
        Ok((run, detail)) => {
            if task == StartupBackfillTask::PoolUpstreamNodeHealthArchives {
                workload_observation.set_discovered_work(
                    i64::try_from(run.scanned).unwrap_or(i64::MAX),
                    format_utc_iso_millis(Utc::now()),
                    "run-window".to_string(),
                );
            }
            workload_observation.set_processed_work(i64::try_from(run.updated).unwrap_or(i64::MAX));
            if observation_parent_task_key == Some("prompt_cache_materialization") {
                observation.set_processed_work(i64::try_from(run.updated).unwrap_or(i64::MAX));
            }
            if run.deferred {
                drop(write_permit.take());
                if run.defer_reason == Some("operator_disabled") {
                    let current_progress =
                        load_startup_backfill_progress(&state.pool, &task_name).await?;
                    let resumed = current_progress.enabled;
                    info!(
                        task = task.log_label(),
                        task_name = %task_name,
                        detail = %detail,
                        resumed,
                        "startup backfill task stopped at a committed micro-batch boundary after operator disable"
                    );
                    if managed_run_id.is_none() {
                        observation
                            .finish_with_status_and_reason("skipped", Some("operator_disabled"));
                    }
                    workload_observation
                        .finish_with_status_and_reason("skipped", Some("operator_disabled"));
                    return Ok((
                        StartupBackfillTaskRunOutcome {
                            actionable: false,
                            failed: false,
                            deferred: false,
                            completed: true,
                            next_due: Utc::now() + ChronoDuration::days(3650),
                        },
                        None,
                    ));
                }
                if let Some(
                    reason @ ("stats_page_pending"
                    | "stats_generation_changed"
                    | "stats_budget_exhausted"),
                ) = run.defer_reason
                {
                    return persist_prompt_cache_materialization_defer(
                        state, task, &task_name, &progress, &run, reason,
                    )
                    .await
                    .map(|outcome| (outcome, None));
                }
                if run.defer_reason == Some("maintenance_database_unavailable") {
                    let retry_at = Utc::now()
                        + ChronoDuration::seconds(STARTUP_BACKFILL_ACTIVE_INTERVAL_SECS as i64);
                    if !crate::maintenance::maintenance_execution_is_manual() {
                        STARTUP_BACKFILL_SCHEDULER.record_next_due(task, retry_at);
                    }
                    return Ok((
                        StartupBackfillTaskRunOutcome {
                            actionable: false,
                            failed: false,
                            deferred: true,
                            completed: true,
                            next_due: retry_at,
                        },
                        None,
                    ));
                }
                info!(
                    task = task.log_label(),
                    task_name = %task_name,
                    defer_reason = run.defer_reason.unwrap_or("unknown"),
                    detail = %detail,
                    "startup backfill task yielded at a prompt-cache micro-batch boundary"
                );
                if managed_run_id.is_none() {
                    observation.finish_with_status_and_reason(
                        "skipped",
                        run.defer_reason.or(Some("deferred")),
                    );
                }
                workload_observation.finish_with_status_and_reason(
                    "skipped",
                    run.defer_reason.or(Some("deferred")),
                );
                return persist_startup_backfill_pressure_defer(
                    state,
                    task,
                    &task_name,
                    &progress,
                    gate,
                    crate::db_pressure::DbPressureDenyReason::BackgroundBusy,
                )
                .await
                .map(|outcome| (outcome, None));
            }
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
            save_startup_backfill_progress_for_task(
                &state.pool,
                task,
                &task_name,
                progress.wake_generation,
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
            (
                StartupBackfillTaskRunOutcome {
                    actionable: startup_backfill_run_is_actionable(&run),
                    failed: false,
                    deferred: false,
                    completed: true,
                    next_due: parse_to_utc_datetime(&next_run_after).unwrap_or_else(Utc::now),
                },
                (task == StartupBackfillTask::ProxyCost).then_some(detail),
            )
        }
        Err(err)
            if startup_backfill_failure_kind(&err)
                == StartupBackfillFailureKind::ArchiveLockBusy =>
        {
            drop(write_permit.take());
            let outcome = persist_startup_backfill_archive_lock_defer(
                state, task, &task_name, &progress, started_at,
            )
            .await?;
            (outcome, None)
        }
        Err(err) => {
            let partial_progress = err.downcast_ref::<BackfillPartialFailure>();
            let mut failure_progress = progress.clone();
            if let Some(partial) = partial_progress {
                failure_progress.cursor_id = partial.next_cursor_id.max(progress.cursor_id);
                failure_progress.last_scanned = partial.scanned;
                failure_progress.last_updated = partial.updated;
            }
            // A bounded backfill may have committed earlier micro-batches before a later
            // source/read failure. Preserve that committed delta in the child workload sample
            // before persisting the failed scheduler state.
            let startup_committed_delta = i64::try_from(
                failure_progress
                    .last_updated
                    .saturating_sub(progress.last_updated),
            )
            .unwrap_or(i64::MAX);
            let prompt_cache_committed_delta = if task
                == StartupBackfillTask::PromptCacheConversationsMaterialization
            {
                crate::prompt_cache_conversations::load_prompt_cache_conversation_migration_progress(
                    &state.pool,
                )
                .await
                .ok()
                .and_then(|progress| progress.completed_keys)
                .unwrap_or_default()
                .saturating_sub(prompt_cache_completed_before)
            } else {
                0
            };
            let committed_delta = partial_progress
                .map(|partial| i64::try_from(partial.updated).unwrap_or(i64::MAX))
                .unwrap_or_else(|| startup_committed_delta.max(prompt_cache_committed_delta));
            workload_observation.set_processed_work(committed_delta);
            if observation_parent_task_key == Some("prompt_cache_materialization") {
                observation.set_processed_work(committed_delta);
            }
            let next_due = match persist_startup_backfill_task_failure(
                state,
                task,
                &task_name,
                &failure_progress,
                started_at,
                &err,
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
            (
                StartupBackfillTaskRunOutcome {
                    actionable: false,
                    failed: true,
                    deferred: false,
                    completed: true,
                    next_due,
                },
                None,
            )
        }
    };

    if managed_run_id.is_none() {
        let status = if outcome.0.failed {
            "failed"
        } else if outcome.0.deferred {
            "skipped"
        } else {
            "success"
        };
        observation.finish_with_status(status);
    }
    workload_observation.finish_with_status(if outcome.0.failed {
        "failed"
    } else if outcome.0.deferred {
        "skipped"
    } else {
        "success"
    });
    Ok(outcome)
}

pub(super) fn startup_backfill_observation_task_key(
    observation_parent_task_key: Option<&'static str>,
) -> &'static str {
    observation_parent_task_key.unwrap_or("startup_backfill")
}

pub(super) struct TaskObservationChildGuard(pub(super) crate::TaskExecutionObservation);

impl Drop for TaskObservationChildGuard {
    fn drop(&mut self) {
        self.0.clear_child();
    }
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
    save_startup_backfill_progress_for_task(
        &state.pool,
        task,
        task_name,
        progress.wake_generation,
        StartupBackfillProgressUpdate {
            cursor_id: progress.cursor_id,
            scanned: progress.last_scanned,
            updated: progress.last_updated,
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

pub(super) fn startup_backfill_run_is_actionable(run: &StartupBackfillRunState) -> bool {
    run.updated > 0 || (run.hit_scan_limit && !run.force_idle)
}

pub(crate) async fn run_startup_backfill_task(
    state: &Arc<AppState>,
    task: StartupBackfillTask,
    cursor_id: i64,
    _zero_update_streak: u32,
    source_unavailable_probe: bool,
) -> Result<(StartupBackfillRunState, String)> {
    run_startup_backfill_task_with_pressure(
        state,
        task,
        cursor_id,
        _zero_update_streak,
        source_unavailable_probe,
        None,
        None,
    )
    .await
}

pub(super) async fn run_startup_backfill_task_with_pressure(
    state: &Arc<AppState>,
    task: StartupBackfillTask,
    cursor_id: i64,
    _zero_update_streak: u32,
    source_unavailable_probe: bool,
    prompt_cache_should_yield: Option<&(dyn Fn() -> bool + Send + Sync)>,
    prompt_cache_materialization_control: Option<(
        &Arc<crate::maintenance_store::PromptCacheMaterializationControl>,
        u64,
    )>,
) -> Result<(StartupBackfillRunState, String)> {
    let scan_limit = startup_backfill_scan_limit(source_unavailable_probe);
    let max_elapsed = Some(startup_backfill_run_budget(source_unavailable_probe));
    let raw_path_fallback_root = state.config.database_path.parent();
    match task {
        StartupBackfillTask::ProxyUsage => {
            let snapshot_max_id = current_proxy_usage_backfill_snapshot_max_id(&state.pool).await?;
            let outcome = backfill_proxy_usage_tokens_from_cursor(
                &state.pool,
                cursor_id,
                snapshot_max_id,
                raw_path_fallback_root,
                Some(scan_limit),
                max_elapsed,
            )
            .await?;
            let detail = format!(
                "skipped_missing_file={} skipped_without_usage={} skipped_decode_error={}",
                outcome.summary.skipped_missing_file,
                outcome.summary.skipped_without_usage,
                outcome.summary.skipped_decode_error
            );
            Ok((
                StartupBackfillRunState {
                    next_cursor_id: outcome.next_cursor_id,
                    scanned: outcome.summary.scanned,
                    updated: outcome.summary.updated,
                    hit_scan_limit: outcome.hit_budget,
                    retry_soon: false,
                    force_idle: false,
                    source_unavailable: false,
                    deferred: false,
                    defer_reason: None,
                    samples: outcome.samples,
                },
                detail,
            ))
        }
        StartupBackfillTask::ProxyCost => {
            let catalog = state.pricing_catalog.read().await.clone();
            let attempt_version = pricing_backfill_attempt_version(&catalog);
            let snapshot_max_id = current_proxy_cost_backfill_snapshot_max_id(&state.pool).await?;
            let outcome = backfill_proxy_missing_costs_from_cursor(
                &state.pool,
                cursor_id,
                snapshot_max_id,
                &catalog,
                &attempt_version,
                Some(scan_limit),
                max_elapsed,
            )
            .await?;
            let detail = format!(
                "catalog_version={} attempt_version={} scanned={} updated={} skipped_unpriced_model={} cursor_id={} state={}",
                catalog.version,
                attempt_version,
                outcome.summary.scanned,
                outcome.summary.updated,
                outcome.summary.skipped_unpriced_model,
                outcome.next_cursor_id,
                if outcome.hit_budget {
                    "continuing"
                } else {
                    "drained"
                },
            );
            Ok((
                StartupBackfillRunState {
                    next_cursor_id: outcome.next_cursor_id,
                    scanned: outcome.summary.scanned,
                    updated: outcome.summary.updated,
                    hit_scan_limit: outcome.hit_budget,
                    retry_soon: false,
                    force_idle: false,
                    source_unavailable: false,
                    deferred: false,
                    defer_reason: None,
                    samples: outcome.samples,
                },
                detail,
            ))
        }
        StartupBackfillTask::PromptCacheKey => {
            let outcome = backfill_proxy_prompt_cache_keys_from_cursor(
                &state.pool,
                cursor_id,
                raw_path_fallback_root,
                Some(scan_limit),
                max_elapsed,
            )
            .await?;
            let detail = format!(
                "examined={} candidates={} skipped_missing_file={} skipped_invalid_json={} skipped_missing_key={}",
                outcome.summary.examined,
                outcome.summary.scanned,
                outcome.summary.skipped_missing_file,
                outcome.summary.skipped_invalid_json,
                outcome.summary.skipped_missing_key
            );
            Ok((
                StartupBackfillRunState {
                    next_cursor_id: outcome.next_cursor_id,
                    scanned: outcome.summary.scanned,
                    updated: outcome.summary.updated,
                    hit_scan_limit: outcome.hit_budget,
                    retry_soon: false,
                    force_idle: false,
                    source_unavailable: false,
                    deferred: false,
                    defer_reason: None,
                    samples: outcome.samples,
                },
                detail,
            ))
        }
        StartupBackfillTask::PromptCacheConversationsMaterialization => {
            let never_yield = || false;
            let should_yield = prompt_cache_should_yield.unwrap_or(&never_yield);
            let Some((control, expected_generation)) = prompt_cache_materialization_control else {
                return Ok((
                    StartupBackfillRunState {
                        next_cursor_id: cursor_id,
                        hit_scan_limit: true,
                        deferred: true,
                        defer_reason: Some("maintenance_database_unavailable"),
                        ..Default::default()
                    },
                    "defer_reason=maintenance_database_unavailable".to_string(),
                ));
            };
            let run_started = Instant::now();
            let run_started_at = format_utc_iso(Utc::now());
            let outcome =
                match run_prompt_cache_conversations_materialization_with_pressure_and_control(
                    &state.pool,
                    scan_limit,
                    max_elapsed,
                    should_yield,
                    control,
                    expected_generation,
                )
                .await
                {
                    Ok(outcome) => outcome,
                    Err(error) => {
                        let failed_outcome = prompt_cache_materialization_failed_outcome(
                            crate::prompt_cache_conversations::load_prompt_cache_conversation_migration_progress(
                                &state.pool,
                            )
                            .await
                            .ok()
                            .map(|progress| progress.phase),
                        );
                        if let Err(record_error) =
                            record_prompt_cache_conversation_materialization_run(
                                &state.pool,
                                &run_started_at,
                                run_started.elapsed().as_millis() as u64,
                                &failed_outcome,
                                "failed",
                                Some(&error.to_string()),
                            )
                            .await
                        {
                            warn!(error = %record_error, "failed to record prompt-cache materialization failure");
                        }
                        return Err(error);
                    }
                };
            if !outcome.control_generation_changed
                && (!outcome.deferred || outcome.scanned > 0 || outcome.updated > 0)
                && let Err(error) = record_prompt_cache_conversation_materialization_run(
                    &state.pool,
                    &run_started_at,
                    run_started.elapsed().as_millis() as u64,
                    &outcome,
                    if outcome.deferred {
                        "deferred"
                    } else {
                        "success"
                    },
                    None,
                )
                .await
            {
                warn!(error = %error, "failed to record prompt-cache materialization run");
            }
            let detail = format!(
                "phase={} scanned={} updated={} complete={} batches={} last_batch_size={} max_batch_size={} batch_elapsed_ms={} deferred={} defer_reason={}",
                outcome.phase,
                outcome.scanned,
                outcome.updated,
                outcome.complete,
                outcome.batch_count,
                outcome.last_batch_size,
                outcome.max_batch_size,
                outcome.batch_elapsed_ms,
                outcome.deferred,
                outcome.defer_reason.unwrap_or("none"),
            );
            Ok((
                StartupBackfillRunState {
                    next_cursor_id: cursor_id,
                    scanned: outcome.scanned,
                    updated: outcome.updated,
                    hit_scan_limit: outcome.hit_scan_limit && !outcome.complete,
                    retry_soon: false,
                    force_idle: outcome.complete,
                    source_unavailable: false,
                    deferred: outcome.deferred,
                    defer_reason: outcome.defer_reason,
                    samples: Vec::new(),
                },
                detail,
            ))
        }
        StartupBackfillTask::RequestedServiceTier => {
            let outcome = backfill_proxy_requested_service_tiers_from_cursor(
                &state.pool,
                cursor_id,
                raw_path_fallback_root,
                Some(scan_limit),
                max_elapsed,
            )
            .await?;
            let detail = format!(
                "examined={} candidates={} skipped_missing_file={} skipped_invalid_json={} skipped_missing_tier={}",
                outcome.summary.examined,
                outcome.summary.scanned,
                outcome.summary.skipped_missing_file,
                outcome.summary.skipped_invalid_json,
                outcome.summary.skipped_missing_tier
            );
            Ok((
                StartupBackfillRunState {
                    next_cursor_id: outcome.next_cursor_id,
                    scanned: outcome.summary.scanned,
                    updated: outcome.summary.updated,
                    hit_scan_limit: outcome.hit_budget,
                    retry_soon: false,
                    force_idle: false,
                    source_unavailable: false,
                    deferred: false,
                    defer_reason: None,
                    samples: outcome.samples,
                },
                detail,
            ))
        }
        StartupBackfillTask::InvocationServiceTier => {
            let outcome = backfill_invocation_service_tiers_from_cursor(
                &state.pool,
                cursor_id,
                raw_path_fallback_root,
                Some(scan_limit),
                max_elapsed,
            )
            .await?;
            let detail = format!(
                "skipped_missing_file={} skipped_missing_tier={}",
                outcome.summary.skipped_missing_file, outcome.summary.skipped_missing_tier
            );
            Ok((
                StartupBackfillRunState {
                    next_cursor_id: outcome.next_cursor_id,
                    scanned: outcome.summary.scanned,
                    updated: outcome.summary.updated,
                    hit_scan_limit: outcome.hit_budget,
                    retry_soon: false,
                    force_idle: false,
                    source_unavailable: false,
                    deferred: false,
                    defer_reason: None,
                    samples: outcome.samples,
                },
                detail,
            ))
        }
        StartupBackfillTask::ReasoningEffort => {
            let outcome = backfill_proxy_reasoning_efforts_from_cursor(
                &state.pool,
                cursor_id,
                raw_path_fallback_root,
                Some(scan_limit),
                max_elapsed,
            )
            .await?;
            let detail = format!(
                "examined={} candidates={} skipped_missing_file={} skipped_invalid_json={} skipped_missing_effort={}",
                outcome.summary.examined,
                outcome.summary.scanned,
                outcome.summary.skipped_missing_file,
                outcome.summary.skipped_invalid_json,
                outcome.summary.skipped_missing_effort
            );
            Ok((
                StartupBackfillRunState {
                    next_cursor_id: outcome.next_cursor_id,
                    scanned: outcome.summary.scanned,
                    updated: outcome.summary.updated,
                    hit_scan_limit: outcome.hit_budget,
                    retry_soon: false,
                    force_idle: false,
                    source_unavailable: false,
                    deferred: false,
                    defer_reason: None,
                    samples: outcome.samples,
                },
                detail,
            ))
        }
        StartupBackfillTask::FailureClassification => {
            let outcome = backfill_failure_classification_from_cursor(
                &state.pool,
                cursor_id,
                raw_path_fallback_root,
                Some(scan_limit),
                max_elapsed,
            )
            .await?;
            Ok((
                StartupBackfillRunState {
                    next_cursor_id: outcome.next_cursor_id,
                    scanned: outcome.summary.scanned,
                    updated: outcome.summary.updated,
                    hit_scan_limit: outcome.hit_budget,
                    retry_soon: false,
                    force_idle: false,
                    source_unavailable: false,
                    deferred: false,
                    defer_reason: None,
                    samples: outcome.samples,
                },
                "failure classification recalculated".to_string(),
            ))
        }
        StartupBackfillTask::PoolAttemptPublicIdLive => {
            let outcome = backfill_pool_upstream_request_attempt_public_ids_from_cursor(
                &state.pool,
                cursor_id,
                Some(scan_limit),
                max_elapsed,
            )
            .await?;
            Ok((
                StartupBackfillRunState {
                    next_cursor_id: outcome.next_cursor_id,
                    scanned: outcome.summary.scanned,
                    updated: outcome.summary.updated,
                    hit_scan_limit: outcome.hit_budget,
                    retry_soon: false,
                    force_idle: false,
                    source_unavailable: false,
                    deferred: false,
                    defer_reason: None,
                    samples: outcome.samples,
                },
                "attempt_public_id live rows".to_string(),
            ))
        }
        StartupBackfillTask::PoolAttemptPublicIdArchives => {
            let outcome =
                backfill_pool_upstream_request_attempt_archive_public_ids_from_batch_cursor(
                    &state.pool,
                    cursor_id,
                    Some(1),
                    max_elapsed,
                )
                .await?;
            Ok((
                StartupBackfillRunState {
                    next_cursor_id: outcome.next_cursor_id,
                    scanned: outcome.summary.scanned_batches,
                    updated: outcome.summary.updated_rows,
                    hit_scan_limit: outcome.hit_budget,
                    retry_soon: false,
                    force_idle: false,
                    source_unavailable: false,
                    deferred: false,
                    defer_reason: None,
                    samples: outcome.samples,
                },
                format!(
                    "updated_batches={} updated_rows={}",
                    outcome.summary.updated_batches, outcome.summary.updated_rows
                ),
            ))
        }
        StartupBackfillTask::UpstreamActivityLive => {
            let updated_accounts =
                backfill_upstream_account_last_activity_from_live_invocations(&state.pool).await?;
            let pending_accounts =
                count_upstream_accounts_missing_live_last_activity(&state.pool).await?;
            Ok((
                StartupBackfillRunState {
                    next_cursor_id: cursor_id,
                    scanned: 0,
                    updated: updated_accounts,
                    hit_scan_limit: false,
                    retry_soon: false,
                    force_idle: false,
                    source_unavailable: false,
                    deferred: false,
                    defer_reason: None,
                    samples: Vec::new(),
                },
                format!("pending_accounts={pending_accounts}"),
            ))
        }
        StartupBackfillTask::UpstreamActivityArchives => {
            let summary = backfill_upstream_account_last_activity_from_archives(
                &state.pool,
                Some(scan_limit),
                max_elapsed,
            )
            .await?;
            let pending_accounts =
                count_upstream_accounts_missing_last_activity(&state.pool).await?;
            let force_idle = summary.waiting_for_manifest_backfill
                || (pending_accounts > 0 && !summary.hit_budget && summary.updated_accounts == 0);
            Ok((
                StartupBackfillRunState {
                    next_cursor_id: cursor_id,
                    scanned: summary.scanned_batches,
                    updated: summary.updated_accounts,
                    hit_scan_limit: pending_accounts > 0 && summary.hit_budget,
                    retry_soon: false,
                    force_idle,
                    source_unavailable: pending_accounts > 0 && force_idle,
                    deferred: false,
                    defer_reason: None,
                    samples: Vec::new(),
                },
                format!(
                    "pending_accounts={pending_accounts} waiting_for_manifest_backfill={}",
                    summary.waiting_for_manifest_backfill
                ),
            ))
        }
        StartupBackfillTask::PoolUpstreamNodeHealthArchives => {
            let cache_summary =
                backfill_pool_upstream_node_health_archives(&state.pool, Some(1), max_elapsed)
                    .await?;
            let hourly_summary = match backfill_pool_upstream_node_health_hourly_archives(
                &state.pool,
                Some(1),
                max_elapsed,
            )
            .await
            {
                Ok(summary) => summary,
                Err(error) => {
                    let (scanned, updated) = error
                        .downcast_ref::<BackfillPartialFailure>()
                        .map(|partial| (partial.scanned, partial.updated))
                        .unwrap_or_default();
                    return Err(anyhow::Error::new(BackfillPartialFailure {
                        source: error,
                        next_cursor_id: cursor_id,
                        scanned: cache_summary.scanned_batches + scanned,
                        updated: cache_summary.materialized_batches + updated,
                    }));
                }
            };
            Ok((
                StartupBackfillRunState {
                    next_cursor_id: cursor_id,
                    scanned: cache_summary.scanned_batches + hourly_summary.scanned_batches,
                    updated: cache_summary.materialized_batches
                        + hourly_summary.materialized_batches,
                    hit_scan_limit: cache_summary.hit_budget || hourly_summary.hit_budget,
                    retry_soon: false,
                    force_idle: cache_summary.pending_batches == 0
                        && hourly_summary.pending_batches == 0,
                    source_unavailable: false,
                    deferred: false,
                    defer_reason: None,
                    samples: Vec::new(),
                },
                format!(
                    "pending_cache_batches={} materialized_cache_batches={} pending_hourly_batches={} materialized_hourly_batches={}",
                    cache_summary.pending_batches,
                    cache_summary.materialized_batches,
                    hourly_summary.pending_batches,
                    hourly_summary.materialized_batches,
                ),
            ))
        }
        StartupBackfillTask::AccountActivityV2Coverage => Err(anyhow!(
            "account activity v2 coverage repair must use its dedicated scheduler path"
        )),
        StartupBackfillTask::LegacyDetailMirrors => {
            let window = reconcile_legacy_detail_mirrors_startup_window(
                &state.pool,
                cursor_id,
                Duration::from_secs(STARTUP_LEGACY_DETAIL_MIRROR_BUDGET_SECS),
            )
            .await?;
            let retry_soon = window.hit_budget
                || (window.inspected_path_count > 0
                    && window.candidate_count
                        >= STARTUP_LEGACY_DETAIL_MIRROR_CANDIDATE_LIMIT as usize);
            info!(
                task = StartupBackfillTask::LegacyDetailMirrors.log_label(),
                candidate_limit = STARTUP_LEGACY_DETAIL_MIRROR_CANDIDATE_LIMIT,
                elapsed_budget_ms = Duration::from_secs(STARTUP_LEGACY_DETAIL_MIRROR_BUDGET_SECS)
                    .as_millis() as u64,
                candidate_count = window.candidate_count,
                inspected_path_count = window.inspected_path_count,
                changed_path_count = window.changed_path_count,
                hit_budget = window.hit_budget,
                wrapped = window.wrapped,
                "startup legacy detail mirror reconciliation pass completed"
            );
            Ok((
                StartupBackfillRunState {
                    next_cursor_id: window.next_cursor_id,
                    scanned: window.inspected_path_count as u64,
                    updated: window.changed_path_count as u64,
                    hit_scan_limit: retry_soon,
                    retry_soon,
                    force_idle: window.candidate_count == 0,
                    source_unavailable: false,
                    deferred: false,
                    defer_reason: None,
                    samples: Vec::new(),
                },
                format!(
                    "candidate_count={} inspected_path_count={} changed_path_count={} hit_budget={} wrapped={} next_cursor_id={}",
                    window.candidate_count,
                    window.inspected_path_count,
                    window.changed_path_count,
                    window.hit_budget,
                    window.wrapped,
                    window.next_cursor_id,
                ),
            ))
        }
        StartupBackfillTask::HistoricalRollups => {
            let window = materialize_historical_rollups_startup_window(
                &state.pool,
                cursor_id,
                Duration::from_secs(STARTUP_HISTORICAL_ROLLUP_BUDGET_SECS),
            )
            .await?;
            let summary = window.summary;
            let updated = window.changed_path_count as u64;
            info!(
                task = StartupBackfillTask::HistoricalRollups.log_label(),
                candidate_limit = 32_u64,
                batch_limit = STARTUP_HISTORICAL_ROLLUP_BATCH_LIMIT,
                elapsed_budget_ms =
                    Duration::from_secs(STARTUP_HISTORICAL_ROLLUP_BUDGET_SECS).as_millis() as u64,
                candidate_count = window.candidate_count,
                inspected_path_count = window.inspected_path_count,
                changed_path_count = window.changed_path_count,
                wrapped = window.wrapped,
                blocked_archive_batches = summary.blocked_archive_batches,
                "startup historical rollup keyset pass completed"
            );
            Ok((
                StartupBackfillRunState {
                    next_cursor_id: window.next_cursor_id,
                    scanned: summary.scanned_archive_batches as u64,
                    updated,
                    hit_scan_limit: window.hit_budget
                        || (updated > 0
                            && (window.candidate_count >= 32
                                || summary.scanned_archive_batches
                                    >= STARTUP_HISTORICAL_ROLLUP_BATCH_LIMIT as usize)),
                    retry_soon: historical_rollup_should_retry_soon(
                        window.hit_budget,
                        window.candidate_count,
                    ),
                    force_idle: window.candidate_count == 0,
                    source_unavailable: false,
                    deferred: false,
                    defer_reason: None,
                    samples: Vec::new(),
                },
                format!(
                    "candidate_count={} inspected_path_count={} changed_path_count={} hit_budget={} wrapped={} next_cursor_id={} materialized_archive_batches={} blocked_archive_batches={}",
                    window.candidate_count,
                    window.inspected_path_count,
                    window.changed_path_count,
                    window.hit_budget,
                    window.wrapped,
                    window.next_cursor_id,
                    summary.materialized_archive_batches,
                    summary.blocked_archive_batches,
                ),
            ))
        }
    }
}
