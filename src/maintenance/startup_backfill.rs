fn push_backfill_sample(samples: &mut Vec<String>, sample: String) {
    if samples.len() < STARTUP_BACKFILL_LOG_SAMPLE_LIMIT {
        samples.push(sample);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StartupBackfillTask {
    ProxyUsage,
    ProxyCost,
    PromptCacheKey,
    RequestedServiceTier,
    InvocationServiceTier,
    ReasoningEffort,
    FailureClassification,
    UpstreamActivityLive,
    UpstreamActivityArchives,
    HistoricalRollups,
}

impl StartupBackfillTask {
    fn ordered_tasks() -> &'static [Self] {
        &[
            Self::ProxyUsage,
            Self::PromptCacheKey,
            Self::RequestedServiceTier,
            Self::InvocationServiceTier,
            Self::ProxyCost,
            Self::ReasoningEffort,
            Self::FailureClassification,
            Self::UpstreamActivityLive,
            Self::UpstreamActivityArchives,
            Self::HistoricalRollups,
        ]
    }

    fn name(self) -> &'static str {
        match self {
            Self::ProxyUsage => STARTUP_BACKFILL_TASK_PROXY_USAGE,
            Self::ProxyCost => STARTUP_BACKFILL_TASK_PROXY_COST,
            Self::PromptCacheKey => STARTUP_BACKFILL_TASK_PROMPT_CACHE_KEY,
            Self::RequestedServiceTier => STARTUP_BACKFILL_TASK_REQUESTED_SERVICE_TIER,
            Self::InvocationServiceTier => STARTUP_BACKFILL_TASK_INVOCATION_SERVICE_TIER,
            Self::ReasoningEffort => STARTUP_BACKFILL_TASK_REASONING_EFFORT,
            Self::FailureClassification => STARTUP_BACKFILL_TASK_FAILURE_CLASSIFICATION,
            Self::UpstreamActivityLive => STARTUP_BACKFILL_TASK_UPSTREAM_ACTIVITY_LIVE,
            Self::UpstreamActivityArchives => STARTUP_BACKFILL_TASK_UPSTREAM_ACTIVITY_ARCHIVES,
            Self::HistoricalRollups => STARTUP_BACKFILL_TASK_HISTORICAL_ROLLUPS,
        }
    }

    fn log_label(self) -> &'static str {
        match self {
            Self::ProxyUsage => "proxy usage",
            Self::ProxyCost => "proxy cost",
            Self::PromptCacheKey => "proxy prompt cache key",
            Self::RequestedServiceTier => "proxy requested service tier",
            Self::InvocationServiceTier => "invocation service tier",
            Self::ReasoningEffort => "proxy reasoning effort",
            Self::FailureClassification => "invocation failure classification",
            Self::UpstreamActivityLive => "upstream activity live rows",
            Self::UpstreamActivityArchives => "upstream activity archives",
            Self::HistoricalRollups => "historical rollup materialization",
        }
    }
}

#[derive(Debug, Clone, FromRow)]
struct StartupBackfillProgressRow {
    task_name: String,
    cursor_id: i64,
    next_run_after: Option<String>,
    zero_update_streak: i64,
    last_started_at: Option<String>,
    last_finished_at: Option<String>,
    last_scanned: i64,
    last_updated: i64,
    last_status: String,
}

#[derive(Debug, Clone)]
struct StartupBackfillProgress {
    task_name: String,
    cursor_id: i64,
    next_run_after: Option<String>,
    zero_update_streak: u32,
    last_started_at: Option<String>,
    last_finished_at: Option<String>,
    last_scanned: u64,
    last_updated: u64,
    last_status: String,
}

impl StartupBackfillProgress {
    fn pending(task_name: impl Into<String>) -> Self {
        Self {
            task_name: task_name.into(),
            cursor_id: 0,
            next_run_after: None,
            zero_update_streak: 0,
            last_started_at: None,
            last_finished_at: None,
            last_scanned: 0,
            last_updated: 0,
            last_status: STARTUP_BACKFILL_STATUS_IDLE.to_string(),
        }
    }

    fn is_due(&self, now: DateTime<Utc>) -> bool {
        self.next_run_after
            .as_deref()
            .and_then(parse_to_utc_datetime)
            .is_none_or(|deadline| deadline <= now)
    }
}

