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
                WHEN startup_backfill_progress.wake_generation > ?11
                    THEN NULL
                ELSE excluded.suspension_reason
            END,
            next_probe_at = CASE
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
    STARTUP_BACKFILL_SCHEDULER.record_next_due(task, retry_at);
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
    let task_name = task.name();
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
        && parse_to_utc_datetime(next_run_after).is_some_and(|deadline| deadline > Utc::now())
    {
        let deadline = parse_to_utc_datetime(next_run_after).expect("validated retry deadline");
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
            scheduler.wake(task);
        } else {
            scheduler.clear_pending(task);
        }
    });
}

pub(super) fn coordinator_for_prompt_cache_run()
-> Option<crate::proxy_sqlite_write_coordinator::ProxySqliteWritePermit> {
    crate::proxy_sqlite_write_coordinator::proxy_sqlite_write_coordinator()
        .try_acquire(crate::proxy_sqlite_write_coordinator::ProxySqliteWriteClass::P2Derived)
}
