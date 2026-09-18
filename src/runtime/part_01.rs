const STARTUP_HOT_READ_HYDRATION_RETRY_INITIAL: Duration = Duration::from_secs(1);
const STARTUP_HOT_READ_HYDRATION_RETRY_MAX: Duration = Duration::from_secs(30);

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

    let pool = initialize_runtime_database(&config, &cli).await?;
    if run_cli_maintenance_if_requested(&pool, &config, &cli).await? {
        return Ok(());
    }
    let state = build_runtime_app_state(config, pool).await?;
    let signal_listener = spawn_shutdown_signal_listener(state.shutdown.clone());
    warm_pool_routing_runtime_cache_best_effort(state.as_ref()).await;
    warm_dashboard_runtime_projection(state.as_ref()).await;
    spawn_dashboard_runtime_projection_reconcile(state.clone());
    spawn_subscription_broadcast_listener(state.clone());
    spawn_system_raw_payload_metrics_inventory(state.clone(), state.shutdown.clone());
    spawn_memory_diagnostics(state.clone(), state.shutdown.clone());
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

async fn initialize_runtime_database(config: &AppConfig, cli: &CliArgs) -> Result<Pool<Sqlite>> {
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
    // Keep all existing minute coverage behind exact reads until the P2 supervisor has invalidated
    // it in bounded transactions. This happens before the HTTP listener is created.
    mark_timeseries_minute_projection_startup_recovery(&pool).await?;
    log_startup_phase("schema", schema_started_at);
    recover_raw_overflow_spools(config).await;
    if should_recover_pending_pool_attempts_on_startup(cli) {
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
    if should_run_blocking_startup_persistent_prep(cli) {
        let prep_summary = run_startup_persistent_prep(&pool, config, cli).await?;
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
    Ok(pool)
}

async fn run_cli_maintenance_if_requested(
    pool: &Pool<Sqlite>,
    config: &AppConfig,
    cli: &CliArgs,
) -> Result<bool> {
    if cli.retention_run_once && cli.command.is_some() {
        bail!("--retention-run-once cannot be combined with maintenance subcommands");
    }
    if let Some(command) = &cli.command {
        run_cli_command(pool, config, command).await?;
        return Ok(true);
    }
    if cli.retention_run_once {
        let summary =
            run_data_retention_maintenance(pool, config, Some(cli.retention_dry_run), None).await?;
        info!(?summary, "retention maintenance run-once finished");
        return Ok(true);
    }
    Ok(false)
}

async fn load_runtime_forward_proxy(
    config: &AppConfig,
    pool: &Pool<Sqlite>,
) -> Result<(
    Arc<RwLock<ProxyModelSettings>>,
    Arc<Mutex<ForwardProxyManager>>,
    [u8; 32],
)> {
    ensure_proxy_encrypted_session_owner_routing_setting_initialized(pool, config).await?;
    ensure_proxy_websocket_settings_initialized(pool, config).await?;
    let proxy_model_settings = Arc::new(RwLock::new(load_proxy_model_settings(pool).await?));
    let forward_proxy_settings = load_forward_proxy_settings(pool).await?;
    let forward_proxy_runtime = load_forward_proxy_runtime_states(pool).await?;
    let oauth_installation_seed = oauth_bridge::load_or_init_oauth_installation_seed(pool).await?;
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
    Ok((proxy_model_settings, forward_proxy, oauth_installation_seed))
}

async fn build_runtime_app_state(config: AppConfig, pool: Pool<Sqlite>) -> Result<Arc<AppState>> {
    let pricing_catalog = load_pricing_catalog(&pool).await?;
    let (proxy_model_settings, forward_proxy, oauth_installation_seed) =
        load_runtime_forward_proxy(&config, &pool).await?;
    let pricing_catalog = Arc::new(RwLock::new(pricing_catalog));

    let http_clients = HttpClients::build(&config)?;
    let upstream_accounts = Arc::new(UpstreamAccountsRuntime::from_env()?);
    let (tx, _rx) = broadcast::channel(128);
    let semaphore = Arc::new(Semaphore::new(config.max_parallel_polls));
    let proxy_raw_async_semaphore = Arc::new(Semaphore::new(proxy_raw_async_writer_limit(&config)));
    let shutdown = CancellationToken::new();
    let process_started_at_utc = Utc::now();

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

    Ok(Arc::new(AppState {
        config: config.clone(),
        pool,
        process_started_at_utc,
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
    }))
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

pub(crate) async fn drain_runtime_after_pending_shutdown(
    state: Arc<AppState>,
    mut shutdown_watcher: JoinHandle<()>,
    handles: RuntimeHandles,
) -> Result<()> {
    let shutdown_cancel = state.shutdown.clone();
    tokio::select! {
        _ = shutdown_cancel.cancelled() => {
            shutdown_watcher.abort();
            let _ = shutdown_watcher.await;
        }
        _ = &mut shutdown_watcher => {}
    }
    drain_runtime_after_shutdown(state, handles).await
}