impl From<StartupBackfillProgressRow> for StartupBackfillProgress {
    fn from(value: StartupBackfillProgressRow) -> Self {
        Self {
            task_name: value.task_name,
            cursor_id: value.cursor_id,
            next_run_after: value.next_run_after,
            zero_update_streak: value.zero_update_streak.max(0) as u32,
            last_started_at: value.last_started_at,
            last_finished_at: value.last_finished_at,
            last_scanned: value.last_scanned.max(0) as u64,
            last_updated: value.last_updated.max(0) as u64,
            last_status: value.last_status,
        }
    }
}

#[derive(Debug, Clone, Default)]
struct StartupBackfillRunState {
    next_cursor_id: i64,
    scanned: u64,
    updated: u64,
    hit_scan_limit: bool,
    force_idle: bool,
    samples: Vec<String>,
}

fn startup_backfill_next_delay(run: &StartupBackfillRunState, zero_update_streak: u32) -> Duration {
    if run.force_idle {
        Duration::from_secs(STARTUP_BACKFILL_IDLE_INTERVAL_SECS)
    } else if run.hit_scan_limit || run.updated > 0 {
        Duration::from_secs(STARTUP_BACKFILL_ACTIVE_INTERVAL_SECS)
    } else if run.scanned == 0 || zero_update_streak > 0 {
        Duration::from_secs(STARTUP_BACKFILL_IDLE_INTERVAL_SECS)
    } else {
        Duration::from_secs(STARTUP_BACKFILL_ACTIVE_INTERVAL_SECS)
    }
}

fn startup_backfill_next_run_after(
    run: &StartupBackfillRunState,
    zero_update_streak: u32,
) -> String {
    format_utc_iso(
        Utc::now()
            + ChronoDuration::from_std(startup_backfill_next_delay(run, zero_update_streak))
                .unwrap_or_else(|_| {
                    ChronoDuration::seconds(STARTUP_BACKFILL_IDLE_INTERVAL_SECS as i64)
                }),
    )
}

#[derive(Debug, Clone)]
struct BackfillBatchOutcome<T> {
    summary: T,
    next_cursor_id: i64,
    hit_budget: bool,
    samples: Vec<String>,
}

fn startup_backfill_query_limit(scanned: u64, scan_limit: Option<u64>) -> i64 {
    let remaining = scan_limit
        .map(|limit| limit.saturating_sub(scanned))
        .unwrap_or(BACKFILL_BATCH_SIZE as u64);
    remaining.min(BACKFILL_BATCH_SIZE as u64).max(1) as i64
}

fn startup_backfill_budget_reached(
    started_at: Instant,
    scanned: u64,
    scan_limit: Option<u64>,
    max_elapsed: Option<Duration>,
) -> bool {
    let hit_scan_limit = scan_limit.is_some_and(|limit| scanned >= limit);
    let hit_elapsed_limit = max_elapsed.is_some_and(|limit| started_at.elapsed() >= limit);
    hit_scan_limit || hit_elapsed_limit
}

fn startup_backfill_samples_text(samples: &[String]) -> String {
    if samples.is_empty() {
        "-".to_string()
    } else {
        samples.join(" | ")
    }
}

async fn startup_backfill_task_progress_key(state: &AppState, task: StartupBackfillTask) -> String {
    match task {
        StartupBackfillTask::ProxyCost => {
            let catalog = state.pricing_catalog.read().await;
            format!(
                "{}:{}",
                task.name(),
                pricing_backfill_attempt_version(&catalog)
            )
        }
        _ => task.name().to_string(),
    }
}

async fn load_startup_backfill_progress(
    pool: &Pool<Sqlite>,
    task_name: &str,
) -> Result<StartupBackfillProgress> {
    Ok(sqlx::query_as::<_, StartupBackfillProgressRow>(
        r#"
        SELECT
            task_name,
            cursor_id,
            next_run_after,
            zero_update_streak,
            last_started_at,
            last_finished_at,
            last_scanned,
            last_updated,
            last_status
        FROM startup_backfill_progress
        WHERE task_name = ?1
        LIMIT 1
        "#,
    )
    .bind(task_name)
    .fetch_optional(pool)
    .await?
    .map(Into::into)
    .unwrap_or_else(|| StartupBackfillProgress::pending(task_name.to_string())))
}

