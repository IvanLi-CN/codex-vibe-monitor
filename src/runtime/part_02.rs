#[derive(Default)]
pub(crate) struct RuntimeHandles {
    pub(crate) server_handle: Option<JoinHandle<()>>,
    pub(crate) poller_handle: Option<JoinHandle<()>>,
    pub(crate) upstream_accounts_handle: Option<JoinHandle<()>>,
    pub(crate) forward_proxy_handle: Option<JoinHandle<()>>,
    pub(crate) pool_orphan_recovery_handle: Option<JoinHandle<()>>,
    pub(crate) retention_handle: Option<JoinHandle<()>>,
    pub(crate) startup_backfill_handle: Option<JoinHandle<()>>,
    pub(crate) startup_hot_read_hydration_handle: Option<JoinHandle<()>>,
}

async fn start_runtime_handle_stage<Shutdown, Stage>(
    shutdown_signal: &Shared<Shutdown>,
    cancel: &CancellationToken,
    stage: Stage,
    handle: &mut Option<JoinHandle<()>>,
) -> bool
where
    Shutdown: Future<Output = ()>,
    Stage: Future<Output = Option<JoinHandle<()>>>,
{
    match run_startup_stage_until_shutdown(shutdown_signal, cancel, stage).await {
        StartupStageOutcome::SkippedByShutdown => true,
        StartupStageOutcome::Completed {
            result,
            shutdown_requested,
        } => {
            *handle = result;
            shutdown_requested
        }
    }
}

async fn start_pre_http_runtime_maintenance<Shutdown>(
    state: &Arc<AppState>,
    shutdown_signal: &Shared<Shutdown>,
    cancel: &CancellationToken,
    handles: &mut RuntimeHandles,
) -> bool
where
    Shutdown: Future<Output = ()>,
{
    if start_runtime_handle_stage(
        shutdown_signal,
        cancel,
        async {
            Some(spawn_upstream_account_maintenance(
                state.clone(),
                cancel.clone(),
            ))
        },
        &mut handles.upstream_accounts_handle,
    )
    .await
    {
        return true;
    }
    if start_runtime_handle_stage(
        shutdown_signal,
        cancel,
        async {
            Some(spawn_forward_proxy_maintenance(
                state.clone(),
                cancel.clone(),
            ))
        },
        &mut handles.forward_proxy_handle,
    )
    .await
    {
        return true;
    }
    if start_runtime_handle_stage(
        shutdown_signal,
        cancel,
        async {
            Some(spawn_data_retention_maintenance(
                state.clone(),
                cancel.clone(),
            ))
        },
        &mut handles.retention_handle,
    )
    .await
    {
        return true;
    }
    start_runtime_handle_stage(
        shutdown_signal,
        cancel,
        async {
            Some(spawn_pool_orphan_recovery_maintenance(
                state.clone(),
                cancel.clone(),
            ))
        },
        &mut handles.pool_orphan_recovery_handle,
    )
    .await
}

async fn start_post_http_runtime_maintenance<Shutdown>(
    state: &Arc<AppState>,
    shutdown_signal: &Shared<Shutdown>,
    cancel: &CancellationToken,
    handles: &mut RuntimeHandles,
) -> bool
where
    Shutdown: Future<Output = ()>,
{
    start_runtime_handle_stage(
        shutdown_signal,
        cancel,
        async {
            Some(spawn_startup_backfill_maintenance(
                state.clone(),
                cancel.clone(),
            ))
        },
        &mut handles.startup_backfill_handle,
    )
    .await
}

