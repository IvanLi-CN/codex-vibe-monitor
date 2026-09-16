pub(crate) async fn load_startup_backfill_progress(
    pool: &Pool<Sqlite>,
    task_name: &str,
) -> Result<StartupBackfillProgress> {
    Ok(sqlx::query_as::<_, StartupBackfillProgressRow>(
        r#"
        SELECT
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
        FROM startup_backfill_progress
        WHERE task_name = ?1
        LIMIT 1
        "#,
    )
    .bind(task_name)
    .fetch_optional(pool)
    .await?
    .map(Into::into)
    .unwrap_or_else(|| StartupBackfillProgress::pending(task_name.to_string())))
}

pub(crate) async fn mark_startup_backfill_running(
    pool: &Pool<Sqlite>,
    task_name: &str,
    cursor_id: i64,
) -> Result<()> {
    let now = format_utc_iso(Utc::now());
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
        VALUES (?1, ?2, NULL, 0, ?3, NULL, 0, 0, ?4, NULL, NULL, 0)
        ON CONFLICT(task_name) DO UPDATE SET
            next_run_after = NULL,
            last_started_at = excluded.last_started_at,
            last_status = excluded.last_status,
            suspension_reason = NULL,
            next_probe_at = NULL
        "#,
    )
    .bind(task_name)
    .bind(cursor_id)
    .bind(&now)
    .bind(STARTUP_BACKFILL_STATUS_RUNNING)
    .execute(pool)
    .await?;
    Ok(())
}

pub(crate) struct StartupBackfillProgressUpdate<'a> {
    pub(crate) cursor_id: i64,
    pub(crate) scanned: u64,
    pub(crate) updated: u64,
    pub(crate) zero_update_streak: u32,
    pub(crate) next_run_after: &'a str,
    pub(crate) status: &'a str,
    pub(crate) suspension_reason: Option<&'a str>,
}

pub(crate) async fn save_startup_backfill_progress(
    pool: &Pool<Sqlite>,
    task_name: &str,
    update: StartupBackfillProgressUpdate<'_>,
) -> Result<()> {
    let finished_at = format_utc_iso(Utc::now());
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
        VALUES (?1, ?2, ?3, ?4, NULL, ?5, ?6, ?7, ?8, ?9, ?10, 0)
        ON CONFLICT(task_name) DO UPDATE SET
            cursor_id = excluded.cursor_id,
            next_run_after = excluded.next_run_after,
            zero_update_streak = excluded.zero_update_streak,
            last_finished_at = excluded.last_finished_at,
            last_scanned = excluded.last_scanned,
            last_updated = excluded.last_updated,
            last_status = excluded.last_status,
            suspension_reason = excluded.suspension_reason,
            next_probe_at = excluded.next_probe_at
        "#,
    )
    .bind(task_name)
    .bind(update.cursor_id)
    .bind(update.next_run_after)
    .bind(i64::from(update.zero_update_streak))
    .bind(&finished_at)
    .bind(update.scanned as i64)
    .bind(update.updated as i64)
    .bind(update.status)
    .bind(update.suspension_reason)
    .bind(if update.suspension_reason.is_some() {
        Some(update.next_run_after)
    } else {
        None
    })
    .execute(pool)
    .await?;
    Ok(())
}

pub(crate) async fn wake_startup_backfill_tasks(
    pool: &Pool<Sqlite>,
    tasks: &[StartupBackfillTask],
    wake_reason: &'static str,
) -> Result<u64> {
    wake_startup_backfill_tasks_with_pricing_catalog(pool, tasks, None, wake_reason).await
}

pub(crate) async fn wake_startup_backfill_tasks_with_pricing_catalog(
    pool: &Pool<Sqlite>,
    tasks: &[StartupBackfillTask],
    pricing_catalog: Option<&PricingCatalog>,
    wake_reason: &'static str,
) -> Result<u64> {
    let mut woken = 0;
    let mut proxy_cost_catalog_missing = false;
    for task in tasks {
        let task_name = match task {
            StartupBackfillTask::ProxyCost => {
                let Some(catalog) = pricing_catalog else {
                    proxy_cost_catalog_missing = true;
                    continue;
                };
                startup_backfill_task_progress_key_for_catalog(*task, catalog)
            }
            _ => task.name().to_string(),
        };
        let outcome = sqlx::query(
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
                next_run_after = NULL,
                next_probe_at = NULL,
                suspension_reason = NULL,
                wake_generation = startup_backfill_progress.wake_generation + 1,
                last_status = ?2
            "#,
        )
        .bind(&task_name)
        .bind(STARTUP_BACKFILL_STATUS_IDLE)
        .execute(pool)
        .await
        .with_context(|| {
            format!(
                "wake startup backfill task={} progress_key={} wake_reason={wake_reason}",
                task.name(),
                task_name
            )
        })?;
        woken += outcome.rows_affected();
        STARTUP_BACKFILL_SCHEDULER.wake(*task);
    }
    if !tasks.is_empty() {
        info!(
            wake_reason,
            woken,
            task_count = tasks.len(),
            "woke affected startup backfill tasks"
        );
    }
    if proxy_cost_catalog_missing {
        return Err(anyhow!(
            "wake startup backfill task={} requires the runtime pricing catalog",
            StartupBackfillTask::ProxyCost.name()
        ));
    }
    Ok(woken)
}

#[derive(Debug, Default)]
pub(crate) struct StartupBackfillMaintenancePass {
    pub(crate) ran_actionable_task: bool,
    pub(crate) had_failure: bool,
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

fn startup_backfill_hourly_rollup_refresh_scope() -> HourlyRollupRefreshScope {
    // The dedicated coverage task owns the active-window planner and its retry
    // deadline. Generic backfill refreshes must never re-enter that planner.
    HourlyRollupRefreshScope::SkipActiveAccountActivityV2CoverageRepair
}

async fn run_startup_backfill_coverage_repair_if_due(
    state: &Arc<AppState>,
    gate: &crate::db_pressure::DbPressureGate,
) -> Result<StartupBackfillTaskRunOutcome> {
    run_startup_backfill_coverage_repair_if_due_with_repair(state, gate, || {
        repair_active_account_activity_v2_coverage(&state.pool)
    })
    .await
}

pub(crate) async fn run_startup_backfill_coverage_repair_if_due_with_repair<Repair, RepairFuture>(
    state: &Arc<AppState>,
    gate: &crate::db_pressure::DbPressureGate,
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

    let coordinator = crate::proxy_sqlite_write_coordinator::proxy_sqlite_write_coordinator();
    let repair_outcome = tokio::select! {
        biased;
        _ = coordinator.wait_for_p2_preemption() => {
            drop(write_permit);
            return Ok(startup_backfill_pressure_defer_outcome(
                task,
                gate,
                crate::db_pressure::DbPressureDenyReason::BackgroundBusy,
            ));
        }
        outcome = repair() => outcome,
    };
    let repair_outcome = match repair_outcome {
        Ok(outcome) => outcome,
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
                return Ok(outcome);
            }
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
