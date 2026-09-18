pub(crate) async fn run_data_retention_maintenance_best_effort(
    state: &Arc<AppState>,
    cancel: &CancellationToken,
    trigger: &'static str,
) -> bool {
    let _ = match run_data_retention_maintenance(&state.pool, &state.config, None, Some(cancel))
        .await
    {
        Ok(summary) => handle_retention_success(state, cancel, trigger, summary).await,
        Err(err) => handle_retention_failure(state, cancel, trigger, err).await,
    };

    // Hourly rollups run through their own P2 scheduler. Retention used to invoke a
    // full refresh here after every committed batch, creating an uncoordinated long
    // write immediately after the maintenance micro-transaction released its permit.
    // Archive materialization already wakes the targeted repair path above.
    true
}

async fn handle_retention_success(
    state: &Arc<AppState>,
    cancel: &CancellationToken,
    trigger: &'static str,
    summary: RetentionRunSummary,
) -> bool {
    if summary.deferred {
        debug!(
            trigger,
            "retention maintenance deferred; preserving the prompt retry schedule"
        );
        invalidate_system_status_cache(state.as_ref()).await;
        return false;
    }
    let touched_anything = summary.touched_anything();
    if touched_anything && !summary.dry_run {
        let task_run = tokio::select! {
            biased;
            _ = cancel.cancelled() => return false,
            result = begin_system_task_run_admitted(
                state.as_ref(),
                crate::proxy_sqlite_write_coordinator::ProxySqliteWriteClass::MaintenanceRetention,
                SystemTaskKind::RetentionArchive,
                trigger,
                Some("retention maintenance completed a write pass".to_string()),
            ) => result.ok(),
        };
        if let Some(handle) = task_run.as_ref() {
            let (brief, detail) = summarize_retention_run_for_system_task(&summary);
            let _ = finish_system_task_run_reliably(
                state.as_ref(),
                Some(cancel),
                handle,
                SystemTaskStatus::Success,
                Some(brief),
                Some(detail),
            )
            .await;
        }
    }
    handle_retention_inventory_reset(state, trigger, &summary).await
}

async fn handle_retention_inventory_reset(
    state: &Arc<AppState>,
    trigger: &'static str,
    summary: &RetentionRunSummary,
) -> bool {
    let reset_pending =
        match crate::system_raw_payload_metrics_inventory_reset_pending(&state.pool).await {
            Ok(pending) => pending,
            Err(error) => {
                warn!(
                    trigger,
                    error = %error,
                    "failed to inspect system raw metrics inventory reset state"
                );
                invalidate_system_status_cache(state.as_ref()).await;
                return false;
            }
        };
    if summary.raw_files_compressed > 0
        || summary.raw_files_removed > 0
        || summary.orphan_raw_files_removed > 0
        || reset_pending
    {
        match reset_retention_raw_payload_metrics_inventory(state.as_ref()).await {
            Ok(true) => {}
            Ok(false) => {
                debug!(
                    trigger,
                    "system raw metrics inventory reset deferred; preserving retry schedule"
                );
                invalidate_system_status_cache(state.as_ref()).await;
                return false;
            }
            Err(error) => {
                warn!(error = %error, "failed to reset system raw metrics inventory after retention");
                invalidate_system_status_cache(state.as_ref()).await;
                return false;
            }
        }
    }
    invalidate_system_status_cache(state.as_ref()).await;
    summary.touched_anything()
}

async fn handle_retention_failure(
    state: &Arc<AppState>,
    cancel: &CancellationToken,
    trigger: &'static str,
    err: anyhow::Error,
) -> bool {
    let pressure_error = crate::db_pressure::global_db_pressure_gate()
        .record_error("data_retention_maintenance", &err);
    retention_record_error("data_retention_maintenance", &err);
    if !state.config.retention_dry_run {
        let task_run = tokio::select! {
            biased;
            _ = cancel.cancelled() => return false,
            result = begin_system_task_run_admitted(
                state.as_ref(),
                crate::proxy_sqlite_write_coordinator::ProxySqliteWriteClass::MaintenanceRetention,
                SystemTaskKind::RetentionArchive,
                trigger,
                Some("retention maintenance failed".to_string()),
            ) => result.ok(),
        };
        if let Some(handle) = task_run.as_ref() {
            let _ = finish_system_task_run_reliably(
                state.as_ref(),
                Some(cancel),
                handle,
                SystemTaskStatus::Failed,
                Some("retention maintenance failed".to_string()),
                Some(err.to_string()),
            )
            .await;
        }
    }
    warn!(trigger, error = %err, retry_soon = pressure_error, "failed to run retention maintenance");
    !pressure_error
}