async fn mark_startup_backfill_running(
    pool: &Pool<Sqlite>,
    task_name: &str,
    cursor_id: i64,
) -> Result<()> {
    let now = format_utc_iso(Utc::now());
    sqlx::query(
        r#"
        INSERT INTO startup_backfill_progress (
            task_name,
            cursor_id,
            next_run_after,
            zero_update_streak,
            last_started_at,
            last_finished_at,
            last_scanned,
            last_updated,
            last_status
        )
        VALUES (?1, ?2, NULL, 0, ?3, NULL, 0, 0, ?4)
        ON CONFLICT(task_name) DO UPDATE SET
            next_run_after = NULL,
            last_started_at = excluded.last_started_at,
            last_status = excluded.last_status
        "#,
    )
    .bind(task_name)
    .bind(cursor_id)
    .bind(&now)
    .bind(STARTUP_BACKFILL_STATUS_RUNNING)
    .execute(pool)
    .await?;
    Ok(())
}

struct StartupBackfillProgressUpdate<'a> {
    cursor_id: i64,
    scanned: u64,
    updated: u64,
    zero_update_streak: u32,
    next_run_after: &'a str,
    status: &'a str,
}

async fn save_startup_backfill_progress(
    pool: &Pool<Sqlite>,
    task_name: &str,
    update: StartupBackfillProgressUpdate<'_>,
) -> Result<()> {
    let finished_at = format_utc_iso(Utc::now());
    sqlx::query(
        r#"
        INSERT INTO startup_backfill_progress (
            task_name,
            cursor_id,
            next_run_after,
            zero_update_streak,
            last_started_at,
            last_finished_at,
            last_scanned,
            last_updated,
            last_status
        )
        VALUES (?1, ?2, ?3, ?4, NULL, ?5, ?6, ?7, ?8)
        ON CONFLICT(task_name) DO UPDATE SET
            cursor_id = excluded.cursor_id,
            next_run_after = excluded.next_run_after,
            zero_update_streak = excluded.zero_update_streak,
            last_finished_at = excluded.last_finished_at,
            last_scanned = excluded.last_scanned,
            last_updated = excluded.last_updated,
            last_status = excluded.last_status
        "#,
    )
    .bind(task_name)
    .bind(update.cursor_id)
    .bind(update.next_run_after)
    .bind(i64::from(update.zero_update_streak))
    .bind(&finished_at)
    .bind(update.scanned as i64)
    .bind(update.updated as i64)
    .bind(update.status)
    .execute(pool)
    .await?;
    Ok(())
}

async fn run_startup_backfill_maintenance_pass(state: Arc<AppState>, cancel: &CancellationToken) {
    for task in StartupBackfillTask::ordered_tasks() {
        if cancel.is_cancelled() {
            info!(
                task = task.log_label(),
                "startup backfill maintenance stopped at a task boundary because shutdown is in progress"
            );
            break;
        }
        if *task == StartupBackfillTask::ProxyUsage && !state.config.proxy_usage_backfill_on_startup
        {
            debug!(
                task = task.log_label(),
                "startup backfill task is disabled by config"
            );
            continue;
        }
        if let Err(err) = run_startup_backfill_task_if_due(&state, *task).await {
            warn!(task = task.log_label(), error = %err, "startup backfill supervisor pass failed");
        }
    }

    refresh_hourly_rollups_for_read_surfaces_best_effort(
        &state.pool,
        state.hourly_rollup_sync_lock.as_ref(),
        "startup backfill maintenance pass",
    )
    .await;
}

fn startup_backfill_task_enabled(state: &AppState, task: StartupBackfillTask) -> bool {
    match task {
        StartupBackfillTask::ProxyUsage => state.config.proxy_usage_backfill_on_startup,
        _ => true,
    }
}