pub(crate) async fn run_runtime_until_shutdown<F>(
    state: Arc<AppState>,
    startup_started_at: Instant,
    spawn_background_hourly_rollup_bootstrap: bool,
    shutdown_signal: F,
) -> Result<()>
where
    F: Future<Output = ()> + Send + 'static,
{
    let cancel = state.shutdown.clone();
    let runtime_init_started_at = Instant::now();
    let shutdown_signal = shutdown_signal.shared();
    let shutdown_cancel = cancel.clone();
    let shutdown_relay_signal = shutdown_signal.clone();
    let shutdown_watcher = tokio::spawn(async move {
        shutdown_relay_signal.await;
        begin_runtime_shutdown(&shutdown_cancel);
    });
    let mut handles = RuntimeHandles::default();

    let sync_shutdown_requested = match run_startup_stage_until_shutdown(
        &shutdown_signal,
        &cancel,
        sync_forward_proxy_routes(state.as_ref()),
    )
    .await
    {
        StartupStageOutcome::SkippedByShutdown => true,
        StartupStageOutcome::Completed {
            result,
            shutdown_requested,
        } => {
            if let Err(err) = result {
                warn!(error = %err, "failed to initialize forward proxy xray routes at startup");
            }
            shutdown_requested
        }
    };
    log_startup_phase("runtime_init", runtime_init_started_at);
    if sync_shutdown_requested
        || start_pre_http_runtime_maintenance(&state, &shutdown_signal, &cancel, &mut handles).await
    {
        return drain_runtime_after_pending_shutdown(state, shutdown_watcher, handles).await;
    }

    let http_ready_started_at = Instant::now();
    let http_shutdown_requested = match run_startup_stage_until_shutdown(
        &shutdown_signal,
        &cancel,
        spawn_http_server(state.clone()),
    )
    .await
    {
        StartupStageOutcome::SkippedByShutdown => true,
        StartupStageOutcome::Completed {
            result,
            shutdown_requested,
        } => {
            let (_http_addr, handle) = result?;
            handles.server_handle = Some(handle);
            shutdown_requested
        }
    };
    if http_shutdown_requested {
        return drain_runtime_after_pending_shutdown(state, shutdown_watcher, handles).await;
    }

    handles.startup_hot_read_hydration_handle = Some(
        publish_http_readiness_and_spawn_hot_read_hydration(state.clone(), startup_started_at),
    );
    log_startup_phase("http_ready", http_ready_started_at);
    spawn_system_status_snapshot_maintenance(state.clone());
    if start_post_http_runtime_maintenance(&state, &shutdown_signal, &cancel, &mut handles).await {
        return drain_runtime_after_pending_shutdown(state, shutdown_watcher, handles).await;
    }

    let startup_hourly_rollup_bootstrap_handle = spawn_background_hourly_rollup_bootstrap
        .then(|| spawn_runtime_startup_hourly_rollup_bootstrap(state.clone(), cancel.clone()));
    tokio::select! {
        biased;
        _ = shutdown_signal => begin_runtime_shutdown(&cancel),
        _ = cancel.cancelled() => {}
    }
    if let Some(handle) = startup_hourly_rollup_bootstrap_handle
        && let Err(err) = handle.await
    {
        error!(
            ?err,
            "background startup hourly rollup bootstrap task terminated unexpectedly"
        );
    }
    drain_runtime_after_pending_shutdown(state, shutdown_watcher, handles).await
}

pub(crate) fn begin_runtime_shutdown(cancel: &CancellationToken) {
    if !cancel.is_cancelled() {
        info!("shutdown signal received; beginning graceful shutdown");
        cancel.cancel();
    }
}

