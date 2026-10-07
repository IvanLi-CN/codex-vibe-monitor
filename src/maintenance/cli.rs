use super::*;

pub(crate) async fn run_cli_command(
    pool: &Pool<Sqlite>,
    config: &AppConfig,
    command: &CliCommand,
) -> Result<()> {
    let raw_path_fallback_root = config.database_path.parent();
    match command {
        CliCommand::Maintenance(args) => match &args.command {
            MaintenanceCommand::InvocationIdentityCleanup(_)
            | MaintenanceCommand::RawOrphanSweep(_) => {
                bail!("owned maintenance must select its runtime route before startup")
            }
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
                    refresh_archive_upstream_activity_manifest(pool, config, opts.dry_run).await?;
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

pub(crate) fn selected_owned_maintenance(cli: &CliArgs) -> Result<Option<(&'static str, bool)>> {
    if (cli.retention_run_once || cli.retention_dry_run) && cli.command.is_some() {
        bail!("retention flags cannot be combined with maintenance subcommands");
    }
    if cli.retention_run_once || (cli.retention_dry_run && cli.command.is_none()) {
        return Ok(Some(("retention_archive", cli.retention_dry_run)));
    }
    Ok(match cli.command.as_ref() {
        Some(CliCommand::Maintenance(MaintenanceCliArgs {
            command: MaintenanceCommand::InvocationIdentityCleanup(options),
        })) => Some(("invocation_identity_cleanup", options.dry_run)),
        Some(CliCommand::Maintenance(MaintenanceCliArgs {
            command: MaintenanceCommand::RawOrphanSweep(options),
        })) => Some(("raw_orphan_sweep", options.dry_run)),
        _ => None,
    })
}

pub(crate) async fn run_owned_maintenance_cli(
    config: &AppConfig,
    task_key: &str,
    dry_run: bool,
) -> Result<()> {
    let route = MaintenanceRuntimeLock::route(config, true)?;
    match route {
        MaintenanceRuntimeRoute::Online => {
            let store = crate::maintenance_store::connect_existing(config).await
                .context("maintenance unavailable: cannot connect to the running service's maintenance store")?;
            let id = store.request_run_with_mode(task_key, dry_run).await?;
            let deadline = Instant::now() + Duration::from_secs(15 * 60);
            loop {
                let row: (String, Option<String>, Option<String>) = sqlx::query_as(
                    "SELECT status,details,error_detail FROM managed_task_runs WHERE id=?",
                )
                .bind(id)
                .fetch_one(&store.pool)
                .await?;
                if !matches!(row.0.as_str(), "requested" | "running") {
                    if !matches!(row.0.as_str(), "success" | "skipped") {
                        bail!(
                            "maintenance request {id} ended with {}: {}",
                            row.0,
                            row.2.as_deref().unwrap_or("see task history")
                        );
                    }
                    println!("{}", row.1.unwrap_or_else(|| "{}".to_string()));
                    return Ok(());
                }
                if Instant::now() >= deadline {
                    bail!("maintenance request {id} is still pending; inspect task history");
                }
                if !matches!(
                    MaintenanceRuntimeLock::route(config, true)?,
                    MaintenanceRuntimeRoute::Online
                ) {
                    bail!("maintenance unavailable: service stopped before request {id} completed");
                }
                sleep(Duration::from_millis(250)).await;
            }
        }
        MaintenanceRuntimeRoute::Offline(mut runtime_lock) => {
            runtime_lock.publish_role("cli:ownership-v1")?;
            let pool = SqlitePoolOptions::new()
                .max_connections(3)
                .connect_with(build_sqlite_connect_options(
                    &config.database_url(),
                    Duration::from_secs(DEFAULT_SQLITE_BUSY_TIMEOUT_SECS),
                )?)
                .await?;
            let mut maintenance_pool = None;
            let mut task_lease = None;
            let outcome = async {
                ensure_schema(&pool).await?;
                initialize_invocation_identity_cleanup_state(&pool).await?;
                let store = crate::maintenance_store::open(config).await?;
                maintenance_pool = Some(store.pool.clone());
                store.migrate_legacy_state(&pool).await?;
                store.apply_initial_task_defaults().await?;
                // Only this selected owner is recovered; no other automatic maintenance is started.
                sqlx::query("UPDATE managed_task_runs SET status='failed',finished_at=?,error_detail='offline runtime recovery' WHERE task_key=? AND status='running'")
                    .bind(format_utc_iso_millis(Utc::now())).bind(task_key).execute(&store.pool).await?;
                let id = store.request_run_with_mode(task_key, dry_run).await?;
                sqlx::query(
                    "UPDATE managed_task_runs SET status='running' WHERE id=? AND status='requested'",
                )
                .bind(id)
                .execute(&store.pool)
            .await?;
            crate::maintenance_store::set_global(Arc::new(store.clone()));
            crate::task_timeline::start_recorder(Arc::new(store.clone())).await;
                task_lease = Some(crate::maintenance_store::try_acquire_task_execution(task_key)
                    .context("maintenance owner already running")?);
                let cache = Arc::new(Mutex::new(PromptCacheConversationsCacheState::default()));
                let shutdown = CancellationToken::new();
                let traversal = Mutex::new(RetentionRawDirectoryTraversal::default());
                let observation = crate::TaskExecutionObservation::begin_for_managed_run(
                    task_key,
                    &crate::maintenance_store::task_title_for_observation(task_key),
                    "manual",
                    crate::maintenance_store::task_execution_class(task_key),
                    "waiting_resources",
                    Some(id),
                );
                let started = Instant::now();
                let options = MaintenanceExecutionOptions {
                    manual: true,
                    admitted: true,
                    dry_run,
                };
                let result = with_maintenance_execution_options(
                    options,
                    crate::with_managed_task_observation(
                        observation.clone(),
                        execute_owned_maintenance(
                            OwnedMaintenanceContext {
                                pool: &pool,
                                config,
                                cache: &cache,
                                circuit: Arc::new(RawCaptureCircuitBreaker::new(
                                    config.resolved_proxy_raw_dir(),
                                )),
                                shutdown: &shutdown,
                                raw_traversal: &traversal,
                            },
                            task_key,
                            options,
                            Some(observation.clone()),
                        ),
                    ),
                )
                .await;
                let finished_at = format_utc_iso_millis(Utc::now());
                match result {
                    Ok(result) => {
                        let status = match result.completion {
                            "failed" => "failed",
                            "deferred" => "skipped",
                            _ => "success",
                        };
                        observation.finish_with_status_and_reason(status, None);
                        let finish = store
                            .finish_run_with_observation(
                                id,
                                status,
                                &finished_at,
                                started.elapsed().as_millis() as i64,
                                Some(&result.summary),
                                None,
                                Some(result.completion),
                                None,
                                Some(&result.details),
                            )
                            .await;
                        println!("{}", result.details);
                        finish?;
                        if status == "failed" {
                            bail!("maintenance owner failed; see the recorded task result");
                        }
                    }
                    Err(error) => {
                        observation.finish_with_status_and_reason("failed", Some("execution_failed"));
                        let finish = store
                            .finish_run(
                                id,
                                "failed",
                                &finished_at,
                                started.elapsed().as_millis() as i64,
                                None,
                                Some(&error.to_string()),
                            )
                            .await;
                        finish?;
                        return Err(error);
                    }
                }
                Ok(())
            }
            .await;
            // Initialization failures must drain workers too, before either execution lock drops.
            crate::task_timeline::drain_after_shutdown().await;
            pool.close().await;
            if let Some(maintenance_pool) = maintenance_pool {
                maintenance_pool.close().await;
            }
            drop(task_lease);
            outcome
        }
    }
}