async fn run_startup_backfill_task_if_due(
    state: &Arc<AppState>,
    task: StartupBackfillTask,
) -> Result<()> {
    if !startup_backfill_task_enabled(state.as_ref(), task) {
        debug!(
            task = task.log_label(),
            "startup backfill task is disabled by config"
        );
        return Ok(());
    }

    let task_name = startup_backfill_task_progress_key(state.as_ref(), task).await;
    let progress = load_startup_backfill_progress(&state.pool, &task_name).await?;
    let now = Utc::now();
    if !progress.is_due(now) {
        debug!(
            task = task.log_label(),
            task_name = %progress.task_name,
            next_run_after = progress.next_run_after.as_deref().unwrap_or("-"),
            last_status = %progress.last_status,
            last_started_at = progress.last_started_at.as_deref().unwrap_or("-"),
            last_finished_at = progress.last_finished_at.as_deref().unwrap_or("-"),
            last_scanned = progress.last_scanned,
            last_updated = progress.last_updated,
            "startup backfill task is not due"
        );
        return Ok(());
    }

    mark_startup_backfill_running(&state.pool, &task_name, progress.cursor_id).await?;

    let started_at = Instant::now();
    match run_startup_backfill_task(state, task, progress.cursor_id).await {
        Ok((run, detail)) => {
            let zero_update_streak = if run.updated == 0 {
                progress.zero_update_streak.saturating_add(1)
            } else {
                0
            };
            let next_cursor_id = run.next_cursor_id.max(progress.cursor_id);
            let next_run_after = startup_backfill_next_run_after(&run, zero_update_streak);
            save_startup_backfill_progress(
                &state.pool,
                &task_name,
                StartupBackfillProgressUpdate {
                    cursor_id: next_cursor_id,
                    scanned: run.scanned,
                    updated: run.updated,
                    zero_update_streak,
                    next_run_after: &next_run_after,
                    status: STARTUP_BACKFILL_STATUS_OK,
                },
            )
            .await?;
            info!(
                task = task.log_label(),
                task_name = %task_name,
                scanned = run.scanned,
                updated = run.updated,
                cursor_id = next_cursor_id,
                hit_scan_limit = run.hit_scan_limit,
                zero_update_streak,
                elapsed_ms = started_at.elapsed().as_millis() as u64,
                next_run_after = %next_run_after,
                detail = %detail,
                samples = %startup_backfill_samples_text(&run.samples),
                "startup backfill pass finished"
            );
        }
        Err(err) => {
            let retry_after = format_utc_iso(
                Utc::now() + ChronoDuration::seconds(STARTUP_BACKFILL_ACTIVE_INTERVAL_SECS as i64),
            );
            save_startup_backfill_progress(
                &state.pool,
                &task_name,
                StartupBackfillProgressUpdate {
                    cursor_id: progress.cursor_id,
                    scanned: 0,
                    updated: 0,
                    zero_update_streak: progress.zero_update_streak,
                    next_run_after: &retry_after,
                    status: STARTUP_BACKFILL_STATUS_FAILED,
                },
            )
            .await?;
            warn!(
                task = task.log_label(),
                task_name = %task_name,
                cursor_id = progress.cursor_id,
                elapsed_ms = started_at.elapsed().as_millis() as u64,
                next_run_after = %retry_after,
                error = %err,
                "startup backfill pass failed"
            );
        }
    }

    Ok(())
}

