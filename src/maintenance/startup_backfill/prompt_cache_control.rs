#[cfg(test)]
use super::task_runner::run_startup_backfill_task_with_pressure;
use super::*;

pub(super) fn prompt_cache_materialization_failed_outcome(
    phase: Option<String>,
) -> PromptCacheConversationMaterializationRun {
    phase
        .map(|phase| PromptCacheConversationMaterializationRun {
            phase,
            ..Default::default()
        })
        .unwrap_or_default()
}

pub(super) async fn save_prompt_cache_materialization_progress(
    pool: &Pool<Sqlite>,
    task_name: &str,
    update: StartupBackfillProgressUpdate<'_>,
    expected_wake_generation: u64,
) -> Result<()> {
    crate::maintenance_store::observe_store_write(
        crate::observability::contention::StoreOperation::Progress,
        save_prompt_cache_materialization_progress_inner(
            pool,
            task_name,
            update,
            expected_wake_generation,
        ),
    )
    .await
}

async fn save_prompt_cache_materialization_progress_inner(
    pool: &Pool<Sqlite>,
    task_name: &str,
    update: StartupBackfillProgressUpdate<'_>,
    expected_wake_generation: u64,
) -> Result<()> {
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
            next_run_after = CASE
                WHEN ?12=1 OR EXISTS(SELECT 1 FROM managed_tasks WHERE task_key='prompt_cache_materialization' AND enabled=0)
                    THEN startup_backfill_progress.next_run_after
                WHEN startup_backfill_progress.wake_generation > ?11
                    THEN NULL
                ELSE excluded.next_run_after
            END,
            zero_update_streak = excluded.zero_update_streak,
            last_finished_at = excluded.last_finished_at,
            last_scanned = excluded.last_scanned,
            last_updated = excluded.last_updated,
            last_status = excluded.last_status,
            suspension_reason = CASE
                WHEN ?12=1 OR EXISTS(SELECT 1 FROM managed_tasks WHERE task_key='prompt_cache_materialization' AND enabled=0)
                    THEN startup_backfill_progress.suspension_reason
                WHEN startup_backfill_progress.wake_generation > ?11
                    THEN NULL
                ELSE excluded.suspension_reason
            END,
            next_probe_at = CASE
                WHEN ?12=1 OR EXISTS(SELECT 1 FROM managed_tasks WHERE task_key='prompt_cache_materialization' AND enabled=0)
                    THEN startup_backfill_progress.next_probe_at
                WHEN startup_backfill_progress.wake_generation > ?11
                    THEN NULL
                ELSE excluded.next_probe_at
            END
        "#,
    )
    .bind(task_name)
    .bind(update.cursor_id)
    .bind(update.next_run_after)
    .bind(i64::from(update.zero_update_streak))
    .bind(format_utc_iso(Utc::now()))
    .bind(update.scanned as i64)
    .bind(update.updated as i64)
    .bind(update.status)
    .bind(update.suspension_reason)
    .bind(if update.suspension_reason.is_some() {
        Some(update.next_run_after)
    } else {
        None
    })
    .bind(i64::try_from(expected_wake_generation).unwrap_or(i64::MAX))
    .bind(crate::maintenance::maintenance_execution_is_manual())
    .execute(pool)
    .await?;
    sync_prompt_cache_materialization_progress(
        pool,
        update.updated as i64,
        update.status,
        update.cursor_id,
    )
    .await?;
    Ok(())
}

async fn sync_prompt_cache_materialization_progress(
    pool: &Pool<Sqlite>,
    completed: i64,
    phase: &str,
    checkpoint: i64,
) -> Result<()> {
    let updated_at = format_utc_iso_millis(Utc::now());
    sqlx::query(
        "INSERT OR IGNORE INTO managed_task_progress(task_key,updated_at,freshness) VALUES('prompt_cache_materialization',?,'fresh')",
    )
    .bind(&updated_at)
    .execute(pool)
    .await?;
    sqlx::query(
        "UPDATE managed_task_progress
         SET total=NULL,completed=?,phase=?,checkpoint=?,updated_at=?,freshness='fresh',
             unit='conversations',source_scope='prompt_cache_materialization',last_progress_at=?,
             next_inspection_at=(SELECT next_trigger_at FROM managed_tasks WHERE task_key='prompt_cache_materialization'),
             next_catchup_at=(SELECT next_catchup_at FROM managed_tasks WHERE task_key='prompt_cache_materialization'),
             catchup_state=(SELECT CASE WHEN enabled=0 THEN 'disabled' WHEN next_catchup_at IS NOT NULL THEN 'scheduled' ELSE 'idle' END FROM managed_tasks WHERE task_key='prompt_cache_materialization')
         WHERE task_key='prompt_cache_materialization'",
    )
    .bind(completed)
    .bind(phase)
    .bind(checkpoint.to_string())
    .bind(&updated_at)
    .bind(&updated_at)
    .execute(pool)
    .await?;
    Ok(())
}