pub(crate) fn should_stop_data_retention_maintenance(shutdown: Option<&CancellationToken>) -> bool {
    let should_stop = shutdown.is_some_and(CancellationToken::is_cancelled);
    if should_stop {
        info!(
            "data retention maintenance stopped at a safe boundary because shutdown is in progress"
        );
    }
    should_stop
}

pub(crate) async fn run_data_retention_maintenance(
    pool: &Pool<Sqlite>,
    config: &AppConfig,
    dry_run_override: Option<bool>,
    shutdown: Option<&CancellationToken>,
) -> Result<RetentionRunSummary> {
    let defer_generation = retention_defer_generation();
    let result = if let Some(shutdown) = shutdown {
        RETENTION_SHUTDOWN
            .scope(
                shutdown.clone(),
                run_data_retention_maintenance_with_task_run_prune(
                    pool,
                    config,
                    dry_run_override,
                    Some(shutdown),
                ),
            )
            .await
    } else {
        run_data_retention_maintenance_with_task_run_prune(pool, config, dry_run_override, None)
            .await
    };
    result.map(|mut summary| {
        summary.deferred = retention_defer_generation() != defer_generation;
        summary
    })
}

async fn run_data_retention_maintenance_with_task_run_prune(
    pool: &Pool<Sqlite>,
    config: &AppConfig,
    dry_run_override: Option<bool>,
    shutdown: Option<&CancellationToken>,
) -> Result<RetentionRunSummary> {
    let dry_run = dry_run_override.unwrap_or(config.retention_dry_run);
    let result =
        run_data_retention_maintenance_inner(pool, config, dry_run_override, shutdown).await;
    // This janitor must run even when an earlier archive stage fails. Otherwise every
    // failed pass adds a task-run record while the retention policy that bounds those
    // records is unreachable until the unrelated failure clears.
    let task_run_prune = if should_stop_data_retention_maintenance(shutdown) {
        Ok(0)
    } else {
        prune_system_task_runs(pool, dry_run).await
    };
    match result {
        Ok(mut summary) => {
            summary.system_task_run_rows_pruned +=
                task_run_prune.context("failed to prune expired system task runs")?;
            Ok(summary)
        }
        Err(error) => {
            if let Err(prune_error) = task_run_prune {
                warn!(error = %prune_error, "failed to prune system task runs after retention failure");
            }
            Err(error)
        }
    }
}

async fn run_data_retention_maintenance_inner(
    pool: &Pool<Sqlite>,
    config: &AppConfig,
    dry_run_override: Option<bool>,
    shutdown: Option<&CancellationToken>,
) -> Result<RetentionRunSummary> {
    let dry_run = dry_run_override.unwrap_or(config.retention_dry_run);
    let mut summary = RetentionRunSummary {
        dry_run,
        ..RetentionRunSummary::default()
    };
    let raw_path_fallback_root = config.database_path.parent();

    if !dry_run {
        // Hourly rollups are a separately scheduled P2 projection. Retention only checks its
        // coverage gate below; rebuilding it here used to add an unbounded write phase before
        // every archive pass.
        let janitor = cleanup_stale_archive_temp_files(config, false)?;
        if janitor.stale_temp_files_removed > 0 {
            info!(
                ?janitor,
                "archive temp janitor removed stale files before retention"
            );
        }
    }

    if should_stop_data_retention_maintenance(shutdown) {
        return Ok(summary);
    }

    if !run_model_route_retention(pool, config, dry_run, &mut summary).await? {
        return Ok(summary);
    }

    run_raw_compression_retention(pool, config, raw_path_fallback_root, dry_run, &mut summary)
        .await?;

    if should_stop_data_retention_maintenance(shutdown) {
        return Ok(summary);
    }

    let (invocation_payload_retention_ready, pruned_archive_batches) =
        run_invocation_detail_retention(
            pool,
            config,
            raw_path_fallback_root,
            dry_run,
            &mut summary,
        )
        .await?;

    if should_stop_data_retention_maintenance(shutdown) {
        return Ok(summary);
    }

    run_invocation_archive_retention(
        pool,
        config,
        raw_path_fallback_root,
        dry_run,
        invocation_payload_retention_ready,
        pruned_archive_batches,
        &mut summary,
    )
    .await?;

    if should_stop_data_retention_maintenance(shutdown) {
        return Ok(summary);
    }

    run_remaining_retention_stages(
        pool,
        config,
        raw_path_fallback_root,
        dry_run,
        shutdown,
        &mut summary,
    )
    .await?;

    info!(
        dry_run = summary.dry_run,
        ?summary,
        "data retention maintenance finished"
    );
    Ok(summary)
}

