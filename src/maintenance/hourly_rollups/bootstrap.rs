use super::live_repair::{
    USAGE_BREAKDOWN_REPAIR_MAX_BOOTSTRAP_PAGES, sync_hourly_rollups_from_live_tables_with_scope,
};
use super::*;

pub(crate) async fn bootstrap_hourly_rollups(pool: &Pool<Sqlite>) -> Result<()> {
    bootstrap_hourly_rollups_with_parallel_work_coverage(pool, None).await
}

pub(crate) async fn bootstrap_hourly_rollups_with_parallel_work_coverage(
    pool: &Pool<Sqlite>,
    invocation_full_detail_days: Option<u64>,
) -> Result<()> {
    bootstrap_hourly_rollups_with_scope(
        pool,
        invocation_full_detail_days,
        HourlyRollupRefreshScope::Full,
    )
    .await
}

pub(crate) async fn bootstrap_hourly_rollups_for_runtime_startup(
    pool: &Pool<Sqlite>,
    invocation_full_detail_days: Option<u64>,
) -> Result<()> {
    bootstrap_hourly_rollups_with_scope(
        pool,
        invocation_full_detail_days,
        runtime_startup_hourly_rollup_refresh_scope(),
    )
    .await
}

pub(crate) async fn bootstrap_hourly_rollups_for_runtime_startup_with_work(
    pool: &Pool<Sqlite>,
    invocation_full_detail_days: Option<u64>,
) -> Result<u64> {
    bootstrap_hourly_rollups_with_scope_and_work(
        pool,
        invocation_full_detail_days,
        runtime_startup_hourly_rollup_refresh_scope(),
    )
    .await
}

async fn bootstrap_hourly_rollups_with_scope(
    pool: &Pool<Sqlite>,
    invocation_full_detail_days: Option<u64>,
    scope: HourlyRollupRefreshScope,
) -> Result<()> {
    bootstrap_hourly_rollups_with_scope_and_work(pool, invocation_full_detail_days, scope)
        .await
        .map(|_| ())
}

async fn bootstrap_hourly_rollups_with_scope_and_work(
    pool: &Pool<Sqlite>,
    invocation_full_detail_days: Option<u64>,
    scope: HourlyRollupRefreshScope,
) -> Result<u64> {
    let usage_breakdown_started_at = Instant::now();
    repair_live_invocation_usage_breakdown_rollups(pool).await?;
    info!(
        rollup_bootstrap_step = "usage_breakdown_repair",
        elapsed_ms = usage_breakdown_started_at.elapsed().as_millis() as u64,
        "hourly rollup bootstrap step completed"
    );

    let live_sync_started_at = Instant::now();
    let work_count = sync_hourly_rollups_from_live_tables_with_scope_and_work(
        pool,
        invocation_full_detail_days,
        scope,
    )
    .await?;
    info!(
        rollup_bootstrap_step = "live_rollup_sync",
        elapsed_ms = live_sync_started_at.elapsed().as_millis() as u64,
        "hourly rollup bootstrap step completed"
    );

    let archive_usage_breakdown_started_at = Instant::now();
    for _ in 0..USAGE_BREAKDOWN_REPAIR_MAX_BOOTSTRAP_PAGES {
        repair_materialized_invocation_archive_usage_breakdown_backfill_state(pool).await?;
        if !usage_breakdown_repair_is_pending(pool).await? {
            break;
        }
    }
    info!(
        rollup_bootstrap_step = "archive_usage_breakdown_repair",
        elapsed_ms = archive_usage_breakdown_started_at.elapsed().as_millis() as u64,
        "hourly rollup bootstrap step completed"
    );

    let archive_marker_started_at = Instant::now();
    repair_materialized_upstream_account_archive_markers(pool).await?;
    info!(
        rollup_bootstrap_step = "upstream_account_archive_marker_repair",
        elapsed_ms = archive_marker_started_at.elapsed().as_millis() as u64,
        "hourly rollup bootstrap step completed"
    );

    let account_stats_probe_started_at = Instant::now();
    let account_stats_hourly_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM upstream_account_stats_hourly")
            .fetch_one(pool)
            .await?;
    let account_stats_minute_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM upstream_account_stats_minute")
            .fetch_one(pool)
            .await?;
    info!(
        rollup_bootstrap_step = "upstream_account_stats_probe",
        elapsed_ms = account_stats_probe_started_at.elapsed().as_millis() as u64,
        account_stats_hourly_count,
        account_stats_minute_count,
        "hourly rollup bootstrap step completed"
    );
    if account_stats_hourly_count == 0 || account_stats_minute_count == 0 {
        let account_stats_rebuild_started_at = Instant::now();
        rebuild_upstream_account_stats_rollups_from_sources(pool).await?;
        repair_materialized_upstream_account_archive_markers(pool).await?;
        info!(
            rollup_bootstrap_step = "upstream_account_stats_rebuild",
            elapsed_ms = account_stats_rebuild_started_at.elapsed().as_millis() as u64,
            "hourly rollup bootstrap step completed"
        );
    }
    Ok(work_count)
}

pub(super) fn runtime_startup_hourly_rollup_refresh_scope() -> HourlyRollupRefreshScope {
    // The dedicated startup task owns active-window coverage and its backoff.
    // Bootstrap still catches up rollups, but must not run that planner twice.
    HourlyRollupRefreshScope::SkipActiveAccountActivityV2CoverageRepair
}

