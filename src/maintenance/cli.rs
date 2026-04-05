async fn run_cli_command(
    pool: &Pool<Sqlite>,
    config: &AppConfig,
    command: &CliCommand,
) -> Result<()> {
    let raw_path_fallback_root = config.database_path.parent();
    match command {
        CliCommand::Maintenance(args) => match &args.command {
            MaintenanceCommand::RawCompression(opts) => {
                let summary = compress_cold_proxy_raw_payloads_with_budget(
                    pool,
                    config,
                    raw_path_fallback_root,
                    opts.dry_run,
                    None,
                )
                .await?;
                let backlog = load_raw_compression_backlog_snapshot(pool, config).await?;
                info!(
                    dry_run = opts.dry_run,
                    ?summary,
                    ?backlog,
                    "maintenance raw compression finished"
                );
            }
            MaintenanceCommand::ArchiveUpstreamActivityManifest(opts) => {
                let summary =
                    refresh_archive_upstream_activity_manifest(pool, opts.dry_run).await?;
                info!(
                    dry_run = opts.dry_run,
                    ?summary,
                    "maintenance archive upstream activity manifest finished"
                );
            }
            MaintenanceCommand::MaterializeHistoricalRollups(opts) => {
                let summary = materialize_historical_rollups(pool, config, opts.dry_run).await?;
                let snapshot = load_historical_rollup_backfill_snapshot(pool, config).await?;
                info!(
                    dry_run = opts.dry_run,
                    ?summary,
                    ?snapshot,
                    "maintenance historical rollup materialization finished"
                );
            }
            MaintenanceCommand::VerifyArchiveStorage(opts) => {
                let summary = verify_archive_storage(pool, config).await?;
                info!(
                    dry_run = opts.dry_run,
                    manifest_rows = summary.manifest_rows,
                    missing_files = summary.missing_files,
                    orphan_files = summary.orphan_files,
                    stale_temp_files = summary.stale_temp_files,
                    stale_temp_bytes = summary.stale_temp_bytes,
                    "maintenance archive storage verification finished"
                );
            }
            MaintenanceCommand::PruneArchiveBatches(opts) => {
                let summary = prune_archive_batches(pool, config, opts.dry_run).await?;
                let snapshot = load_historical_rollup_backfill_snapshot(pool, config).await?;
                info!(
                    dry_run = opts.dry_run,
                    expired_archive_batches_deleted = summary.expired_archive_batches_deleted,
                    legacy_archive_batches_deleted = summary.legacy_archive_batches_deleted,
                    ?snapshot,
                    "maintenance archive prune finished"
                );
            }
            MaintenanceCommand::PruneLegacyArchiveBatches(opts) => {
                let summary = prune_legacy_archive_batches(pool, config, opts.dry_run).await?;
                let snapshot = load_historical_rollup_backfill_snapshot(pool, config).await?;
                info!(
                    dry_run = opts.dry_run,
                    ?summary,
                    ?snapshot,
                    "maintenance legacy archive prune finished"
                );
            }
        },
    }
    Ok(())
}