async fn run_model_route_retention(
    pool: &Pool<Sqlite>,
    config: &AppConfig,
    dry_run: bool,
    summary: &mut RetentionRunSummary,
) -> Result<bool> {
    if dry_run {
        summary.model_route_rows_pruned =
            crate::upstream_accounts::count_expired_model_routes(pool).await? as usize;
        return Ok(true);
    }
    let Some(admission) = acquire_retention_write_admission("model_route_purge").await else {
        return Ok(false);
    };
    let execute_started = Instant::now();
    let candidate_limit = retention_candidate_limit(config, "model_route_purge");
    summary.model_route_rows_pruned =
        crate::upstream_accounts::purge_model_routes_bounded(pool, candidate_limit).await? as usize;
    retention_record_commit!(
        "model_route_purge",
        admission.admission_mode(),
        summary.model_route_rows_pruned,
        summary.model_route_rows_pruned.saturating_mul(128),
        Duration::ZERO,
        admission.lock_wait(),
        execute_started.elapsed(),
        Duration::ZERO,
        admission.p1_waiter_count,
        usize::from(summary.model_route_rows_pruned >= candidate_limit),
    );
    drop(admission);
    Ok(true)
}

async fn run_raw_compression_retention(
    pool: &Pool<Sqlite>,
    config: &AppConfig,
    raw_path_fallback_root: Option<&Path>,
    dry_run: bool,
    summary: &mut RetentionRunSummary,
) -> Result<()> {
    let raw_compression =
        compress_cold_proxy_raw_payloads(pool, config, raw_path_fallback_root, dry_run)
            .await
            .context("failed to compress cold proxy raw payloads during retention")?;
    summary.raw_files_compression_candidates += raw_compression.files_considered;
    summary.raw_files_compressed += raw_compression.files_compressed;
    summary.raw_bytes_before += raw_compression.bytes_before;
    summary.raw_bytes_after += raw_compression.bytes_after;
    summary.raw_bytes_after_estimated += raw_compression.estimated_bytes_after;
    if !dry_run {
        log_raw_compression_backlog_if_needed(pool, config).await?;
    }
    Ok(())
}

async fn run_invocation_detail_retention(
    pool: &Pool<Sqlite>,
    config: &AppConfig,
    raw_path_fallback_root: Option<&Path>,
    dry_run: bool,
    summary: &mut RetentionRunSummary,
) -> Result<(bool, usize)> {
    let payload_loss_days = config
        .invocation_success_full_days
        .min(config.invocation_max_days);
    let ready = dry_run
        || parallel_work_minute_coverage_ready_for_payload_retention(
            pool,
            shanghai_retention_cutoff(payload_loss_days).timestamp(),
        )
        .await
        .context("failed to verify parallel-work minute coverage before invocation retention")?;
    if ready {
        let pruned = prune_old_invocation_details(pool, config, raw_path_fallback_root, dry_run)
            .await
            .context("failed to prune old invocation details during retention")?;
        summary.invocation_details_pruned += pruned.0;
        summary.archive_batches_touched += pruned.1;
        summary.raw_files_removed += pruned.2;
        return Ok((true, pruned.1));
    }
    info!(
        payload_loss_days,
        "invocation detail pruning deferred until parallel-work minute coverage catches up"
    );
    Ok((false, 0))
}

