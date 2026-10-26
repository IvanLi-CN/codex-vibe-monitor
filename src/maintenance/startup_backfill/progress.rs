use super::*;

pub(super) fn startup_backfill_progress_pool(pool: &Pool<Sqlite>) -> Option<&Pool<Sqlite>> {
    if cfg!(test) {
        return Some(pool);
    }
    if let Some(store) = crate::maintenance_store::global() {
        return Some(&store.pool);
    }
    None
}
pub(super) fn startup_backfill_progress_due(progress: &StartupBackfillProgress) -> DateTime<Utc> {
    progress
        .next_run_after
        .as_deref()
        .and_then(parse_to_utc_datetime)
        .unwrap_or_else(Utc::now)
}

#[derive(Debug, Clone, FromRow)]
pub(crate) struct StartupBackfillProgressRow {
    task_name: String,
    cursor_id: i64,
    next_run_after: Option<String>,
    zero_update_streak: i64,
    last_started_at: Option<String>,
    last_finished_at: Option<String>,
    last_scanned: i64,
    last_updated: i64,
    last_status: String,
    suspension_reason: Option<String>,
    next_probe_at: Option<String>,
    wake_generation: i64,
    enabled: i64,
}

#[derive(Debug, Clone)]
pub(crate) struct StartupBackfillProgress {
    pub(crate) task_name: String,
    pub(crate) cursor_id: i64,
    pub(crate) next_run_after: Option<String>,
    pub(crate) zero_update_streak: u32,
    pub(crate) last_started_at: Option<String>,
    pub(crate) last_finished_at: Option<String>,
    pub(crate) last_scanned: u64,
    pub(crate) last_updated: u64,
    pub(crate) last_status: String,
    pub(crate) suspension_reason: Option<String>,
    pub(crate) next_probe_at: Option<String>,
    pub(crate) wake_generation: u64,
    pub(crate) enabled: bool,
}

impl StartupBackfillProgress {
    pub(crate) fn pending(task_name: impl Into<String>) -> Self {
        Self {
            task_name: task_name.into(),
            cursor_id: 0,
            next_run_after: None,
            zero_update_streak: 0,
            last_started_at: None,
            last_finished_at: None,
            last_scanned: 0,
            last_updated: 0,
            last_status: STARTUP_BACKFILL_STATUS_IDLE.to_string(),
            suspension_reason: None,
            next_probe_at: None,
            wake_generation: 0,
            enabled: true,
        }
    }

    pub(crate) fn is_due(&self, now: DateTime<Utc>) -> bool {
        self.enabled
            && self
                .next_run_after
                .as_deref()
                .and_then(parse_to_utc_datetime)
                .is_none_or(|deadline| deadline <= now)
    }
}

impl From<StartupBackfillProgressRow> for StartupBackfillProgress {
    fn from(value: StartupBackfillProgressRow) -> Self {
        Self {
            task_name: value.task_name,
            cursor_id: value.cursor_id,
            next_run_after: value.next_run_after,
            zero_update_streak: value.zero_update_streak.max(0) as u32,
            last_started_at: value.last_started_at,
            last_finished_at: value.last_finished_at,
            last_scanned: value.last_scanned.max(0) as u64,
            last_updated: value.last_updated.max(0) as u64,
            last_status: value.last_status,
            suspension_reason: value.suspension_reason,
            next_probe_at: value.next_probe_at,
            wake_generation: value.wake_generation.max(0) as u64,
            enabled: value.enabled != 0,
        }
    }
}

pub(crate) async fn startup_backfill_task_progress_key(
    state: &AppState,
    task: StartupBackfillTask,
) -> String {
    match task {
        StartupBackfillTask::ProxyCost => {
            let catalog = state.pricing_catalog.read().await;
            startup_backfill_task_progress_key_for_catalog(task, &catalog)
        }
        _ => task.name().to_string(),
    }
}

pub(crate) fn startup_backfill_task_progress_key_for_catalog(
    task: StartupBackfillTask,
    catalog: &PricingCatalog,
) -> String {
    match task {
        StartupBackfillTask::ProxyCost => format!(
            "{}:{}",
            task.name(),
            pricing_backfill_attempt_version(catalog)
        ),
        _ => task.name().to_string(),
    }
}

pub(crate) async fn load_startup_backfill_progress(
    pool: &Pool<Sqlite>,
    task_name: &str,
) -> Result<StartupBackfillProgress> {
    let Some(pool) = startup_backfill_progress_pool(pool) else {
        let mut pending = StartupBackfillProgress::pending(task_name.to_string());
        pending.enabled = false;
        pending.suspension_reason = Some("maintenance_database_unavailable".to_string());
        return Ok(pending);
    };
    load_startup_backfill_progress_from_pool(pool, task_name).await
}

