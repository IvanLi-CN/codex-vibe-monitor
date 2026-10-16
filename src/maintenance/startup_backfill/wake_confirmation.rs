use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum StartupBackfillWakeConfirmation {
    Confirmed {
        wake_task: bool,
        next_due: Option<DateTime<Utc>>,
    },
    Retry,
    Obsolete,
    Cancelled,
}

pub(super) struct WakeProgressWriteContext<'a> {
    pub(super) task_name: &'a str,
    pub(super) managed_task_key: &'a str,
    pub(super) managed_tasks_present: bool,
    pub(super) expected_generation: Option<i64>,
    pub(super) now: &'a str,
    pub(super) ticket: WakeObligationTicket,
    pub(super) scheduler: &'a StartupBackfillScheduler,
}

pub(super) async fn confirm_startup_backfill_progress_write(
    pool: &Pool<Sqlite>,
    task: StartupBackfillTask,
    context: WakeProgressWriteContext<'_>,
) -> Result<u64> {
    let WakeProgressWriteContext {
        task_name,
        managed_task_key,
        managed_tasks_present,
        expected_generation,
        now,
        ticket,
        scheduler,
    } = context;
    let managed_guard = if managed_tasks_present {
        "AND EXISTS(SELECT 1 FROM managed_tasks WHERE task_key=? AND enabled!=0)"
    } else {
        ""
    };
    let result = observe_wake_attempt(task, async {
        let result = crate::maintenance_store::observe_store_write(
            crate::observability::contention::StoreOperation::Wake,
            async {
                if !scheduler.is_current_wake_obligation(ticket) {
                    return Ok::<Option<sqlx::sqlite::SqliteQueryResult>, anyhow::Error>(None);
                }
                let result = if let Some(expected_generation) = expected_generation {
                    let query_sql = format!(
                        r#"UPDATE startup_backfill_progress
                           SET next_run_after=CASE
                                   WHEN last_status='failed'
                                        AND next_run_after IS NOT NULL
                                        AND next_run_after > ?
                                       THEN next_run_after
                                   ELSE NULL
                               END,
                               last_status=CASE
                                   WHEN last_status='failed'
                                        AND next_run_after IS NOT NULL
                                        AND next_run_after > ?
                                       THEN last_status
                                   ELSE ?
                               END,
                               suspension_reason=CASE
                                   WHEN last_status='failed'
                                        AND next_run_after IS NOT NULL
                                        AND next_run_after > ?
                                       THEN suspension_reason
                                   ELSE NULL
                               END,
                               next_probe_at=CASE
                                   WHEN last_status='failed'
                                        AND next_run_after IS NOT NULL
                                        AND next_run_after > ?
                                       THEN next_probe_at
                                   ELSE NULL
                               END,
                               wake_generation=wake_generation + 1
                           WHERE task_name=?
                             AND enabled!=0
                             AND wake_generation=?
                             {managed_guard}"#,
                        managed_guard = managed_guard,
                    );
                    let mut query = sqlx::query(&query_sql)
                        .bind(now)
                        .bind(now)
                        .bind(STARTUP_BACKFILL_STATUS_IDLE)
                        .bind(now)
                        .bind(now)
                        .bind(task_name)
                        .bind(expected_generation);
                    if managed_tasks_present {
                        query = query.bind(managed_task_key);
                    }
                    query.execute(pool).await?
                } else {
                    let insert_guard = if managed_tasks_present {
                        "WHERE EXISTS(SELECT 1 FROM managed_tasks WHERE task_key=? AND enabled!=0)"
                    } else {
                        "WHERE 1=1"
                    };
                    let query_sql = format!(
                        r#"INSERT INTO startup_backfill_progress (
                                task_name,cursor_id,next_run_after,zero_update_streak,last_started_at,
                                last_finished_at,last_scanned,last_updated,last_status,suspension_reason,
                                next_probe_at,wake_generation,enabled
                            )
                            SELECT ?,0,NULL,0,NULL,NULL,0,0,?,NULL,NULL,1,1
                            {insert_guard}
                            ON CONFLICT(task_name) DO UPDATE SET
                                next_run_after=CASE
                                    WHEN startup_backfill_progress.last_status='failed'
                                         AND startup_backfill_progress.next_run_after IS NOT NULL
                                         AND startup_backfill_progress.next_run_after > ?
                                        THEN startup_backfill_progress.next_run_after
                                    ELSE NULL
                                END,
                                last_status=CASE
                                    WHEN startup_backfill_progress.last_status='failed'
                                         AND startup_backfill_progress.next_run_after IS NOT NULL
                                         AND startup_backfill_progress.next_run_after > ?
                                        THEN startup_backfill_progress.last_status
                                    ELSE ?
                                END,
                                suspension_reason=CASE
                                    WHEN startup_backfill_progress.last_status='failed'
                                         AND startup_backfill_progress.next_run_after IS NOT NULL
                                         AND startup_backfill_progress.next_run_after > ?
                                        THEN startup_backfill_progress.suspension_reason
                                    ELSE NULL
                                END,
                                next_probe_at=CASE
                                    WHEN startup_backfill_progress.last_status='failed'
                                         AND startup_backfill_progress.next_run_after IS NOT NULL
                                         AND startup_backfill_progress.next_run_after > ?
                                        THEN startup_backfill_progress.next_probe_at
                                    ELSE NULL
                                END,
                                wake_generation=startup_backfill_progress.wake_generation + 1
                            WHERE startup_backfill_progress.enabled!=0
                              {managed_guard}"#,
                        insert_guard = insert_guard,
                        managed_guard = managed_guard,
                    );
                    let mut query = sqlx::query(&query_sql)
                        .bind(task_name)
                        .bind(STARTUP_BACKFILL_STATUS_IDLE);
                    if managed_tasks_present {
                        query = query.bind(managed_task_key);
                    }
                    query = query
                        .bind(now)
                        .bind(now)
                        .bind(STARTUP_BACKFILL_STATUS_IDLE)
                        .bind(now)
                        .bind(now);
                    if managed_tasks_present {
                        query = query.bind(managed_task_key);
                    }
                    query.execute(pool).await?
                };
                Ok::<_, anyhow::Error>(Some(result))
            },
        )
        .await?;
        let rows_affected = result.map_or(0, |result| result.rows_affected());
        Ok::<(u64, bool), anyhow::Error>((rows_affected, rows_affected > 0))
    })
    .await?;
    Ok(result)
}