async fn drain_runtime_background_workers(state: &AppState, handles: RuntimeHandles) {
    if let Some(server_handle) = handles.server_handle {
        info!("http server graceful drain started");
        if let Err(err) = server_handle.await {
            error!(?err, "http server terminated unexpectedly");
        }
        info!("http server graceful drain finished");
    }

    if let Some(poller_handle) = handles.poller_handle {
        if let Err(err) = poller_handle.await {
            error!(?err, "poller task terminated unexpectedly");
        }
        info!("scheduler drained");
    }
    if let Some(upstream_accounts_handle) = handles.upstream_accounts_handle
        && let Err(err) = upstream_accounts_handle.await
    {
        error!(
            ?err,
            "upstream account maintenance task terminated unexpectedly"
        );
    }
    state.upstream_accounts.drain_background_tasks().await;
    if let Some(forward_proxy_handle) = handles.forward_proxy_handle
        && let Err(err) = forward_proxy_handle.await
    {
        error!(
            ?err,
            "forward proxy maintenance task terminated unexpectedly"
        );
    }
    if let Some(pool_orphan_recovery_handle) = handles.pool_orphan_recovery_handle
        && let Err(err) = pool_orphan_recovery_handle.await
    {
        error!(
            ?err,
            "pool orphan recovery maintenance task terminated unexpectedly"
        );
    }
    if let Some(retention_handle) = handles.retention_handle
        && let Err(err) = retention_handle.await
    {
        error!(?err, "retention maintenance task terminated unexpectedly");
    }
    if let Some(startup_backfill_handle) = handles.startup_backfill_handle
        && let Err(err) = startup_backfill_handle.await
    {
        error!(
            ?err,
            "startup backfill maintenance task terminated unexpectedly"
        );
    }
    if let Some(startup_hot_read_hydration_handle) = handles.startup_hot_read_hydration_handle
        && let Err(err) = startup_hot_read_hydration_handle.await
    {
        error!(
            ?err,
            "startup hot-read hydration coordinator terminated unexpectedly"
        );
    }
}

pub(crate) async fn drain_runtime_after_shutdown(
    state: Arc<AppState>,
    handles: RuntimeHandles,
) -> Result<()> {
    drain_runtime_background_workers(state.as_ref(), handles).await;
    let runtime_shutdown_summary = state.proxy_runtime_invocations.shutdown_summary();
    if runtime_shutdown_summary.running_count > 0 {
        warn!(
            running_snapshot_shutdown_skipped_count = runtime_shutdown_summary.running_count,
            oldest_age_ms = runtime_shutdown_summary.oldest_age_ms,
            "skipping P2 memory running snapshots during graceful shutdown"
        );
    }
    state.sqlite_batch_writer.shutdown_and_drain().await;

    let broadcast_handles = {
        let mut guard = state.proxy_summary_quota_broadcast_handle.lock().await;
        std::mem::take(&mut *guard)
    };
    if !broadcast_handles.is_empty() {
        for broadcast_handle in broadcast_handles {
            if let Err(err) = broadcast_handle.await {
                error!(
                    ?err,
                    "summary/quota broadcast worker terminated unexpectedly"
                );
            }
        }
        info!("summary/quota broadcast worker drained");
    }

    state.xray_supervisor.lock().await.shutdown_all().await;
    info!("shutdown complete");

    Ok(())
}

pub(crate) fn init_tracing() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,tower_http=info".into()),
        )
        .with_target(false)
        .init();
}

pub(crate) fn log_startup_phase(phase: &'static str, started_at: Instant) {
    info!(
        phase,
        elapsed_ms = started_at.elapsed().as_millis() as u64,
        "startup phase finished"
    );
}

pub(crate) fn spawn_runtime_startup_hourly_rollup_bootstrap(
    state: Arc<AppState>,
    cancel: CancellationToken,
) -> JoinHandle<()> {
    tokio::spawn(run_runtime_startup_hourly_rollup_bootstrap(state, cancel))
}

