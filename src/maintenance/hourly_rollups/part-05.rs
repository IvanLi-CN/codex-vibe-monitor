pub(crate) async fn replay_forward_proxy_archives_into_hourly_rollups_tx(
    tx: &mut SqliteConnection,
) -> Result<HistoricalRollupArchiveReplaySummary> {
    replay_forward_proxy_archives_into_hourly_rollups_tx_with_limits(
        tx,
        Instant::now(),
        None,
        None,
        0,
    )
    .await
}

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

async fn bootstrap_hourly_rollups_with_scope(
    pool: &Pool<Sqlite>,
    invocation_full_detail_days: Option<u64>,
    scope: HourlyRollupRefreshScope,
) -> Result<()> {
    let usage_breakdown_started_at = Instant::now();
    repair_live_invocation_usage_breakdown_rollups(pool).await?;
    info!(
        rollup_bootstrap_step = "usage_breakdown_repair",
        elapsed_ms = usage_breakdown_started_at.elapsed().as_millis() as u64,
        "hourly rollup bootstrap step completed"
    );

    let live_sync_started_at = Instant::now();
    sync_hourly_rollups_from_live_tables_with_scope(pool, invocation_full_detail_days, scope)
        .await?;
    info!(
        rollup_bootstrap_step = "live_rollup_sync",
        elapsed_ms = live_sync_started_at.elapsed().as_millis() as u64,
        "hourly rollup bootstrap step completed"
    );

    let archive_usage_breakdown_started_at = Instant::now();
    repair_materialized_invocation_archive_usage_breakdown_backfill_state(pool).await?;
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
    Ok(())
}

fn runtime_startup_hourly_rollup_refresh_scope() -> HourlyRollupRefreshScope {
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

#[derive(Debug, Clone)]
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

pub(crate) async fn delete_rows_by_ids(
    tx: &mut sqlx::SqliteConnection,
    table: &str,
    ids: &[i64],
) -> Result<()> {
    if ids.is_empty() {
        return Ok(());
    }
    let mut query = QueryBuilder::<Sqlite>::new(format!("DELETE FROM {table} WHERE id IN ("));
    {
        let mut separated = query.separated(", ");
        for id in ids {
            separated.push_bind(id);
        }
    }
    query.push(")");
    query.build().execute(&mut *tx).await?;
    Ok(())
}

pub(crate) async fn sweep_orphan_proxy_raw_files(
    pool: &Pool<Sqlite>,
    config: &AppConfig,
    raw_path_fallback_root: Option<&Path>,
    dry_run: bool,
) -> Result<usize> {
    let raw_dir = config.resolved_proxy_raw_dir();
    if !raw_dir.exists() {
        return Ok(0);
    }

    let referenced = sqlx::query_scalar::<_, String>(
        r#"
        SELECT path
        FROM (
            SELECT request_raw_path AS path
            FROM codex_invocations
            WHERE request_raw_path IS NOT NULL
            UNION
            SELECT response_raw_path AS path
            FROM codex_invocations
            WHERE response_raw_path IS NOT NULL
            UNION
            SELECT response_raw_path AS path
            FROM pool_upstream_request_attempts
            WHERE response_raw_path IS NOT NULL
        )
        WHERE path IS NOT NULL
        "#,
    )
    .fetch_all(pool)
    .await?;

    let mut referenced_paths = HashSet::new();
    for path in referenced {
        for candidate in resolved_raw_path_candidates(&path, raw_path_fallback_root) {
            referenced_paths.insert(candidate);
        }
    }

    let min_file_age = Duration::from_secs(DEFAULT_ORPHAN_SWEEP_MIN_AGE_SECS);
    let mut removed = 0usize;
    for entry in fs::read_dir(&raw_dir)
        .with_context(|| format!("failed to read raw payload directory {}", raw_dir.display()))?
    {
        let entry = entry?;
        let path = entry.path();
        if !entry.file_type()?.is_file() {
            continue;
        }
        let age = match entry.metadata().and_then(|metadata| metadata.modified()) {
            Ok(modified) => modified.elapsed().unwrap_or_default(),
            Err(err) => {
                warn!(path = %path.display(), error = %err, "failed to inspect orphan raw payload file age");
                continue;
            }
        };
        if age < min_file_age {
            continue;
        }
        let normalized = normalize_path_for_compare(&path);
        if referenced_paths.contains(&normalized) {
            continue;
        }
        if dry_run {
            removed += 1;
            continue;
        }
        match fs::remove_file(&path) {
            Ok(_) => removed += 1,
            Err(err) if err.kind() == io::ErrorKind::NotFound => {}
            Err(err) => {
                warn!(path = %path.display(), error = %err, "failed to remove orphan raw payload file");
            }
        }
    }

    Ok(removed)
}
