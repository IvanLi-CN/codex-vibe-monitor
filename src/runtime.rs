use super::*;
use std::collections::VecDeque;

const STARTUP_HOT_READ_HYDRATION_RETRY_INITIAL: Duration = Duration::from_secs(1);
const STARTUP_HOT_READ_HYDRATION_RETRY_MAX: Duration = Duration::from_secs(30);
const STARTUP_HOURLY_ROLLUP_P2_PREEMPTION_RETRY_MAX: Duration = Duration::from_secs(300);
const MANAGED_TASK_FINISH_RETRY_ATTEMPTS: usize = 4;
const MANAGED_TASK_FINISH_RETRY_INTERVAL: Duration = Duration::from_millis(50);
const MANAGED_TASK_FINISH_ATTEMPT_TIMEOUT: Duration = Duration::from_millis(250);

pub(crate) fn next_startup_hourly_rollup_p2_preemption_retry(current: Duration) -> Duration {
    current
        .checked_mul(2)
        .unwrap_or(STARTUP_HOURLY_ROLLUP_P2_PREEMPTION_RETRY_MAX)
        .min(STARTUP_HOURLY_ROLLUP_P2_PREEMPTION_RETRY_MAX)
}

pub(crate) fn publish_http_readiness_and_spawn_hot_read_hydration(
    state: Arc<AppState>,
    startup_started_at: Instant,
) -> JoinHandle<()> {
    publish_http_readiness_and_spawn_hot_read_hydration_with_options(
        state,
        startup_started_at,
        SUMMARY_PROJECTION_STARTUP_BUILD_DEADLINE,
        None,
    )
}

#[cfg(test)]
pub(crate) fn publish_http_readiness_and_spawn_hot_read_hydration_with_test_summary_deadline(
    state: Arc<AppState>,
    startup_started_at: Instant,
    summary_deadline: Duration,
) -> JoinHandle<()> {
    publish_http_readiness_and_spawn_hot_read_hydration_with_options(
        state,
        startup_started_at,
        summary_deadline,
        None,
    )
}

#[cfg(test)]
pub(crate) fn publish_http_readiness_and_spawn_hot_read_hydration_with_test_summary_delay(
    state: Arc<AppState>,
    startup_started_at: Instant,
    summary_start_delay: Duration,
) -> JoinHandle<()> {
    publish_http_readiness_and_spawn_hot_read_hydration_with_options(
        state,
        startup_started_at,
        SUMMARY_PROJECTION_STARTUP_BUILD_DEADLINE,
        Some(summary_start_delay),
    )
}

fn publish_http_readiness_and_spawn_hot_read_hydration_with_options(
    state: Arc<AppState>,
    startup_started_at: Instant,
    summary_deadline: Duration,
    summary_start_delay: Option<Duration>,
) -> JoinHandle<()> {
    state.startup_ready.store(true, Ordering::Release);
    info!(
        time_to_health_ms = startup_started_at.elapsed().as_millis() as u64,
        "application readiness reached"
    );

    tokio::spawn(async move {
        // Spawn the independent system-status worker before taking the Summary priority
        // reservation. A busy database gate must never delay the readiness-side status cache.
        let system_status_handle = tokio::spawn(hydrate_system_status_at_startup(state.clone()));
        let summary_startup_priority =
            crate::db_pressure::global_db_pressure_gate().reserve_priority_background();
        let summary_state = state.clone();
        let summary_handle = tokio::spawn(async move {
            let startup_priority = hydrate_summary_at_startup(
                summary_state.clone(),
                summary_deadline,
                summary_start_delay,
                summary_startup_priority,
            )
            .await;
            if let Some(startup_priority) = startup_priority {
                spawn_summary_snapshot_maintenance(summary_state.clone());
                spawn_summary_coverage_recovery_maintenance(summary_state, startup_priority);
                true
            } else {
                false
            }
        });
        let (summary_result, system_status_result) =
            tokio::join!(summary_handle, system_status_handle);

        match summary_result {
            Ok(true) | Ok(false) => {}
            Err(error) => {
                warn!(
                    ?error,
                    "summary projection startup hydration worker ended unexpectedly"
                );
            }
        }
        if let Err(error) = system_status_result {
            warn!(
                ?error,
                "system status startup hydration worker ended unexpectedly"
            );
        }
    })
}

async fn hydrate_summary_at_startup(
    state: Arc<AppState>,
    summary_deadline: Duration,
    summary_start_delay: Option<Duration>,
    mut startup_priority: crate::db_pressure::DbBackgroundPriorityReservation,
) -> Option<crate::db_pressure::DbBackgroundPriorityReservation> {
    if let Some(delay) = summary_start_delay {
        tokio::select! {
            _ = state.shutdown.cancelled() => return None,
            _ = tokio::time::sleep(delay) => {}
        }
    }

    let mut retry_delay = STARTUP_HOT_READ_HYDRATION_RETRY_INITIAL;
    loop {
        tokio::select! {
            _ = state.shutdown.cancelled() => return None,
            result = hydrate_summary_snapshots_with_deadline(state.as_ref(), summary_deadline) => match result {
                Ok(()) => {
                    info!("summary projection startup hydration completed");
                    return Some(startup_priority);
                }
                Err(error) => {
                    // A failed Bootstrap must not reserve the only background slot across its
                    // exponential retry delay. The next cold attempt receives a fresh bounded
                    // reservation when it begins.
                    drop(startup_priority);
                    warn!(?error, "summary projection startup hydration failed; retaining unavailable or last-good response");
                }
            }
        }

        warn!(
            retry_after_secs = retry_delay.as_secs(),
            "summary projection startup hydration deferred behind bounded pressure backoff"
        );
        tokio::select! {
            _ = state.shutdown.cancelled() => return None,
            _ = tokio::time::sleep(retry_delay) => {}
        }
        retry_delay = retry_delay
            .saturating_mul(2)
            .min(STARTUP_HOT_READ_HYDRATION_RETRY_MAX);
        startup_priority =
            crate::db_pressure::global_db_pressure_gate().reserve_priority_background();
    }
}

async fn hydrate_system_status_at_startup(state: Arc<AppState>) {
    let mut retry_delay = STARTUP_HOT_READ_HYDRATION_RETRY_INITIAL;
    loop {
        tokio::select! {
            _ = state.shutdown.cancelled() => return,
            result = hydrate_system_status_snapshot(state.as_ref()) => match result {
                Ok(()) => {
                    info!("system status startup hydration completed");
                    return;
                }
                Err(error) => {
                    warn!(?error, "system status startup hydration failed; retaining unavailable or last-good response");
                }
            }
        }

        warn!(
            retry_after_secs = retry_delay.as_secs(),
            "system status startup hydration deferred behind bounded pressure backoff"
        );
        tokio::select! {
            _ = state.shutdown.cancelled() => return,
            _ = tokio::time::sleep(retry_delay) => {}
        }
        retry_delay = retry_delay
            .saturating_mul(2)
            .min(STARTUP_HOT_READ_HYDRATION_RETRY_MAX);
    }
}

