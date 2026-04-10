pub(crate) async fn run() -> Result<()> {
    dotenv().ok();
    dotenvy::from_filename(".env.local").ok();
    init_tracing();
    let startup_started_at = Instant::now();

    let cli = CliArgs::parse();
    let config = AppConfig::from_sources(&cli)?;
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
    if should_run_blocking_startup_hourly_rollup_bootstrap(&cli) {
        let rollup_bootstrap_started_at = Instant::now();
        bootstrap_hourly_rollups(&pool).await?;
        log_startup_phase("hourly_rollup_bootstrap", rollup_bootstrap_started_at);
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
    let proxy_model_settings = Arc::new(RwLock::new(load_proxy_model_settings(&pool).await?));
    let forward_proxy_settings = load_forward_proxy_settings(&pool).await?;
    let forward_proxy_runtime = load_forward_proxy_runtime_states(&pool).await?;
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
    let proxy_raw_async_semaphore =
        Arc::new(Semaphore::new(proxy_raw_async_writer_limit(&config)));
    let shutdown = CancellationToken::new();

    let state = Arc::new(AppState {
        config: config.clone(),
        pool,
        hourly_rollup_sync_lock: Arc::new(Mutex::new(())),
        http_clients,
        broadcaster: tx.clone(),
        broadcast_state_cache: Arc::new(Mutex::new(BroadcastStateCache::default())),
        proxy_summary_quota_broadcast_seq: Arc::new(AtomicU64::new(0)),
        proxy_summary_quota_broadcast_running: Arc::new(AtomicBool::new(false)),
        proxy_summary_quota_broadcast_handle: Arc::new(Mutex::new(Vec::new())),
        startup_ready: Arc::new(AtomicBool::new(false)),
        shutdown: shutdown.clone(),
        semaphore: semaphore.clone(),
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
        prompt_cache_conversation_cache: Arc::new(Mutex::new(
            PromptCacheConversationsCacheState::default(),
        )),
        maintenance_stats_cache: Arc::new(Mutex::new(StatsMaintenanceCacheState::default())),
        pool_routing_reservations: Arc::new(std::sync::Mutex::new(HashMap::new())),
        pool_live_attempt_ids: Arc::new(std::sync::Mutex::new(HashSet::new())),
        pool_group_429_retry_delay_override: None,
        pool_no_available_wait: PoolNoAvailableWaitSettings::default(),
        upstream_accounts,
    });

    let signal_listener = spawn_shutdown_signal_listener(state.shutdown.clone());

    run_runtime_until_shutdown(state, startup_started_at, async move {
        let _ = signal_listener.await;
    })
    .await
}

const POOL_EARLY_PHASE_ORPHAN_RECOVERY_INTERVAL: Duration = Duration::from_secs(60);

fn begin_runtime_shutdown_if_requested<F>(
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

enum StartupStageOutcome<T> {
    SkippedByShutdown,
    Completed { result: T, shutdown_requested: bool },
}

struct TrackedStartupStage<Stage> {
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

async fn run_startup_stage_until_shutdown<T, Stage, Shutdown>(
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
async fn drain_runtime_after_pending_shutdown(
    state: Arc<AppState>,
    mut shutdown_watcher: JoinHandle<()>,
    server_handle: Option<JoinHandle<()>>,
    poller_handle: Option<JoinHandle<()>>,
    upstream_accounts_handle: Option<JoinHandle<()>>,
    forward_proxy_handle: Option<JoinHandle<()>>,
    pool_orphan_recovery_handle: Option<JoinHandle<()>>,
    retention_handle: Option<JoinHandle<()>>,
    startup_backfill_handle: Option<JoinHandle<()>>,
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
    )
    .await
}

pub(crate) async fn run_runtime_until_shutdown<F>(
    state: Arc<AppState>,
    startup_started_at: Instant,
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
    let mut poller_handle = None;
    let mut upstream_accounts_handle = None;
    let mut forward_proxy_handle = None;
    let mut pool_orphan_recovery_handle = None;
    let mut retention_handle = None;
    let mut server_handle = None;
    let mut startup_backfill_handle = None;

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
        )
        .await;
    }

    let scheduler_stage = run_startup_stage_until_shutdown(&shutdown_signal, &cancel, async {
        if state.config.crs_stats.is_some() {
            Some(spawn_scheduler(state.clone(), cancel.clone()))
        } else {
            info!("crs stats relay is disabled; scheduler will not start");
            None
        }
    })
    .await;
    let scheduler_shutdown_requested = match scheduler_stage {
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
            )
            .await;
        }
        StartupStageOutcome::Completed {
            result,
            shutdown_requested,
        } => {
            poller_handle = result;
            shutdown_requested
        }
    };
    if scheduler_shutdown_requested {
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
        )
        .await;
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
        )
        .await;
    }

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
        )
        .await;
    }

    state.startup_ready.store(true, Ordering::Release);
    log_startup_phase("http_ready", http_ready_started_at);
    info!(
        time_to_health_ms = startup_started_at.elapsed().as_millis() as u64,
        "application readiness reached"
    );

    tokio::select! {
        biased;
        _ = shutdown_signal => begin_runtime_shutdown(&cancel),
        _ = cancel.cancelled() => {}
    }

    drain_runtime_after_pending_shutdown(
        state,
        shutdown_watcher,
        server_handle,
        poller_handle,
        upstream_accounts_handle,
        forward_proxy_handle,
        pool_orphan_recovery_handle,
        retention_handle,
        startup_backfill_handle,
    )
    .await
}

