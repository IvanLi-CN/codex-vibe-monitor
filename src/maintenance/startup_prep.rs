use super::*;

#[derive(Debug, Default)]
pub(crate) struct StartupPersistentPrepSummary {
    pub(crate) stale_archive_temp_files_removed: usize,
    pub(crate) refreshed_manifest_batches: usize,
    pub(crate) refreshed_manifest_account_rows: usize,
    pub(crate) missing_manifest_files: usize,
    pub(crate) backfilled_archive_expiries: usize,
    pub(crate) bootstrapped_hourly_rollups: bool,
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

pub(crate) fn should_run_blocking_startup_hourly_rollup_bootstrap(cli: &CliArgs) -> bool {
    cli.command.is_none() && !cli.retention_run_once && !cli.retention_dry_run
}

pub(crate) async fn run_startup_persistent_prep_inner(
    pool: &Pool<Sqlite>,
    config: &AppConfig,
    cli: &CliArgs,
    include_hourly_rollup_bootstrap: bool,
) -> Result<StartupPersistentPrepSummary> {
    if !should_run_startup_persistent_prep(cli) {
        return Ok(StartupPersistentPrepSummary::default());
    }

    let janitor_summary = cleanup_stale_archive_temp_files(config, false)?;
    let manifest_refresh = refresh_archive_upstream_activity_manifest(pool, false).await?;
    let archive_expiry_backfill_count = backfill_invocation_archive_expiries(pool, config).await?;
    if include_hourly_rollup_bootstrap {
        bootstrap_hourly_rollups(pool).await?;
        ensure_invocation_summary_rollups_ready_best_effort(pool).await?;
    }
    let historical_rollup_snapshot = load_historical_rollup_backfill_snapshot(pool, config).await?;

    Ok(StartupPersistentPrepSummary {
        stale_archive_temp_files_removed: janitor_summary.stale_temp_files_removed,
        refreshed_manifest_batches: manifest_refresh.refreshed_batches,
        refreshed_manifest_account_rows: manifest_refresh.account_rows_written,
        missing_manifest_files: manifest_refresh.missing_files,
        backfilled_archive_expiries: archive_expiry_backfill_count,
        bootstrapped_hourly_rollups: include_hourly_rollup_bootstrap,
        pending_historical_rollup_archive_batches: historical_rollup_snapshot.legacy_archive_pending
            as usize,
    })
}

pub(crate) async fn run_startup_persistent_prep(
    pool: &Pool<Sqlite>,
    config: &AppConfig,
    cli: &CliArgs,
) -> Result<StartupPersistentPrepSummary> {
    run_startup_persistent_prep_inner(pool, config, cli, true).await
}