pub(super) async fn confirm_startup_backfill_wake_obligations(cancel: &CancellationToken) {
    if cancel.is_cancelled() {
        return;
    }
    let Some(store) = crate::maintenance_store::global() else {
        for failure in STARTUP_BACKFILL_SCHEDULER.defer_due_wake_obligations(Utc::now()) {
            if failure.should_warn {
                warn!(
                    attempt = failure.attempt,
                    next_due = %failure.next_due,
                    "startup backfill wake confirmation deferred because maintenance store is unavailable"
                );
            }
        }
        return;
    };
    confirm_startup_backfill_wake_obligations_with_pool(&store.pool, cancel).await;
}

async fn confirm_startup_backfill_wake_obligations_with_pool(
    pool: &Pool<Sqlite>,
    cancel: &CancellationToken,
) {
    let attempts = STARTUP_BACKFILL_SCHEDULER.claim_due_wake_obligations(Utc::now());
    for attempt in attempts {
        if cancel.is_cancelled() {
            break;
        }
        if !STARTUP_BACKFILL_SCHEDULER.is_current_wake_obligation(attempt.ticket) {
            continue;
        }
        let confirmation = match confirm_startup_backfill_wake_obligation(
            pool,
            cancel,
            &attempt,
            &STARTUP_BACKFILL_SCHEDULER,
        )
        .await
        {
            Ok(confirmation) => confirmation,
            Err(error) => {
                if let Some(failure) =
                    STARTUP_BACKFILL_SCHEDULER.fail_wake_obligation(attempt.ticket, Utc::now())
                    && failure.should_warn
                {
                    warn!(
                        task = attempt.key.task.log_label(),
                        attempt = failure.attempt,
                        next_due = %failure.next_due,
                        error = %error,
                        "startup backfill wake confirmation failed"
                    );
                }
                continue;
            }
        };
        match confirmation {
            StartupBackfillWakeConfirmation::Confirmed {
                wake_task,
                next_due,
            } => {
                if let Some(observation_ticket) =
                    STARTUP_BACKFILL_SCHEDULER.complete_wake_obligation(attempt.ticket)
                {
                    confirm_scheduler_wake_observation(
                        &STARTUP_BACKFILL_SCHEDULER,
                        observation_ticket,
                    );
                    if let Some(next_due) = next_due {
                        STARTUP_BACKFILL_SCHEDULER.record_next_due(attempt.key.task, next_due);
                    } else if wake_task {
                        STARTUP_BACKFILL_SCHEDULER.wake(attempt.key.task);
                    }
                }
            }
            StartupBackfillWakeConfirmation::Retry => {
                if let Some(failure) =
                    STARTUP_BACKFILL_SCHEDULER.fail_wake_obligation(attempt.ticket, Utc::now())
                    && failure.should_warn
                {
                    warn!(
                        task = attempt.key.task.log_label(),
                        attempt = failure.attempt,
                        next_due = %failure.next_due,
                        "startup backfill wake confirmation will retry"
                    );
                }
            }
            StartupBackfillWakeConfirmation::Obsolete => {
                if let Some(observation_ticket) =
                    STARTUP_BACKFILL_SCHEDULER.complete_wake_obligation(attempt.ticket)
                {
                    confirm_scheduler_wake_observation(
                        &STARTUP_BACKFILL_SCHEDULER,
                        observation_ticket,
                    );
                }
            }
            StartupBackfillWakeConfirmation::Cancelled => break,
        }
    }
}