pub(crate) async fn refresh_hourly_rollups_for_read_surfaces(pool: &Pool<Sqlite>) -> Result<()> {
    refresh_hourly_rollups_for_read_surfaces_with_scope(pool, HourlyRollupRefreshScope::Full).await
}

async fn refresh_hourly_rollups_for_read_surfaces_with_scope(
    pool: &Pool<Sqlite>,
    scope: HourlyRollupRefreshScope,
) -> Result<()> {
    sync_hourly_rollups_from_live_tables_with_scope(pool, None, scope).await?;
    ensure_invocation_summary_rollups_ready_best_effort(pool).await?;
    Ok(())
}

pub(crate) async fn ensure_hourly_rollups_caught_up(state: &AppState) -> Result<()> {
    let _guard = state.hourly_rollup_sync_lock.lock().await;
    sync_hourly_rollups_from_live_tables_with_parallel_work_coverage(
        &state.pool,
        Some(state.config.invocation_max_days),
    )
    .await
}

pub(crate) async fn refresh_hourly_rollups_for_read_surfaces_best_effort(
    pool: &Pool<Sqlite>,
    hourly_rollup_sync_lock: &Mutex<()>,
    cancel: &CancellationToken,
    reason: &'static str,
    scope: HourlyRollupRefreshScope,
) -> bool {
    let gate = crate::db_pressure::global_db_pressure_gate();
    let _guard = tokio::select! {
        biased;
        _ = cancel.cancelled() => {
            debug!(reason, "background hourly rollup refresh cancelled while waiting for sync lock");
            return true;
        }
        guard = hourly_rollup_sync_lock.lock() => guard,
    };
    let _permit = match gate.try_begin_background("hourly_rollup_refresh") {
        Ok(permit) => permit,
        Err(deny_reason) => {
            warn!(
                reason,
                deny_reason = %deny_reason,
                "background hourly rollup refresh skipped because database pressure gate is closed"
            );
            return true;
        }
    };
    let _write_permit =
        match crate::proxy_sqlite_write_coordinator::proxy_sqlite_write_coordinator()
            .try_acquire(crate::proxy_sqlite_write_coordinator::ProxySqliteWriteClass::P2Derived)
        {
            Some(permit) => permit,
            None => {
                debug!(
                    reason,
                    defer_reason = "coordinator_priority",
                    "background hourly rollup refresh skipped before SQLite access"
                );
                return true;
            }
        };

    let coordinator = crate::proxy_sqlite_write_coordinator::proxy_sqlite_write_coordinator();
    let refresh_result = tokio::select! {
        _ = cancel.cancelled() => {
            debug!(reason, "background hourly rollup refresh cancelled during SQLite work");
            return true;
        }
        _ = coordinator.wait_for_p2_preemption() => {
            debug!(reason, "background hourly rollup refresh yielded to higher-priority SQLite writes");
            return true;
        }
        result = refresh_hourly_rollups_for_read_surfaces_with_scope(pool, scope) => result,
    };
    if let Err(err) = refresh_result {
        gate.record_error("hourly_rollup_refresh", &err);
        warn!(
            error = %err,
            reason,
            "background hourly rollup refresh failed; keeping existing rollups for read surfaces"
        );
    }
    false
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum ActiveAccountActivityV2RepairResult {
    Repaired(ActiveAccountActivityV2RepairOutcome),
    Deferred,
    Failed,
}

pub(crate) async fn repair_active_account_activity_v2_coverage_best_effort(
    pool: &Pool<Sqlite>,
    hourly_rollup_sync_lock: &Mutex<()>,
    reason: &'static str,
) -> ActiveAccountActivityV2RepairResult {
    let gate = crate::db_pressure::global_db_pressure_gate();
    let _guard = hourly_rollup_sync_lock.lock().await;
    let _permit = match gate.try_begin_background("account_activity_v2_priority_repair") {
        Ok(permit) => permit,
        Err(deny_reason) => {
            debug!(
                reason,
                deny_reason = %deny_reason,
                wake_reason = "active_window_coverage_check",
                "active Dashboard coverage repair deferred by database pressure gate"
            );
            return ActiveAccountActivityV2RepairResult::Deferred;
        }
    };
    let _write_permit =
        match crate::proxy_sqlite_write_coordinator::proxy_sqlite_write_coordinator()
            .try_acquire(crate::proxy_sqlite_write_coordinator::ProxySqliteWriteClass::P2Derived)
        {
            Some(permit) => permit,
            None => {
                debug!(
                    reason,
                    defer_reason = "coordinator_priority",
                    wake_reason = "active_window_coverage_check",
                    "active Dashboard coverage repair deferred before SQLite access"
                );
                return ActiveAccountActivityV2RepairResult::Deferred;
            }
        };
    let coordinator = crate::proxy_sqlite_write_coordinator::proxy_sqlite_write_coordinator();
    let repair_result = tokio::select! {
        _ = coordinator.wait_for_p2_preemption() => {
            debug!(reason, wake_reason = "active_window_coverage_check", "active Dashboard coverage repair yielded to higher-priority SQLite writes");
            return ActiveAccountActivityV2RepairResult::Deferred;
        }
        result = repair_active_account_activity_v2_coverage(pool) => result,
    };
    match repair_result {
        Ok(outcome) => ActiveAccountActivityV2RepairResult::Repaired(outcome),
        Err(err) => {
            gate.record_error("account_activity_v2_priority_repair", &err);
            warn!(
                error = %err,
                reason,
                wake_reason = "active_window_coverage_check",
                "active Dashboard coverage repair failed"
            );
            ActiveAccountActivityV2RepairResult::Failed
        }
    }
}