pub(crate) async fn run() -> Result<()> {
    dotenv().ok();
    dotenvy::from_filename(".env.local").ok();
    RuntimeProjectionMode::reject_removed_legacy_env()?;
    init_tracing();
    let startup_started_at = Instant::now();

    let cli = CliArgs::parse();
    let config = AppConfig::from_sources(&cli)?;
    let spawn_background_hourly_rollup_bootstrap =
        should_spawn_background_startup_hourly_rollup_bootstrap(&cli);
    let (backend_ver, frontend_ver) = detect_versions(config.static_dir.as_deref());
    info!(?config, backend_version = %backend_ver, frontend_version = %frontend_ver, "starting codex vibe monitor");

    let database_url = config.database_url();
    ensure_db_directory(&config.database_path)?;
    let connect_opts = build_sqlite_connect_options(
        &database_url,
        Duration::from_secs(DEFAULT_SQLITE_BUSY_TIMEOUT_SECS),
    )?;
    let db_connect_started_at = Instant::now();
    let pool = SqlitePoolOptions::new()
        .max_connections(5)
        .connect_with(connect_opts)
        .await
        .context("failed to open sqlite database")?;
    log_startup_phase("db_connect", db_connect_started_at);

    let schema_started_at = Instant::now();
    ensure_schema(&pool).await?;
    let _maintenance_store = match crate::maintenance_store::open(&config).await {
        Ok(store) => {
            if let Err(error) = store.migrate_legacy_state(&pool).await {
                warn!(error = %error, "legacy task state migration did not complete; keeping maintenance observation unavailable until the next startup retry");
            } else if let Err(error) = store.apply_initial_task_defaults().await {
                warn!(error = %error, "initial managed task defaults could not be applied; keeping maintenance observation unavailable until the next startup retry");
            } else {
                match store.recover_incomplete_runs().await {
                    Ok(recovered_runs) => {
                        if recovered_runs > 0 {
                            warn!(
                                recovered_runs,
                                "recovered incomplete managed task runs at startup"
                            );
                        }
                        crate::maintenance_store::set_global(Arc::new(store.clone()));
                    }
                    Err(error) => {
                        warn!(error = %error, "incomplete managed task runs could not be recovered; keeping maintenance observation unavailable until the next startup retry");
                    }
                }
            }
            Some(Arc::new(store))
        }
        Err(error) => {
            warn!(error = %error, path = %config.maintenance_database_path().display(), "maintenance database unavailable; operational observation will be stale");
            None
        }
    };
    log_startup_phase("schema", schema_started_at);
    if should_recover_pending_pool_attempts_on_startup(&cli) {
        let recovered_running_invocations = recover_orphaned_proxy_invocations(&pool).await?;
        if recovered_running_invocations > 0 {
            warn!(
                recovered_running_invocations,
                "recovered orphaned running invocation rows at startup"
            );
        }
        let recovered_pending_pool_attempts =
            recover_orphaned_pool_upstream_request_attempts(&pool).await?;
        if recovered_pending_pool_attempts > 0 {
            warn!(
                recovered_pending_pool_attempts,
                "recovered orphaned pending pool attempt rows at startup"
            );
        }
    }
    if should_run_blocking_startup_persistent_prep(&cli) {
        let prep_summary = run_startup_persistent_prep(&pool, &config, &cli).await?;
        info!(
            stale_archive_temp_files_removed = prep_summary.stale_archive_temp_files_removed,
            refreshed_manifest_batches = prep_summary.refreshed_manifest_batches,
            refreshed_manifest_account_rows = prep_summary.refreshed_manifest_account_rows,
            missing_manifest_files = prep_summary.missing_manifest_files,
            backfilled_archive_expiries = prep_summary.backfilled_archive_expiries,
            bootstrapped_hourly_rollups = prep_summary.bootstrapped_hourly_rollups,
            pending_manifest_batches = prep_summary.pending_manifest_batches,
            pending_historical_rollup_archive_batches =
                prep_summary.pending_historical_rollup_archive_batches,
            "startup persistent prep finished"
        );
        if prep_summary.pending_historical_rollup_archive_batches > 0 {
            warn!(
                pending_historical_rollup_archive_batches =
                    prep_summary.pending_historical_rollup_archive_batches,
                "legacy archive batches still need historical rollup materialization"
            );
        }
    }
    if cli.retention_run_once && cli.command.is_some() {
        bail!("--retention-run-once cannot be combined with maintenance subcommands");
    }
    if let Some(command) = &cli.command {
        run_cli_command(&pool, &config, command).await?;
        return Ok(());
    }
    if cli.retention_run_once {
        let summary =
            run_data_retention_maintenance(&pool, &config, Some(cli.retention_dry_run), None)
                .await?;
        info!(?summary, "retention maintenance run-once finished");
        return Ok(());
    }

    let pricing_catalog = load_pricing_catalog(&pool).await?;
    ensure_proxy_encrypted_session_owner_routing_setting_initialized(&pool, &config).await?;
    ensure_proxy_websocket_settings_initialized(&pool, &config).await?;
    let proxy_model_settings = Arc::new(RwLock::new(load_proxy_model_settings(&pool).await?));
    let forward_proxy_settings = load_forward_proxy_settings(&pool).await?;
    let forward_proxy_runtime = load_forward_proxy_runtime_states(&pool).await?;
    let oauth_installation_seed = oauth_bridge::load_or_init_oauth_installation_seed(&pool).await?;
    let forward_proxy = Arc::new(Mutex::new(ForwardProxyManager::with_algo(
        forward_proxy_settings,
        forward_proxy_runtime,
        config.forward_proxy_algo,
    )));
    let resolved_proxy_raw_dir = config.resolved_proxy_raw_dir();
    fs::create_dir_all(&resolved_proxy_raw_dir).with_context(|| {
        format!(
            "failed to create proxy raw payload directory: {}",
            resolved_proxy_raw_dir.display()
        )
    })?;
    let pricing_catalog = Arc::new(RwLock::new(pricing_catalog));

    let http_clients = HttpClients::build(&config)?;
    let upstream_accounts = Arc::new(UpstreamAccountsRuntime::from_env()?);
    let (tx, _rx) = broadcast::channel(128);
    let semaphore = Arc::new(Semaphore::new(config.max_parallel_polls));
    let proxy_raw_async_semaphore = Arc::new(Semaphore::new(proxy_raw_async_writer_limit(&config)));
    let shutdown = CancellationToken::new();
    let process_started_at_utc = Utc::now();
    let performance_telemetry =
        PerformanceTelemetryRuntime::start(&config, process_started_at_utc, shutdown.clone());

    let prompt_cache_conversation_cache =
        Arc::new(Mutex::new(PromptCacheConversationsCacheState::default()));
    let proxy_runtime_invocations =
        Arc::new(RuntimeProjectionHub::new(RuntimeProjectionMode::Auto));
    let dashboard_network_speed_cache =
        Arc::new(DashboardNetworkSpeedCache::new(process_started_at_utc));
    proxy_runtime_invocations
        .bind_dashboard_network_speed_cache(dashboard_network_speed_cache.clone())?;
    let dashboard_activity_snapshot_cache =
        Arc::new(Mutex::new(DashboardActivitySnapshotCacheState::default()));
    let terminal_projection_hub = Arc::new(TerminalProjectionHub::default());
    let long_term_projection_runtime = Arc::new(Mutex::new(LongTermProjectionRuntime::default()));
    let memory_diagnostics = Arc::new(MemoryDiagnosticsRuntime::new());
    let sqlite_batch_writer = SqliteBatchWriter::spawn(
        pool.clone(),
        shutdown.clone(),
        prompt_cache_conversation_cache.clone(),
        pricing_catalog.clone(),
        &config.database_path,
    );
    sqlite_batch_writer.set_terminal_runtime_store(proxy_runtime_invocations.clone());
    sqlite_batch_writer
        .set_dashboard_activity_snapshot_cache(dashboard_activity_snapshot_cache.clone());
    sqlite_batch_writer.set_terminal_projection_hub(terminal_projection_hub.clone());
    let pool_account_selection_runtime = Arc::new(PoolAccountSelectionRuntime::default());
    let subscription_hub = Arc::new(SubscriptionHub::new());
    sqlite_batch_writer.set_summary_delta_hub(subscription_hub.clone());
    terminal_projection_hub.set_runtime_mutation_bus(subscription_hub.runtime_mutation_bus());

    let state = Arc::new(AppState {
        config: config.clone(),
        pool,
        process_started_at_utc,
        performance_telemetry,
        sqlite_batch_writer,
        pool_account_selection_runtime,
        proxy_runtime_invocations,
        dashboard_network_speed_cache,
        oauth_installation_seed,
        hourly_rollup_sync_lock: Arc::new(Mutex::new(())),
        http_clients,
        broadcaster: tx.clone(),
        broadcast_state_cache: Arc::new(Mutex::new(BroadcastStateCache::default())),
        subscription_hub: subscription_hub.clone(),
        proxy_summary_quota_broadcast_seq: Arc::new(AtomicU64::new(0)),
        proxy_summary_quota_broadcast_running: Arc::new(AtomicBool::new(false)),
        proxy_summary_quota_broadcast_handle: Arc::new(Mutex::new(Vec::new())),
        dashboard_activity_live_broadcast_seq: Arc::new(AtomicU64::new(0)),
        dashboard_activity_live_broadcast_running: Arc::new(AtomicBool::new(false)),
        startup_ready: Arc::new(AtomicBool::new(false)),
        shutdown: shutdown.clone(),
        semaphore: semaphore.clone(),
        proxy_request_in_flight: Arc::new(AtomicUsize::new(0)),
        proxy_raw_async_semaphore,
        raw_capture_circuit: Arc::new(RawCaptureCircuitBreaker::new(
            config.resolved_proxy_raw_dir(),
        )),
        proxy_model_settings,
        proxy_model_settings_update_lock: Arc::new(Mutex::new(())),
        forward_proxy,
        xray_supervisor: Arc::new(Mutex::new(XraySupervisor::new(
            config.xray_binary.clone(),
            config.xray_runtime_dir.clone(),
        ))),
        forward_proxy_settings_update_lock: Arc::new(Mutex::new(())),
        forward_proxy_subscription_refresh_lock: Arc::new(Mutex::new(())),
        pricing_settings_update_lock: Arc::new(Mutex::new(())),
        pricing_catalog,
        prompt_cache_conversation_cache,
        dashboard_activity_snapshot_cache,
        terminal_projection_hub,
        long_term_projection_runtime,
        memory_diagnostics,
        maintenance_stats_cache: Arc::new(Mutex::new(StatsMaintenanceCacheState::default())),
        system_status_cache: Arc::new(Mutex::new(SystemStatusCacheState::default())),
        pool_routing_reservations: Arc::new(std::sync::Mutex::new(HashMap::new())),
        pool_routing_availability: PoolRoutingAvailabilitySignal::default(),
        pool_routing_runtime_cache: Arc::new(Mutex::new(None)),
        #[cfg(test)]
        pool_routing_test_data_version_connection: Arc::new(Mutex::new(None)),
        pool_model_routing_cache_write_lock: Arc::new(Mutex::new(())),
        pool_live_attempt_ids: Arc::new(std::sync::Mutex::new(HashSet::new())),
        pool_group_429_retry_delay_override: None,
        #[cfg(test)]
        fallback_proxy_429_retry_delay_override: None,
        pool_no_available_wait: PoolNoAvailableWaitSettings::default(),
        upstream_accounts,
    });
    // Listen for shutdown before the readiness-gated hydration loop so an unavailable
    // persistent baseline can be interrupted cleanly without publishing partial HTTP state.
    let signal_listener = spawn_shutdown_signal_listener(state.shutdown.clone());
    // Durable startup warm-ups may wait behind SQLite recovery or an overloaded pool. Complete
    // the early routing and dashboard reads before progressing through the remaining startup
    // stages. Summary/System Status hydration itself is intentionally deferred to the final
    // HTTP-readiness boundary below, so its 15s/60s service clocks cannot expire beforehand.
    warm_pool_routing_runtime_cache_best_effort(state.as_ref()).await;
    warm_dashboard_runtime_projection(state.as_ref()).await;
    if let Err(error) = hydrate_raw_capture_circuit(state.as_ref()).await {
        warn!(error = %error, "raw capture circuit hydration failed; keeping capture fail-closed");
    }
    recover_raw_overflow_spools_with_circuit(state.as_ref()).await;
    spawn_dashboard_runtime_projection_reconcile(state.clone());
    spawn_subscription_broadcast_listener(state.clone());
    spawn_system_raw_payload_metrics_inventory(state.clone(), state.shutdown.clone());
    spawn_memory_diagnostics(state.clone(), state.shutdown.clone());
    spawn_performance_telemetry_sampler(state.clone());
    warm_pool_routing_runtime_cache_best_effort(state.as_ref()).await;

    run_runtime_until_shutdown(
        state,
        startup_started_at,
        spawn_background_hourly_rollup_bootstrap,
        async move {
            let _ = signal_listener.await;
        },
    )
    .await
}