pub(super) async fn persist_prompt_cache_materialization_defer(
    state: &Arc<AppState>,
    task: StartupBackfillTask,
    task_name: &str,
    progress: &StartupBackfillProgress,
    run: &StartupBackfillRunState,
    defer_reason: &'static str,
) -> Result<StartupBackfillTaskRunOutcome> {
    let retry_at =
        Utc::now() + ChronoDuration::seconds(STARTUP_BACKFILL_ACTIVE_INTERVAL_SECS as i64);
    let retry_after = format_utc_iso(retry_at);
    save_startup_backfill_progress_for_task(
        &state.pool,
        task,
        task_name,
        progress.wake_generation,
        StartupBackfillProgressUpdate {
            cursor_id: progress.cursor_id,
            scanned: run.scanned,
            updated: run.updated,
            zero_update_streak: progress.zero_update_streak,
            next_run_after: &retry_after,
            status: STARTUP_BACKFILL_STATUS_IDLE,
            suspension_reason: Some(defer_reason),
        },
    )
    .await?;
    if !crate::maintenance::maintenance_execution_is_manual() {
        STARTUP_BACKFILL_SCHEDULER.record_next_due(task, retry_at);
    }
    info!(
        task = task.log_label(),
        task_name,
        scanned = run.scanned,
        updated = run.updated,
        defer_reason,
        next_run_after = %retry_after,
        "prompt-cache materialization saved a bounded continuation"
    );
    Ok(StartupBackfillTaskRunOutcome {
        actionable: false,
        failed: false,
        deferred: true,
        completed: true,
        next_due: retry_at,
    })
}

pub(crate) async fn set_prompt_cache_materialization_enabled_with_store(
    _business_pool: &Pool<Sqlite>,
    store: &crate::maintenance_store::MaintenanceStore,
    task: StartupBackfillTask,
    enabled: bool,
) -> Result<StartupBackfillProgress> {
    if task != StartupBackfillTask::PromptCacheConversationsMaterialization {
        return Err(anyhow!(
            "unexpected task for prompt-cache materialization control"
        ));
    }
    let task_name = task.name();
    let suffix = crate::maintenance_store::managed_startup_backfill_suffix(task_name)
        .ok_or_else(|| anyhow!("unknown startup backfill task: {task_name}"))?;
    let task_key = format!("startup_backfill.{suffix}");
    let update = store
        .update_prompt_cache_materialization_control(&task_key, task_name, enabled)
        .await?
        .ok_or_else(|| anyhow!("managed startup backfill task not found: {task_key}"))?;
    if update.changed {
        apply_prompt_cache_control_schedule(
            &store.prompt_cache_materialization_control,
            update.snapshot,
            &STARTUP_BACKFILL_SCHEDULER,
            task,
        );
    }
    load_startup_backfill_progress_from_pool(&store.pool, task_name).await
}

pub(crate) async fn wake_prompt_cache_materialization_with_store(
    store: &crate::maintenance_store::MaintenanceStore,
    wake_reason: &'static str,
) -> Result<u64> {
    wake_prompt_cache_materialization_with_scheduler(
        store,
        wake_reason,
        &STARTUP_BACKFILL_SCHEDULER,
    )
    .await
}

