use super::*;

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
            let updated = sqlx::query(
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