pub(crate) async fn set_startup_backfill_task_enabled(
    pool: &Pool<Sqlite>,
    task: StartupBackfillTask,
    enabled: bool,
) -> Result<StartupBackfillProgress> {
    let task_name = task.name();
    if crate::maintenance_store::managed_startup_backfill_suffix(task_name).is_none() {
        return Err(anyhow!("unknown startup backfill task: {task_name}"));
    }
    if task == StartupBackfillTask::PromptCacheConversationsMaterialization {
        let store = crate::maintenance_store::global()
            .ok_or_else(|| anyhow!("maintenance database unavailable"))?;
        return set_prompt_cache_materialization_enabled_with_store(pool, store, task, enabled)
            .await;
    }
    let mut previous_control = None;
    if let Some(store) = crate::maintenance_store::global() {
        let suffix = crate::maintenance_store::managed_startup_backfill_suffix(task_name)
            .expect("validated managed startup backfill task");
        let managed_key = format!("startup_backfill.{suffix}");
        previous_control = store
            .detail(&managed_key)
            .await
            .with_context(|| format!("failed to load managed task control for {managed_key}"))?
            .map(|detail| detail.task);
        if !store
            .update_control(&managed_key, Some(enabled), None, None)
            .await
            .with_context(|| format!("failed to update managed task control for {managed_key}"))?
        {
            return Err(anyhow!(
                "managed startup backfill task not found: {managed_key}"
            ));
        }
    }
    match set_startup_backfill_progress_enabled(pool, task, enabled).await {
        Ok(progress) => Ok(progress),
        Err(progress_error) => {
            if let (Some(store), Some(previous)) = (
                crate::maintenance_store::global(),
                previous_control.as_ref(),
            ) && let Err(rollback_error) = store.restore_control_state(previous).await
            {
                return Err(anyhow!(
                    "startup backfill control update failed: {progress_error}; rollback failed: {rollback_error}"
                ));
            }
            Err(progress_error)
        }
    }
}

pub(crate) async fn set_startup_backfill_progress_enabled(
    pool: &Pool<Sqlite>,
    task: StartupBackfillTask,
    enabled: bool,
) -> Result<StartupBackfillProgress> {
    let Some(pool) = startup_backfill_progress_pool(pool) else {
        #[cfg(test)]
        {
            return Ok(StartupBackfillProgress::pending(task.name().to_string()));
        }
        #[cfg(not(test))]
        {
            return Err(anyhow!("maintenance database unavailable"));
        }
    };
    let task_name = task.name();
    let like_pattern = format!("{task_name}:%");
    if crate::maintenance_store::managed_startup_backfill_suffix(task_name).is_none() {
        return Err(anyhow!("unknown startup backfill task: {task_name}"));
    }
    let disabled_until = format_utc_iso(Utc::now() + ChronoDuration::days(3650));
    crate::maintenance_store::observe_store_write(
        crate::observability::contention::StoreOperation::Control,
        async {
            sqlx::query(
                "INSERT INTO startup_backfill_progress (
                    task_name,cursor_id,next_run_after,zero_update_streak,last_started_at,last_finished_at,
                    last_scanned,last_updated,last_status,suspension_reason,next_probe_at,wake_generation,enabled
                 ) VALUES (?,0,?,0,NULL,NULL,0,0,'idle',?,NULL,0,?)
                 ON CONFLICT(task_name) DO UPDATE SET
                     enabled=excluded.enabled,
                     next_run_after=excluded.next_run_after,
                     suspension_reason=excluded.suspension_reason,
                     next_probe_at=NULL,
                     wake_generation=startup_backfill_progress.wake_generation + 1",
            )
            .bind(task_name)
            .bind(if enabled {
                None
            } else {
                Some(disabled_until.as_str())
            })
            .bind(if enabled {
                None
            } else {
                Some("operator_disabled")
            })
            .bind(if enabled { 1_i64 } else { 0_i64 })
            .execute(pool)
            .await
            .with_context(|| format!("failed to update startup backfill task control for {task_name}"))
            .map(|_| ())
        },
    )
    .await?;
    if task == StartupBackfillTask::ProxyCost {
        crate::maintenance_store::observe_store_write(
            crate::observability::contention::StoreOperation::Control,
            async {
                sqlx::query(
                    "UPDATE startup_backfill_progress
                     SET enabled=?, next_run_after=?, suspension_reason=?, next_probe_at=NULL,
                         wake_generation=wake_generation + 1
                     WHERE task_name LIKE ?",
                )
                .bind(if enabled { 1_i64 } else { 0_i64 })
                .bind(if enabled {
                    None
                } else {
                    Some(disabled_until.as_str())
                })
                .bind(if enabled {
                    None
                } else {
                    Some("operator_disabled")
                })
                .bind(like_pattern)
                .execute(pool)
                .await
                .with_context(|| {
                    format!("failed to update versioned startup backfill control for {task_name}")
                })
                .map(|_| ())
            },
        )
        .await?;
    }

    if enabled {
        let ticket = STARTUP_BACKFILL_SCHEDULER
            .record_notification(task, WakeDecision::New)
            .expect("enabled startup backfill control wake must create an observation ticket");
        STARTUP_BACKFILL_SCHEDULER.wake(task);
        record_scheduler_wake_confirmation(&STARTUP_BACKFILL_SCHEDULER, ticket);
    } else {
        let _ = STARTUP_BACKFILL_SCHEDULER.record_notification(task, WakeDecision::Disabled);
        STARTUP_BACKFILL_SCHEDULER.clear_next_due(task);
    }
    load_startup_backfill_progress(pool, task_name).await
}