async fn run_invocation_archive_retention(
    pool: &Pool<Sqlite>,
    config: &AppConfig,
    raw_path_fallback_root: Option<&Path>,
    dry_run: bool,
    retention_ready: bool,
    pruned_archive_batches: usize,
    summary: &mut RetentionRunSummary,
) -> Result<()> {
    let payload_loss_days = config
        .invocation_success_full_days
        .min(config.invocation_max_days);
    let invocation_archive = if retention_ready {
        archive_old_invocations(pool, config, raw_path_fallback_root, dry_run)
            .await
            .context("failed to archive old invocations during retention")?
    } else {
        info!(
            payload_loss_days,
            "invocation archival deferred until parallel-work minute coverage catches up"
        );
        (0, 0, 0)
    };
    summary.invocation_rows_archived += invocation_archive.0;
    summary.archive_batches_touched += invocation_archive.1;
    summary.raw_files_removed += invocation_archive.2;
    if !dry_run && (pruned_archive_batches > 0 || invocation_archive.1 > 0) {
        let manifest_refresh = refresh_archive_upstream_activity_manifest(pool, config, false)
            .await
            .context("failed to refresh upstream activity manifest after invocation archive materialization")?;
        debug!(
            refreshed_batches = manifest_refresh.refreshed_batches,
            pending_batches = manifest_refresh.pending_batches,
            account_rows_written = manifest_refresh.account_rows_written,
            "refreshed upstream activity manifest before waking archive backfill"
        );
        wake_retention_startup_backfill_tasks(
            pool,
            &[
                StartupBackfillTask::UpstreamActivityArchives,
                StartupBackfillTask::HistoricalRollups,
            ],
            if pruned_archive_batches > 0 {
                "invocation_detail_prune_archive_materialized"
            } else {
                "invocation_archive_materialized"
            },
        )
        .await?;
    }
    Ok(())
}

async fn run_remaining_retention_stages(
    pool: &Pool<Sqlite>,
    config: &AppConfig,
    raw_path_fallback_root: Option<&Path>,
    dry_run: bool,
    shutdown: Option<&CancellationToken>,
    summary: &mut RetentionRunSummary,
) -> Result<()> {
    let proxy_archive = archive_timestamped_dataset(
        pool,
        config,
        archive_table_spec("forward_proxy_attempts"),
        "SELECT id, occurred_at AS timestamp_value FROM forward_proxy_attempts WHERE occurred_at < ?1 ORDER BY occurred_at ASC, id ASC LIMIT ?2",
        shanghai_utc_cutoff_string(config.forward_proxy_attempts_retention_days),
        dry_run,
    )
    .await
    .context("failed to archive forward proxy attempts during retention")?;
    summary.forward_proxy_attempt_rows_archived += proxy_archive.0;
    summary.archive_batches_touched += proxy_archive.1;
    if should_stop_data_retention_maintenance(shutdown) {
        return Ok(());
    }

    let pool_attempt_archive = archive_timestamped_dataset(
        pool,
        config,
        archive_table_spec("pool_upstream_request_attempts"),
        "SELECT id, occurred_at AS timestamp_value FROM pool_upstream_request_attempts WHERE occurred_at < ?1 ORDER BY occurred_at ASC, id ASC LIMIT ?2",
        shanghai_local_cutoff_string(config.pool_upstream_request_attempts_retention_days),
        dry_run,
    )
    .await
    .context("failed to archive pool upstream request attempts during retention")?;
    summary.pool_upstream_request_attempt_rows_archived += pool_attempt_archive.0;
    summary.archive_batches_touched += pool_attempt_archive.1;
    if should_stop_data_retention_maintenance(shutdown) {
        return Ok(());
    }

    let quota_archive = compact_old_quota_snapshots(pool, config, dry_run)
        .await
        .context("failed to compact old quota snapshots during retention")?;
    summary.quota_snapshot_rows_archived += quota_archive.0;
    summary.archive_batches_touched += quota_archive.1;
    if should_stop_data_retention_maintenance(shutdown) {
        return Ok(());
    }

    summary.orphan_raw_files_removed +=
        sweep_orphan_proxy_raw_files(pool, config, raw_path_fallback_root, dry_run)
            .await
            .context("failed to sweep orphan proxy raw files during retention")?;
    if should_stop_data_retention_maintenance(shutdown) {
        return Ok(());
    }

    summary.archive_batches_deleted += cleanup_expired_archive_batches(pool, config, dry_run)
        .await
        .context("failed to clean up expired archive batches during retention")?;
    if should_stop_data_retention_maintenance(shutdown) {
        return Ok(());
    }

    if !dry_run && summary.touched_anything() {
        run_best_effort_retention_pragma(
            pool,
            "PRAGMA wal_checkpoint(PASSIVE)",
            "retention wal checkpoint",
        )
        .await?;
        run_best_effort_retention_pragma(pool, "PRAGMA optimize", "retention optimize pragma")
            .await?;
    }
    Ok(())
}