async fn run_runtime_startup_hourly_rollup_bootstrap(
    state: Arc<AppState>,
    cancel: CancellationToken,
) {
    let started_at = Instant::now();
    let pressure_gate = crate::db_pressure::global_db_pressure_gate();
    let coordinator = crate::proxy_sqlite_write_coordinator::proxy_sqlite_write_coordinator();
    let task_start_window = format_utc_iso_millis(Utc::now());
    loop {
        let Some(task_run) = record_startup_hourly_rollup_bootstrap_task(
            state.as_ref(),
            &cancel,
            &task_start_window,
            pressure_gate,
            &coordinator,
        )
        .await
        else {
            return;
        };
        let Some((rollup_guard, pressure_permit, write_permit)) =
            acquire_startup_hourly_rollup_work(
                state.as_ref(),
                &cancel,
                &task_run,
                started_at,
                pressure_gate,
                &coordinator,
            )
            .await
        else {
            return;
        };
        let hourly_rollups_started_at = Instant::now();
        let hourly_rollups =
            wait_for_startup_hourly_rollup_repair(&state, &cancel, &coordinator).await;
        let Some(hourly_rollups) = hourly_rollups else {
            drop(write_permit);
            drop(pressure_permit);
            drop(rollup_guard);
            if !finish_cancelled_startup_hourly_rollup_bootstrap(
                state.as_ref(),
                &cancel,
                &task_run,
                started_at,
                HOURLY_ROLLUP_REPAIR_CANCELLED_SUMMARY,
                HOURLY_ROLLUP_REPAIR_CANCELLED_LOG,
            )
            .await
            {
                return;
            }
            wait_before_startup_hourly_rollup_retry(&cancel).await;
            continue;
        };
        if let Err(err) = hourly_rollups {
            drop(write_permit);
            drop(pressure_permit);
            drop(rollup_guard);
            finish_failed_startup_hourly_rollup_bootstrap(
                state.as_ref(),
                &cancel,
                &task_run,
                pressure_gate,
                &err,
                started_at,
            )
            .await;
            return;
        }
        log_startup_hourly_rollup_repair(hourly_rollups_started_at);
        let summary_rollups_started_at = Instant::now();
        let summary_rollups =
            wait_for_startup_summary_rollup_repair(&state, &cancel, &coordinator).await;
        drop(rollup_guard);
        let Some(summary_rollups) = summary_rollups else {
            drop(write_permit);
            drop(pressure_permit);
            if !finish_cancelled_startup_hourly_rollup_bootstrap(
                state.as_ref(),
                &cancel,
                &task_run,
                started_at,
                SUMMARY_ROLLUP_REPAIR_CANCELLED_SUMMARY,
                SUMMARY_ROLLUP_REPAIR_CANCELLED_LOG,
            )
            .await
            {
                return;
            }
            wait_before_startup_hourly_rollup_retry(&cancel).await;
            continue;
        };
        if let Err(err) = summary_rollups {
            drop(write_permit);
            drop(pressure_permit);
            finish_failed_startup_hourly_rollup_bootstrap(
                state.as_ref(),
                &cancel,
                &task_run,
                pressure_gate,
                &err,
                started_at,
            )
            .await;
            return;
        }
        drop(write_permit);
        drop(pressure_permit);
        finish_completed_startup_hourly_rollup_bootstrap(
            state.as_ref(),
            &cancel,
            &task_run,
            (
                started_at,
                hourly_rollups_started_at,
                summary_rollups_started_at,
            ),
        )
        .await;
        break;
    }
}

const HOURLY_ROLLUP_REPAIR_CANCELLED_SUMMARY: &str =
    "background hourly rollup bootstrap cancelled during hourly rollup repair";
const HOURLY_ROLLUP_REPAIR_CANCELLED_LOG: &str =
    "background startup hourly rollup bootstrap cancelled during hourly rollup repair";
const SUMMARY_ROLLUP_REPAIR_CANCELLED_SUMMARY: &str =
    "background hourly rollup bootstrap cancelled during summary rollup repair";
const SUMMARY_ROLLUP_REPAIR_CANCELLED_LOG: &str =
    "background startup hourly rollup bootstrap cancelled during summary rollup repair";

fn log_startup_hourly_rollup_repair(started_at: Instant) {
    info!(
        elapsed_ms = started_at.elapsed().as_millis() as u64,
        "background startup hourly rollup bootstrap completed hourly rollup repair"
    );
}