#[cfg(test)]
pub(crate) async fn confirm_startup_backfill_wake_obligations_for_test(pool: &Pool<Sqlite>) {
    confirm_startup_backfill_wake_obligations_with_pool(pool, &CancellationToken::new()).await;
}

pub(super) async fn confirm_startup_backfill_wake_obligation(
    pool: &Pool<Sqlite>,
    cancel: &CancellationToken,
    attempt: &WakeObligationAttempt,
    scheduler: &StartupBackfillScheduler,
) -> Result<StartupBackfillWakeConfirmation> {
    if cancel.is_cancelled() {
        return Ok(StartupBackfillWakeConfirmation::Cancelled);
    }
    if !scheduler.is_current_wake_obligation(attempt.ticket) {
        return Ok(StartupBackfillWakeConfirmation::Obsolete);
    }
    if attempt.key.task == StartupBackfillTask::PromptCacheConversationsMaterialization {
        return confirm_prompt_cache_materialization_wake(pool, cancel, attempt, scheduler).await;
    }

    let task = attempt.key.task;
    let task_name = attempt
        .key
        .progress_key
        .as_deref()
        .unwrap_or_else(|| task.name());
    let managed_task_key = crate::maintenance_store::managed_startup_backfill_suffix(task.name())
        .map(|suffix| format!("startup_backfill.{suffix}"));
    let managed_tasks_present = sqlx::query_scalar::<_, i64>(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='managed_tasks')",
    )
    .fetch_one(pool)
    .await?
        != 0;
    let managed_enabled = if managed_tasks_present {
        sqlx::query_scalar::<_, bool>("SELECT enabled FROM managed_tasks WHERE task_key=?")
            .bind(
                managed_task_key
                    .as_deref()
                    .ok_or_else(|| anyhow!("unknown startup backfill task"))?,
            )
            .fetch_optional(pool)
            .await?
    } else {
        None
    };
    if managed_enabled == Some(false) {
        return Ok(StartupBackfillWakeConfirmation::Obsolete);
    }
    let progress = sqlx::query_as::<_, (bool, i64, Option<String>, String)>(
        "SELECT enabled,wake_generation,next_run_after,last_status
         FROM startup_backfill_progress WHERE task_name=?",
    )
    .bind(task_name)
    .fetch_optional(pool)
    .await?;
    if progress.as_ref().is_some_and(|progress| !progress.0) {
        return Ok(StartupBackfillWakeConfirmation::Obsolete);
    }
    if cancel.is_cancelled() {
        return Ok(StartupBackfillWakeConfirmation::Cancelled);
    }
    if !scheduler.is_current_wake_obligation(attempt.ticket) {
        return Ok(StartupBackfillWakeConfirmation::Obsolete);
    }
    let expected_generation = progress.as_ref().map(|progress| progress.1);
    let now = format_utc_iso(Utc::now());
    let managed_task_key_ref = managed_task_key
        .as_deref()
        .ok_or_else(|| anyhow!("unknown startup backfill task"))?;
    let rows_affected = confirm_startup_backfill_progress_write(
        pool,
        task,
        WakeProgressWriteContext {
            task_name,
            managed_task_key: managed_task_key_ref,
            managed_tasks_present,
            expected_generation,
            now: &now,
            ticket: attempt.ticket,
            scheduler,
        },
    )
    .await?;
    if !scheduler.is_current_wake_obligation(attempt.ticket) {
        return Ok(StartupBackfillWakeConfirmation::Obsolete);
    }
    if cancel.is_cancelled() {
        return Ok(StartupBackfillWakeConfirmation::Cancelled);
    }
    let managed_enabled_after = if managed_tasks_present {
        sqlx::query_scalar::<_, bool>("SELECT enabled FROM managed_tasks WHERE task_key=?")
            .bind(managed_task_key.as_deref().unwrap_or_default())
            .fetch_optional(pool)
            .await?
    } else {
        None
    };
    if managed_enabled_after == Some(false) {
        return Ok(StartupBackfillWakeConfirmation::Obsolete);
    }
    if rows_affected == 0 {
        return Ok(StartupBackfillWakeConfirmation::Retry);
    }
    let progress_after = sqlx::query_as::<_, (bool, i64, Option<String>)>(
        "SELECT enabled,wake_generation,next_run_after
         FROM startup_backfill_progress WHERE task_name=?",
    )
    .bind(task_name)
    .fetch_optional(pool)
    .await?;
    let Some((enabled_after, generation_after, next_run_after)) = progress_after else {
        return Ok(StartupBackfillWakeConfirmation::Retry);
    };
    if !enabled_after {
        return Ok(StartupBackfillWakeConfirmation::Obsolete);
    }
    if managed_enabled_after == Some(false) {
        return Ok(StartupBackfillWakeConfirmation::Obsolete);
    }
    if expected_generation
        .is_some_and(|generation| generation_after != generation.saturating_add(1))
    {
        return Ok(StartupBackfillWakeConfirmation::Retry);
    }
    let managed_enabled_after = if managed_tasks_present {
        sqlx::query_scalar::<_, bool>("SELECT enabled FROM managed_tasks WHERE task_key=?")
            .bind(managed_task_key.as_deref().unwrap_or_default())
            .fetch_optional(pool)
            .await?
    } else {
        None
    };
    if managed_enabled_after == Some(false) {
        return Ok(StartupBackfillWakeConfirmation::Obsolete);
    }
    let next_due = next_run_after
        .as_deref()
        .and_then(parse_to_utc_datetime)
        .filter(|deadline| *deadline > Utc::now());
    Ok(StartupBackfillWakeConfirmation::Confirmed {
        wake_task: next_due.is_none(),
        next_due,
    })
}
