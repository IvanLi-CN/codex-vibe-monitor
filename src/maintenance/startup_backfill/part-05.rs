pub(crate) async fn persist_startup_backfill_task_failure(
    state: &Arc<AppState>,
    task: StartupBackfillTask,
    task_name: &str,
    progress: &StartupBackfillProgress,
    started_at: Instant,
    err: &anyhow::Error,
) -> Result<DateTime<Utc>> {
    let failure_kind = startup_backfill_failure_kind(err);
    let retry_after = format_utc_iso(
        Utc::now() + ChronoDuration::seconds(STARTUP_BACKFILL_ACTIVE_INTERVAL_SECS as i64),
    );
    save_startup_backfill_progress(
        &state.pool,
        task_name,
        StartupBackfillProgressUpdate {
            cursor_id: progress.cursor_id,
            scanned: 0,
            updated: 0,
            zero_update_streak: progress.zero_update_streak,
            next_run_after: &retry_after,
            status: STARTUP_BACKFILL_STATUS_FAILED,
            suspension_reason: None,
        },
    )
    .await?;
    warn!(
        task = task.log_label(),
        task_name,
        cursor_id = progress.cursor_id,
        elapsed_ms = started_at.elapsed().as_millis() as u64,
        next_run_after = %retry_after,
        failure_kind = failure_kind.telemetry_reason(),
        retry_kind = "bounded_operation_backoff",
        error = %err,
        "startup backfill pass failed"
    );
    Ok(parse_to_utc_datetime(&retry_after).unwrap_or_else(Utc::now))
}

fn startup_backfill_run_is_actionable(run: &StartupBackfillRunState) -> bool {
    run.updated > 0 || (run.hit_scan_limit && !run.force_idle)
}

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
            run_proxy_usage_backfill(
                state,
                cursor_id,
                raw_path_fallback_root,
                scan_limit,
                max_elapsed,
            )
            .await
        }
        StartupBackfillTask::ProxyCost => {
            run_proxy_cost_backfill_task(state, cursor_id, scan_limit, max_elapsed).await
        }
        StartupBackfillTask::PromptCacheKey => {
            run_prompt_cache_key_backfill(
                state,
                cursor_id,
                raw_path_fallback_root,
                scan_limit,
                max_elapsed,
            )
            .await
        }
        StartupBackfillTask::RequestedServiceTier => {
            run_requested_service_tier_backfill(
                state,
                cursor_id,
                raw_path_fallback_root,
                scan_limit,
                max_elapsed,
            )
            .await
        }
        StartupBackfillTask::InvocationServiceTier => {
            run_invocation_service_tier_backfill(
                state,
                cursor_id,
                raw_path_fallback_root,
                scan_limit,
                max_elapsed,
            )
            .await
        }
        StartupBackfillTask::ReasoningEffort => {
            run_reasoning_effort_backfill(
                state,
                cursor_id,
                raw_path_fallback_root,
                scan_limit,
                max_elapsed,
            )
            .await
        }
        StartupBackfillTask::FailureClassification => {
            run_failure_classification_backfill(
                state,
                cursor_id,
                raw_path_fallback_root,
                scan_limit,
                max_elapsed,
            )
            .await
        }
        StartupBackfillTask::PoolAttemptPublicIdLive => {
            run_pool_attempt_public_id_live_backfill(state, cursor_id, scan_limit, max_elapsed)
                .await
        }
        StartupBackfillTask::PoolAttemptPublicIdArchives => {
            run_pool_attempt_public_id_archives_backfill(state, cursor_id, max_elapsed).await
        }
        StartupBackfillTask::UpstreamActivityLive => {
            run_upstream_activity_live_backfill(state, cursor_id).await
        }
        StartupBackfillTask::UpstreamActivityArchives => {
            run_upstream_activity_archives_backfill(state, cursor_id, scan_limit, max_elapsed).await
        }
        StartupBackfillTask::PoolUpstreamNodeHealthArchives => {
            run_pool_upstream_node_health_backfill(state, cursor_id, max_elapsed).await
        }
        StartupBackfillTask::AccountActivityV2Coverage => Err(anyhow!(
            "account activity v2 coverage repair must use its dedicated scheduler path"
        )),
        StartupBackfillTask::LegacyDetailMirrors => {
            run_legacy_detail_mirrors_backfill(state, cursor_id).await
        }
        StartupBackfillTask::HistoricalRollups => {
            run_historical_rollups_backfill(state, cursor_id).await
        }
    }
}