async fn finish_completed_startup_hourly_rollup_bootstrap(
    state: &AppState,
    cancel: &CancellationToken,
    task_run: &SystemTaskRunHandle,
    (started_at, hourly_rollups_started_at, summary_rollups_started_at): (
        Instant,
        Instant,
        Instant,
    ),
) {
    finish_successful_startup_hourly_rollup_bootstrap(
        state,
        cancel,
        task_run,
        started_at,
        hourly_rollups_started_at.elapsed().as_millis() as u64,
        summary_rollups_started_at.elapsed().as_millis() as u64,
    )
    .await;
}

async fn acquire_startup_hourly_rollup_work<'a>(
    state: &'a AppState,
    cancel: &CancellationToken,
    task_run: &SystemTaskRunHandle,
    started_at: Instant,
    pressure_gate: &crate::db_pressure::DbPressureGate,
    coordinator: &Arc<crate::proxy_sqlite_write_coordinator::ProxySqliteWriteCoordinator>,
) -> Option<(
    tokio::sync::MutexGuard<'a, ()>,
    crate::db_pressure::DbBackgroundPermit,
    crate::proxy_sqlite_write_coordinator::ProxySqliteWritePermit,
)> {
    // Task history is released before waiting for this lock; work admission is reacquired only
    // after the lock is held.
    let rollup_guard = tokio::select! {
        biased;
        _ = cancel.cancelled() => {
            info!(
                elapsed_ms = started_at.elapsed().as_millis() as u64,
                "background startup hourly rollup bootstrap cancelled before acquiring its synchronization lock"
            );
            finish_runtime_startup_hourly_rollup_bootstrap_task(
                state,
                cancel,
                Some(task_run),
                SystemTaskStatus::Skipped,
                "background hourly rollup bootstrap cancelled before acquiring its synchronization lock",
                None,
            ).await;
            return None;
        }
        guard = state.hourly_rollup_sync_lock.lock() => guard,
    };
    let Some((pressure_permit, write_permit)) =
        acquire_startup_hourly_rollup_admission(cancel, pressure_gate, coordinator).await
    else {
        drop(rollup_guard);
        finish_runtime_startup_hourly_rollup_bootstrap_task(
            state,
            cancel,
            Some(task_run),
            SystemTaskStatus::Skipped,
            "background hourly rollup bootstrap cancelled before reacquiring SQLite write admission",
            None,
        )
        .await;
        return None;
    };
    Some((rollup_guard, pressure_permit, write_permit))
}

async fn wait_for_startup_hourly_rollup_repair(
    state: &AppState,
    cancel: &CancellationToken,
    coordinator: &Arc<crate::proxy_sqlite_write_coordinator::ProxySqliteWriteCoordinator>,
) -> Option<Result<()>> {
    tokio::select! {
        biased;
        _ = cancel.cancelled() => None,
        _ = coordinator.wait_for_p2_preemption() => None,
        result = bootstrap_hourly_rollups_for_runtime_startup(
            &state.pool,
            Some(state.config.invocation_max_days),
        ) => Some(result),
    }
}

async fn wait_for_startup_summary_rollup_repair(
    state: &AppState,
    cancel: &CancellationToken,
    coordinator: &Arc<crate::proxy_sqlite_write_coordinator::ProxySqliteWriteCoordinator>,
) -> Option<Result<()>> {
    tokio::select! {
        biased;
        _ = cancel.cancelled() => None,
        _ = coordinator.wait_for_p2_preemption() => None,
        result = ensure_invocation_summary_rollups_ready_best_effort(&state.pool) => Some(result),
    }
}