pub(super) async fn wake_prompt_cache_materialization_with_scheduler(
    store: &crate::maintenance_store::MaintenanceStore,
    wake_reason: &'static str,
    scheduler: &StartupBackfillScheduler,
) -> Result<u64> {
    let task = StartupBackfillTask::PromptCacheConversationsMaterialization;
    let decision = if store
        .prompt_cache_materialization_control
        .snapshot()
        .is_some_and(|snapshot| !snapshot.enabled)
    {
        WakeDecision::Disabled
    } else {
        super::startup_backfill_wake_decision(wake_reason)
    };
    let ticket = scheduler.record_notification(task, decision);
    let Some(snapshot) = store.prompt_cache_materialization_control.snapshot() else {
        return Ok(0);
    };
    if !snapshot.enabled {
        return Ok(0);
    }
    if scheduler.has_future_pressure_deadline(task, Utc::now()) {
        return Ok(0);
    }

    let Some(_wake_guard) = store
        .prompt_cache_materialization_control
        .lock_current_generation(snapshot.generation)
        .await
    else {
        return Ok(0);
    };
    let changed = super::observe_wake_attempt(task, async {
        let changed =
            wake_prompt_cache_materialization_write(store, task.name(), scheduler).await?;
        Ok((changed, changed > 0))
    })
    .await?;
    if let Some(ticket) = ticket {
        super::confirm_scheduler_wake_observation(scheduler, ticket);
    }
    if changed > 0
        && store
            .prompt_cache_materialization_control
            .snapshot()
            .is_some_and(|current| current.enabled && current.generation == snapshot.generation)
    {
        scheduler.wake(task);
        info!(
            task = task.log_label(),
            wake_reason, "woke prompt-cache materialization"
        );
        Ok(changed)
    } else {
        Ok(0)
    }
}

async fn wake_prompt_cache_materialization_write(
    store: &crate::maintenance_store::MaintenanceStore,
    task_name: &str,
    scheduler: &StartupBackfillScheduler,
) -> Result<u64> {
    let task = StartupBackfillTask::PromptCacheConversationsMaterialization;
    let changed = crate::maintenance_store::observe_store_write(
        crate::observability::contention::StoreOperation::Wake,
        async {
            let mut transaction = store.pool.begin().await?;
            let progress = sqlx::query_as::<_, (bool, Option<String>)>(
                "SELECT enabled,next_run_after FROM startup_backfill_progress WHERE task_name=?",
            )
            .bind(task_name)
            .fetch_optional(&mut *transaction)
            .await?;
            let Some((progress_enabled, next_run_after)) = progress else {
                transaction.commit().await?;
                return Ok(0);
            };
            if !progress_enabled {
                transaction.commit().await?;
                return Ok(0);
            }
            if scheduler.has_pending_wake(task) {
                transaction.commit().await?;
                return Ok(0);
            }
            if let Some(next_run_after) = next_run_after.as_deref()
                && parse_to_utc_datetime(next_run_after)
                    .is_some_and(|deadline| deadline > Utc::now())
            {
                let deadline =
                    parse_to_utc_datetime(next_run_after).expect("validated retry deadline");
                if scheduler.has_future_pressure_deadline(task, Utc::now()) {
                    transaction.commit().await?;
                    scheduler.record_next_due(task, deadline);
                    return Ok(0);
                }
            }
            let changed = sqlx::query(
                "UPDATE startup_backfill_progress
                 SET next_run_after=NULL, suspension_reason=NULL, next_probe_at=NULL,
                     last_status=?, wake_generation=wake_generation + 1
                 WHERE task_name=? AND enabled != 0
                   AND EXISTS (
                       SELECT 1 FROM managed_tasks
                       WHERE task_key='startup_backfill.prompt_cache_conversations_materialization'
                         AND enabled=1
                   )",
            )
            .bind(STARTUP_BACKFILL_STATUS_IDLE)
            .bind(task_name)
            .execute(&mut *transaction)
            .await?
            .rows_affected();
            transaction.commit().await?;
            Ok(changed)
        },
    )
    .await?;
    Ok(changed)
}