async fn run_proxy_usage_backfill(
    state: &Arc<AppState>,
    cursor_id: i64,
    raw_path_fallback_root: Option<&std::path::Path>,
    scan_limit: u64,
    max_elapsed: Option<Duration>,
) -> Result<(StartupBackfillRunState, String)> {
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

async fn run_proxy_cost_backfill_task(
    state: &Arc<AppState>,
    cursor_id: i64,
    scan_limit: u64,
    max_elapsed: Option<Duration>,
) -> Result<(StartupBackfillRunState, String)> {
    let outcome = run_proxy_cost_backfill(state, cursor_id, scan_limit, max_elapsed).await?;
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

async fn run_prompt_cache_key_backfill(
    state: &Arc<AppState>,
    cursor_id: i64,
    raw_path_fallback_root: Option<&std::path::Path>,
    scan_limit: u64,
    max_elapsed: Option<Duration>,
) -> Result<(StartupBackfillRunState, String)> {
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

async fn run_requested_service_tier_backfill(
    state: &Arc<AppState>,
    cursor_id: i64,
    raw_path_fallback_root: Option<&std::path::Path>,
    scan_limit: u64,
    max_elapsed: Option<Duration>,
) -> Result<(StartupBackfillRunState, String)> {
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

async fn run_invocation_service_tier_backfill(
    state: &Arc<AppState>,
    cursor_id: i64,
    raw_path_fallback_root: Option<&std::path::Path>,
    scan_limit: u64,
    max_elapsed: Option<Duration>,
) -> Result<(StartupBackfillRunState, String)> {
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

async fn run_reasoning_effort_backfill(
    state: &Arc<AppState>,
    cursor_id: i64,
    raw_path_fallback_root: Option<&std::path::Path>,
    scan_limit: u64,
    max_elapsed: Option<Duration>,
) -> Result<(StartupBackfillRunState, String)> {
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

async fn run_failure_classification_backfill(
    state: &Arc<AppState>,
    cursor_id: i64,
    raw_path_fallback_root: Option<&std::path::Path>,
    scan_limit: u64,
    max_elapsed: Option<Duration>,
) -> Result<(StartupBackfillRunState, String)> {
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

async fn run_pool_attempt_public_id_live_backfill(
    state: &Arc<AppState>,
    cursor_id: i64,
    scan_limit: u64,
    max_elapsed: Option<Duration>,
) -> Result<(StartupBackfillRunState, String)> {
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

async fn run_pool_attempt_public_id_archives_backfill(
    state: &Arc<AppState>,
    cursor_id: i64,
    max_elapsed: Option<Duration>,
) -> Result<(StartupBackfillRunState, String)> {
    let outcome = backfill_pool_upstream_request_attempt_archive_public_ids_from_batch_cursor(
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

async fn run_upstream_activity_live_backfill(
    state: &Arc<AppState>,
    cursor_id: i64,
) -> Result<(StartupBackfillRunState, String)> {
    let updated_accounts =
        backfill_upstream_account_last_activity_from_live_invocations(&state.pool).await?;
    let pending_accounts = count_upstream_accounts_missing_live_last_activity(&state.pool).await?;
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

async fn run_upstream_activity_archives_backfill(
    state: &Arc<AppState>,
    cursor_id: i64,
    scan_limit: u64,
    max_elapsed: Option<Duration>,
) -> Result<(StartupBackfillRunState, String)> {
    let summary = backfill_upstream_account_last_activity_from_archives(
        &state.pool,
        Some(scan_limit),
        max_elapsed,
    )
    .await?;
    let pending_accounts = count_upstream_accounts_missing_last_activity(&state.pool).await?;
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

async fn run_pool_upstream_node_health_backfill(
    state: &Arc<AppState>,
    cursor_id: i64,
    max_elapsed: Option<Duration>,
) -> Result<(StartupBackfillRunState, String)> {
    let cache_summary =
        backfill_pool_upstream_node_health_archives(&state.pool, Some(1), max_elapsed).await?;
    let hourly_summary =
        backfill_pool_upstream_node_health_hourly_archives(&state.pool, Some(1), max_elapsed)
            .await?;
    Ok((
        StartupBackfillRunState {
            next_cursor_id: cursor_id,
            scanned: cache_summary.scanned_batches + hourly_summary.scanned_batches,
            updated: cache_summary.cached_rows + hourly_summary.materialized_rows,
            hit_scan_limit: cache_summary.hit_budget || hourly_summary.hit_budget,
            retry_soon: false,
            force_idle: cache_summary.pending_batches == 0 && hourly_summary.pending_batches == 0,
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

async fn run_legacy_detail_mirrors_backfill(
    state: &Arc<AppState>,
    cursor_id: i64,
) -> Result<(StartupBackfillRunState, String)> {
    let window = reconcile_legacy_detail_mirrors_startup_window(
        &state.pool,
        cursor_id,
        Duration::from_secs(STARTUP_LEGACY_DETAIL_MIRROR_BUDGET_SECS),
    )
    .await?;
    let retry_soon = window.hit_budget
        || (window.inspected_path_count > 0
            && window.candidate_count >= STARTUP_LEGACY_DETAIL_MIRROR_CANDIDATE_LIMIT as usize);
    info!(
        task = StartupBackfillTask::LegacyDetailMirrors.log_label(),
        candidate_limit = STARTUP_LEGACY_DETAIL_MIRROR_CANDIDATE_LIMIT,
        elapsed_budget_ms =
            Duration::from_secs(STARTUP_LEGACY_DETAIL_MIRROR_BUDGET_SECS).as_millis() as u64,
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

async fn run_historical_rollups_backfill(
    state: &Arc<AppState>,
    cursor_id: i64,
) -> Result<(StartupBackfillRunState, String)> {
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