async fn finish_cancelled_startup_hourly_rollup_bootstrap(
    state: &AppState,
    cancel: &CancellationToken,
    task_run: &SystemTaskRunHandle,
    started_at: Instant,
    summary: &'static str,
    log_message: &'static str,
) -> bool {
    info!(
        elapsed_ms = started_at.elapsed().as_millis() as u64,
        "{log_message}"
    );
    finish_runtime_startup_hourly_rollup_bootstrap_task(
        state,
        cancel,
        Some(task_run),
        SystemTaskStatus::Skipped,
        summary,
        None,
    )
    .await;
    !cancel.is_cancelled()
}

async fn wait_before_startup_hourly_rollup_retry(cancel: &CancellationToken) {
    tokio::select! {
        biased;
        _ = cancel.cancelled() => {},
        _ = tokio::time::sleep(Duration::from_secs(
            BACKGROUND_DB_PRESSURE_RETRY_INTERVAL_SECS,
        )) => {}
    }
}

async fn finish_failed_startup_hourly_rollup_bootstrap(
    state: &AppState,
    cancel: &CancellationToken,
    task_run: &SystemTaskRunHandle,
    pressure_gate: &crate::db_pressure::DbPressureGate,
    err: &anyhow::Error,
    started_at: Instant,
) {
    pressure_gate.record_error("startup_hourly_rollup_bootstrap", err);
    finish_runtime_startup_hourly_rollup_bootstrap_task(
        state,
        cancel,
        Some(task_run),
        SystemTaskStatus::Failed,
        "background hourly rollup bootstrap failed; existing rollups remain available",
        Some(err.to_string()),
    )
    .await;
    warn!(
        error = %err,
        elapsed_ms = started_at.elapsed().as_millis() as u64,
        "background startup hourly rollup bootstrap failed; keeping existing rollups"
    );
}

async fn finish_successful_startup_hourly_rollup_bootstrap(
    state: &AppState,
    cancel: &CancellationToken,
    task_run: &SystemTaskRunHandle,
    started_at: Instant,
    hourly_rollups_elapsed_ms: u64,
    summary_rollups_elapsed_ms: u64,
) {
    finish_runtime_startup_hourly_rollup_bootstrap_task(
        state,
        cancel,
        Some(task_run),
        SystemTaskStatus::Success,
        &format!(
            "background hourly rollup bootstrap completed: hourly_rollups_ms={hourly_rollups_elapsed_ms} summary_rollups_ms={summary_rollups_elapsed_ms}"
        ),
        None,
    )
    .await;
    info!(
        elapsed_ms = started_at.elapsed().as_millis() as u64,
        hourly_rollups_elapsed_ms,
        summary_rollups_elapsed_ms,
        "background startup hourly rollup bootstrap completed"
    );
}