pub(crate) const POOL_EARLY_PHASE_ORPHAN_RECOVERY_INTERVAL: Duration = Duration::from_secs(60);

pub(crate) async fn warm_pool_routing_runtime_cache_best_effort(state: &AppState) {
    refresh_pool_routing_runtime_cache_best_effort(state, "startup warmup").await;
}

pub(crate) async fn refresh_pool_routing_runtime_cache_best_effort(
    state: &AppState,
    operation: &'static str,
) {
    if let Err(err) = refresh_pool_routing_runtime_cache(state).await {
        warn!(
            error = %err,
            operation,
            "failed to refresh pool routing runtime cache; falling back to lazy pool routing resolution"
        );
    }
}

pub(crate) fn begin_runtime_shutdown_if_requested<F>(
    shutdown_signal: &Shared<F>,
    cancel: &CancellationToken,
) -> bool
where
    F: Future<Output = ()>,
{
    if cancel.is_cancelled() {
        return true;
    }
    if shutdown_signal.clone().now_or_never().is_some() {
        begin_runtime_shutdown(cancel);
        return true;
    }
    false
}

pub(crate) enum StartupStageOutcome<T> {
    SkippedByShutdown,
    Completed { result: T, shutdown_requested: bool },
}

pub(crate) struct TrackedStartupStage<Stage> {
    stage: std::pin::Pin<Box<Stage>>,
    started: bool,
}

impl<Stage> TrackedStartupStage<Stage> {
    fn new(stage: Stage) -> Self {
        Self {
            stage: Box::pin(stage),
            started: false,
        }
    }

    fn has_started(&self) -> bool {
        self.started
    }
}

impl<Stage> TrackedStartupStage<Stage>
where
    Stage: Future,
{
    async fn finish(&mut self) -> Stage::Output {
        self.started = true;
        self.stage.as_mut().await
    }
}

impl<Stage> Future for TrackedStartupStage<Stage>
where
    Stage: Future,
{
    type Output = Stage::Output;

    fn poll(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        let this = self.as_mut().get_mut();
        this.started = true;
        this.stage.as_mut().poll(cx)
    }
}

