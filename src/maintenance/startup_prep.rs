use super::*;

#[cfg(test)]
#[derive(Debug, Default)]
pub(crate) struct StartupPersistentPrepInterleavingTestHooks {
    pub(crate) manifest_finished: tokio::sync::Notify,
    pub(crate) continue_after_manifest: tokio::sync::Notify,
}

#[cfg(test)]
static STARTUP_PERSISTENT_PREP_INTERLEAVING_HOOKS: std::sync::OnceLock<
    std::sync::Mutex<Option<std::sync::Arc<StartupPersistentPrepInterleavingTestHooks>>>,
> = std::sync::OnceLock::new();

#[cfg(test)]
pub(crate) fn install_startup_persistent_prep_interleaving_hooks()
-> std::sync::Arc<StartupPersistentPrepInterleavingTestHooks> {
    let hooks = std::sync::Arc::new(StartupPersistentPrepInterleavingTestHooks::default());
    *STARTUP_PERSISTENT_PREP_INTERLEAVING_HOOKS
        .get_or_init(|| std::sync::Mutex::new(None))
        .lock()
        .expect("startup prep interleaving hooks lock") = Some(std::sync::Arc::clone(&hooks));
    hooks
}

#[cfg(test)]
pub(crate) fn clear_startup_persistent_prep_interleaving_hooks() {
    *STARTUP_PERSISTENT_PREP_INTERLEAVING_HOOKS
        .get_or_init(|| std::sync::Mutex::new(None))
        .lock()
        .expect("startup prep interleaving hooks lock") = None;
}

#[cfg(test)]
async fn pause_after_startup_persistent_prep_manifest() {
    let Some(hooks) = STARTUP_PERSISTENT_PREP_INTERLEAVING_HOOKS
        .get_or_init(|| std::sync::Mutex::new(None))
        .lock()
        .expect("startup prep interleaving hooks lock")
        .clone()
    else {
        return;
    };
    hooks.manifest_finished.notify_one();
    hooks.continue_after_manifest.notified().await;
}

#[derive(Debug, Default)]
pub(crate) struct StartupPersistentPrepSummary {
    pub(crate) deferred: bool,
    pub(crate) stale_archive_temp_files_removed: usize,
    pub(crate) refreshed_manifest_batches: usize,
    pub(crate) refreshed_manifest_account_rows: usize,
    pub(crate) missing_manifest_files: usize,
    pub(crate) backfilled_archive_expiries: usize,
    pub(crate) bootstrapped_hourly_rollups: bool,
    pub(crate) pending_manifest_batches: usize,
    pub(crate) pending_historical_rollup_archive_batches: usize,
}

#[derive(Debug, Default)]
pub(crate) struct StatsMaintenanceCacheState {
    pub(crate) cached_at: Option<Instant>,
    pub(crate) response: Option<StatsMaintenanceResponse>,
}

impl StatsMaintenanceCacheState {
    pub(crate) fn fresh_response(&self) -> Option<StatsMaintenanceResponse> {
        let cached_at = self.cached_at?;
        if cached_at.elapsed() > Duration::from_secs(STATS_MAINTENANCE_CACHE_TTL_SECS) {
            return None;
        }
        self.response.clone()
    }

    pub(crate) fn store(&mut self, response: StatsMaintenanceResponse) {
        self.cached_at = Some(Instant::now());
        self.response = Some(response);
    }
}

pub(crate) fn should_run_startup_persistent_prep(cli: &CliArgs) -> bool {
    if cli.command.is_some() {
        return false;
    }
    if cli.retention_run_once {
        return !cli.retention_dry_run;
    }
    true
}

pub(crate) fn should_run_blocking_startup_persistent_prep(cli: &CliArgs) -> bool {
    cli.command.is_none() && cli.retention_run_once && !cli.retention_dry_run
}

pub(crate) fn should_spawn_background_startup_hourly_rollup_bootstrap(cli: &CliArgs) -> bool {
    cli.command.is_none() && !cli.retention_run_once && !cli.retention_dry_run
}

pub(crate) async fn run_startup_persistent_prep_inner(
    pool: &Pool<Sqlite>,
    config: &AppConfig,
    cli: &CliArgs,
    include_hourly_rollup_bootstrap: bool,
) -> Result<StartupPersistentPrepSummary> {
    run_startup_persistent_prep_inner_with_store(
        None,
        pool,
        config,
        cli,
        include_hourly_rollup_bootstrap,
    )
    .await
}