async fn run_startup_backfill_task(
    state: &Arc<AppState>,
    task: StartupBackfillTask,
    cursor_id: i64,
) -> Result<(StartupBackfillRunState, String)> {
    let max_elapsed = Some(Duration::from_secs(STARTUP_BACKFILL_RUN_BUDGET_SECS));
    let raw_path_fallback_root = state.config.database_path.parent();
    match task {
        StartupBackfillTask::ProxyUsage => {
            let snapshot_max_id = current_proxy_usage_backfill_snapshot_max_id(&state.pool).await?;
            let outcome = backfill_proxy_usage_tokens_from_cursor(
                &state.pool,
                cursor_id,
                snapshot_max_id,
                raw_path_fallback_root,
                Some(STARTUP_BACKFILL_SCAN_LIMIT),
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
                    force_idle: false,
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
                Some(STARTUP_BACKFILL_SCAN_LIMIT),
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
                    force_idle: false,
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
                Some(STARTUP_BACKFILL_SCAN_LIMIT),
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
                    force_idle: false,
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
                Some(STARTUP_BACKFILL_SCAN_LIMIT),
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
                    force_idle: false,
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
                Some(STARTUP_BACKFILL_SCAN_LIMIT),
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
                    force_idle: false,
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
                Some(STARTUP_BACKFILL_SCAN_LIMIT),
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
                    force_idle: false,
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
                Some(STARTUP_BACKFILL_SCAN_LIMIT),
                max_elapsed,
            )
            .await?;
            Ok((
                StartupBackfillRunState {
                    next_cursor_id: outcome.next_cursor_id,
                    scanned: outcome.summary.scanned,
                    updated: outcome.summary.updated,
                    hit_scan_limit: outcome.hit_budget,
                    force_idle: false,
                    samples: outcome.samples,
                },
                "failure classification recalculated".to_string(),
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
                    force_idle: false,
                    samples: Vec::new(),
                },
                format!("pending_accounts={pending_accounts}"),
            ))
        }
        StartupBackfillTask::UpstreamActivityArchives => {
            let summary = backfill_upstream_account_last_activity_from_archives(
                &state.pool,
                Some(STARTUP_BACKFILL_SCAN_LIMIT),
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
                    force_idle,
                    samples: Vec::new(),
                },
                format!(
                    "pending_accounts={pending_accounts} waiting_for_manifest_backfill={}",
                    summary.waiting_for_manifest_backfill
                ),
            ))
        }
        StartupBackfillTask::HistoricalRollups => {
            let before = load_historical_rollup_backfill_snapshot(&state.pool, &state.config).await?;
            if before.legacy_archive_pending == 0 {
                return Ok((
                    StartupBackfillRunState {
                        next_cursor_id: cursor_id,
                        scanned: 0,
                        updated: 0,
                        hit_scan_limit: false,
                        force_idle: true,
                        samples: Vec::new(),
                    },
                    "pending_archive_batches=0".to_string(),
                ));
            }

            let summary = materialize_historical_rollups_bounded(
                &state.pool,
                &state.config,
                false,
                Some(1),
                Some(Duration::from_secs(STARTUP_BACKFILL_RUN_BUDGET_SECS)),
            )
            .await?;
            let after = load_historical_rollup_backfill_snapshot(&state.pool, &state.config).await?;
            let archive_progress = before
                .legacy_archive_pending
                .saturating_sub(after.legacy_archive_pending);
            let bucket_progress = before.pending_buckets.saturating_sub(after.pending_buckets);
            Ok((
                StartupBackfillRunState {
                    next_cursor_id: cursor_id,
                    scanned: summary.scanned_archive_batches as u64,
                    updated: archive_progress.max(bucket_progress),
                    hit_scan_limit: after.legacy_archive_pending > 0,
                    force_idle: after.legacy_archive_pending == 0,
                    samples: Vec::new(),
                },
                format!(
                    "pending_before={} pending_after={} bucket_progress={} alert_level={:?}",
                    before.legacy_archive_pending,
                    after.legacy_archive_pending,
                    bucket_progress,
                    after.alert_level
                ),
            ))
        }
    }
}

fn spawn_startup_backfill_maintenance(
    state: Arc<AppState>,
    cancel: CancellationToken,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        if cancel.is_cancelled() {
            info!("startup backfill maintenance skipped because shutdown is already in progress");
            return;
        }
        let prep_cli = CliArgs::default();
        if should_run_startup_persistent_prep(&prep_cli) {
            match run_startup_persistent_prep_inner(&state.pool, &state.config, &prep_cli, false)
                .await
            {
                Ok(summary) => {
                    info!(
                        refreshed_manifest_batches = summary.refreshed_manifest_batches,
                        refreshed_manifest_account_rows = summary.refreshed_manifest_account_rows,
                        missing_manifest_files = summary.missing_manifest_files,
                        backfilled_archive_expiries = summary.backfilled_archive_expiries,
                        bootstrapped_hourly_rollups = summary.bootstrapped_hourly_rollups,
                        "startup background prep finished"
                    );
                }
                Err(err) => warn!(error = %err, "startup background prep failed"),
            }
        }
        run_startup_backfill_maintenance_pass(state.clone(), &cancel).await;

        let mut ticker = interval(Duration::from_secs(STARTUP_BACKFILL_ACTIVE_INTERVAL_SECS));
        ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
        ticker.tick().await;

        loop {
            tokio::select! {
                _ = cancel.cancelled() => {
                    info!("startup backfill maintenance received shutdown");
                    break;
                }
                _ = ticker.tick() => {
                    run_startup_backfill_maintenance_pass(state.clone(), &cancel).await;
                }
            }
        }
    })
}
