pub(crate) async fn run_startup_backfill_task(
    state: &Arc<AppState>,
    task: StartupBackfillTask,
    cursor_id: i64,
    _zero_update_streak: u32,
    source_unavailable_probe: bool,
) -> Result<(StartupBackfillRunState, String)> {
    let scan_limit = startup_backfill_scan_limit(source_unavailable_probe);
    let max_elapsed = Some(startup_backfill_run_budget(source_unavailable_probe));
    let raw_path_fallback_root = state.config.database_path.parent();
    match task {
        StartupBackfillTask::ProxyUsage => {
            let snapshot_max_id = current_proxy_usage_backfill_snapshot_max_id(&state.pool).await?;
            let outcome = backfill_proxy_usage_tokens_from_cursor(
                &state.pool,
                cursor_id,
                snapshot_max_id,
                raw_path_fallback_root,
                Some(scan_limit),
                max_elapsed,
            )
            .await?;
            let detail = format!(
                "skipped_missing_file={} skipped_without_usage={} skipped_decode_error={}",
                outcome.summary.skipped_missing_file,
                outcome.summary.skipped_without_usage,
                outcome.summary.skipped_decode_error
            );
            Ok((
                StartupBackfillRunState {
                    next_cursor_id: outcome.next_cursor_id,
                    scanned: outcome.summary.scanned,
                    updated: outcome.summary.updated,
                    hit_scan_limit: outcome.hit_budget,
                    retry_soon: false,
                    force_idle: false,
                    source_unavailable: false,
                    samples: outcome.samples,
                },
                detail,
            ))
        }
        StartupBackfillTask::ProxyCost => {
            let catalog = state.pricing_catalog.read().await.clone();
            let attempt_version = pricing_backfill_attempt_version(&catalog);
            let requested_tier_price_version =
                proxy_price_version(&catalog.version, ProxyPricingMode::RequestedTier);
            let response_tier_price_version =
                proxy_price_version(&catalog.version, ProxyPricingMode::ResponseTier);
            let snapshot_max_id = current_proxy_cost_backfill_snapshot_max_id(
                &state.pool,
                &attempt_version,
                &requested_tier_price_version,
                &response_tier_price_version,
            )
            .await?;
            let outcome = backfill_proxy_missing_costs_from_cursor(
                &state.pool,
                cursor_id,
                snapshot_max_id,
                &catalog,
                &attempt_version,
                &requested_tier_price_version,
                &response_tier_price_version,
                Some(scan_limit),
                max_elapsed,
            )
            .await?;
            let detail = format!(
                "skipped_unpriced_model={}",
                outcome.summary.skipped_unpriced_model
            );
            Ok((
                StartupBackfillRunState {
                    next_cursor_id: outcome.next_cursor_id,
                    scanned: outcome.summary.scanned,
                    updated: outcome.summary.updated,
                    hit_scan_limit: outcome.hit_budget,
                    retry_soon: false,
                    force_idle: false,
                    source_unavailable: false,
                    samples: outcome.samples,
                },
                detail,
            ))
        }
        StartupBackfillTask::PromptCacheKey => {
            let outcome = backfill_proxy_prompt_cache_keys_from_cursor(
                &state.pool,
                cursor_id,
                raw_path_fallback_root,
                Some(scan_limit),
                max_elapsed,
            )
            .await?;
            let detail = format!(
                "skipped_missing_file={} skipped_invalid_json={} skipped_missing_key={}",
                outcome.summary.skipped_missing_file,
                outcome.summary.skipped_invalid_json,
                outcome.summary.skipped_missing_key
            );
            Ok((
                StartupBackfillRunState {
                    next_cursor_id: outcome.next_cursor_id,
                    scanned: outcome.summary.scanned,
                    updated: outcome.summary.updated,
                    hit_scan_limit: outcome.hit_budget,
                    retry_soon: false,
                    force_idle: false,
                    source_unavailable: false,
                    samples: outcome.samples,
                },
                detail,
            ))
        }
        StartupBackfillTask::RequestedServiceTier => {
            let outcome = backfill_proxy_requested_service_tiers_from_cursor(
                &state.pool,
                cursor_id,
                raw_path_fallback_root,
                Some(scan_limit),
                max_elapsed,
            )
            .await?;
            let detail = format!(
                "skipped_missing_file={} skipped_invalid_json={} skipped_missing_tier={}",
                outcome.summary.skipped_missing_file,
                outcome.summary.skipped_invalid_json,
                outcome.summary.skipped_missing_tier
            );
            Ok((
                StartupBackfillRunState {
                    next_cursor_id: outcome.next_cursor_id,
                    scanned: outcome.summary.scanned,
                    updated: outcome.summary.updated,
                    hit_scan_limit: outcome.hit_budget,
                    retry_soon: false,
                    force_idle: false,
                    source_unavailable: false,
                    samples: outcome.samples,
                },
                detail,
            ))
        }
        StartupBackfillTask::InvocationServiceTier => {
            let outcome = backfill_invocation_service_tiers_from_cursor(
                &state.pool,
                cursor_id,
                raw_path_fallback_root,
                Some(scan_limit),
                max_elapsed,
            )
            .await?;
            let detail = format!(
                "skipped_missing_file={} skipped_missing_tier={}",
                outcome.summary.skipped_missing_file, outcome.summary.skipped_missing_tier
            );
            Ok((
                StartupBackfillRunState {
                    next_cursor_id: outcome.next_cursor_id,
                    scanned: outcome.summary.scanned,
                    updated: outcome.summary.updated,
                    hit_scan_limit: outcome.hit_budget,
                    retry_soon: false,
                    force_idle: false,
                    source_unavailable: false,
                    samples: outcome.samples,
                },
                detail,
            ))
        }
        StartupBackfillTask::ReasoningEffort => {
            let outcome = backfill_proxy_reasoning_efforts_from_cursor(
                &state.pool,
                cursor_id,
                raw_path_fallback_root,
                Some(scan_limit),
                max_elapsed,
            )
            .await?;
            let detail = format!(
                "skipped_missing_file={} skipped_invalid_json={} skipped_missing_effort={}",
                outcome.summary.skipped_missing_file,
                outcome.summary.skipped_invalid_json,
                outcome.summary.skipped_missing_effort
            );
            Ok((
                StartupBackfillRunState {
                    next_cursor_id: outcome.next_cursor_id,
                    scanned: outcome.summary.scanned,
                    updated: outcome.summary.updated,
                    hit_scan_limit: outcome.hit_budget,
                    retry_soon: false,
                    force_idle: false,
                    source_unavailable: false,
                    samples: outcome.samples,
                },
                detail,
            ))
        }
        StartupBackfillTask::FailureClassification => {
            let outcome = backfill_failure_classification_from_cursor(
                &state.pool,
                cursor_id,
                raw_path_fallback_root,
                Some(scan_limit),
                max_elapsed,
            )
            .await?;
            Ok((
                StartupBackfillRunState {
                    next_cursor_id: outcome.next_cursor_id,
                    scanned: outcome.summary.scanned,
                    updated: outcome.summary.updated,
                    hit_scan_limit: outcome.hit_budget,
                    retry_soon: false,
                    force_idle: false,
                    source_unavailable: false,
                    samples: outcome.samples,
                },
                "failure classification recalculated".to_string(),
            ))
        }
        StartupBackfillTask::PoolAttemptPublicIdLive => {
            let outcome = backfill_pool_upstream_request_attempt_public_ids_from_cursor(
                &state.pool,
                cursor_id,
                Some(scan_limit),
                max_elapsed,
            )
            .await?;
            Ok((
                StartupBackfillRunState {
                    next_cursor_id: outcome.next_cursor_id,
                    scanned: outcome.summary.scanned,
                    updated: outcome.summary.updated,
                    hit_scan_limit: outcome.hit_budget,
                    retry_soon: false,
                    force_idle: false,
                    source_unavailable: false,
                    samples: outcome.samples,
                },
                "attempt_public_id live rows".to_string(),
            ))
        }
        StartupBackfillTask::PoolAttemptPublicIdArchives => {
            let outcome =
                backfill_pool_upstream_request_attempt_archive_public_ids_from_batch_cursor(
                    &state.pool,
                    cursor_id,
                    Some(1),
                    max_elapsed,
                )
                .await?;
            Ok((
                StartupBackfillRunState {
                    next_cursor_id: outcome.next_cursor_id,
                    scanned: outcome.summary.scanned_batches,
                    updated: outcome.summary.updated_rows,
                    hit_scan_limit: outcome.hit_budget,
                    retry_soon: false,
                    force_idle: false,
                    source_unavailable: false,
                    samples: outcome.samples,
                },
                format!(
                    "updated_batches={} updated_rows={}",
                    outcome.summary.updated_batches, outcome.summary.updated_rows
                ),
            ))
        }
        StartupBackfillTask::UpstreamActivityLive => {
            let updated_accounts =
                backfill_upstream_account_last_activity_from_live_invocations(&state.pool).await?;
            let pending_accounts =
                count_upstream_accounts_missing_live_last_activity(&state.pool).await?;
            Ok((
                StartupBackfillRunState {
                    next_cursor_id: cursor_id,
                    scanned: 0,
                    updated: updated_accounts,
                    hit_scan_limit: false,
                    retry_soon: false,
                    force_idle: false,
                    source_unavailable: false,
                    samples: Vec::new(),
                },
                format!("pending_accounts={pending_accounts}"),
            ))
        }
        StartupBackfillTask::UpstreamActivityArchives => {
            let summary = backfill_upstream_account_last_activity_from_archives(
                &state.pool,
                Some(scan_limit),
                max_elapsed,
            )
            .await?;
            let pending_accounts =
                count_upstream_accounts_missing_last_activity(&state.pool).await?;
            let force_idle = summary.waiting_for_manifest_backfill
                || (pending_accounts > 0 && !summary.hit_budget && summary.updated_accounts == 0);
            Ok((
                StartupBackfillRunState {
                    next_cursor_id: cursor_id,
                    scanned: summary.scanned_batches,
                    updated: summary.updated_accounts,
                    hit_scan_limit: pending_accounts > 0 && summary.hit_budget,
                    retry_soon: false,
                    force_idle,
                    source_unavailable: pending_accounts > 0 && force_idle,
                    samples: Vec::new(),
                },
                format!(
                    "pending_accounts={pending_accounts} waiting_for_manifest_backfill={}",
                    summary.waiting_for_manifest_backfill
                ),
            ))
        }
        StartupBackfillTask::PoolUpstreamNodeHealthArchives => {
            let cache_summary =
                backfill_pool_upstream_node_health_archives(&state.pool, Some(1), max_elapsed)
                    .await?;
            let hourly_summary = backfill_pool_upstream_node_health_hourly_archives(
                &state.pool,
                Some(1),
                max_elapsed,
            )
            .await?;
            Ok((
                StartupBackfillRunState {
                    next_cursor_id: cursor_id,
                    scanned: cache_summary.scanned_batches + hourly_summary.scanned_batches,
                    updated: cache_summary.cached_rows + hourly_summary.materialized_rows,
                    hit_scan_limit: cache_summary.hit_budget || hourly_summary.hit_budget,
                    retry_soon: false,
                    force_idle: cache_summary.pending_batches == 0
                        && hourly_summary.pending_batches == 0,
                    source_unavailable: false,
                    samples: Vec::new(),
                },
                format!(
                    "pending_cache_batches={} materialized_cache_batches={} pending_hourly_batches={} materialized_hourly_batches={}",
                    cache_summary.pending_batches,
                    cache_summary.materialized_batches,
                    hourly_summary.pending_batches,
                    hourly_summary.materialized_batches,
                ),
            ))
        }
        StartupBackfillTask::AccountActivityV2Coverage => Err(anyhow!(
            "account activity v2 coverage repair must use its dedicated scheduler path"
        )),
        StartupBackfillTask::LegacyDetailMirrors => {
            let window = reconcile_legacy_detail_mirrors_startup_window(
                &state.pool,
                cursor_id,
                Duration::from_secs(STARTUP_LEGACY_DETAIL_MIRROR_BUDGET_SECS),
            )
            .await?;
            let retry_soon = window.hit_budget
                || (window.inspected_path_count > 0
                    && window.candidate_count
                        >= STARTUP_LEGACY_DETAIL_MIRROR_CANDIDATE_LIMIT as usize);
            info!(
                task = StartupBackfillTask::LegacyDetailMirrors.log_label(),
                candidate_limit = STARTUP_LEGACY_DETAIL_MIRROR_CANDIDATE_LIMIT,
                elapsed_budget_ms = Duration::from_secs(STARTUP_LEGACY_DETAIL_MIRROR_BUDGET_SECS)
                    .as_millis() as u64,
                candidate_count = window.candidate_count,
                inspected_path_count = window.inspected_path_count,
                changed_path_count = window.changed_path_count,
                hit_budget = window.hit_budget,
                wrapped = window.wrapped,
                "startup legacy detail mirror reconciliation pass completed"
            );
            Ok((
                StartupBackfillRunState {
                    next_cursor_id: window.next_cursor_id,
                    scanned: window.inspected_path_count as u64,
                    updated: window.changed_path_count as u64,
                    hit_scan_limit: retry_soon,
                    retry_soon,
                    force_idle: window.candidate_count == 0,
                    source_unavailable: false,
                    samples: Vec::new(),
                },
                format!(
                    "candidate_count={} inspected_path_count={} changed_path_count={} hit_budget={} wrapped={} next_cursor_id={}",
                    window.candidate_count,
                    window.inspected_path_count,
                    window.changed_path_count,
                    window.hit_budget,
                    window.wrapped,
                    window.next_cursor_id,
                ),
            ))
        }
        StartupBackfillTask::HistoricalRollups => {
            let window = materialize_historical_rollups_startup_window(
                &state.pool,
                cursor_id,
                Duration::from_secs(STARTUP_HISTORICAL_ROLLUP_BUDGET_SECS),
            )
            .await?;
            let summary = window.summary;
            let updated = window.changed_path_count as u64;
            info!(
                task = StartupBackfillTask::HistoricalRollups.log_label(),
                candidate_limit = 32_u64,
                batch_limit = STARTUP_HISTORICAL_ROLLUP_BATCH_LIMIT,
                elapsed_budget_ms =
                    Duration::from_secs(STARTUP_HISTORICAL_ROLLUP_BUDGET_SECS).as_millis() as u64,
                candidate_count = window.candidate_count,
                inspected_path_count = window.inspected_path_count,
                changed_path_count = window.changed_path_count,
                wrapped = window.wrapped,
                blocked_archive_batches = summary.blocked_archive_batches,
                "startup historical rollup keyset pass completed"
            );
            Ok((
                StartupBackfillRunState {
                    next_cursor_id: window.next_cursor_id,
                    scanned: summary.scanned_archive_batches as u64,
                    updated,
                    hit_scan_limit: window.hit_budget
                        || (updated > 0
                            && (window.candidate_count >= 32
                                || summary.scanned_archive_batches
                                    >= STARTUP_HISTORICAL_ROLLUP_BATCH_LIMIT as usize)),
                    retry_soon: historical_rollup_should_retry_soon(
                        window.hit_budget,
                        window.candidate_count,
                    ),
                    force_idle: window.candidate_count == 0,
                    source_unavailable: false,
                    samples: Vec::new(),
                },
                format!(
                    "candidate_count={} inspected_path_count={} changed_path_count={} hit_budget={} wrapped={} next_cursor_id={} materialized_archive_batches={} blocked_archive_batches={}",
                    window.candidate_count,
                    window.inspected_path_count,
                    window.changed_path_count,
                    window.hit_budget,
                    window.wrapped,
                    window.next_cursor_id,
                    summary.materialized_archive_batches,
                    summary.blocked_archive_batches,
                ),
            ))
        }
    }
}