pub(crate) async fn mark_startup_backfill_running(
    pool: &Pool<Sqlite>,
    task_name: &str,
    cursor_id: i64,
) -> Result<()> {
    let Some(pool) = startup_backfill_progress_pool(pool) else {
        return Ok(());
    };
    if task_name == StartupBackfillTask::PromptCacheConversationsMaterialization.name() {
        return prompt_cache_control::mark_prompt_cache_materialization_running(
            pool, task_name, cursor_id,
        )
        .await;
    }
    let now = format_utc_iso(Utc::now());
    crate::maintenance_store::observe_store_write(
        crate::observability::contention::StoreOperation::Progress,
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
        },
    )
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
    let Some(pool) = startup_backfill_progress_pool(pool) else {
        return Ok(());
    };
    save_startup_backfill_progress_to_pool(pool, task_name, update).await
}

pub(crate) async fn save_startup_backfill_progress_to_pool(
    pool: &Pool<Sqlite>,
    task_name: &str,
    update: StartupBackfillProgressUpdate<'_>,
) -> Result<()> {
    let finished_at = format_utc_iso(Utc::now());
    crate::maintenance_store::observe_store_write(
        crate::observability::contention::StoreOperation::Progress,
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
                VALUES (?1, ?2, ?3, ?4, NULL, ?5, ?6, ?7, ?8, ?9, ?10, 0)
                ON CONFLICT(task_name) DO UPDATE SET
                    cursor_id = excluded.cursor_id,
                    next_run_after = excluded.next_run_after,
                    zero_update_streak = excluded.zero_update_streak,
                    last_finished_at = excluded.last_finished_at,
                    last_scanned = excluded.last_scanned,
                    last_updated = excluded.last_updated,
                    last_status = CASE
                        WHEN startup_backfill_progress.suspension_reason = ?13
                            THEN startup_backfill_progress.last_status
                        ELSE excluded.last_status
                    END,
                    suspension_reason = CASE
                        WHEN startup_backfill_progress.suspension_reason = ?13
                            THEN startup_backfill_progress.suspension_reason
                        ELSE excluded.suspension_reason
                    END,
                    next_probe_at = CASE
                        WHEN startup_backfill_progress.suspension_reason = ?13
                            THEN startup_backfill_progress.next_probe_at
                        ELSE excluded.next_probe_at
                    END
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
            .bind(crate::maintenance_store::MIGRATION_HOLD_REASON)
            .execute(pool)
            .await?;
            Ok(())
        },
    )
    .await?;
    Ok(())
}

pub(super) async fn save_startup_backfill_progress_for_task(
    pool: &Pool<Sqlite>,
    task: StartupBackfillTask,
    task_name: &str,
    expected_wake_generation: u64,
    update: StartupBackfillProgressUpdate<'_>,
) -> Result<()> {
    if task == StartupBackfillTask::PromptCacheConversationsMaterialization {
        let Some(progress_pool) = startup_backfill_progress_pool(pool) else {
            return Ok(());
        };
        return save_prompt_cache_materialization_progress(
            progress_pool,
            task_name,
            update,
            expected_wake_generation,
        )
        .await;
    }
    save_startup_backfill_progress(pool, task_name, update).await
}