pub(super) async fn confirm_prompt_cache_materialization_wake(
    pool: &Pool<Sqlite>,
    cancel: &CancellationToken,
    attempt: &WakeObligationAttempt,
    scheduler: &StartupBackfillScheduler,
    prompt_cache_control: Option<&crate::maintenance_store::PromptCacheMaterializationControl>,
) -> Result<(super::StartupBackfillWakeConfirmation, bool)> {
    let task = StartupBackfillTask::PromptCacheConversationsMaterialization;
    let task_name = task.name();
    let managed_task_key = "startup_backfill.prompt_cache_conversations_materialization";
    let prompt_cache_control = prompt_cache_control.or_else(|| {
        crate::maintenance_store::global()
            .map(|store| store.prompt_cache_materialization_control.as_ref())
    });
    if !scheduler.is_current_wake_obligation(attempt.ticket) {
        return Ok((super::StartupBackfillWakeConfirmation::Obsolete, false));
    }
    if let Some(next_due) = scheduler
        .next_due_for(task)
        .filter(|_| scheduler.has_future_pressure_deadline(task, Utc::now()))
    {
        return Ok((
            super::StartupBackfillWakeConfirmation::Deferred { next_due },
            false,
        ));
    }
    let control_guard = if let Some(expected_generation) = attempt.key.control_generation {
        let Some(prompt_cache_control) = prompt_cache_control else {
            return Ok((super::StartupBackfillWakeConfirmation::Retry, false));
        };
        let Some(guard) = prompt_cache_control
            .lock_current_generation(expected_generation)
            .await
        else {
            return Ok((super::StartupBackfillWakeConfirmation::Obsolete, false));
        };
        Some(guard)
    } else {
        None
    };
    if cancel.is_cancelled() {
        return Ok((super::StartupBackfillWakeConfirmation::Cancelled, false));
    }
    if !scheduler.is_current_wake_obligation(attempt.ticket) {
        return Ok((super::StartupBackfillWakeConfirmation::Obsolete, false));
    }
    if let Some(next_due) = scheduler
        .next_due_for(task)
        .filter(|_| scheduler.has_future_pressure_deadline(task, Utc::now()))
    {
        return Ok((
            super::StartupBackfillWakeConfirmation::Deferred { next_due },
            false,
        ));
    }

    let managed_tasks_present = sqlx::query_scalar::<_, i64>(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='managed_tasks')",
    )
    .fetch_one(pool)
    .await?
        != 0;
    let managed_enabled = if managed_tasks_present {
        sqlx::query_scalar::<_, bool>("SELECT enabled FROM managed_tasks WHERE task_key=?")
            .bind(managed_task_key)
            .fetch_optional(pool)
            .await?
    } else {
        None
    };
    if managed_enabled == Some(false) {
        return Ok((super::StartupBackfillWakeConfirmation::Obsolete, false));
    }
    let progress = sqlx::query_as::<_, (bool, i64, Option<String>, String)>(
        "SELECT enabled,wake_generation,next_run_after,last_status
         FROM startup_backfill_progress WHERE task_name=?",
    )
    .bind(task_name)
    .fetch_optional(pool)
    .await?;
    if progress.as_ref().is_some_and(|progress| !progress.0) {
        return Ok((super::StartupBackfillWakeConfirmation::Obsolete, false));
    }
    if cancel.is_cancelled() {
        return Ok((super::StartupBackfillWakeConfirmation::Cancelled, false));
    }

    let expected_generation = progress.as_ref().map(|progress| progress.1);
    let now = super::format_utc_iso(super::Utc::now());
    let rows_affected = super::confirm_startup_backfill_progress_write(
        pool,
        task,
        super::WakeProgressWriteContext {
            task_name,
            managed_task_key,
            managed_tasks_present,
            expected_generation,
            now: &now,
            ticket: attempt.ticket,
            scheduler,
        },
    )
    .await?;
    if !scheduler.is_current_wake_obligation(attempt.ticket) {
        return Ok((
            super::StartupBackfillWakeConfirmation::Obsolete,
            rows_affected > 0,
        ));
    }
    if cancel.is_cancelled() {
        return Ok((
            super::StartupBackfillWakeConfirmation::Cancelled,
            rows_affected > 0,
        ));
    }
    if let Some(expected_generation) = attempt.key.control_generation
        && let Some(prompt_cache_control) = prompt_cache_control
        && prompt_cache_control
            .snapshot()
            .is_none_or(|snapshot| !snapshot.enabled || snapshot.generation != expected_generation)
    {
        return Ok((
            super::StartupBackfillWakeConfirmation::Obsolete,
            rows_affected > 0,
        ));
    }
    if rows_affected == 0 {
        let managed_enabled_after = if managed_tasks_present {
            sqlx::query_scalar::<_, bool>("SELECT enabled FROM managed_tasks WHERE task_key=?")
                .bind(managed_task_key)
                .fetch_optional(pool)
                .await?
        } else {
            None
        };
        return Ok((
            if managed_enabled_after == Some(false) {
                super::StartupBackfillWakeConfirmation::Obsolete
            } else {
                super::StartupBackfillWakeConfirmation::Retry
            },
            rows_affected > 0,
        ));
    }
    let progress_after = sqlx::query_as::<_, (bool, i64, Option<String>)>(
        "SELECT enabled,wake_generation,next_run_after
         FROM startup_backfill_progress WHERE task_name=?",
    )
    .bind(task_name)
    .fetch_optional(pool)
    .await?;
    let Some((enabled_after, generation_after, next_run_after)) = progress_after else {
        return Ok((
            super::StartupBackfillWakeConfirmation::Retry,
            rows_affected > 0,
        ));
    };
    if !enabled_after {
        return Ok((
            super::StartupBackfillWakeConfirmation::Obsolete,
            rows_affected > 0,
        ));
    }
    if expected_generation
        .is_some_and(|generation| generation_after != generation.saturating_add(1))
    {
        return Ok((
            super::StartupBackfillWakeConfirmation::Retry,
            rows_affected > 0,
        ));
    }
    let managed_enabled_after = if managed_tasks_present {
        sqlx::query_scalar::<_, bool>("SELECT enabled FROM managed_tasks WHERE task_key=?")
            .bind(managed_task_key)
            .fetch_optional(pool)
            .await?
    } else {
        None
    };
    if managed_enabled_after == Some(false) {
        return Ok((
            super::StartupBackfillWakeConfirmation::Obsolete,
            rows_affected > 0,
        ));
    }
    let next_due = next_run_after
        .as_deref()
        .and_then(super::parse_to_utc_datetime)
        .filter(|deadline| *deadline > super::Utc::now());
    drop(control_guard);
    Ok((
        super::StartupBackfillWakeConfirmation::Confirmed {
            wake_task: next_due.is_none(),
            next_due,
        },
        rows_affected > 0,
    ))
}