pub(crate) async fn run_startup_persistent_prep_best_effort(
    state: &Arc<AppState>,
    prep_cli: &CliArgs,
) -> bool {
    if !should_run_startup_persistent_prep(prep_cli) {
        return true;
    }

    // The individual manifest and expiry writers acquire the maintenance permit themselves.
    // Holding the global background slot here would make those nested admissions reject every
    // startup pass before they can do any work.
    let defer_generation = super::retention::retention_defer_generation();
    match run_startup_persistent_prep_inner(&state.pool, &state.config, prep_cli, false).await {
        Ok(summary) => {
            let deferred = super::retention::retention_defer_generation() != defer_generation;
            info!(
                refreshed_manifest_batches = summary.refreshed_manifest_batches,
                refreshed_manifest_account_rows = summary.refreshed_manifest_account_rows,
                missing_manifest_files = summary.missing_manifest_files,
                backfilled_archive_expiries = summary.backfilled_archive_expiries,
                bootstrapped_hourly_rollups = summary.bootstrapped_hourly_rollups,
                pending_manifest_batches = summary.pending_manifest_batches,
                deferred,
                "startup background prep finished"
            );
            !deferred && summary.pending_manifest_batches == 0
        }
        Err(err) => {
            let pressure_error = crate::db_pressure::global_db_pressure_gate()
                .record_error("startup_persistent_prep", &err);
            warn!(error = %err, retry_soon = pressure_error, "startup background prep failed");
            !pressure_error
        }
    }
}