async fn record_startup_hourly_rollup_bootstrap_task(
    state: &AppState,
    cancel: &CancellationToken,
    task_start_window: &str,
    pressure_gate: &crate::db_pressure::DbPressureGate,
    coordinator: &Arc<crate::proxy_sqlite_write_coordinator::ProxySqliteWriteCoordinator>,
) -> Option<SystemTaskRunHandle> {
    loop {
        let pressure_permit = match pressure_gate
            .try_begin_background("startup_hourly_rollup_bootstrap_task_history")
        {
            Ok(permit) => permit,
            Err(reason) => {
                let retry_after = match reason {
                    crate::db_pressure::DbPressureDenyReason::PressureCooldown { remaining_ms } => {
                        Duration::from_millis(remaining_ms.max(1))
                    }
                    crate::db_pressure::DbPressureDenyReason::BackgroundBusy => {
                        Duration::from_secs(BACKGROUND_DB_PRESSURE_RETRY_INTERVAL_SECS)
                    }
                };
                info!(
                    defer_reason = %reason,
                    retry_after_ms = retry_after.as_millis() as u64,
                    "background startup hourly rollup bootstrap task history deferred"
                );
                tokio::select! {
                    biased;
                    _ = cancel.cancelled() => return None,
                    _ = tokio::time::sleep(retry_after) => continue,
                }
            }
        };
        let Some(write_permit) = coordinator
            .try_acquire(crate::proxy_sqlite_write_coordinator::ProxySqliteWriteClass::P2Derived)
        else {
            drop(pressure_permit);
            tokio::select! {
                biased;
                _ = cancel.cancelled() => return None,
                _ = tokio::time::sleep(
                    STARTUP_HOURLY_ROLLUP_TASK_HISTORY_COORDINATOR_RETRY_INTERVAL,
                ) => continue,
            }
        };
        let result = tokio::select! {
            biased;
            _ = cancel.cancelled() => {
                drop(write_permit);
                drop(pressure_permit);
                finish_orphaned_startup_hourly_rollup_bootstrap_task(
                    state,
                    cancel,
                    task_start_window,
                ).await;
                return None;
            }
            result = begin_runtime_startup_hourly_rollup_task(
                state,
                SystemTaskKind::HourlyRollupBootstrap,
                "startup",
                Some("background hourly rollup bootstrap started".to_string()),
            ) => result,
        };
        drop(write_permit);
        drop(pressure_permit);
        match result {
            Ok(task_run) => return Some(task_run),
            Err(err) => {
                warn!(
                    error = %err,
                    "failed to record background startup hourly rollup bootstrap start; retrying"
                );
                tokio::select! {
                    biased;
                    _ = cancel.cancelled() => return None,
                    _ = tokio::time::sleep(Duration::from_secs(
                        BACKGROUND_DB_PRESSURE_RETRY_INTERVAL_SECS,
                    )) => {}
                }
            }
        }
    }
}

async fn acquire_startup_hourly_rollup_admission(
    cancel: &CancellationToken,
    pressure_gate: &crate::db_pressure::DbPressureGate,
    coordinator: &Arc<crate::proxy_sqlite_write_coordinator::ProxySqliteWriteCoordinator>,
) -> Option<(
    crate::db_pressure::DbBackgroundPermit,
    crate::proxy_sqlite_write_coordinator::ProxySqliteWritePermit,
)> {
    loop {
        let pressure_permit =
            match pressure_gate.try_begin_background("startup_hourly_rollup_bootstrap") {
                Ok(permit) => permit,
                Err(reason) => {
                    let retry_after = match reason {
                        crate::db_pressure::DbPressureDenyReason::PressureCooldown {
                            remaining_ms,
                        } => Duration::from_millis(remaining_ms.max(1)),
                        crate::db_pressure::DbPressureDenyReason::BackgroundBusy => {
                            Duration::from_secs(BACKGROUND_DB_PRESSURE_RETRY_INTERVAL_SECS)
                        }
                    };
                    tokio::select! {
                        biased;
                        _ = cancel.cancelled() => return None,
                        _ = tokio::time::sleep(retry_after) => continue,
                    }
                }
            };
        let Some(write_permit) = coordinator
            .try_acquire(crate::proxy_sqlite_write_coordinator::ProxySqliteWriteClass::P2Derived)
        else {
            drop(pressure_permit);
            tokio::select! {
                biased;
                _ = cancel.cancelled() => return None,
                _ = tokio::time::sleep(Duration::from_secs(
                    BACKGROUND_DB_PRESSURE_RETRY_INTERVAL_SECS,
                )) => continue,
            }
        };
        return Some((pressure_permit, write_permit));
    }
}

async fn begin_runtime_startup_hourly_rollup_task(
    state: &AppState,
    task_kind: SystemTaskKind,
    trigger_kind: &'static str,
    summary: Option<String>,
) -> Result<SystemTaskRunHandle> {
    #[cfg(test)]
    {
        crate::api::begin_system_task_run(&state.pool, task_kind, trigger_kind, summary).await
    }
    #[cfg(not(test))]
    {
        crate::api::begin_system_task_run_nonblocking(
            &state.config.database_url(),
            task_kind,
            trigger_kind,
            summary,
        )
        .await
    }
}