pub(crate) async fn run_startup_stage_until_shutdown<T, Stage, Shutdown>(
    shutdown_signal: &Shared<Shutdown>,
    cancel: &CancellationToken,
    stage: Stage,
) -> StartupStageOutcome<T>
where
    Stage: Future<Output = T>,
    Shutdown: Future<Output = ()>,
{
    if begin_runtime_shutdown_if_requested(shutdown_signal, cancel) {
        return StartupStageOutcome::SkippedByShutdown;
    }

    let stage = TrackedStartupStage::new(stage);
    tokio::pin!(stage);
    tokio::select! {
        biased;
        _ = shutdown_signal.clone() => {
            begin_runtime_shutdown(cancel);
            if stage.as_ref().get_ref().has_started() {
                StartupStageOutcome::Completed {
                    result: stage.as_mut().get_mut().finish().await,
                    shutdown_requested: true,
                }
            } else {
                StartupStageOutcome::SkippedByShutdown
            }
        }
        result = &mut stage => StartupStageOutcome::Completed {
            shutdown_requested: begin_runtime_shutdown_if_requested(shutdown_signal, cancel),
            result,
        },
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn drain_runtime_after_pending_shutdown(
    state: Arc<AppState>,
    mut shutdown_watcher: JoinHandle<()>,
    server_handle: Option<JoinHandle<()>>,
    poller_handle: Option<JoinHandle<()>>,
    upstream_accounts_handle: Option<JoinHandle<()>>,
    forward_proxy_handle: Option<JoinHandle<()>>,
    pool_orphan_recovery_handle: Option<JoinHandle<()>>,
    retention_handle: Option<JoinHandle<()>>,
    startup_backfill_handle: Option<JoinHandle<()>>,
    startup_hot_read_hydration_handle: Option<JoinHandle<()>>,
) -> Result<()> {
    let shutdown_cancel = state.shutdown.clone();
    tokio::select! {
        _ = shutdown_cancel.cancelled() => {
            shutdown_watcher.abort();
            let _ = shutdown_watcher.await;
        }
        _ = &mut shutdown_watcher => {}
    }
    drain_runtime_after_shutdown(
        state,
        server_handle,
        poller_handle,
        upstream_accounts_handle,
        forward_proxy_handle,
        pool_orphan_recovery_handle,
        retention_handle,
        startup_backfill_handle,
        startup_hot_read_hydration_handle,
    )
    .await
}

async fn flush_terminal_journal_replay_before_startup(state: &AppState) -> Result<()> {
    let mut remaining = state
        .sqlite_batch_writer
        .terminal_journal_stats()
        .replay_count;
    while remaining > 0 {
        info!(
            replay_count = remaining,
            "flushing terminal journal replay before retention startup"
        );
        state
            .sqlite_batch_writer
            .flush_now(&state.pool)
            .await
            .context("failed to flush terminal journal replay before retention startup")?;
        let next_remaining = state
            .sqlite_batch_writer
            .terminal_journal_stats()
            .replay_count;
        if next_remaining >= remaining {
            bail!("terminal journal replay made no progress: remaining={remaining}");
        }
        remaining = next_remaining;
    }
    Ok(())
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
    let poller_handle = None;
    let mut upstream_accounts_handle = None;
    let mut forward_proxy_handle = None;
    let mut pool_orphan_recovery_handle = None;
    let mut retention_handle = None;
    let mut server_handle = None;
    let mut startup_backfill_handle = None;
    let mut startup_hot_read_hydration_handle = None;

    let sync_stage = run_startup_stage_until_shutdown(
        &shutdown_signal,
        &cancel,
        sync_forward_proxy_routes(state.as_ref()),
    )
    .await;
    let sync_shutdown_requested = match sync_stage {
        StartupStageOutcome::SkippedByShutdown => {
            return drain_runtime_after_pending_shutdown(
                state,
                shutdown_watcher,
                server_handle,
                poller_handle,
                upstream_accounts_handle,
                forward_proxy_handle,
                pool_orphan_recovery_handle,
                retention_handle,
                startup_backfill_handle,
                startup_hot_read_hydration_handle,
            )
            .await;
        }
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
    if sync_shutdown_requested {
        return drain_runtime_after_pending_shutdown(
            state,
            shutdown_watcher,
            server_handle,
            poller_handle,
            upstream_accounts_handle,
            forward_proxy_handle,
            pool_orphan_recovery_handle,
            retention_handle,
            startup_backfill_handle,
            startup_hot_read_hydration_handle,
        )
        .await;
    }

    let upstream_accounts_stage =
        run_startup_stage_until_shutdown(&shutdown_signal, &cancel, async {
            Some(spawn_upstream_account_maintenance(
                state.clone(),
                cancel.clone(),
            ))
        })
        .await;
    let upstream_accounts_shutdown_requested = match upstream_accounts_stage {
        StartupStageOutcome::SkippedByShutdown => {
            return drain_runtime_after_pending_shutdown(
                state,
                shutdown_watcher,
                server_handle,
                poller_handle,
                upstream_accounts_handle,
                forward_proxy_handle,
                pool_orphan_recovery_handle,
                retention_handle,
                startup_backfill_handle,
                startup_hot_read_hydration_handle,
            )
            .await;
        }
        StartupStageOutcome::Completed {
            result,
            shutdown_requested,
        } => {
            upstream_accounts_handle = result;
            shutdown_requested
        }
    };
    if upstream_accounts_shutdown_requested {
        return drain_runtime_after_pending_shutdown(
            state,
            shutdown_watcher,
            server_handle,
            poller_handle,
            upstream_accounts_handle,
            forward_proxy_handle,
            pool_orphan_recovery_handle,
            retention_handle,
            startup_backfill_handle,
            startup_hot_read_hydration_handle,
        )
        .await;
    }

    let forward_proxy_stage = run_startup_stage_until_shutdown(&shutdown_signal, &cancel, async {
        Some(spawn_forward_proxy_maintenance(
            state.clone(),
            cancel.clone(),
        ))
    })
    .await;
    let forward_proxy_shutdown_requested = match forward_proxy_stage {
        StartupStageOutcome::SkippedByShutdown => {
            return drain_runtime_after_pending_shutdown(
                state,
                shutdown_watcher,
                server_handle,
                poller_handle,
                upstream_accounts_handle,
                forward_proxy_handle,
                pool_orphan_recovery_handle,
                retention_handle,
                startup_backfill_handle,
                startup_hot_read_hydration_handle,
            )
            .await;
        }
        StartupStageOutcome::Completed {
            result,
            shutdown_requested,
        } => {
            forward_proxy_handle = result;
            shutdown_requested
        }
    };
    if forward_proxy_shutdown_requested {
        return drain_runtime_after_pending_shutdown(
            state,
            shutdown_watcher,
            server_handle,
            poller_handle,
            upstream_accounts_handle,
            forward_proxy_handle,
            pool_orphan_recovery_handle,
            retention_handle,
            startup_backfill_handle,
            startup_hot_read_hydration_handle,
        )
        .await;
    }

    let replay_state = state.clone();
    let replay_flush_result = tokio::select! {
        biased;
        _ = shutdown_signal.clone() => {
            begin_runtime_shutdown(&cancel);
            return drain_runtime_after_pending_shutdown(
                state,
                shutdown_watcher,
                server_handle,
                poller_handle,
                upstream_accounts_handle,
                forward_proxy_handle,
                pool_orphan_recovery_handle,
                retention_handle,
                startup_backfill_handle,
                startup_hot_read_hydration_handle,
            )
            .await;
        }
        _ = cancel.cancelled() => {
            return drain_runtime_after_pending_shutdown(
                state,
                shutdown_watcher,
                server_handle,
                poller_handle,
                upstream_accounts_handle,
                forward_proxy_handle,
                pool_orphan_recovery_handle,
                retention_handle,
                startup_backfill_handle,
                startup_hot_read_hydration_handle,
            )
            .await;
        }
        result = flush_terminal_journal_replay_before_startup(replay_state.as_ref()) => result,
    };
    if let Err(err) = replay_flush_result {
        if cancel.is_cancelled() {
            return drain_runtime_after_pending_shutdown(
                state,
                shutdown_watcher,
                server_handle,
                poller_handle,
                upstream_accounts_handle,
                forward_proxy_handle,
                pool_orphan_recovery_handle,
                retention_handle,
                startup_backfill_handle,
                startup_hot_read_hydration_handle,
            )
            .await;
        }
        return Err(err);
    }

    let retention_stage = run_startup_stage_until_shutdown(&shutdown_signal, &cancel, async {
        Some(spawn_data_retention_maintenance(
            state.clone(),
            cancel.clone(),
        ))
    })
    .await;
    let retention_shutdown_requested = match retention_stage {
        StartupStageOutcome::SkippedByShutdown => {
            return drain_runtime_after_pending_shutdown(
                state,
                shutdown_watcher,
                server_handle,
                poller_handle,
                upstream_accounts_handle,
                forward_proxy_handle,
                pool_orphan_recovery_handle,
                retention_handle,
                startup_backfill_handle,
                startup_hot_read_hydration_handle,
            )
            .await;
        }
        StartupStageOutcome::Completed {
            result,
            shutdown_requested,
        } => {
            retention_handle = result;
            shutdown_requested
        }
    };
    if retention_shutdown_requested {
        return drain_runtime_after_pending_shutdown(
            state,
            shutdown_watcher,
            server_handle,
            poller_handle,
            upstream_accounts_handle,
            forward_proxy_handle,
            pool_orphan_recovery_handle,
            retention_handle,
            startup_backfill_handle,
            startup_hot_read_hydration_handle,
        )
        .await;
    }

    let pool_orphan_recovery_stage =
        run_startup_stage_until_shutdown(&shutdown_signal, &cancel, async {
            Some(spawn_pool_orphan_recovery_maintenance(
                state.clone(),
                cancel.clone(),
            ))
        })
        .await;
    let pool_orphan_recovery_shutdown_requested = match pool_orphan_recovery_stage {
        StartupStageOutcome::SkippedByShutdown => {
            return drain_runtime_after_pending_shutdown(
                state,
                shutdown_watcher,
                server_handle,
                poller_handle,
                upstream_accounts_handle,
                forward_proxy_handle,
                pool_orphan_recovery_handle,
                retention_handle,
                startup_backfill_handle,
                startup_hot_read_hydration_handle,
            )
            .await;
        }
        StartupStageOutcome::Completed {
            result,
            shutdown_requested,
        } => {
            pool_orphan_recovery_handle = result;
            shutdown_requested
        }
    };
    if pool_orphan_recovery_shutdown_requested {
        return drain_runtime_after_pending_shutdown(
            state,
            shutdown_watcher,
            server_handle,
            poller_handle,
            upstream_accounts_handle,
            forward_proxy_handle,
            pool_orphan_recovery_handle,
            retention_handle,
            startup_backfill_handle,
            startup_hot_read_hydration_handle,
        )
        .await;
    }

    let http_ready_started_at = Instant::now();
    let http_stage = run_startup_stage_until_shutdown(
        &shutdown_signal,
        &cancel,
        spawn_http_server(state.clone()),
    )
    .await;
    let http_shutdown_requested = match http_stage {
        StartupStageOutcome::SkippedByShutdown => {
            return drain_runtime_after_pending_shutdown(
                state,
                shutdown_watcher,
                server_handle,
                poller_handle,
                upstream_accounts_handle,
                forward_proxy_handle,
                pool_orphan_recovery_handle,
                retention_handle,
                startup_backfill_handle,
                startup_hot_read_hydration_handle,
            )
            .await;
        }
        StartupStageOutcome::Completed {
            result,
            shutdown_requested,
        } => {
            let (_http_addr, handle) = result?;
            server_handle = Some(handle);
            shutdown_requested
        }
    };
    if http_shutdown_requested {
        return drain_runtime_after_pending_shutdown(
            state,
            shutdown_watcher,
            server_handle,
            poller_handle,
            upstream_accounts_handle,
            forward_proxy_handle,
            pool_orphan_recovery_handle,
            retention_handle,
            startup_backfill_handle,
            startup_hot_read_hydration_handle,
        )
        .await;
    }

    startup_hot_read_hydration_handle = Some(publish_http_readiness_and_spawn_hot_read_hydration(
        state.clone(),
        startup_started_at,
    ));
    log_startup_phase("http_ready", http_ready_started_at);
    spawn_system_status_snapshot_maintenance(state.clone());
    spawn_invocation_timeline_snapshot_maintenance(state.clone());

    let startup_backfill_stage =
        run_startup_stage_until_shutdown(&shutdown_signal, &cancel, async {
            Some(spawn_startup_backfill_maintenance(
                state.clone(),
                cancel.clone(),
            ))
        })
        .await;
    let startup_backfill_shutdown_requested = match startup_backfill_stage {
        StartupStageOutcome::SkippedByShutdown => {
            return drain_runtime_after_pending_shutdown(
                state,
                shutdown_watcher,
                server_handle,
                poller_handle,
                upstream_accounts_handle,
                forward_proxy_handle,
                pool_orphan_recovery_handle,
                retention_handle,
                startup_backfill_handle,
                startup_hot_read_hydration_handle,
            )
            .await;
        }
        StartupStageOutcome::Completed {
            result,
            shutdown_requested,
        } => {
            startup_backfill_handle = result;
            shutdown_requested
        }
    };
    if startup_backfill_shutdown_requested {
        return drain_runtime_after_pending_shutdown(
            state,
            shutdown_watcher,
            server_handle,
            poller_handle,
            upstream_accounts_handle,
            forward_proxy_handle,
            pool_orphan_recovery_handle,
            retention_handle,
            startup_backfill_handle,
            startup_hot_read_hydration_handle,
        )
        .await;
    }

    let managed_task_dispatcher_handle = spawn_managed_task_dispatcher(state.clone());

    let startup_hourly_rollup_bootstrap_handle = spawn_background_hourly_rollup_bootstrap
        .then(|| spawn_runtime_startup_hourly_rollup_bootstrap(state.clone(), cancel.clone()));

    tokio::select! {
        biased;
        _ = shutdown_signal => begin_runtime_shutdown(&cancel),
        _ = cancel.cancelled() => {}
    }

    if let Some(startup_hourly_rollup_bootstrap_handle) = startup_hourly_rollup_bootstrap_handle
        && let Err(err) = startup_hourly_rollup_bootstrap_handle.await
    {
        error!(
            ?err,
            "background startup hourly rollup bootstrap task terminated unexpectedly"
        );
    }

    let runtime_result = drain_runtime_after_pending_shutdown(
        state,
        shutdown_watcher,
        server_handle,
        poller_handle,
        upstream_accounts_handle,
        forward_proxy_handle,
        pool_orphan_recovery_handle,
        retention_handle,
        startup_backfill_handle,
        startup_hot_read_hydration_handle,
    )
    .await;
    if let Err(error) = managed_task_dispatcher_handle.await {
        warn!(error = %error, "managed task dispatcher task terminated during shutdown");
    }
    runtime_result
}

fn spawn_managed_task_dispatcher(state: Arc<AppState>) -> JoinHandle<()> {
    tokio::spawn(async move {
        let Some(store) = crate::maintenance_store::global().cloned() else {
            return;
        };
        let mut pending_finishes = VecDeque::new();
        loop {
            tokio::select! {
                _ = state.shutdown.cancelled() => {
                    drain_managed_task_finishes_on_shutdown(
                        state.as_ref(),
                        &store,
                        &mut pending_finishes,
                    )
                    .await;
                    return;
                },
                _ = tokio::time::sleep(Duration::from_millis(250)) => {}
            }
            if let Some(finish) = pending_finishes.pop_front()
                && let Err(error) = finish_managed_task_run_bounded(&store, &finish).await
            {
                warn!(
                    run_id = finish.run_id,
                    task = %finish.task_key,
                    error = %error,
                    "managed task dispatcher deferred task-history finalization"
                );
                pending_finishes.push_back(finish);
            }
            if let Err(error) = store.enqueue_due_runs().await {
                warn!(error = %error, "managed task dispatcher failed to enqueue scheduled runs");
            }
            if let Err(error) = store.cleanup_expired_history_if_due().await {
                warn!(error = %error, "managed task dispatcher failed to clean expired history");
            }
            let claim = match store.claim_requested_run().await {
                Ok(claim) => claim,
                Err(error) => {
                    warn!(error = %error, "managed task dispatcher failed to claim a requested run");
                    continue;
                }
            };
            let Some((run_id, task_key, _started_at, trigger_kind)) = claim else {
                continue;
            };
            let Some(_execution_lease) =
                crate::maintenance_store::try_acquire_task_execution(&task_key)
            else {
                pending_finishes.push_back(ManagedTaskFinish {
                    run_id,
                    task_key,
                    status: SystemTaskStatus::Failed,
                    finished_at: format_utc_iso_millis(Utc::now()),
                    duration_ms: 0,
                    summary: Some("检测到同一任务正在运行，未重复执行".to_string()),
                    detail: None,
                });
                continue;
            };
            let (result, duration_ms) = {
                let observation = (!task_key.eq("prompt_cache_materialization")
                    && !task_key.eq("timeseries_minute_projection")
                    && !task_key.eq("startup_backfill")
                    && !task_key.starts_with("startup_backfill."))
                .then(|| {
                    crate::TaskExecutionObservation::begin(
                        &task_key,
                        &crate::maintenance_store::task_title_for_observation(&task_key),
                        &trigger_kind,
                        crate::maintenance_store::task_execution_class(&task_key),
                        "processing",
                    )
                });
                let started_at = Instant::now();
                let result = run_managed_task_once(&state, &task_key).await;
                let duration_ms = started_at.elapsed().as_millis().min(i64::MAX as u128) as i64;
                drop(observation);
                (result, duration_ms)
            };
            let (status, summary, detail) = match result {
                Ok(summary) => (SystemTaskStatus::Success, Some(summary), None),
                Err(error) => (
                    SystemTaskStatus::Failed,
                    Some(format!("{task_key} 手动运行失败")),
                    Some(error.to_string()),
                ),
            };
            let task_dimension = managed_task_metric_dimension(&task_key);
            state.performance_telemetry.record_duration_ms(
                "maintenance.task_run_duration_ms",
                task_dimension,
                duration_ms as f64,
            );
            state.performance_telemetry.record_counter(
                "maintenance.task_run_count",
                task_dimension,
                1,
            );
            state.performance_telemetry.record_counter(
                if status == SystemTaskStatus::Success {
                    "maintenance.task_run_success_count"
                } else {
                    "maintenance.task_run_failure_count"
                },
                task_dimension,
                1,
            );
            let finish = ManagedTaskFinish {
                run_id,
                task_key,
                status,
                finished_at: format_utc_iso_millis(Utc::now()),
                duration_ms,
                summary,
                detail,
            };
            if let Err(error) = finish_managed_task_run_bounded(&store, &finish).await {
                warn!(
                    run_id = finish.run_id,
                    task = %finish.task_key,
                    error = %error,
                    "managed task dispatcher deferred task-history finalization"
                );
                pending_finishes.push_back(finish);
            }
        }
    })
}

struct ManagedTaskFinish {
    run_id: i64,
    task_key: String,
    status: SystemTaskStatus,
    finished_at: String,
    duration_ms: i64,
    summary: Option<String>,
    detail: Option<String>,
}

async fn finish_managed_task_run_bounded(
    store: &crate::maintenance_store::MaintenanceStore,
    finish: &ManagedTaskFinish,
) -> Result<()> {
    let mut last_error = None;
    for attempt in 0..=MANAGED_TASK_FINISH_RETRY_ATTEMPTS {
        match tokio::time::timeout(
            MANAGED_TASK_FINISH_ATTEMPT_TIMEOUT,
            store.finish_run(
                finish.run_id,
                finish.status.as_str(),
                &finish.finished_at,
                finish.duration_ms,
                finish.summary.as_deref(),
                finish.detail.as_deref(),
            ),
        )
        .await
        {
            Ok(Ok(())) => return Ok(()),
            Ok(Err(error)) => last_error = Some(error),
            Err(error) => {
                last_error = Some(anyhow!(
                    "managed task finish timed out after {} ms: {error}",
                    MANAGED_TASK_FINISH_ATTEMPT_TIMEOUT.as_millis()
                ));
            }
        }
        if attempt < MANAGED_TASK_FINISH_RETRY_ATTEMPTS {
            tokio::time::sleep(MANAGED_TASK_FINISH_RETRY_INTERVAL).await;
        }
    }
    Err(last_error.unwrap_or_else(|| anyhow!("managed task finish failed without an error")))
}

async fn drain_managed_task_finishes_on_shutdown(
    state: &AppState,
    store: &crate::maintenance_store::MaintenanceStore,
    pending_finishes: &mut VecDeque<ManagedTaskFinish>,
) {
    while let Some(finish) = pending_finishes.pop_front() {
        if let Err(error) = finish_managed_task_run_bounded(store, &finish).await {
            // Replay resolves the maintenance row by run_id; task_kind is legacy journal metadata.
            let recovery = BatchedSystemTaskFinish {
                run_id: finish.run_id,
                task_kind: SystemTaskKind::StartupBackfill,
                trigger_kind: "managed_dispatcher".to_string(),
                status: finish.status,
                summary: finish.summary.clone(),
                detail: finish.detail.clone(),
                finished_at: finish.finished_at.clone(),
                duration_ms: finish.duration_ms,
            };
            let quarantined = state.sqlite_batch_writer.quarantine_system_task_finish(
                &recovery,
                "managed task finish could not be finalized before shutdown",
            );
            warn!(
                run_id = finish.run_id,
                task = %finish.task_key,
                error = %error,
                quarantined,
                "managed task dispatcher could not finalize a pending run before shutdown"
            );
        }
    }
}

fn managed_task_metric_dimension(task_key: &str) -> &'static str {
    match task_key {
        "retention_archive" => "retention_archive",
        "upstream_account_maintenance" => "upstream_account_maintenance",
        "forward_proxy_subscription_refresh" => "forward_proxy_subscription_refresh",
        "pool_orphan_recovery" => "pool_orphan_recovery",
        "startup_hourly_rollup_bootstrap" => "startup_hourly_rollup_bootstrap",
        "system_status_snapshot" => "system_status_snapshot",
        "invocation_timeline_snapshot" => "invocation_timeline_snapshot",
        "summary_snapshot" => "summary_snapshot",
        "summary_coverage_recovery" => "summary_coverage_recovery",
        "dashboard_runtime_projection_reconcile" => "dashboard_runtime_projection_reconcile",
        "long_term_projection" => "long_term_projection",
        "timeseries_minute_projection" => "timeseries_minute_projection",
        "raw_payload_metrics_inventory" => "raw_payload_metrics_inventory",
        "prompt_cache_materialization" => "prompt_cache_materialization",
        "startup_backfill" => "startup_backfill",
        "raw_compression" => "raw_compression",
        "archive_upstream_activity_manifest" => "archive_upstream_activity_manifest",
        "materialize_historical_rollups" => "materialize_historical_rollups",
        "verify_archive_storage" => "verify_archive_storage",
        "prune_archive_batches" => "prune_archive_batches",
        "prune_legacy_archive_batches" => "prune_legacy_archive_batches",
        key if key.starts_with("startup_backfill.") => "startup_backfill_child",
        _ => "unknown_managed_task",
    }
}

async fn run_managed_task_once(state: &Arc<AppState>, task_key: &str) -> Result<String> {
    match task_key {
        "retention_archive" => {
            let summary =
                run_data_retention_maintenance(&state.pool, &state.config, Some(false), None)
                    .await?;
            let (brief, _detail) = crate::api::summarize_retention_run_for_system_task(&summary);
            Ok(brief)
        }
        "forward_proxy_subscription_refresh" => {
            refresh_forward_proxy_subscriptions(state.clone(), false, None).await?;
            Ok("正向代理订阅刷新完成".to_string())
        }
        "summary_snapshot" => {
            crate::api::refresh_summary_snapshots(state.as_ref()).await?;
            Ok("汇总快照刷新完成".to_string())
        }
        "summary_coverage_recovery" => {
            crate::api::SummaryCoverageRecoverySupervisor::run_with_priority_reservation(
                state.as_ref(),
                None,
            )
            .await?;
            Ok("汇总覆盖恢复完成".to_string())
        }
        "pool_orphan_recovery" => {
            let outcome = recover_stale_pool_early_phase_orphans_runtime(state.as_ref()).await?;
            Ok(format!(
                "恢复连接池尝试 {} 条，调用 {} 条",
                outcome.recovered_attempts, outcome.recovered_invocations
            ))
        }
        "upstream_account_maintenance" => {
            run_upstream_account_maintenance_once(state.clone()).await?;
            Ok("上游账号维护完成".to_string())
        }
        "system_status_snapshot" => {
            hydrate_system_status_snapshot(state.as_ref()).await?;
            Ok("系统状态快照刷新完成".to_string())
        }
        "invocation_timeline_snapshot" => {
            cleanup_timeline_snapshot_rows_once(&state.pool)
                .await
                .map_err(|_| anyhow!("调用时间线快照清理失败"))?;
            Ok("调用时间线快照清理完成".to_string())
        }
        "dashboard_runtime_projection_reconcile" => {
            let result = reconcile_dashboard_runtime_projection_once(state.as_ref())
                .await
                .map_err(|_| anyhow!("仪表盘运行投影校对失败"))?;
            let _ = result;
            Ok("仪表盘运行投影校对完成".to_string())
        }
        "long_term_projection" => {
            run_long_term_projection_once_managed(state.as_ref()).await?;
            Ok("长期统计投影刷新完成".to_string())
        }
        "timeseries_minute_projection" => {
            crate::api::flush_timeseries_minute_projection_managed(state.as_ref(), "managed_task")
                .await
                .map_err(|_| anyhow!("分钟时序投影刷新失败"))?;
            Ok("分钟时序投影刷新完成".to_string())
        }
        "raw_payload_metrics_inventory" => {
            let reset =
                resume_retention_raw_payload_metrics_inventory_reset(state.as_ref()).await?;
            Ok(if reset {
                "原始载荷指标盘点已推进".to_string()
            } else {
                "原始载荷指标盘点无需处理".to_string()
            })
        }
        "prompt_cache_materialization" => {
            let task = crate::StartupBackfillTask::PromptCacheConversationsMaterialization;
            let pass = crate::run_startup_backfill_maintenance_pass_managed(
                state.clone(),
                &state.shutdown,
                Some(&[task]),
                Some("prompt_cache_materialization"),
            )
            .await;
            if pass.had_failure {
                bail!(
                    pass.detail
                        .unwrap_or_else(|| "Prompt 缓存物化失败".to_string())
                );
            }
            Ok("Prompt 缓存物化完成".to_string())
        }
        "startup_hourly_rollup_bootstrap" => {
            bootstrap_hourly_rollups_for_runtime_startup(
                &state.pool,
                Some(state.config.invocation_max_days),
            )
            .await?;
            Ok("启动时小时汇总补齐完成".to_string())
        }
        "raw_compression" => {
            let summary = compress_cold_proxy_raw_payloads(
                &state.pool,
                &state.config,
                state.config.database_path.parent(),
                false,
            )
            .await?;
            Ok(format!("原始载荷压缩完成：{summary:?}"))
        }
        "archive_upstream_activity_manifest" => {
            let summary =
                refresh_archive_upstream_activity_manifest(&state.pool, &state.config, false)
                    .await?;
            Ok(format!("上游活动归档清单完成：{summary:?}"))
        }
        "materialize_historical_rollups" => {
            let summary = materialize_historical_rollups(&state.pool, &state.config, false).await?;
            Ok(format!("历史汇总物化完成：{summary:?}"))
        }
        "verify_archive_storage" => {
            let summary = verify_archive_storage(&state.pool, &state.config).await?;
            Ok(format!("归档存储校验完成：{summary:?}"))
        }
        "prune_archive_batches" => {
            let summary = prune_archive_batches(&state.pool, &state.config, false).await?;
            Ok(format!("归档批次清理完成：{summary:?}"))
        }
        "prune_legacy_archive_batches" => {
            let summary = prune_legacy_archive_batches(&state.pool, &state.config, false).await?;
            Ok(format!("旧归档批次清理完成：{summary:?}"))
        }
        "startup_backfill" => {
            let pass = crate::run_startup_backfill_maintenance_pass_managed(
                state.clone(),
                &state.shutdown,
                None,
                None,
            )
            .await;
            if pass.had_failure {
                bail!(
                    pass.detail
                        .unwrap_or_else(|| "启动回填存在失败".to_string())
                );
            }
            Ok(if pass.ran_actionable_task {
                "启动回填处理完成".to_string()
            } else {
                "启动回填没有可处理项".to_string()
            })
        }
        key if key.starts_with("startup_backfill.") => {
            let name = key.trim_start_matches("startup_backfill.");
            let task = managed_startup_backfill_task(name)?;
            let catalog = state.pricing_catalog.read().await.clone();
            crate::wake_startup_backfill_tasks_with_pricing_catalog(
                &state.pool,
                &[task],
                Some(&catalog),
                "manual_run",
            )
            .await?;
            let pass = crate::run_startup_backfill_maintenance_pass_managed(
                state.clone(),
                &state.shutdown,
                Some(&[task]),
                None,
            )
            .await;
            if pass.had_failure {
                bail!(
                    pass.detail
                        .unwrap_or_else(|| format!("启动回填子任务失败: {name}"))
                );
            }
            Ok(format!("启动回填子任务 {name} 处理完成"))
        }
        _ => bail!("任务暂不支持立即运行: {task_key}"),
    }
}

fn managed_startup_backfill_task(name: &str) -> Result<crate::StartupBackfillTask> {
    crate::StartupBackfillTask::from_managed_key(name)
        .ok_or_else(|| anyhow!("未知启动回填子任务: {name}"))
}

pub(crate) fn begin_runtime_shutdown(cancel: &CancellationToken) {
    if !cancel.is_cancelled() {
        info!("shutdown signal received; beginning graceful shutdown");
        cancel.cancel();
    }
}

pub(crate) async fn drain_runtime_after_shutdown(
    state: Arc<AppState>,
    server_handle: Option<JoinHandle<()>>,
    poller_handle: Option<JoinHandle<()>>,
    upstream_accounts_handle: Option<JoinHandle<()>>,
    forward_proxy_handle: Option<JoinHandle<()>>,
    pool_orphan_recovery_handle: Option<JoinHandle<()>>,
    retention_handle: Option<JoinHandle<()>>,
    startup_backfill_handle: Option<JoinHandle<()>>,
    startup_hot_read_hydration_handle: Option<JoinHandle<()>>,
) -> Result<()> {
    if let Some(server_handle) = server_handle {
        info!("http server graceful drain started");
        if let Err(err) = server_handle.await {
            error!(?err, "http server terminated unexpectedly");
        }
        info!("http server graceful drain finished");
    }

    if let Some(poller_handle) = poller_handle {
        if let Err(err) = poller_handle.await {
            error!(?err, "poller task terminated unexpectedly");
        }
        info!("scheduler drained");
    }
    if let Some(upstream_accounts_handle) = upstream_accounts_handle
        && let Err(err) = upstream_accounts_handle.await
    {
        error!(
            ?err,
            "upstream account maintenance task terminated unexpectedly"
        );
    }
    state.upstream_accounts.drain_background_tasks().await;
    if let Some(forward_proxy_handle) = forward_proxy_handle
        && let Err(err) = forward_proxy_handle.await
    {
        error!(
            ?err,
            "forward proxy maintenance task terminated unexpectedly"
        );
    }
    if let Some(pool_orphan_recovery_handle) = pool_orphan_recovery_handle
        && let Err(err) = pool_orphan_recovery_handle.await
    {
        error!(
            ?err,
            "pool orphan recovery maintenance task terminated unexpectedly"
        );
    }
    if let Some(retention_handle) = retention_handle
        && let Err(err) = retention_handle.await
    {
        error!(?err, "retention maintenance task terminated unexpectedly");
    }
    if let Some(startup_backfill_handle) = startup_backfill_handle
        && let Err(err) = startup_backfill_handle.await
    {
        error!(
            ?err,
            "startup backfill maintenance task terminated unexpectedly"
        );
    }
    if let Some(startup_hot_read_hydration_handle) = startup_hot_read_hydration_handle
        && let Err(err) = startup_hot_read_hydration_handle.await
    {
        error!(
            ?err,
            "startup hot-read hydration coordinator terminated unexpectedly"
        );
    }

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

    state.performance_telemetry.shutdown_and_drain().await;
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
    tokio::spawn(async move {
        if crate::maintenance_store::legacy_worker_should_skip("startup_hourly_rollup_bootstrap")
            .await
        {
            return;
        }
        let started_at = Instant::now();
        let pressure_gate = crate::db_pressure::global_db_pressure_gate();
        let coordinator = crate::proxy_sqlite_write_coordinator::proxy_sqlite_write_coordinator();
        // Every task row created by this bootstrap attempt is timestamped after this point.
        // Using the exact lower bound avoids losing a row when SQLite admission takes longer
        // than the old one-second grace window during shutdown.
        let task_start_window = format_utc_iso_millis(Utc::now());
        let initial_p2_preemption_retry =
            Duration::from_secs(BACKGROUND_DB_PRESSURE_RETRY_INTERVAL_SECS);
        let mut p2_preemption_retry = initial_p2_preemption_retry;
        loop {
            if crate::maintenance_store::legacy_worker_should_skip(
                "startup_hourly_rollup_bootstrap",
            )
            .await
            {
                return;
            }
            let Some(_execution_lease) = crate::maintenance_store::try_acquire_task_execution(
                "startup_hourly_rollup_bootstrap",
            ) else {
                tokio::select! {
                    biased;
                    _ = cancel.cancelled() => return,
                    _ = tokio::time::sleep(Duration::from_secs(1)) => continue,
                }
            };
            // Task history is admitted and recorded before waiting for the synchronization lock,
            // but both permits are released immediately so a lock wait cannot occupy the only
            // background pressure slot.
            let mut task_run = loop {
                let pressure_permit = match pressure_gate
                    .try_begin_background("startup_hourly_rollup_bootstrap_task_history")
                {
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
                        info!(
                            defer_reason = %reason,
                            retry_after_ms = retry_after.as_millis() as u64,
                            "background startup hourly rollup bootstrap task history deferred"
                        );
                        tokio::select! {
                            biased;
                            _ = cancel.cancelled() => return,
                            _ = tokio::time::sleep(retry_after) => continue,
                        }
                    }
                };
                let Some(write_permit) = coordinator.try_acquire(
                    crate::proxy_sqlite_write_coordinator::ProxySqliteWriteClass::P2Derived,
                ) else {
                    drop(pressure_permit);
                    tokio::select! {
                        biased;
                        _ = cancel.cancelled() => return,
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
                            state.as_ref(),
                            &cancel,
                            &task_start_window,
                        ).await;
                        return;
                    }
                    result = begin_runtime_startup_hourly_rollup_task(
                        state.as_ref(),
                        SystemTaskKind::HourlyRollupBootstrap,
                        "startup",
                        Some("background hourly rollup bootstrap started".to_string()),
                    ) => result,
                };
                drop(write_permit);
                drop(pressure_permit);
                match result {
                    Ok(task_run) => break task_run,
                    Err(err) => {
                        warn!(
                            error = %err,
                            "failed to record background startup hourly rollup bootstrap start; retrying"
                        );
                        tokio::select! {
                            biased;
                            _ = cancel.cancelled() => return,
                            _ = tokio::time::sleep(Duration::from_secs(
                                BACKGROUND_DB_PRESSURE_RETRY_INTERVAL_SECS,
                            )) => {}
                        }
                    }
                }
            };
            // The task-history admission is released before waiting for this lock. The actual
            // rollup work reacquires both gates only after the lock is owned.
            let rollup_guard = tokio::select! {
                biased;
                _ = cancel.cancelled() => {
                    info!(
                        elapsed_ms = started_at.elapsed().as_millis() as u64,
                        "background startup hourly rollup bootstrap cancelled before acquiring its synchronization lock"
                    );
                    finish_runtime_startup_hourly_rollup_bootstrap_task(
                        state.as_ref(),
                        &cancel,
                        Some(&task_run),
                        SystemTaskStatus::Skipped,
                        "background hourly rollup bootstrap cancelled before acquiring its synchronization lock",
                        None,
                    ).await;
                    return;
                }
                guard = state.hourly_rollup_sync_lock.lock() => guard,
            };
            let (pressure_permit, write_permit) = loop {
                let pressure_permit = match pressure_gate
                    .try_begin_background("startup_hourly_rollup_bootstrap")
                {
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
                            _ = cancel.cancelled() => {
                                drop(rollup_guard);
                                finish_runtime_startup_hourly_rollup_bootstrap_task(
                                    state.as_ref(),
                                    &cancel,
                                    Some(&task_run),
                                    SystemTaskStatus::Skipped,
                                    "background hourly rollup bootstrap cancelled before reacquiring SQLite write admission",
                                    None,
                                ).await;
                                return;
                            }
                            _ = tokio::time::sleep(retry_after) => continue,
                        }
                    }
                };
                let Some(write_permit) = coordinator.try_acquire(
                    crate::proxy_sqlite_write_coordinator::ProxySqliteWriteClass::P2Derived,
                ) else {
                    drop(pressure_permit);
                    tokio::select! {
                        biased;
                        _ = cancel.cancelled() => {
                            drop(rollup_guard);
                            finish_runtime_startup_hourly_rollup_bootstrap_task(
                                state.as_ref(),
                                &cancel,
                                Some(&task_run),
                                SystemTaskStatus::Skipped,
                                "background hourly rollup bootstrap cancelled before reacquiring SQLite write admission",
                                None,
                            ).await;
                            return;
                        }
                        _ = tokio::time::sleep(Duration::from_secs(
                            BACKGROUND_DB_PRESSURE_RETRY_INTERVAL_SECS,
                        )) => continue,
                    }
                };
                break (pressure_permit, write_permit);
            };

            task_run.observation = Some(crate::TaskExecutionObservation::begin(
                SystemTaskKind::HourlyRollupBootstrap.as_str(),
                &crate::maintenance_store::task_title_for_observation(
                    SystemTaskKind::HourlyRollupBootstrap.as_str(),
                ),
                "startup",
                crate::maintenance_store::task_execution_class(
                    SystemTaskKind::HourlyRollupBootstrap.as_str(),
                ),
                "processing",
            ));

            let hourly_rollups_started_at = Instant::now();
            let hourly_rollups = tokio::select! {
                biased;
                _ = cancel.cancelled() => None,
                _ = coordinator.wait_for_p2_preemption() => None,
                result = bootstrap_hourly_rollups_for_runtime_startup(
                    &state.pool,
                    Some(state.config.invocation_max_days),
                ) => Some(result),
            };
            let Some(hourly_rollups) = hourly_rollups else {
                drop(write_permit);
                drop(pressure_permit);
                drop(rollup_guard);
                info!(
                    elapsed_ms = started_at.elapsed().as_millis() as u64,
                    "background startup hourly rollup bootstrap cancelled during hourly rollup repair"
                );
                finish_runtime_startup_hourly_rollup_bootstrap_task(
                    state.as_ref(),
                    &cancel,
                    Some(&task_run),
                    SystemTaskStatus::Skipped,
                    "background hourly rollup bootstrap cancelled during hourly rollup repair",
                    None,
                )
                .await;
                if cancel.is_cancelled() {
                    return;
                }
                let retry_after = p2_preemption_retry;
                p2_preemption_retry =
                    next_startup_hourly_rollup_p2_preemption_retry(p2_preemption_retry);
                tokio::select! {
                    biased;
                    _ = cancel.cancelled() => return,
                    _ = tokio::time::sleep(retry_after) => continue,
                }
            };
            if let Err(err) = hourly_rollups {
                drop(write_permit);
                drop(pressure_permit);
                drop(rollup_guard);
                pressure_gate.record_error("startup_hourly_rollup_bootstrap", &err);
                finish_runtime_startup_hourly_rollup_bootstrap_task(
                    state.as_ref(),
                    &cancel,
                    Some(&task_run),
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
                return;
            }
            let hourly_rollups_elapsed_ms = hourly_rollups_started_at.elapsed().as_millis() as u64;
            info!(
                elapsed_ms = hourly_rollups_elapsed_ms,
                "background startup hourly rollup bootstrap completed hourly rollup repair"
            );

            let summary_rollups_started_at = Instant::now();
            let summary_rollups = tokio::select! {
                biased;
                _ = cancel.cancelled() => None,
                _ = coordinator.wait_for_p2_preemption() => None,
                result = ensure_invocation_summary_rollups_ready_best_effort(&state.pool) => Some(result),
            };
            drop(rollup_guard);
            let Some(summary_rollups) = summary_rollups else {
                drop(write_permit);
                drop(pressure_permit);
                info!(
                    elapsed_ms = started_at.elapsed().as_millis() as u64,
                    "background startup hourly rollup bootstrap cancelled during summary rollup repair"
                );
                finish_runtime_startup_hourly_rollup_bootstrap_task(
                    state.as_ref(),
                    &cancel,
                    Some(&task_run),
                    SystemTaskStatus::Skipped,
                    "background hourly rollup bootstrap cancelled during summary rollup repair",
                    None,
                )
                .await;
                if cancel.is_cancelled() {
                    return;
                }
                let retry_after = p2_preemption_retry;
                p2_preemption_retry =
                    next_startup_hourly_rollup_p2_preemption_retry(p2_preemption_retry);
                tokio::select! {
                    biased;
                    _ = cancel.cancelled() => return,
                    _ = tokio::time::sleep(retry_after) => continue,
                }
            };
            if let Err(err) = summary_rollups {
                drop(write_permit);
                drop(pressure_permit);
                pressure_gate.record_error("startup_hourly_rollup_bootstrap", &err);
                finish_runtime_startup_hourly_rollup_bootstrap_task(
                    state.as_ref(),
                    &cancel,
                    Some(&task_run),
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
                return;
            }
            drop(write_permit);
            drop(pressure_permit);
            let summary_rollups_elapsed_ms =
                summary_rollups_started_at.elapsed().as_millis() as u64;
            let elapsed_ms = started_at.elapsed().as_millis() as u64;
            finish_runtime_startup_hourly_rollup_bootstrap_task(
            state.as_ref(),
            &cancel,
            Some(&task_run),
            SystemTaskStatus::Success,
            &format!(
                "background hourly rollup bootstrap completed: hourly_rollups_ms={hourly_rollups_elapsed_ms} summary_rollups_ms={summary_rollups_elapsed_ms}"
            ),
            None,
        )
        .await;
            info!(
                elapsed_ms,
                hourly_rollups_elapsed_ms,
                summary_rollups_elapsed_ms,
                "background startup hourly rollup bootstrap completed"
            );
            break;
        }
    })
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

pub(crate) async fn finish_orphaned_startup_hourly_rollup_bootstrap_task(
    state: &AppState,
    cancel: &CancellationToken,
    started_at_from: &str,
) {
    #[cfg(not(test))]
    let Some(store) = crate::maintenance_store::global() else {
        return;
    };
    let deadline = Instant::now() + Duration::from_millis(250);
    loop {
        #[cfg(test)]
        let task_query = sqlx::query_as::<_, (i64, String)>(
            r#"
            SELECT id, trigger_kind
            FROM system_task_runs
            WHERE task_kind = ?1
              AND trigger_kind = 'startup'
              AND status = ?2
              AND summary = 'background hourly rollup bootstrap started'
              AND started_at >= ?3
            ORDER BY id DESC
            LIMIT 1
            "#,
        )
        .bind("hourly_rollup_bootstrap")
        .bind(SystemTaskStatus::Running.as_str())
        .bind(started_at_from)
        .fetch_optional(&state.pool);
        #[cfg(not(test))]
        let task_query = sqlx::query_as::<_, (i64, String)>(
            r#"
                SELECT id, trigger_kind
                FROM managed_task_runs
                WHERE task_key = ?1
                  AND trigger_kind = 'startup'
                  AND status = ?2
                  AND summary = 'background hourly rollup bootstrap started'
                  AND started_at >= ?3
                ORDER BY id DESC
                LIMIT 1
                "#,
        )
        .bind(SystemTaskKind::HourlyRollupBootstrap.as_str())
        .bind(SystemTaskStatus::Running.as_str())
        .bind(started_at_from)
        .fetch_optional(&store.pool);
        let task = tokio::time::timeout(Duration::from_millis(50), task_query).await;
        match task {
            Ok(Ok(Some((id, trigger_kind)))) => {
                let task_run = SystemTaskRunHandle {
                    id,
                    task_kind: SystemTaskKind::HourlyRollupBootstrap,
                    trigger_kind,
                    started_at: Instant::now(),
                    observation: None,
                };
                finish_runtime_startup_hourly_rollup_bootstrap_task(
                    state,
                    cancel,
                    Some(&task_run),
                    SystemTaskStatus::Skipped,
                    "background hourly rollup bootstrap cancelled before acquiring its synchronization lock",
                    None,
                )
                .await;
                return;
            }
            Ok(Ok(None)) | Err(_) => {}
            Ok(Err(err)) => {
                debug!(error = %err, "failed to inspect for an orphaned startup hourly rollup bootstrap task");
                return;
            }
        }
        if Instant::now() >= deadline {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

pub(crate) async fn finish_runtime_startup_hourly_rollup_bootstrap_task(
    state: &AppState,
    cancel: &CancellationToken,
    task_run: Option<&SystemTaskRunHandle>,
    status: SystemTaskStatus,
    summary: &str,
    detail: Option<String>,
) {
    if let Some(task_run) = task_run {
        let finished = finish_system_task_run_reliably(
            state,
            Some(cancel),
            task_run,
            status,
            Some(summary.to_string()),
            detail,
        )
        .await;
        if !finished {
            warn!(
                task_kind = task_run.task_kind.as_str(),
                trigger_kind = %task_run.trigger_kind,
                timeout_ms = STARTUP_HOURLY_ROLLUP_BOOTSTRAP_CANCELLED_TASK_FINISH_TIMEOUT.as_millis() as u64,
                "failed to durably finalize startup hourly rollup bootstrap task-history after bounded retries"
            );
        }
    }
}

pub(crate) fn spawn_forward_proxy_maintenance(
    state: Arc<AppState>,
    cancel: CancellationToken,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let startup_known_subscription_keys = {
            let manager = state.forward_proxy.lock().await;
            snapshot_known_subscription_proxy_keys(&manager)
        };
        if cancel.is_cancelled() {
            info!("forward proxy maintenance skipped because shutdown is already in progress");
            return;
        }
        if crate::maintenance_store::legacy_worker_should_skip("forward_proxy_subscription_refresh")
            .await
        {
            info!("forward proxy legacy worker skipped by managed task control");
            return;
        }
        {
            let Some(_execution_lease) = crate::maintenance_store::try_acquire_task_execution(
                "forward_proxy_subscription_refresh",
            ) else {
                return;
            };
            let observation = crate::TaskExecutionObservation::begin(
                "forward_proxy_subscription_refresh",
                &crate::maintenance_store::task_title_for_observation(
                    "forward_proxy_subscription_refresh",
                ),
                "startup",
                crate::maintenance_store::task_execution_class(
                    "forward_proxy_subscription_refresh",
                ),
                "processing",
            );
            let startup_run = tokio::select! {
                biased;
                _ = cancel.cancelled() => return,
                result = begin_system_task_run_admitted(
                    state.as_ref(),
                    crate::proxy_sqlite_write_coordinator::ProxySqliteWriteClass::P2Derived,
                    SystemTaskKind::ForwardProxySubscriptionRefresh,
                    "startup",
                    Some("forward proxy subscription refresh started".to_string()),
                ) => match result {
                    Ok(run) => {
                        observation.finish();
                        Some(run)
                    }
                    Err(error) => {
                        warn!(%error, "failed to record forward proxy startup refresh");
                        None
                    }
                },
            };
            if let Err(err) = refresh_forward_proxy_subscriptions(
                state.clone(),
                true,
                Some(startup_known_subscription_keys),
            )
            .await
            {
                if let Some(run) = startup_run.as_ref() {
                    let _ = finish_system_task_run_reliably(
                        state.as_ref(),
                        Some(&cancel),
                        run,
                        SystemTaskStatus::Failed,
                        Some("forward proxy startup refresh failed".to_string()),
                        Some(err.to_string()),
                    )
                    .await;
                }
                warn!(error = %err, "failed to refresh forward proxy subscriptions at startup");
            } else if let Some(run) = startup_run.as_ref() {
                let _ = finish_system_task_run_reliably(
                    state.as_ref(),
                    Some(&cancel),
                    run,
                    SystemTaskStatus::Success,
                    Some("forward proxy startup refresh completed".to_string()),
                    None,
                )
                .await;
            }
        }

        let mut ticker = interval(Duration::from_secs(60));
        ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                _ = cancel.cancelled() => {
                    info!("forward proxy maintenance received shutdown");
                    break;
                }
                _ = ticker.tick() => {
                    if crate::maintenance_store::legacy_worker_should_skip(
                        "forward_proxy_subscription_refresh",
                    )
                    .await
                    {
                        continue;
                    }
                    let Some(_execution_lease) =
                        crate::maintenance_store::try_acquire_task_execution(
                            "forward_proxy_subscription_refresh",
                        )
                    else {
                        continue;
                    };
                    let observation = crate::TaskExecutionObservation::begin(
                        "forward_proxy_subscription_refresh",
                        &crate::maintenance_store::task_title_for_observation(
                            "forward_proxy_subscription_refresh",
                        ),
                        "interval",
                        crate::maintenance_store::task_execution_class(
                            "forward_proxy_subscription_refresh",
                        ),
                        "processing",
                    );
                    let task_run = tokio::select! {
                        biased;
                        _ = cancel.cancelled() => break,
                        result = begin_system_task_run_admitted(
                            state.as_ref(),
                            crate::proxy_sqlite_write_coordinator::ProxySqliteWriteClass::P2Derived,
                            SystemTaskKind::ForwardProxySubscriptionRefresh,
                            "interval",
                            Some("forward proxy interval refresh started".to_string()),
                        ) => match result {
                            Ok(run) => {
                                observation.finish();
                                Some(run)
                            }
                            Err(error) => {
                                warn!(%error, "failed to record forward proxy interval refresh");
                                None
                            }
                        },
                    };
                    if let Err(err) = refresh_forward_proxy_subscriptions(state.clone(), false, None).await {
                        if let Some(run) = task_run.as_ref() {
                            let _ = finish_system_task_run_reliably(
                                state.as_ref(),
                                Some(&cancel),
                                run,
                                SystemTaskStatus::Failed,
                                Some("forward proxy interval refresh failed".to_string()),
                                Some(err.to_string()),
                            )
                            .await;
                        }
                        warn!(error = %err, "failed to refresh forward proxy subscriptions");
                    } else if let Some(run) = task_run.as_ref() {
                        let _ = finish_system_task_run_reliably(
                            state.as_ref(),
                            Some(&cancel),
                            run,
                            SystemTaskStatus::Success,
                            Some("forward proxy interval refresh completed".to_string()),
                            None,
                        )
                        .await;
                    }
                    if let Err(err) = flush_dashboard_network_socket_minute_rollups(
                        &state.pool,
                        state.dashboard_network_speed_cache.as_ref(),
                        Utc::now(),
                    )
                    .await
                    {
                        warn!(error = %err, "failed to flush dashboard socket minute rollups");
                    }
                }
            }
        }
    })
}