async fn run_startup_persistent_prep_best_effort_cancellable(
    state: &Arc<AppState>,
    prep_cli: &CliArgs,
    cancel: &CancellationToken,
) -> Option<bool> {
    tokio::select! {
        biased;
        _ = cancel.cancelled() => None,
        result = run_startup_persistent_prep_best_effort(state, prep_cli) => Some(result),
    }
}

pub(crate) async fn run_pressure_eligible_startup_backfill_tasks(
    state: Arc<AppState>,
    cancel: &CancellationToken,
    gate: &crate::db_pressure::DbPressureGate,
) {
    if cancel.is_cancelled() {
        return;
    }
    let tasks = STARTUP_BACKFILL_SCHEDULER.take_pressure_deferred_tasks();
    if tasks.is_empty() {
        return;
    }
    debug!(
        task_count = tasks.len(),
        "pressure eligibility changed; dispatching deferred startup backfill tasks"
    );
    run_startup_backfill_maintenance_pass_with_gate_inner(state, cancel, Some(&tasks), gate).await;
}

pub(crate) fn spawn_startup_backfill_maintenance(
    state: Arc<AppState>,
    cancel: CancellationToken,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        if cancel.is_cancelled() {
            info!("startup backfill maintenance skipped because shutdown is already in progress");
            return;
        }
        let prep_cli = CliArgs::default();
        let Some(prep_pending) =
            run_startup_persistent_prep_best_effort_cancellable(&state, &prep_cli, &cancel).await
        else {
            return;
        };
        let mut startup_prep_pending = prep_pending;
        let mut startup_prep_retry_at = startup_prep_pending
            .then(|| Instant::now() + Duration::from_secs(STARTUP_BACKFILL_ACTIVE_INTERVAL_SECS));
        run_startup_backfill_maintenance_pass(state.clone(), &cancel, None).await;
        // Register before either P2 supervisor is scheduled so long-term pruning cannot
        // reclaim a terminal event ahead of the minute projection consumer.
        state
            .terminal_projection_hub
            .activate_timeseries_consumer(0);
        spawn_long_term_projection_supervisor(state.clone(), cancel.clone());
        spawn_timeseries_minute_projection_supervisor(state.clone(), cancel.clone());

        let mut observed_generation = STARTUP_BACKFILL_SCHEDULER.generation();

        loop {
            let gate = crate::db_pressure::global_db_pressure_gate();
            let observed_pressure_eligibility = gate.eligibility_generation();
            let next_due = STARTUP_BACKFILL_SCHEDULER.next_due();
            let mut wait_for = startup_backfill_wait_duration(next_due);
            if let Some(retry_at) = startup_prep_retry_at {
                wait_for = wait_for.min(retry_at.saturating_duration_since(Instant::now()));
            }

            tokio::select! {
                _ = cancel.cancelled() => {
                    info!("startup backfill maintenance received shutdown");
                    break;
                }
                _ = STARTUP_BACKFILL_SCHEDULER.wait_for_wake(observed_generation) => {
                    observed_generation = STARTUP_BACKFILL_SCHEDULER.generation();
                    let tasks = STARTUP_BACKFILL_SCHEDULER.drain_woken_tasks();
                    if !tasks.is_empty() {
                        run_startup_backfill_maintenance_pass(state.clone(), &cancel, Some(&tasks)).await;
                    }
                }
                _ = gate.wait_for_eligibility_change(observed_pressure_eligibility) => {
                    run_pressure_eligible_startup_backfill_tasks(
                        state.clone(),
                        &cancel,
                        gate,
                    ).await;
                }
                _ = sleep(wait_for) => {
                    observed_generation = STARTUP_BACKFILL_SCHEDULER.generation();
                    STARTUP_BACKFILL_SCHEDULER.drain_woken_tasks();
                    if startup_prep_pending
                        && startup_prep_retry_at.is_none_or(|retry_at| retry_at <= Instant::now())
                    {
                        let Some(prep_pending) =
                            run_startup_persistent_prep_best_effort_cancellable(&state, &prep_cli, &cancel)
                                .await
                        else {
                            break;
                        };
                        startup_prep_pending = prep_pending;
                        startup_prep_retry_at = startup_prep_pending.then(|| {
                            Instant::now()
                                + Duration::from_secs(STARTUP_BACKFILL_ACTIVE_INTERVAL_SECS)
                        });
                    }
                    let due_tasks = STARTUP_BACKFILL_SCHEDULER.drain_due_tasks(Utc::now());
                    if !due_tasks.is_empty() {
                        run_startup_backfill_maintenance_pass(
                            state.clone(),
                            &cancel,
                            Some(&due_tasks),
                        )
                        .await;
                    }
                }
            }
        }
    })
}