pub(crate) async fn run_best_effort_retention_pragma(
    pool: &Pool<Sqlite>,
    sql: &str,
    description: &'static str,
) -> Result<()> {
    let Some(admission) = acquire_retention_write_admission("retention_pragma").await else {
        return Ok(());
    };
    let execute_started = Instant::now();
    match sqlx::query(sql)
        .execute(pool)
        .await
        .with_context(|| format!("failed to run {description}"))
    {
        Ok(_) => {
            retention_record_commit!(
                "retention_pragma",
                admission.admission_mode(),
                1,
                0,
                Duration::ZERO,
                admission.lock_wait(),
                execute_started.elapsed(),
                Duration::ZERO,
                admission.p1_waiter_count,
                0,
            );
            Ok(())
        }
        Err(err) if is_sqlite_lock_error(&err) => {
            retention_record_error("retention_pragma", &err);
            warn!(error = %err, sql, "{description} skipped because the database is busy");
            Ok(())
        }
        Err(err) => {
            retention_record_error("retention_pragma", &err);
            Err(err)
        }
    }
}

pub(crate) async fn compress_cold_proxy_raw_payloads(
    pool: &Pool<Sqlite>,
    config: &AppConfig,
    raw_path_fallback_root: Option<&Path>,
    dry_run: bool,
) -> Result<RawCompressionPassSummary> {
    compress_cold_proxy_raw_payloads_with_budget(
        pool,
        config,
        raw_path_fallback_root,
        dry_run,
        Some(config.retention_catchup_budget),
    )
    .await
}

pub(crate) async fn compress_cold_proxy_raw_payloads_with_budget(
    pool: &Pool<Sqlite>,
    config: &AppConfig,
    raw_path_fallback_root: Option<&Path>,
    dry_run: bool,
    catchup_budget: Option<Duration>,
) -> Result<RawCompressionPassSummary> {
    if config.proxy_raw_compression == RawCompressionCodec::None {
        return Ok(RawCompressionPassSummary::default());
    }

    let mut summary = RawCompressionPassSummary::default();
    let started_at = Instant::now();
    let batch_limit = if dry_run {
        i64::MAX as usize
    } else {
        retention_candidate_limit(config, "raw_compression")
    };

    loop {
        let (request_summary, request_hit_batch_limit) = compress_cold_proxy_raw_payload_lane(
            pool,
            config,
            raw_path_fallback_root,
            dry_run,
            RawPayloadField::Request,
            batch_limit,
        )
        .await?;
        accumulate_raw_compression_summary(&mut summary, request_summary);

        let (response_summary, response_hit_batch_limit) = compress_cold_proxy_raw_payload_lane(
            pool,
            config,
            raw_path_fallback_root,
            dry_run,
            RawPayloadField::Response,
            batch_limit,
        )
        .await?;
        accumulate_raw_compression_summary(&mut summary, response_summary);

        let (attempt_summary, attempt_hit_batch_limit) =
            compress_cold_pool_attempt_response_raw_lane(
                pool,
                config,
                raw_path_fallback_root,
                dry_run,
                batch_limit,
            )
            .await?;
        accumulate_raw_compression_summary(&mut summary, attempt_summary);

        if !request_hit_batch_limit && !response_hit_batch_limit && !attempt_hit_batch_limit {
            break;
        }
        if dry_run {
            break;
        }
        if let Some(limit) = catchup_budget
            && started_at.elapsed() >= limit
        {
            break;
        }
    }

    Ok(summary)
}