#[cfg(test)]
pub(crate) async fn wake_prompt_cache_materialization_for_test(
    store: &crate::maintenance_store::MaintenanceStore,
    wake_reason: &'static str,
) -> Result<u64> {
    let scheduler = StartupBackfillScheduler::default();
    wake_prompt_cache_materialization_with_scheduler(store, wake_reason, &scheduler).await
}

#[cfg(test)]
pub(crate) async fn run_prompt_cache_materialization_with_control_for_test(
    state: &Arc<AppState>,
    control: &Arc<crate::maintenance_store::PromptCacheMaterializationControl>,
    expected_generation: u64,
) -> Result<()> {
    run_startup_backfill_task_with_pressure(
        state,
        StartupBackfillTask::PromptCacheConversationsMaterialization,
        0,
        0,
        false,
        None,
        Some((control, expected_generation)),
    )
    .await
    .map(|_| ())
}

pub(super) fn prompt_cache_tasks_when_startup_backfill_root_is_skipped(
    selected_tasks: Option<&[StartupBackfillTask]>,
) -> Vec<StartupBackfillTask> {
    selected_tasks
        .unwrap_or(StartupBackfillTask::ordered_tasks())
        .iter()
        .copied()
        .filter(|task| *task == StartupBackfillTask::PromptCacheConversationsMaterialization)
        .collect()
}

pub(super) fn legacy_startup_backfill_tasks(
    selected_tasks: Option<&[StartupBackfillTask]>,
) -> Vec<StartupBackfillTask> {
    selected_tasks
        .unwrap_or(StartupBackfillTask::ordered_tasks())
        .iter()
        .copied()
        .filter(|task| *task != StartupBackfillTask::PromptCacheConversationsMaterialization)
        .collect()
}

pub(super) fn requeue_legacy_startup_backfill_tasks(
    selected_tasks: Option<&[StartupBackfillTask]>,
) {
    let Some(selected_tasks) = selected_tasks else {
        return;
    };
    let paused_tasks = legacy_startup_backfill_tasks(Some(selected_tasks));
    if !paused_tasks.is_empty() {
        super::requeue_startup_backfill_tasks(
            &paused_tasks,
            chrono::Utc::now() + chrono::Duration::minutes(5),
        );
    }
}