fn begin_runtime_shutdown(cancel: &CancellationToken) {
    if !cancel.is_cancelled() {
        info!("shutdown signal received; beginning graceful shutdown");
        cancel.cancel();
    }
}

async fn drain_scheduler_inflight(mut inflight: Vec<JoinHandle<()>>) {
    inflight.retain(|handle| !handle.is_finished());
    for handle in inflight {
        let _ = handle.await;
    }
}

async fn drain_runtime_after_shutdown(
    state: Arc<AppState>,
    server_handle: Option<JoinHandle<()>>,
    poller_handle: Option<JoinHandle<()>>,
    upstream_accounts_handle: Option<JoinHandle<()>>,
    forward_proxy_handle: Option<JoinHandle<()>>,
    pool_orphan_recovery_handle: Option<JoinHandle<()>>,
    retention_handle: Option<JoinHandle<()>>,
    startup_backfill_handle: Option<JoinHandle<()>>,
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

fn spawn_scheduler(state: Arc<AppState>, cancel: CancellationToken) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut inflight: Vec<JoinHandle<()>> = Vec::new();
        if cancel.is_cancelled() {
            info!("scheduler startup skipped because shutdown is already in progress");
            return;
        }
        match schedule_poll(state.clone(), &cancel).await {
            Ok(Some(handle)) => inflight.push(handle),
            Ok(None) => {
                info!("scheduler startup skipped because shutdown is already in progress");
                return;
            }
            Err(err) => warn!(?err, "initial poll failed"),
        }

        let mut ticker = interval(state.config.poll_interval);
        ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);

        loop {
            tokio::select! {
                _ = cancel.cancelled() => {
                    info!("scheduler received shutdown; waiting for in-flight polls");
                    drain_scheduler_inflight(inflight).await;
                    break;
                }
                _ = ticker.tick() => {
                    match schedule_poll(state.clone(), &cancel).await {
                        Ok(Some(handle)) => {
                            inflight.push(handle);
                            inflight.retain(|handle| !handle.is_finished());
                        }
                        Ok(None) => {
                            info!("scheduler received shutdown while waiting to start a new poll; waiting for in-flight polls");
                            drain_scheduler_inflight(inflight).await;
                            break;
                        }
                        Err(err) => {
                            warn!(?err, "scheduled poll failed");
                        }
                    }
                }
            }
        }
    })
}

fn spawn_forward_proxy_maintenance(
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
        if let Err(err) = refresh_forward_proxy_subscriptions(
            state.clone(),
            true,
            Some(startup_known_subscription_keys),
        )
        .await
        {
            warn!(error = %err, "failed to refresh forward proxy subscriptions at startup");
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
                    if let Err(err) = refresh_forward_proxy_subscriptions(state.clone(), false, None).await {
                        warn!(error = %err, "failed to refresh forward proxy subscriptions");
                    }
                }
            }
        }
    })
}

fn spawn_pool_orphan_recovery_maintenance(
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