pub(crate) async fn run_startup_persistent_prep_inner_with_store(
    maintenance_store: Option<&crate::maintenance_store::MaintenanceStore>,
    pool: &Pool<Sqlite>,
    config: &AppConfig,
    cli: &CliArgs,
    include_hourly_rollup_bootstrap: bool,
) -> Result<StartupPersistentPrepSummary> {
    if !should_run_startup_persistent_prep(cli) {
        return Ok(StartupPersistentPrepSummary::default());
    }

    let migration_hold_signal = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let mut prep_future = Box::pin(crate::maintenance::with_migration_hold_signal(
        std::sync::Arc::clone(&migration_hold_signal),
        run_startup_persistent_prep_inner_with_store_unwatched(
            maintenance_store,
            pool,
            config,
            include_hourly_rollup_bootstrap,
        ),
    ));
    let result = if let Some(store) = maintenance_store {
        tokio::select! {
            biased;
            result = &mut prep_future => result,
            hold_result = wait_for_startup_persistent_prep_migration_hold(store) => {
                migration_hold_signal.store(true, std::sync::atomic::Ordering::Release);
                match hold_result {
                    Ok(()) => (&mut prep_future).await,
                    Err(error) => {
                        let _ = (&mut prep_future).await;
                        Err(error)
                    }
                }
            }
        }
    } else {
        prep_future.await
    }?;

    let mut result = result;
    if migration_hold_signal.load(std::sync::atomic::Ordering::Acquire) {
        result.deferred = true;
    }
    if let Some(store) = maintenance_store
        && store
            .migration_task_is_held(crate::maintenance_store::STARTUP_PERSISTENT_PREP_TASK_KEY)
            .await?
    {
        result.deferred = true;
    }
    Ok(result)
}

async fn wait_for_startup_persistent_prep_migration_hold(
    store: &crate::maintenance_store::MaintenanceStore,
) -> Result<()> {
    loop {
        if store
            .migration_task_is_held(crate::maintenance_store::STARTUP_PERSISTENT_PREP_TASK_KEY)
            .await?
        {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

fn startup_persistent_prep_deferred_summary() -> StartupPersistentPrepSummary {
    StartupPersistentPrepSummary {
        deferred: true,
        ..StartupPersistentPrepSummary::default()
    }
}

async fn startup_persistent_prep_hold_is_active(
    maintenance_store: Option<&crate::maintenance_store::MaintenanceStore>,
) -> Result<bool> {
    if crate::maintenance::migration_hold_signal_reached() {
        return Ok(true);
    }
    let Some(store) = maintenance_store else {
        return Ok(false);
    };
    store
        .migration_task_is_held(crate::maintenance_store::STARTUP_PERSISTENT_PREP_TASK_KEY)
        .await
}

async fn run_startup_persistent_prep_inner_with_store_unwatched(
    maintenance_store: Option<&crate::maintenance_store::MaintenanceStore>,
    pool: &Pool<Sqlite>,
    config: &AppConfig,
    include_hourly_rollup_bootstrap: bool,
) -> Result<StartupPersistentPrepSummary> {
    if startup_persistent_prep_hold_is_active(maintenance_store).await? {
        return Ok(startup_persistent_prep_deferred_summary());
    }

    let janitor_summary = cleanup_stale_archive_temp_files(config, false)?;
    if startup_persistent_prep_hold_is_active(maintenance_store).await? {
        return Ok(startup_persistent_prep_deferred_summary());
    }
    let manifest_refresh = refresh_archive_upstream_activity_manifest(pool, config, false).await?;
    #[cfg(test)]
    pause_after_startup_persistent_prep_manifest().await;
    if startup_persistent_prep_hold_is_active(maintenance_store).await? {
        return Ok(startup_persistent_prep_deferred_summary());
    }
    let archive_expiry_backfill_count = backfill_invocation_archive_expiries(pool, config).await?;
    if include_hourly_rollup_bootstrap {
        if startup_persistent_prep_hold_is_active(maintenance_store).await? {
            return Ok(startup_persistent_prep_deferred_summary());
        }
        bootstrap_hourly_rollups_with_parallel_work_coverage(
            pool,
            Some(config.invocation_max_days),
        )
        .await?;
        if startup_persistent_prep_hold_is_active(maintenance_store).await? {
            return Ok(startup_persistent_prep_deferred_summary());
        }
        ensure_invocation_summary_rollups_ready_best_effort(pool).await?;
    }
    let historical_rollup_pending_hint = count_historical_rollup_startup_pending_hint(pool).await?;

    Ok(StartupPersistentPrepSummary {
        deferred: false,
        stale_archive_temp_files_removed: janitor_summary.stale_temp_files_removed,
        refreshed_manifest_batches: manifest_refresh.refreshed_batches,
        refreshed_manifest_account_rows: manifest_refresh.account_rows_written,
        missing_manifest_files: manifest_refresh.missing_files,
        backfilled_archive_expiries: archive_expiry_backfill_count,
        bootstrapped_hourly_rollups: include_hourly_rollup_bootstrap,
        pending_manifest_batches: manifest_refresh
            .pending_batches
            .saturating_sub(manifest_refresh.refreshed_batches)
            .max(manifest_refresh.candidate_remaining_hint),
        pending_historical_rollup_archive_batches: historical_rollup_pending_hint
            .pending_archive_batches,
    })
}

pub(crate) async fn run_startup_persistent_prep(
    pool: &Pool<Sqlite>,
    config: &AppConfig,
    cli: &CliArgs,
) -> Result<StartupPersistentPrepSummary> {
    run_startup_persistent_prep_inner(pool, config, cli, true).await
}