pub(crate) fn spawn_pool_orphan_recovery_maintenance(
    state: Arc<AppState>,
    cancel: CancellationToken,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut ticker = interval(POOL_EARLY_PHASE_ORPHAN_RECOVERY_INTERVAL);
        ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                _ = cancel.cancelled() => {
                    info!("pool orphan recovery maintenance received shutdown");
                    break;
                }
                _ = ticker.tick() => {
                    if crate::maintenance_store::legacy_worker_should_skip("pool_orphan_recovery").await {
                        continue;
                    }
                    let Some(_execution_lease) =
                        crate::maintenance_store::try_acquire_task_execution("pool_orphan_recovery")
                    else {
                        continue;
                    };
                    let _observation = crate::TaskExecutionObservation::begin(
                        "pool_orphan_recovery",
                        &crate::maintenance_store::task_title_for_observation(
                            "pool_orphan_recovery",
                        ),
                        "interval",
                        crate::maintenance_store::task_execution_class("pool_orphan_recovery"),
                        "processing",
                    );
                    match recover_stale_pool_early_phase_orphans_runtime(state.as_ref()).await {
                        Ok(outcome) => {
                            if outcome.recovered_attempts > 0 || outcome.recovered_invocations > 0 {
                                warn!(
                                    recovered_attempts = outcome.recovered_attempts,
                                    recovered_invocations = outcome.recovered_invocations,
                                    "runtime pool orphan recovery swept stale early-phase rows"
                                );
                            }
                        }
                        Err(err) => {
                            warn!(error = %err, "failed to recover stale pool early-phase orphans at runtime");
                        }
                    }
                }
            }
        }
    })
}

#[cfg(test)]
mod managed_task_dispatch_tests {
    use super::managed_startup_backfill_task;

    #[test]
    fn all_registered_startup_backfill_children_resolve_for_run_now() {
        for key in crate::maintenance_store::STARTUP_BACKFILL_TASKS {
            assert!(
                managed_startup_backfill_task(key).is_ok(),
                "registered child task {key} cannot run"
            );
        }
        assert!(managed_startup_backfill_task("unknown_child").is_err());
    }
}