pub(super) async fn run_background_startup_backfill_pass_with_root_state(
    state: Arc<AppState>,
    cancel: &CancellationToken,
    selected_tasks: Option<&[StartupBackfillTask]>,
    gate: &crate::db_pressure::DbPressureGate,
    startup_backfill_root_skipped: bool,
) -> StartupBackfillMaintenancePass {
    if startup_backfill_root_skipped {
        // Prompt-cache materialization has its own managed control; the root task gates legacy backfills.
        let tasks = prompt_cache_tasks_when_startup_backfill_root_is_skipped(selected_tasks);
        requeue_legacy_startup_backfill_tasks(selected_tasks);
        if tasks.is_empty() {
            return StartupBackfillMaintenancePass::default();
        }
        return super::run_startup_backfill_maintenance_pass_with_gate_held(
            state,
            cancel,
            Some(&tasks),
            gate,
        )
        .await;
    }
    let legacy_tasks = legacy_startup_backfill_tasks(selected_tasks);
    let prompt_tasks = prompt_cache_tasks_when_startup_backfill_root_is_skipped(selected_tasks);
    let mut pass = if legacy_tasks.is_empty() {
        StartupBackfillMaintenancePass::default()
    } else {
        super::run_startup_backfill_maintenance_pass_with_gate_held(
            state.clone(),
            cancel,
            Some(&legacy_tasks),
            gate,
        )
        .await
    };
    if !prompt_tasks.is_empty() {
        let prompt_pass = super::run_startup_backfill_maintenance_pass_with_gate_held(
            state,
            cancel,
            Some(&prompt_tasks),
            gate,
        )
        .await;
        pass.ran_actionable_task |= prompt_pass.ran_actionable_task;
        pass.had_failure |= prompt_pass.had_failure;
        pass.deferred |= prompt_pass.deferred;
        pass.detail = match (pass.detail.take(), prompt_pass.detail) {
            (Some(left), Some(right)) => Some(format!("{left}; {right}")),
            (left, right) => left.or(right),
        };
    }
    pass
}

pub(super) fn prompt_cache_stale_result_outcome() -> StartupBackfillTaskRunOutcome {
    StartupBackfillTaskRunOutcome {
        actionable: false,
        failed: false,
        deferred: false,
        completed: false,
        next_due: Utc::now()
            + ChronoDuration::seconds(STARTUP_BACKFILL_ACTIVE_INTERVAL_SECS as i64),
    }
}

pub(super) fn apply_prompt_cache_control_schedule(
    control: &crate::maintenance_store::PromptCacheMaterializationControl,
    snapshot: crate::maintenance_store::PromptCacheMaterializationControlSnapshot,
    scheduler: &StartupBackfillScheduler,
    task: StartupBackfillTask,
) {
    control.with_current_generation(snapshot.generation, || {
        if snapshot.enabled {
            let ticket = scheduler
                .record_notification(task, WakeDecision::New)
                .expect("enabled prompt-cache control wake must create an observation ticket");
            scheduler.wake(task);
            super::record_scheduler_wake_confirmation(scheduler, ticket);
        } else {
            let _ = scheduler.record_notification(task, WakeDecision::Disabled);
            scheduler.clear_pending(task);
        }
    });
}

pub(super) fn coordinator_for_prompt_cache_run()
-> Option<crate::proxy_sqlite_write_coordinator::ProxySqliteWritePermit> {
    crate::proxy_sqlite_write_coordinator::proxy_sqlite_write_coordinator()
        .try_acquire(crate::proxy_sqlite_write_coordinator::ProxySqliteWriteClass::P2Derived)
}

pub(super) async fn mark_prompt_cache_materialization_running(
    pool: &Pool<Sqlite>,
    task_name: &str,
    cursor_id: i64,
) -> Result<()> {
    crate::maintenance_store::observe_store_write(
        crate::observability::contention::StoreOperation::Progress,
        mark_prompt_cache_materialization_running_inner(pool, task_name, cursor_id),
    )
    .await
}

async fn mark_prompt_cache_materialization_running_inner(
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
            next_run_after = CASE
                WHEN ?5=1 OR EXISTS(SELECT 1 FROM managed_tasks WHERE task_key='prompt_cache_materialization' AND enabled=0)
                    THEN startup_backfill_progress.next_run_after
                ELSE NULL
            END,
            last_started_at = excluded.last_started_at,
            last_status = excluded.last_status,
            suspension_reason = CASE
                WHEN ?5=1 OR EXISTS(SELECT 1 FROM managed_tasks WHERE task_key='prompt_cache_materialization' AND enabled=0)
                    THEN startup_backfill_progress.suspension_reason
                ELSE NULL
            END,
            next_probe_at = CASE
                WHEN ?5=1 OR EXISTS(SELECT 1 FROM managed_tasks WHERE task_key='prompt_cache_materialization' AND enabled=0)
                    THEN startup_backfill_progress.next_probe_at
                ELSE NULL
            END
        "#,
    )
    .bind(task_name)
    .bind(cursor_id)
    .bind(&now)
    .bind(STARTUP_BACKFILL_STATUS_RUNNING)
    .bind(crate::maintenance::maintenance_execution_is_manual())
    .execute(pool)
    .await?;
    sync_prompt_cache_materialization_progress(pool, 0, STARTUP_BACKFILL_STATUS_RUNNING, cursor_id)
        .await?;
    Ok(())
}