pub(crate) async fn load_startup_backfill_progress_from_pool(
    pool: &Pool<Sqlite>,
    task_name: &str,
) -> Result<StartupBackfillProgress> {
    let progress = sqlx::query_as::<_, StartupBackfillProgressRow>(
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
            wake_generation,
            enabled
        FROM startup_backfill_progress
        WHERE task_name = ?1
        LIMIT 1
        "#,
    )
    .bind(task_name)
    .fetch_optional(pool)
    .await?;
    let managed_tasks_present = sqlx::query_scalar::<_, i64>(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='managed_tasks')",
    )
    .fetch_one(pool)
    .await
    .unwrap_or(0)
        != 0;
    let managed_task_key = managed_tasks_present
        .then(|| crate::maintenance_store::managed_startup_backfill_suffix(task_name))
        .flatten()
        .map(|suffix| format!("startup_backfill.{suffix}"));
    let managed_enabled = if let Some(task_key) = managed_task_key.as_deref() {
        sqlx::query_scalar::<_, bool>("SELECT enabled FROM managed_tasks WHERE task_key=?")
            .bind(task_key)
            .fetch_optional(pool)
            .await?
    } else {
        None
    };
    let is_prompt_cache_materialization =
        crate::maintenance_store::managed_startup_backfill_suffix(task_name)
            == Some("prompt_cache_conversations_materialization");
    if is_prompt_cache_materialization && managed_enabled.is_none() {
        return Err(anyhow!("maintenance database control is unavailable"));
    }
    if let Some(progress) = progress {
        let mut progress: StartupBackfillProgress = progress.into();
        if is_prompt_cache_materialization {
            progress.enabled = managed_enabled.unwrap_or(false);
            return Ok(progress);
        }
        if let Some(enabled) = managed_enabled
            && progress.enabled != enabled
        {
            let disabled_until = format_utc_iso_millis(Utc::now() + ChronoDuration::days(3650));
            let (next_run_after, suspension_reason) = if enabled {
                (None, None)
            } else {
                (
                    Some(disabled_until.clone()),
                    Some("operator_disabled".to_string()),
                )
            };
            let updated = crate::maintenance_store::observe_store_write(
                crate::observability::contention::StoreOperation::Control,
                async {
                    let result = sqlx::query(
                        "UPDATE startup_backfill_progress
                         SET enabled=?, next_run_after=?, suspension_reason=?, next_probe_at=NULL,
                             wake_generation=wake_generation + 1
                         WHERE task_name=?
                           AND EXISTS (
                               SELECT 1 FROM managed_tasks
                               WHERE task_key=? AND enabled=?
                           )",
                    )
                    .bind(if enabled { 1_i64 } else { 0_i64 })
                    .bind(&next_run_after)
                    .bind(&suspension_reason)
                    .bind(task_name)
                    .bind(
                        managed_task_key
                            .as_deref()
                            .expect("managed task key exists for managed progress"),
                    )
                    .bind(if enabled { 1_i64 } else { 0_i64 })
                    .execute(pool)
                    .await?;
                    Ok::<_, anyhow::Error>(result)
                },
            )
            .await?;
            let effective_enabled = if updated.rows_affected() == 0 {
                sqlx::query_scalar::<_, bool>("SELECT enabled FROM managed_tasks WHERE task_key=?")
                    .bind(
                        managed_task_key
                            .as_deref()
                            .expect("managed task key exists for managed progress"),
                    )
                    .fetch_optional(pool)
                    .await?
                    .unwrap_or(false)
            } else {
                enabled
            };
            let (next_run_after, suspension_reason) = if effective_enabled {
                (None, None)
            } else {
                (
                    Some(format_utc_iso_millis(
                        Utc::now() + ChronoDuration::days(3650),
                    )),
                    Some("operator_disabled".to_string()),
                )
            };
            progress.enabled = effective_enabled;
            progress.next_run_after = next_run_after;
            progress.suspension_reason = suspension_reason;
            progress.next_probe_at = None;
            progress.wake_generation = progress.wake_generation.saturating_add(1);
        }
        return Ok(progress);
    }

    let enabled = match (managed_enabled, managed_tasks_present) {
        (Some(enabled), _) => enabled,
        (None, false)
            if crate::maintenance_store::managed_startup_backfill_suffix(task_name).is_some() =>
        {
            true
        }
        _ => false,
    };
    let mut pending = StartupBackfillProgress::pending(task_name.to_string());
    pending.enabled = enabled;
    if !enabled {
        pending.next_run_after = Some(format_utc_iso_millis(
            Utc::now() + ChronoDuration::days(3650),
        ));
        pending.suspension_reason = Some("operator_disabled".to_string());
    }
    Ok(pending)
}
