use super::*;

#[derive(Debug, Default)]
pub(crate) struct RetentionRunSummary {
    pub(crate) dry_run: bool,
    pub(crate) deferred: bool,
    pub(crate) budget_exhausted: bool,
    pub(crate) work_budget_ms: Option<u64>,
    pub(crate) elapsed_ms: Option<u64>,
    pub(crate) settlement_ms: Option<u64>,
    pub(crate) wait_reason: Option<String>,
    pub(crate) backlog_total: Option<i64>,
    pub(crate) backlog_observed_at: Option<String>,
    pub(crate) source_max_invocation_id: Option<i64>,
    pub(crate) raw_files_compression_candidates: usize,
    pub(crate) raw_files_compressed: usize,
    pub(crate) raw_bytes_before: u64,
    pub(crate) raw_bytes_after: u64,
    pub(crate) raw_bytes_after_estimated: u64,
    pub(crate) invocation_details_pruned: usize,
    pub(crate) invocation_rows_archived: usize,
    pub(crate) prompt_cache_conversations_released: usize,
    pub(crate) forward_proxy_attempt_rows_archived: usize,
    pub(crate) pool_upstream_request_attempt_rows_archived: usize,
    pub(crate) quota_snapshot_rows_archived: usize,
    pub(crate) archive_batches_touched: usize,
    pub(crate) archive_batches_deleted: usize,
    pub(crate) raw_files_removed: usize,
    pub(crate) orphan_raw_files_removed: usize,
    pub(crate) model_route_rows_pruned: usize,
    pub(crate) system_task_run_rows_pruned: usize,
    pub(crate) fatal_error: Option<String>,
    pub(crate) recoverable_failure: bool,
    pub(crate) orphan_cleanup_completed: bool,
    pub(crate) batches: Vec<RetentionBatchMetrics>,
    pub(crate) timeout_count: Option<u64>,
    pub(crate) arrival_rates: HashMap<String, f64>,
}

impl RetentionRunSummary {
    pub(crate) fn apply_recorded_defer_reason(&mut self, defer_reason: Option<String>) {
        self.deferred |= defer_reason.is_some();
        // Stage catches use a generic typed-admission placeholder. Publish the
        // recorded cause without replacing an already specific stop reason.
        if self.wait_reason.as_deref() == Some("retention_write_admission")
            && defer_reason.is_some()
        {
            self.wait_reason = defer_reason;
        } else if self.wait_reason.is_none() && (self.deferred || self.budget_exhausted) {
            self.wait_reason = if self.budget_exhausted {
                Some("retention_work_budget".to_string())
            } else {
                defer_reason
            };
        }
    }

    pub(crate) fn processed_row_count(&self) -> u64 {
        self.invocation_details_pruned as u64
            + self.invocation_rows_archived as u64
            + self.forward_proxy_attempt_rows_archived as u64
            + self.pool_upstream_request_attempt_rows_archived as u64
            + self.quota_snapshot_rows_archived as u64
            + self.model_route_rows_pruned as u64
            + self.system_task_run_rows_pruned as u64
    }

    fn touched_anything(&self) -> bool {
        self.raw_files_compression_candidates > 0
            || self.raw_files_compressed > 0
            || self.invocation_details_pruned > 0
            || self.invocation_rows_archived > 0
            || self.prompt_cache_conversations_released > 0
            || self.forward_proxy_attempt_rows_archived > 0
            || self.pool_upstream_request_attempt_rows_archived > 0
            || self.quota_snapshot_rows_archived > 0
            || self.archive_batches_deleted > 0
            || self.raw_files_removed > 0
            || self.orphan_raw_files_removed > 0
            || self.model_route_rows_pruned > 0
            || self.system_task_run_rows_pruned > 0
    }

    pub(crate) fn completion(&self) -> &'static str {
        if self.fatal_error.is_some() {
            "failed"
        } else if self.recoverable_failure
            || self.budget_exhausted
            || self.wait_reason.as_deref() == Some("parallel_work_minute_coverage")
        {
            "partial"
        } else if self.deferred {
            if self.touched_anything() {
                "partial"
            } else {
                "deferred"
            }
        } else if self
            .backlog_total
            .is_some_and(|total| total.max(0) as u64 > self.invocation_rows_archived as u64)
        {
            "partial"
        } else {
            "completed"
        }
    }

    pub(crate) fn core_completion(&self) -> &'static str {
        if self.fatal_error.is_some() {
            "failed"
        } else if self.recoverable_failure {
            "partial"
        } else if self.invocation_rows_archived > 0 || self.invocation_details_pruned > 0 {
            if self.budget_exhausted || self.deferred {
                "partial"
            } else {
                "completed"
            }
        } else if self.budget_exhausted || self.deferred {
            "deferred"
        } else {
            "completed"
        }
    }
}

#[derive(Debug)]
pub(crate) struct RetentionOwnerRunSummary {
    pub(crate) task_key: String,
    pub(crate) source_scope: String,
    pub(crate) unit: String,
    pub(crate) global_total: Option<i64>,
    pub(crate) source_max_id: Option<i64>,
    pub(crate) discovered: usize,
    pub(crate) selected: usize,
    pub(crate) committed: usize,
    pub(crate) batches: usize,
    pub(crate) archive_batches: Vec<RetentionBatchMetrics>,
    pub(crate) raw_files_removed: usize,
    pub(crate) raw_files_planned: usize,
    pub(crate) completion: &'static str,
    pub(crate) wait_reason: Option<String>,
    pub(crate) fatal_error: Option<String>,
    pub(crate) has_more: Option<bool>,
    pub(crate) elapsed_ms: u64,
}

impl RetentionOwnerRunSummary {
    fn new(task_key: &str, source_scope: &str, unit: &str) -> Self {
        Self {
            task_key: task_key.to_string(),
            source_scope: source_scope.to_string(),
            unit: unit.to_string(),
            global_total: None,
            source_max_id: None,
            discovered: 0,
            selected: 0,
            committed: 0,
            batches: 0,
            archive_batches: Vec::new(),
            raw_files_removed: 0,
            raw_files_planned: 0,
            completion: "complete",
            wait_reason: None,
            fatal_error: None,
            has_more: None,
            elapsed_ms: 0,
        }
    }

    fn finish(mut self, started_at: Instant) -> Self {
        self.elapsed_ms = started_at.elapsed().as_millis() as u64;
        self
    }
}

fn classify_retention_owner_error(error: &anyhow::Error) -> (&'static str, Option<String>) {
    if is_retention_archive_file_operation_budget_exhausted(error) {
        return (
            "deferred",
            Some(RETENTION_ARCHIVE_FILE_OPERATION_BUDGET_REASON.to_string()),
        );
    }
    if is_retention_archive_file_operation_no_progress(error) {
        return (
            "failed",
            Some(format!(
                "fatal_error;reason={RETENTION_ARCHIVE_FILE_NO_PROGRESS_REASON};failure_fingerprint:{}",
                retention_error_fingerprint(error)
            )),
        );
    }
    if retention_run_budget_expired() {
        return ("deferred", Some("retention_work_budget".to_string()));
    }
    if is_retention_write_deferred(error) {
        return ("deferred", Some("retention_write_admission".to_string()));
    }
    if crate::is_sqlite_lock_error(error) {
        return (
            "deferred",
            Some(RETENTION_RECOVERY_DEFER_SQLITE_PRESSURE.to_string()),
        );
    }
    (
        "failed",
        Some(format!(
            "fatal_error;failure_fingerprint:{}",
            retention_error_fingerprint(error)
        )),
    )
}

fn apply_retention_owner_error(summary: &mut RetentionOwnerRunSummary, error: &anyhow::Error) {
    let (completion, reason) = classify_retention_owner_error(error);
    summary.completion = if completion == "deferred" && summary.committed > 0 {
        "partial"
    } else {
        completion
    };
    if completion == "failed" {
        summary.fatal_error = reason;
    } else {
        summary.wait_reason = reason;
    }
}

fn apply_retention_owner_post_commit_error(
    summary: &mut RetentionOwnerRunSummary,
    discovered: usize,
    committed: usize,
    error: &anyhow::Error,
) {
    // The owner has already committed its deletion before the follow-up coverage probe.
    summary.discovered = discovered;
    summary.selected = committed;
    summary.committed = committed;
    summary.has_more = Some(true);
    apply_retention_owner_error(summary, error);
}

fn record_retention_owner_batch_facts(summary: &mut RetentionOwnerRunSummary, dataset: &str) {
    let (selected, committed, batches, raw_files_removed) = batch_plan::TASK_BATCH_METRICS
        .try_with(|metrics| {
            metrics
                .borrow()
                .iter()
                .filter(|batch| batch.dataset == dataset)
                .fold(
                    (0usize, 0usize, 0usize, 0usize),
                    |(selected, committed, batches, raw_files_removed), batch| {
                        (
                            selected.saturating_add(batch.batch_rows),
                            committed.saturating_add(batch.committed_rows),
                            batches + 1,
                            raw_files_removed.saturating_add(batch.raw_files_removed),
                        )
                    },
                )
        })
        .unwrap_or_default();
    summary.discovered = batch_plan::discovered_rows(dataset)
        .unwrap_or_default()
        .max(selected);
    summary.selected = selected;
    summary.committed = committed;
    summary.batches = batches;
    summary.raw_files_removed = raw_files_removed;
}

fn retention_owner_success_facts(
    dataset: &str,
    observed_rows: usize,
    dry_run: bool,
) -> (usize, usize, usize) {
    let (batch_selected, batch_committed, _) = batch_plan::TASK_BATCH_METRICS
        .try_with(|metrics| {
            metrics
                .borrow()
                .iter()
                .filter(|batch| batch.dataset == dataset)
                .fold(
                    (0usize, 0usize, 0usize),
                    |(selected, committed, batches), batch| {
                        (
                            selected.saturating_add(batch.batch_rows),
                            committed.saturating_add(batch.committed_rows),
                            batches + 1,
                        )
                    },
                )
        })
        .unwrap_or_default();
    let selected = if dry_run {
        observed_rows
    } else {
        batch_selected.max(observed_rows)
    };
    let committed = if dry_run {
        0
    } else {
        batch_committed.max(observed_rows)
    };
    let discovered = batch_plan::discovered_rows(dataset)
        .unwrap_or_default()
        .max(selected);
    (discovered, selected, committed)
}

fn apply_retention_owner_batch_error(
    summary: &mut RetentionOwnerRunSummary,
    dataset: &str,
    error: &anyhow::Error,
) {
    record_retention_owner_batch_facts(summary, dataset);
    if let Some(has_more) = batch_plan::candidate_window_has_more(dataset) {
        summary.has_more = Some(has_more);
    }
    apply_retention_owner_error(summary, error);
}

fn set_retention_owner_raw_file_facts(
    summary: &mut RetentionOwnerRunSummary,
    raw_files: usize,
    dry_run: bool,
) {
    if dry_run {
        summary.raw_files_planned = raw_files;
        summary.raw_files_removed = 0;
    } else {
        summary.raw_files_removed = raw_files;
    }
}

fn apply_retention_owner_success(
    summary: &mut RetentionOwnerRunSummary,
    discovered: usize,
    selected: usize,
    committed: usize,
    batches: usize,
    raw_files_removed: usize,
    has_more: Option<bool>,
) {
    summary.discovered = discovered;
    summary.selected = selected;
    summary.committed = committed;
    summary.batches = batches;
    summary.raw_files_removed = raw_files_removed;
    summary.has_more = has_more;
    if let Some(reason) = retention_run_defer_reason() {
        summary.wait_reason = Some(reason);
    }
    summary.completion = if summary.wait_reason.is_some() || summary.has_more == Some(true) {
        if summary.committed > 0 {
            "partial"
        } else {
            "deferred"
        }
    } else if selected == 0 {
        "no-work"
    } else {
        "complete"
    };
}

async fn invocation_archive_fence(pool: &Pool<Sqlite>, cutoff: &str) -> Result<(Option<i64>, i64)> {
    let source_max_id =
        sqlx::query_scalar::<_, Option<i64>>("SELECT MAX(id) FROM codex_invocations")
            .fetch_one(pool)
            .await?;
    let total = match source_max_id {
        Some(source_max_id) => {
            sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM codex_invocations WHERE occurred_at < ?1 AND id <= ?2",
            )
            .bind(cutoff)
            .bind(source_max_id)
            .fetch_one(pool)
            .await?
        }
        None => 0,
    };
    Ok((source_max_id, total))
}

async fn remaining_invocation_archive_rows(
    pool: &Pool<Sqlite>,
    cutoff: &str,
    source_max_id: Option<i64>,
) -> Result<i64> {
    let Some(source_max_id) = source_max_id else {
        return Ok(0);
    };
    let remaining = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM codex_invocations WHERE occurred_at < ?1 AND id <= ?2",
    )
    .bind(cutoff)
    .bind(source_max_id)
    .fetch_one(pool)
    .await?;
    Ok(remaining)
}

async fn run_invocation_archive_owner_inner(
    pool: &Pool<Sqlite>,
    config: &AppConfig,
    dry_run: bool,
    shutdown: &CancellationToken,
) -> Result<RetentionOwnerRunSummary> {
    let started_at = Instant::now();
    let cutoff = shanghai_local_cutoff_string(config.invocation_max_days);
    let (source_max_id, global_total) = invocation_archive_fence(pool, &cutoff).await?;
    let mut summary = RetentionOwnerRunSummary::new(
        "retention_archive",
        "expired_invocations:retention_policy",
        "invocation rows",
    );
    summary.source_max_id = source_max_id;
    summary.global_total = Some(global_total);

    if shutdown.is_cancelled() {
        summary.completion = "deferred";
        summary.wait_reason = Some("shutdown".to_string());
        return Ok(summary.finish(started_at));
    }

    if !dry_run {
        let janitor = cleanup_stale_archive_temp_files(config, false)?;
        if janitor.stale_temp_files_removed > 0 {
            info!(
                ?janitor,
                "archive temp janitor removed stale files before invocation archive owner"
            );
        }
        for (stage, result) in [
            (
                "legacy_reconcile",
                reconcile_staged_archive_replacements(pool, Some(config))
                    .await
                    .map(|_| ()),
            ),
            (
                "prepared_reconcile",
                reconcile_retention_prepared_archives(pool, config)
                    .await
                    .map(|_| ()),
            ),
            (
                "legacy_reconcile",
                reconcile_legacy_retention_archive_segments(pool, config)
                    .await
                    .map(|_| ()),
            ),
        ] {
            if let Err(error) = result {
                if is_retention_write_deferred(&error)
                    || is_retention_archive_file_operation_budget_exhausted(&error)
                {
                    summary.wait_reason = Some(if is_retention_write_deferred(&error) {
                        "retention_write_admission".to_string()
                    } else {
                        RETENTION_ARCHIVE_FILE_OPERATION_BUDGET_REASON.to_string()
                    });
                } else {
                    retention_recovery_persist_latest_failure_best_effort(pool, stage, &error)
                        .await;
                    summary.wait_reason = Some("retention_recovery_failure".to_string());
                }
                retention_recovery_log_event(
                    tracing::Level::WARN,
                    stage,
                    "invocation archive recovery preflight deferred",
                );
            }
        }
    }

    if global_total == 0 && summary.wait_reason.is_none() {
        summary.completion = "no-work";
        return Ok(summary.finish(started_at));
    }

    let payload_loss_days = config
        .invocation_success_full_days
        .min(config.invocation_max_days);
    if !dry_run
        && !parallel_work_minute_coverage_ready_for_payload_retention(
            pool,
            shanghai_retention_cutoff(payload_loss_days).timestamp(),
        )
        .await
        .context("failed to verify parallel-work minute coverage before invocation archive")?
    {
        summary.completion = "deferred";
        summary.wait_reason = Some("parallel_work_minute_coverage".to_string());
        return Ok(summary.finish(started_at));
    }

    let archive = archive_old_invocations_with_source_max(
        pool,
        config,
        config.database_path.parent(),
        dry_run,
        &cutoff,
        source_max_id,
    )
    .await;
    match archive {
        Ok((selected, batches, raw_files_removed, _prompt_cache_keys)) => {
            let (discovered, selected_scope, committed) =
                retention_owner_success_facts("codex_invocations", selected, dry_run);
            let has_more = if dry_run {
                batch_plan::candidate_window_has_more("codex_invocations")
            } else {
                match remaining_invocation_archive_rows(pool, &cutoff, source_max_id).await {
                    Ok(remaining) => Some(remaining > 0),
                    Err(error) => {
                        warn!(
                            error = %error,
                            "invocation archive backlog observation failed after commit"
                        );
                        Some(true)
                    }
                }
            };
            apply_retention_owner_success(
                &mut summary,
                discovered,
                selected_scope,
                committed,
                batches,
                raw_files_removed,
                has_more,
            );
            set_retention_owner_raw_file_facts(&mut summary, raw_files_removed, dry_run);
        }
        Err(error) => {
            record_retention_owner_batch_facts(&mut summary, "codex_invocations");
            summary.has_more = Some(
                match remaining_invocation_archive_rows(pool, &cutoff, source_max_id).await {
                    Ok(remaining) => remaining > 0,
                    Err(observation_error) => {
                        warn!(
                            error = %observation_error,
                            "invocation archive backlog observation failed after error"
                        );
                        true
                    }
                },
            );
            apply_retention_owner_error(&mut summary, &error);
        }
    }
    if summary.wait_reason.is_some() && summary.completion == "complete" {
        summary.completion = if summary.committed > 0 || summary.has_more == Some(true) {
            "partial"
        } else {
            "deferred"
        };
    }
    Ok(summary.finish(started_at))
}

async fn run_timestamped_archive_owner(
    pool: &Pool<Sqlite>,
    config: &AppConfig,
    task_key: &str,
    source_scope: &str,
    select_sql: &str,
    cutoff: String,
    dry_run: bool,
) -> Result<RetentionOwnerRunSummary> {
    let started_at = Instant::now();
    let mut summary = RetentionOwnerRunSummary::new(task_key, source_scope, "rows");
    let dataset = if task_key == "pool_upstream_attempt_archive" {
        "pool_upstream_request_attempts"
    } else {
        "forward_proxy_attempts"
    };
    let archive = archive_timestamped_dataset(
        pool,
        config,
        archive_table_spec(dataset),
        select_sql,
        cutoff,
        dry_run,
        config.database_path.parent(),
    )
    .await;
    match archive {
        Ok((selected, batches, raw_files_removed)) => {
            let (discovered, selected_scope, committed) =
                retention_owner_success_facts(dataset, selected, dry_run);
            apply_retention_owner_success(
                &mut summary,
                discovered,
                selected_scope,
                committed,
                batches,
                raw_files_removed,
                batch_plan::candidate_window_has_more(dataset),
            );
            set_retention_owner_raw_file_facts(&mut summary, raw_files_removed, dry_run);
        }
        Err(error) => apply_retention_owner_batch_error(&mut summary, dataset, &error),
    }
    Ok(summary.finish(started_at))
}

async fn run_invocation_detail_prune_owner(
    pool: &Pool<Sqlite>,
    config: &AppConfig,
    dry_run: bool,
) -> Result<RetentionOwnerRunSummary> {
    let started_at = Instant::now();
    let mut summary = RetentionOwnerRunSummary::new(
        "invocation_detail_prune",
        "codex_invocation_details:retention_policy",
        "invocation rows",
    );
    let payload_loss_days = config
        .invocation_success_full_days
        .min(config.invocation_max_days);
    if !dry_run
        && !parallel_work_minute_coverage_ready_for_payload_retention(
            pool,
            shanghai_retention_cutoff(payload_loss_days).timestamp(),
        )
        .await
        .context(
            "failed to verify parallel-work minute coverage before invocation detail pruning",
        )?
    {
        summary.completion = "deferred";
        summary.wait_reason = Some("parallel_work_minute_coverage".to_string());
        return Ok(summary.finish(started_at));
    }
    match prune_old_invocation_details(pool, config, config.database_path.parent(), dry_run).await {
        Ok((selected, batches, raw_files_removed)) => {
            let (discovered, selected_scope, committed) =
                retention_owner_success_facts("codex_invocation_details", selected, dry_run);
            apply_retention_owner_success(
                &mut summary,
                discovered,
                selected_scope,
                committed,
                batches,
                raw_files_removed,
                batch_plan::candidate_window_has_more("codex_invocation_details"),
            );
            set_retention_owner_raw_file_facts(&mut summary, raw_files_removed, dry_run);
        }
        Err(error) => {
            apply_retention_owner_batch_error(&mut summary, "codex_invocation_details", &error)
        }
    }
    Ok(summary.finish(started_at))
}

async fn run_quota_snapshot_compaction_owner(
    pool: &Pool<Sqlite>,
    config: &AppConfig,
    dry_run: bool,
) -> Result<RetentionOwnerRunSummary> {
    let started_at = Instant::now();
    let mut summary = RetentionOwnerRunSummary::new(
        "quota_snapshot_compaction",
        "codex_quota_snapshots:retention_policy",
        "quota snapshot rows",
    );
    match compact_old_quota_snapshots(pool, config, dry_run).await {
        Ok((selected, batches)) => {
            let (discovered, selected_scope, committed) =
                retention_owner_success_facts("codex_quota_snapshots", selected, dry_run);
            apply_retention_owner_success(
                &mut summary,
                discovered,
                selected_scope,
                committed,
                batches,
                0,
                batch_plan::candidate_window_has_more("codex_quota_snapshots"),
            );
        }
        Err(error) => {
            apply_retention_owner_batch_error(&mut summary, "codex_quota_snapshots", &error)
        }
    }
    Ok(summary.finish(started_at))
}

async fn run_model_route_expiry_owner(
    pool: &Pool<Sqlite>,
    config: &AppConfig,
    dry_run: bool,
) -> Result<RetentionOwnerRunSummary> {
    let started_at = Instant::now();
    let mut summary = RetentionOwnerRunSummary::new(
        "model_route_expiry",
        "model_routes:expiry",
        "model route rows",
    );
    let candidate_limit = retention_candidate_limit(config, "model_route_purge");
    if dry_run {
        match crate::upstream_accounts::inspect_expired_model_routes_bounded(pool, candidate_limit)
            .await
        {
            Ok((discovered, has_more)) => apply_retention_owner_success(
                &mut summary,
                discovered,
                discovered.min(candidate_limit),
                0,
                0,
                0,
                Some(has_more),
            ),
            Err(error) => apply_retention_owner_error(&mut summary, &error),
        }
    } else {
        let (discovered, _) = match crate::upstream_accounts::inspect_expired_model_routes_bounded(
            pool,
            candidate_limit,
        )
        .await
        {
            Ok(facts) => facts,
            Err(error) => {
                apply_retention_owner_error(&mut summary, &error);
                return Ok(summary.finish(started_at));
            }
        };
        summary.discovered = discovered;
        let Some(admission) = acquire_retention_write_admission("model_route_purge").await else {
            apply_retention_owner_success(&mut summary, discovered, 0, 0, 0, 0, Some(true));
            return Ok(summary.finish(started_at));
        };
        let execute_started = Instant::now();
        let committed =
            match crate::upstream_accounts::purge_model_routes_bounded(pool, candidate_limit).await
            {
                Ok(committed) => committed as usize,
                Err(error) => {
                    apply_retention_owner_error(&mut summary, &error);
                    return Ok(summary.finish(started_at));
                }
            };
        let commit_started = Instant::now();
        retention_record_commit!(
            "model_route_purge",
            admission.admission_mode(),
            committed,
            committed.saturating_mul(128),
            Duration::ZERO,
            admission.lock_wait(),
            commit_started.duration_since(execute_started),
            commit_started.elapsed(),
            admission.p1_waiter_count(),
            usize::from(committed >= candidate_limit),
        );
        drop(admission);
        summary.selected = committed;
        summary.committed = committed;
        let has_more = match crate::upstream_accounts::inspect_expired_model_routes_bounded(
            pool,
            candidate_limit,
        )
        .await
        {
            Ok((remaining, _has_more)) => remaining > 0,
            Err(error) => {
                apply_retention_owner_post_commit_error(
                    &mut summary,
                    discovered,
                    committed,
                    &error,
                );
                return Ok(summary.finish(started_at));
            }
        };
        apply_retention_owner_success(
            &mut summary,
            discovered,
            committed,
            committed,
            0,
            0,
            Some(has_more),
        );
    }
    Ok(summary.finish(started_at))
}

async fn run_task_run_history_cleanup_owner(
    pool: &Pool<Sqlite>,
    dry_run: bool,
) -> Result<RetentionOwnerRunSummary> {
    let started_at = Instant::now();
    let mut summary = RetentionOwnerRunSummary::new(
        "task_run_history_cleanup",
        "task_runs:history_retention",
        "task-run rows",
    );
    let legacy_selected = match prune_system_task_runs_for_owner(pool, dry_run).await {
        Ok(selected) => selected,
        Err(error) => {
            let committed = error
                .downcast_ref::<SystemTaskRunRetentionPartialError>()
                .map(|partial| partial.committed)
                .unwrap_or_default();
            summary.discovered = committed;
            summary.selected = committed;
            summary.committed = if dry_run { 0 } else { committed };
            summary.batches = committed
                .saturating_add(SYSTEM_TASK_RUN_RETENTION_TERMINAL_BATCH_ROWS - 1)
                / SYSTEM_TASK_RUN_RETENTION_TERMINAL_BATCH_ROWS;
            summary.has_more = Some(true);
            if let Some(partial) = error.downcast_ref::<SystemTaskRunRetentionPartialError>() {
                apply_retention_owner_error(&mut summary, &partial.cause);
            } else {
                apply_retention_owner_error(&mut summary, &error);
            }
            return Ok(summary.finish(started_at));
        }
    };
    let legacy_batches = legacy_selected
        .saturating_add(SYSTEM_TASK_RUN_RETENTION_TERMINAL_BATCH_ROWS.saturating_sub(1))
        / SYSTEM_TASK_RUN_RETENTION_TERMINAL_BATCH_ROWS;
    let managed_history = if let Some(store) = crate::maintenance_store::global() {
        match store.cleanup_expired_history_bounded(dry_run).await {
            Ok(result) => result,
            Err(error) => {
                summary.discovered = legacy_selected;
                summary.selected = legacy_selected;
                summary.committed = if dry_run { 0 } else { legacy_selected };
                summary.batches = legacy_batches;
                apply_retention_owner_error(&mut summary, &error);
                return Ok(summary.finish(started_at));
            }
        }
    } else {
        crate::maintenance_store::ManagedTaskHistoryCleanupResult::default()
    };
    let selected = legacy_selected.saturating_add(managed_history.selected);
    let committed = if dry_run {
        0
    } else {
        legacy_selected.saturating_add(managed_history.committed)
    };
    let batches = legacy_batches + usize::from(managed_history.selected > 0);
    apply_retention_owner_success(
        &mut summary,
        selected,
        selected,
        committed,
        batches,
        0,
        Some(
            legacy_selected >= SYSTEM_TASK_RUN_RETENTION_MAX_ROWS_PER_PASS
                || managed_history.has_more,
        ),
    );
    if !dry_run && summary.completion == "complete" && committed != selected {
        summary.completion = if committed > 0 { "partial" } else { "deferred" };
    }
    Ok(summary.finish(started_at))
}

async fn run_retention_owner_task_inner(
    pool: &Pool<Sqlite>,
    config: &AppConfig,
    task_key: &str,
    dry_run: bool,
    shutdown: &CancellationToken,
) -> Result<RetentionOwnerRunSummary> {
    if shutdown.is_cancelled() {
        let mut summary = RetentionOwnerRunSummary::new(task_key, task_key, "rows");
        summary.completion = "deferred";
        summary.wait_reason = Some("shutdown".to_string());
        return Ok(summary);
    }
    match task_key {
        "retention_archive" => {
            run_invocation_archive_owner_inner(pool, config, dry_run, shutdown).await
        }
        "pool_upstream_attempt_archive" => {
            run_timestamped_archive_owner(
                pool,
                config,
                task_key,
                "pool_upstream_request_attempts:retention_policy",
                "SELECT id, occurred_at AS timestamp_value FROM pool_upstream_request_attempts WHERE occurred_at < ?1 ORDER BY occurred_at ASC, id ASC LIMIT ?2",
                shanghai_local_cutoff_string(config.pool_upstream_request_attempts_retention_days),
                dry_run,
            )
            .await
        }
        "forward_proxy_attempt_archive" => {
            run_timestamped_archive_owner(
                pool,
                config,
                task_key,
                "forward_proxy_attempts:retention_policy",
                "SELECT id, occurred_at AS timestamp_value FROM forward_proxy_attempts WHERE occurred_at < ?1 ORDER BY occurred_at ASC, id ASC LIMIT ?2",
                shanghai_utc_cutoff_string(config.forward_proxy_attempts_retention_days),
                dry_run,
            )
            .await
        }
        "invocation_detail_prune" => run_invocation_detail_prune_owner(pool, config, dry_run).await,
        "quota_snapshot_compaction" => {
            run_quota_snapshot_compaction_owner(pool, config, dry_run).await
        }
        "model_route_expiry" => run_model_route_expiry_owner(pool, config, dry_run).await,
        "task_run_history_cleanup" => run_task_run_history_cleanup_owner(pool, dry_run).await,
        _ => bail!("unsupported retention owner task: {task_key}"),
    }
}

fn populate_retention_owner_batch_metrics(
    batches: &mut [RetentionBatchMetrics],
    arrival_rates: &HashMap<String, f64>,
    elapsed: Duration,
) {
    let run_seconds = elapsed.as_secs_f64();
    for batch in batches {
        batch.committed_rows_per_second =
            (run_seconds > 0.0).then_some(batch.committed_rows as f64 / run_seconds);
        batch.arrival_rows_per_second = arrival_rates.get(&batch.dataset).copied();
        batch.service_rate_multiple = batch
            .arrival_rows_per_second
            .filter(|arrival| *arrival > 0.0)
            .and_then(|arrival| {
                batch
                    .committed_rows_per_second
                    .map(|service| service / arrival)
            });
    }
}

pub(crate) async fn run_retention_owner_task(
    pool: &Pool<Sqlite>,
    config: &AppConfig,
    task_key: &str,
    dry_run: bool,
    shutdown: &CancellationToken,
) -> Result<RetentionOwnerRunSummary> {
    let started_at = Instant::now();
    let result = RETENTION_MAINTENANCE_ACTIVE
        .scope(
            (),
            RETENTION_SHUTDOWN.scope(
                shutdown.clone(),
                RETENTION_CURRENT_PREPARED_KEY.scope(
                    RefCell::new(None),
                    batch_plan::TASK_DISCOVERED_ROWS.scope(
                        RefCell::new(std::collections::BTreeMap::new()),
                        batch_plan::TASK_CANDIDATE_WINDOWS.scope(
                            RefCell::new(std::collections::BTreeMap::new()),
                            batch_plan::TASK_BATCH_METRICS.scope(
                                RefCell::new(Vec::new()),
                                RETENTION_RUN_DEFER_REASON.scope(RefCell::new(None), async {
                                    let result = run_retention_owner_task_inner(
                                        pool, config, task_key, dry_run, shutdown,
                                    )
                                    .await;
                                    let archive_batches = batch_plan::TASK_BATCH_METRICS
                                        .with(|metrics| metrics.borrow().clone());
                                    let candidate_window_has_more =
                                        batch_plan::TASK_CANDIDATE_WINDOWS.with(|windows| {
                                            let windows = windows.borrow();
                                            (!windows.is_empty())
                                                .then(|| windows.values().any(|has_more| *has_more))
                                        });
                                    (result, archive_batches, candidate_window_has_more)
                                }),
                            ),
                        ),
                    ),
                ),
            ),
        )
        .await;
    let arrival_rates = batch_plan::arrival_rates(pool).await;
    match result {
        (Ok(mut summary), archive_batches, candidate_window_has_more) => {
            summary.archive_batches = archive_batches;
            summary.raw_files_removed = summary.raw_files_removed.max(
                summary
                    .archive_batches
                    .iter()
                    .map(|batch| batch.raw_files_removed)
                    .sum(),
            );
            populate_retention_owner_batch_metrics(
                &mut summary.archive_batches,
                &arrival_rates,
                started_at.elapsed(),
            );
            if summary.has_more.is_none() {
                summary.has_more = candidate_window_has_more;
            }
            if dry_run {
                summary.raw_files_planned =
                    summary.raw_files_planned.max(summary.raw_files_removed);
                summary.raw_files_removed = 0;
            }
            Ok(summary.finish(started_at))
        }
        (Err(error), archive_batches, candidate_window_has_more) => {
            let mut summary = RetentionOwnerRunSummary::new(task_key, task_key, "rows");
            summary.archive_batches = archive_batches;
            populate_retention_owner_batch_metrics(
                &mut summary.archive_batches,
                &arrival_rates,
                started_at.elapsed(),
            );
            summary.raw_files_removed = summary
                .archive_batches
                .iter()
                .map(|batch| batch.raw_files_removed)
                .sum();
            summary.has_more = candidate_window_has_more;
            apply_retention_owner_error(&mut summary, &error);
            Ok(summary.finish(started_at))
        }
    }
}

fn mark_archive_file_operation_budget_exhausted(
    summary: &mut RetentionRunSummary,
    error: &anyhow::Error,
) -> bool {
    if !is_retention_archive_file_operation_budget_exhausted(error) {
        return false;
    }
    summary.deferred = true;
    summary.budget_exhausted = true;
    summary.wait_reason = Some(RETENTION_ARCHIVE_FILE_OPERATION_BUDGET_REASON.to_string());
    true
}

#[derive(Debug, FromRow)]
pub(crate) struct SystemTaskRunRetentionCandidate {
    pub(crate) id: i64,
}

pub(crate) fn system_task_run_retention_now_epoch_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u64::MAX as u128) as u64
}

pub(crate) fn system_task_run_retention_schedule_next(now_epoch_ms: u64, delay: Duration) {
    let next = now_epoch_ms.saturating_add(delay.as_millis().min(u64::MAX as u128) as u64);
    let mut current = SYSTEM_TASK_RUN_RETENTION_NEXT_PASS_EPOCH_MS.load(Ordering::Acquire);
    while next > current {
        match SYSTEM_TASK_RUN_RETENTION_NEXT_PASS_EPOCH_MS.compare_exchange(
            current,
            next,
            Ordering::AcqRel,
            Ordering::Acquire,
        ) {
            Ok(_) => return,
            Err(actual) => current = actual,
        }
    }
}

pub(crate) fn system_task_run_retention_pass_is_due(now_epoch_ms: u64) -> bool {
    let _schedule_guard = SYSTEM_TASK_RUN_RETENTION_SCHEDULE_LOCK
        .lock()
        .expect("system task retention schedule lock");
    let next = SYSTEM_TASK_RUN_RETENTION_NEXT_PASS_EPOCH_MS.load(Ordering::Acquire);
    if next > now_epoch_ms {
        return false;
    }
    SYSTEM_TASK_RUN_RETENTION_NEXT_PASS_EPOCH_MS
        .compare_exchange(
            next,
            now_epoch_ms.saturating_add(
                SYSTEM_TASK_RUN_RETENTION_PASS_INTERVAL
                    .as_millis()
                    .min(u64::MAX as u128) as u64,
            ),
            Ordering::AcqRel,
            Ordering::Acquire,
        )
        .is_ok()
}

#[cfg(test)]
pub(crate) fn reset_system_task_run_retention_schedule() {
    let _schedule_guard = SYSTEM_TASK_RUN_RETENTION_SCHEDULE_LOCK
        .lock()
        .expect("system task retention schedule lock");
    SYSTEM_TASK_RUN_RETENTION_NEXT_PASS_EPOCH_MS.store(0, Ordering::Release);
}

pub(crate) fn system_task_run_retention_handle_pressure(error: &anyhow::Error) -> bool {
    if !crate::db_pressure::is_db_pressure_error(error) {
        return false;
    }
    let now_epoch_ms = system_task_run_retention_now_epoch_ms();
    system_task_run_retention_schedule_next(
        now_epoch_ms,
        SYSTEM_TASK_RUN_RETENTION_PRESSURE_BACKOFF,
    );
    retention_record_defer("system_task_run_retention", "sqlite_pressure");
    crate::db_pressure::global_db_pressure_gate().record_error("system_task_run_retention", error);
    retention_record_error("system_task_run_retention", error);
    warn!(error = %error, "system task run retention deferred after SQLite pressure");
    true
}

pub(crate) fn system_task_run_retention_admission_requires_pressure_backoff(
    reason: Option<crate::db_pressure::DbPressureDenyReason>,
) -> bool {
    matches!(
        reason,
        Some(crate::db_pressure::DbPressureDenyReason::PressureCooldown { .. })
    )
}

#[derive(Debug)]
struct SystemTaskRunRetentionPartialError {
    committed: usize,
    cause: anyhow::Error,
}

impl std::fmt::Display for SystemTaskRunRetentionPartialError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "system task run retention failed after committing {} rows: {}",
            self.committed, self.cause
        )
    }
}

impl std::error::Error for SystemTaskRunRetentionPartialError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(self.cause.as_ref())
    }
}

fn system_task_run_retention_partial_error(
    committed: usize,
    cause: anyhow::Error,
) -> anyhow::Error {
    anyhow::Error::new(SystemTaskRunRetentionPartialError { committed, cause })
}

pub(crate) async fn prune_system_task_runs(pool: &Pool<Sqlite>, dry_run: bool) -> Result<usize> {
    prune_system_task_runs_inner(pool, dry_run, true).await
}

async fn prune_system_task_runs_for_owner(pool: &Pool<Sqlite>, dry_run: bool) -> Result<usize> {
    prune_system_task_runs_inner(pool, dry_run, false).await
}

async fn prune_system_task_runs_inner(
    pool: &Pool<Sqlite>,
    dry_run: bool,
    respect_process_schedule: bool,
) -> Result<usize> {
    let now_epoch_ms = system_task_run_retention_now_epoch_ms();
    if respect_process_schedule && !system_task_run_retention_pass_is_due(now_epoch_ms) {
        return Ok(0);
    }

    let success_cutoff = format_utc_iso_millis(Utc::now() - ChronoDuration::days(30));
    let failed_cutoff = format_utc_iso_millis(Utc::now() - ChronoDuration::days(180));
    if dry_run {
        let candidates: i64 = sqlx::query_scalar(
            r#"
            WITH ranked AS (
                SELECT
                    task_kind,
                    status,
                    started_at,
                    ROW_NUMBER() OVER (
                        PARTITION BY task_kind, status
                        ORDER BY started_at DESC, id DESC
                    ) AS retention_rank
                FROM system_task_runs
                WHERE status IN ('success', 'skipped', 'failed')
                  AND strftime('%Y-%m-%dT%H:%M:%fZ', started_at) = started_at
            )
            SELECT COUNT(*)
            FROM (
                SELECT 1
                FROM ranked
                WHERE retention_rank > ?1
                  AND (
                      (status IN ('success', 'skipped')
                        AND (started_at < ?2 OR retention_rank > 5000))
                      OR (status = 'failed'
                        AND (started_at < ?3 OR retention_rank > 10000))
                  )
                LIMIT ?4
            )
            "#,
        )
        .bind(SYSTEM_TASK_RUN_RETENTION_KEEP_RECENT)
        .bind(&success_cutoff)
        .bind(&failed_cutoff)
        .bind(SYSTEM_TASK_RUN_RETENTION_MAX_ROWS_PER_PASS as i64 + 1)
        .fetch_one(pool)
        .await
        .map_err(anyhow::Error::from)
        .or_else(|error| {
            if system_task_run_retention_handle_pressure(&error) {
                Ok(0)
            } else {
                Err(error.context("failed to count system task retention candidates"))
            }
        })
        .map_err(|error| system_task_run_retention_partial_error(0, error))?;
        return Ok((candidates.max(0) as usize).min(SYSTEM_TASK_RUN_RETENTION_MAX_ROWS_PER_PASS));
    }
    let mut pruned = 0usize;
    let candidates = match sqlx::query_as::<_, SystemTaskRunRetentionCandidate>(
        r#"
            WITH ranked AS (
                SELECT
                    id,
                    task_kind,
                    status,
                    started_at,
                    ROW_NUMBER() OVER (
                        PARTITION BY task_kind, status
                        ORDER BY started_at DESC, id DESC
                    ) AS retention_rank
                FROM system_task_runs
                WHERE status IN ('success', 'skipped', 'failed')
                  AND strftime('%Y-%m-%dT%H:%M:%fZ', started_at) = started_at
            )
            SELECT id
            FROM ranked
            WHERE retention_rank > ?1
              AND (
                  (status IN ('success', 'skipped')
                    AND (started_at < ?2 OR retention_rank > 5000))
                  OR (status = 'failed'
                    AND (started_at < ?3 OR retention_rank > 10000))
              )
            ORDER BY started_at ASC, id ASC
            LIMIT ?4
            "#,
    )
    .bind(SYSTEM_TASK_RUN_RETENTION_KEEP_RECENT)
    .bind(&success_cutoff)
    .bind(&failed_cutoff)
    .bind(SYSTEM_TASK_RUN_RETENTION_MAX_ROWS_PER_PASS as i64)
    .fetch_all(pool)
    .await
    {
        Ok(candidates) => candidates,
        Err(error) => {
            let error = anyhow::Error::from(error);
            if system_task_run_retention_handle_pressure(&error) {
                return Ok(0);
            }
            return Err(system_task_run_retention_partial_error(
                0,
                error.context("failed to select system task retention candidates"),
            ));
        }
    };
    if candidates.is_empty() {
        return Ok(0);
    }

    for candidates in candidates.chunks(SYSTEM_TASK_RUN_RETENTION_TERMINAL_BATCH_ROWS) {
        let Some(admission) = acquire_retention_write_admission("system_task_run_retention").await
        else {
            // Admission can be denied before a SQL call observes the pressure error. A
            // cooldown is distinct from shutdown or a normally busy background slot.
            if system_task_run_retention_admission_requires_pressure_backoff(
                crate::db_pressure::global_db_pressure_gate().background_deny_reason(),
            ) {
                system_task_run_retention_schedule_next(
                    system_task_run_retention_now_epoch_ms(),
                    SYSTEM_TASK_RUN_RETENTION_PRESSURE_BACKOFF,
                );
            }
            break;
        };
        let execute_started = Instant::now();
        let ids = candidates
            .iter()
            .map(|candidate| candidate.id)
            .collect::<Vec<_>>();
        let mut delete = QueryBuilder::<Sqlite>::new("DELETE FROM system_task_runs WHERE id IN (");
        let mut separated = delete.separated(", ");
        for id in &ids {
            separated.push_bind(id);
        }
        separated.push_unseparated(")");
        let mut transaction = match pool.begin().await {
            Ok(transaction) => transaction,
            Err(error) => {
                let error = anyhow::Error::from(error);
                if system_task_run_retention_handle_pressure(&error) {
                    return Ok(pruned);
                }
                return Err(system_task_run_retention_partial_error(
                    pruned,
                    error.context("failed to begin system task retention batch"),
                ));
            }
        };
        let deleted = match delete.build().execute(&mut *transaction).await {
            Ok(result) => result.rows_affected() as usize,
            Err(error) => {
                let error = anyhow::Error::from(error);
                if system_task_run_retention_handle_pressure(&error) {
                    return Ok(pruned);
                }
                return Err(system_task_run_retention_partial_error(
                    pruned,
                    error.context("failed to delete system task retention batch"),
                ));
            }
        };
        if let Err(error) = transaction.commit().await {
            let error = anyhow::Error::from(error);
            if system_task_run_retention_handle_pressure(&error) {
                return Ok(pruned);
            }
            return Err(system_task_run_retention_partial_error(
                pruned,
                error.context("failed to commit system task retention batch"),
            ));
        }
        retention_record_commit!(
            "system_task_run_retention",
            admission.admission_mode(),
            deleted,
            deleted.saturating_mul(128),
            Duration::ZERO,
            admission.lock_wait(),
            execute_started.elapsed(),
            Duration::ZERO,
            admission.p1_waiter_count(),
            usize::from(candidates.len() == SYSTEM_TASK_RUN_RETENTION_TERMINAL_BATCH_ROWS),
        );
        pruned += deleted;
        if deleted < candidates.len() {
            break;
        }
    }

    Ok(pruned)
}

#[derive(Debug, FromRow)]
pub(crate) struct RetentionBacklogObservationSnapshot {
    pub(crate) invocation_count: i64,
    pub(crate) oldest_occurred_at: Option<String>,
    pub(crate) source_max_invocation_id: Option<i64>,
}

pub(crate) fn retention_backlog_observer_bucket_start(
    value: chrono::DateTime<Utc>,
) -> chrono::DateTime<Utc> {
    value
        - ChronoDuration::seconds(i64::from(value.minute() * 60 + value.second()))
        - ChronoDuration::nanoseconds(i64::from(value.nanosecond()))
}

pub(crate) fn retention_backlog_max_overdue_seconds(
    occurred_at: Option<&str>,
    retention_days: u64,
    observed_at: chrono::DateTime<Utc>,
) -> Option<i64> {
    let occurred_at = crate::stats::parse_to_utc_datetime(occurred_at?)?;
    let local_date = occurred_at.with_timezone(&Shanghai).date_naive();
    let eligible_date =
        local_date.checked_add_signed(ChronoDuration::days(retention_days as i64))?;
    let eligible_local = eligible_date.and_hms_opt(0, 0, 0)?;
    let eligible_at = Shanghai
        .from_local_datetime(&eligible_local)
        .single()?
        .with_timezone(&Utc);
    Some((observed_at - eligible_at).num_seconds().max(0))
}

pub(crate) async fn clear_retention_backlog_observer_progress_handler(
    connection: &mut sqlx::pool::PoolConnection<Sqlite>,
    deadline: Instant,
) -> Result<()> {
    let mut handle = tokio::time::timeout(
        deadline.saturating_duration_since(Instant::now()),
        connection.lock_handle(),
    )
    .await
    .map_err(|_| anyhow!("retention backlog observer progress-handler cleanup exceeded 2s"))??;
    handle.remove_progress_handler();
    Ok(())
}

pub(crate) async fn observe_retention_backlog_once(state: &AppState) -> Result<()> {
    let Some(store) = crate::maintenance_store::global() else {
        return Ok(());
    };
    let _background_permit = crate::db_pressure::global_db_pressure_gate()
        .try_begin_background("retention_backlog_observer")
        .map_err(|reason| anyhow!("retention backlog observer deferred: {reason}"))?;
    let observed_at = Utc::now();
    let cutoff = shanghai_local_cutoff_string(state.config.invocation_max_days);
    let deadline = Instant::now() + RETENTION_BACKLOG_OBSERVER_QUERY_BUDGET;
    let remaining = deadline.saturating_duration_since(Instant::now());
    if remaining.is_zero() {
        return Err(anyhow!(
            "retention backlog observer budget expired before connection acquisition"
        ));
    }
    let mut connection = tokio::time::timeout(remaining, state.pool.acquire())
        .await
        .map_err(|_| anyhow!("retention backlog observer connection acquisition exceeded 2s"))??;
    let interrupted = Arc::new(AtomicBool::new(false));
    {
        let interrupted = interrupted.clone();
        let mut handle = tokio::time::timeout(
            deadline.saturating_duration_since(Instant::now()),
            connection.lock_handle(),
        )
        .await
        .map_err(|_| {
            anyhow!("retention backlog observer SQLite handle acquisition exceeded 2s")
        })??;
        handle.set_progress_handler(RETENTION_BACKLOG_OBSERVER_PROGRESS_OPS, move || {
            let within_budget = Instant::now() < deadline;
            if !within_budget {
                interrupted.store(true, Ordering::Release);
            }
            within_budget
        });
    }
    let query = sqlx::query_as::<_, RetentionBacklogObservationSnapshot>(
        "WITH source AS (
             SELECT MAX(id) AS source_max_invocation_id
             FROM codex_invocations
         ), eligible AS (
             SELECT invocations.occurred_at
             FROM codex_invocations AS invocations
             CROSS JOIN source
             WHERE invocations.occurred_at < ?1
               AND invocations.id <= source.source_max_invocation_id
         )
         SELECT
             COUNT(*) AS invocation_count,
             MIN(eligible.occurred_at) AS oldest_occurred_at,
             source.source_max_invocation_id AS source_max_invocation_id
         FROM eligible
         CROSS JOIN source",
    )
    .bind(&cutoff)
    .fetch_one(&mut *connection);
    let result =
        tokio::time::timeout(deadline.saturating_duration_since(Instant::now()), query).await;
    if let Err(error) =
        clear_retention_backlog_observer_progress_handler(&mut connection, deadline).await
    {
        connection.close_on_drop();
        return Err(error);
    }
    let snapshot = match result {
        Ok(Ok(snapshot)) => snapshot,
        Ok(Err(error)) if interrupted.load(Ordering::Acquire) => {
            connection.close_on_drop();
            return Err(anyhow!(
                "retention backlog observer SQLite query was cancelled: {error}"
            ));
        }
        Ok(Err(error)) => {
            connection.close_on_drop();
            return Err(error.into());
        }
        Err(_) => {
            connection.close_on_drop();
            return Err(anyhow!("retention backlog observer query exceeded 2s"));
        }
    };
    drop(connection);
    record_backlog_observation(
        snapshot.invocation_count,
        snapshot
            .oldest_occurred_at
            .as_deref()
            .and_then(crate::stats::parse_to_utc_datetime),
    );
    store
        .record_retention_backlog_observation(
            crate::maintenance_store::RetentionBacklogObservation {
                bucket_start: format_utc_iso_millis(retention_backlog_observer_bucket_start(
                    observed_at,
                )),
                observed_at: format_utc_iso_millis(observed_at),
                invocation_count: snapshot.invocation_count,
                max_overdue_seconds: retention_backlog_max_overdue_seconds(
                    snapshot.oldest_occurred_at.as_deref(),
                    state.config.invocation_max_days,
                    observed_at,
                ),
                retention_days: state.config.invocation_max_days as i64,
                cutoff,
                source_max_invocation_id: snapshot.source_max_invocation_id,
            },
        )
        .await?;
    Ok(())
}

pub(crate) fn spawn_retention_backlog_observer(
    state: Arc<AppState>,
    cancel: CancellationToken,
) -> JoinHandle<()> {
    crate::observability::contention::spawn_performance(state.observability.clone(), async move {
        let mut ticker = interval(RETENTION_BACKLOG_OBSERVER_INTERVAL);
        ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                _ = cancel.cancelled() => return,
                _ = ticker.tick() => {
                    if let Err(error) = observe_retention_backlog_once(&state).await {
                        debug!(error = %error, "retention backlog observation was unavailable");
                    }
                }
            }
        }
    })
}

pub(crate) fn should_stop_data_retention_maintenance(shutdown: Option<&CancellationToken>) -> bool {
    let should_stop = shutdown.is_some_and(CancellationToken::is_cancelled);
    if should_stop {
        info!(
            "data retention maintenance stopped at a safe boundary because shutdown is in progress"
        );
    }
    if !should_stop && retention_run_budget_expired() {
        info!(
            "data retention maintenance stopped at a safe boundary because an operation deadline expired"
        );
    }
    should_stop || retention_run_budget_expired()
}

pub(crate) async fn run_data_retention_maintenance(
    pool: &Pool<Sqlite>,
    config: &AppConfig,
    dry_run_override: Option<bool>,
    shutdown: Option<&CancellationToken>,
) -> Result<RetentionRunSummary> {
    run_data_retention_maintenance_with_prompt_cache(pool, config, dry_run_override, shutdown, None)
        .await
}

pub(crate) async fn run_data_retention_maintenance_with_prompt_cache(
    pool: &Pool<Sqlite>,
    config: &AppConfig,
    dry_run_override: Option<bool>,
    shutdown: Option<&CancellationToken>,
    prompt_cache_conversation_cache: Option<&Arc<Mutex<PromptCacheConversationsCacheState>>>,
) -> Result<RetentionRunSummary> {
    let run_started_at = Instant::now();
    let run = async {
        if let Some(shutdown) = shutdown {
            RETENTION_SHUTDOWN
                .scope(
                    shutdown.clone(),
                    run_data_retention_maintenance_with_task_run_prune(
                        pool,
                        config,
                        dry_run_override,
                        Some(shutdown),
                        prompt_cache_conversation_cache,
                    ),
                )
                .await
        } else {
            run_data_retention_maintenance_with_task_run_prune(
                pool,
                config,
                dry_run_override,
                None,
                prompt_cache_conversation_cache,
            )
            .await
        }
    };
    let result = RETENTION_MAINTENANCE_ACTIVE
        .scope(
            (),
            RETENTION_CURRENT_PREPARED_KEY.scope(
                RefCell::new(None),
                batch_plan::TASK_DISCOVERED_ROWS.scope(
                    RefCell::new(std::collections::BTreeMap::new()),
                    batch_plan::TASK_CANDIDATE_WINDOWS.scope(
                        RefCell::new(std::collections::BTreeMap::new()),
                        batch_plan::TASK_BATCH_METRICS.scope(
                            RefCell::new(Vec::new()),
                            RETENTION_RUN_DEFER_REASON.scope(RefCell::new(None), async {
                                run.await.map(|mut summary| {
                                    batch_plan::collect_run_metrics(&mut summary);
                                    let defer_reason = RETENTION_RUN_DEFER_REASON
                                        .with(|reason| reason.borrow().clone());
                                    (summary, defer_reason)
                                })
                            }),
                        ),
                    ),
                ),
            ),
        )
        .await;
    result.map(|(mut summary, defer_reason)| {
        summary.apply_recorded_defer_reason(defer_reason);
        summary.work_budget_ms = None;
        summary.elapsed_ms = Some(run_started_at.elapsed().as_millis() as u64);
        summary.timeout_count = None;
        let run_seconds = run_started_at.elapsed().as_secs_f64();
        for batch in &mut summary.batches {
            batch.committed_rows_per_second =
                (run_seconds > 0.0).then_some(batch.committed_rows as f64 / run_seconds);
            batch.arrival_rows_per_second = summary.arrival_rates.get(&batch.dataset).copied();
            batch.service_rate_multiple = batch
                .arrival_rows_per_second
                .filter(|arrival| *arrival > 0.0)
                .and_then(|arrival| {
                    batch
                        .committed_rows_per_second
                        .map(|service| service / arrival)
                });
        }
        summary
    })
}

pub(crate) async fn run_data_retention_maintenance_with_task_run_prune(
    pool: &Pool<Sqlite>,
    config: &AppConfig,
    dry_run_override: Option<bool>,
    shutdown: Option<&CancellationToken>,
    prompt_cache_conversation_cache: Option<&Arc<Mutex<PromptCacheConversationsCacheState>>>,
) -> Result<RetentionRunSummary> {
    let dry_run = dry_run_override.unwrap_or(config.retention_dry_run);
    let result = run_data_retention_maintenance_inner(
        pool,
        config,
        dry_run_override,
        shutdown,
        prompt_cache_conversation_cache,
    )
    .await;
    // This janitor must run even when an earlier archive stage fails. Otherwise every
    // failed pass adds a task-run record while the retention policy that bounds those
    // records is unreachable until the unrelated failure clears.
    let settlement_started_at = Instant::now();
    let stop_for_shutdown_or_budget = should_stop_data_retention_maintenance(shutdown);
    let budget_exhausted = retention_run_budget_expired();
    let task_run_prune = if stop_for_shutdown_or_budget {
        Ok(0)
    } else {
        prune_system_task_runs(pool, dry_run).await
    };
    let settlement_ms = settlement_started_at.elapsed().as_millis() as u64;
    match result {
        Ok(mut summary) => {
            summary.budget_exhausted |= budget_exhausted;
            summary.settlement_ms = Some(settlement_ms);
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

pub(crate) async fn run_data_retention_maintenance_inner(
    pool: &Pool<Sqlite>,
    config: &AppConfig,
    dry_run_override: Option<bool>,
    shutdown: Option<&CancellationToken>,
    _prompt_cache_conversation_cache: Option<&Arc<Mutex<PromptCacheConversationsCacheState>>>,
) -> Result<RetentionRunSummary> {
    let dry_run = dry_run_override.unwrap_or(config.retention_dry_run);
    let mut summary = RetentionRunSummary {
        dry_run,
        arrival_rates: batch_plan::arrival_rates(pool).await,
        ..RetentionRunSummary::default()
    };
    let archive_cutoff = shanghai_local_cutoff_string(config.invocation_max_days);
    let source_max_query =
        sqlx::query_scalar::<_, Option<i64>>("SELECT MAX(id) FROM codex_invocations")
            .fetch_one(pool);
    let source_max_result = if let Some(remaining) = retention_run_remaining_budget() {
        tokio::time::timeout(remaining, source_max_query)
            .await
            .ok()
            .and_then(Result::ok)
    } else {
        source_max_query.await.ok()
    };
    if let Some(source_max_invocation_id) = source_max_result {
        summary.source_max_invocation_id = source_max_invocation_id;
        if let Some(source_max_invocation_id) = source_max_invocation_id {
            let backlog_query = sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM codex_invocations WHERE occurred_at < ?1 AND id <= ?2",
            )
            .bind(&archive_cutoff)
            .bind(source_max_invocation_id)
            .fetch_one(pool);
            summary.backlog_total = if let Some(remaining) = retention_run_remaining_budget() {
                tokio::time::timeout(remaining, backlog_query)
                    .await
                    .ok()
                    .and_then(Result::ok)
            } else {
                backlog_query.await.ok()
            };
            if summary.backlog_total.is_some() {
                summary.backlog_observed_at = Some(format_utc_iso_millis(Utc::now()));
            }
        } else {
            summary.backlog_total = Some(0);
            summary.backlog_observed_at = Some(format_utc_iso_millis(Utc::now()));
        }
    }
    workload::record_pending_population(config, &summary, source_max_result.is_some());
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
        if let Err(error) = reconcile_staged_archive_replacements(pool, Some(config)).await {
            if mark_archive_file_operation_budget_exhausted(&mut summary, &error) {
                debug!(error = %error, "staged archive replacement reconciliation hit the archive file operation budget");
            } else if is_retention_write_deferred(&error) {
                retention_recovery_record_deferred("legacy_reconcile");
            } else {
                retention_recovery_persist_latest_failure_best_effort(
                    pool,
                    "legacy_reconcile",
                    &error,
                )
                .await;
            }
            retention_recovery_log_event(
                tracing::Level::WARN,
                "staged_replacement_reconcile",
                "staged archive replacement reconciliation deferred",
            );
        }
        if let Err(error) = reconcile_retention_prepared_archives(pool, config).await {
            if mark_archive_file_operation_budget_exhausted(&mut summary, &error) {
                debug!(error = %error, "prepared archive reconciliation hit the archive file operation budget");
            } else if is_retention_write_deferred(&error) {
                retention_recovery_record_deferred("prepared_reconcile");
            } else {
                retention_recovery_persist_latest_failure_best_effort(
                    pool,
                    "prepared_reconcile",
                    &error,
                )
                .await;
            }
            retention_recovery_log_event(
                tracing::Level::WARN,
                "prepared_reconcile",
                "retention prepared archive reconciliation deferred",
            );
        }
        if let Err(error) = reconcile_legacy_retention_archive_segments(pool, config).await {
            if mark_archive_file_operation_budget_exhausted(&mut summary, &error) {
                debug!(error = %error, "legacy archive reconciliation hit the archive file operation budget");
            } else if is_retention_write_deferred(&error) {
                retention_recovery_record_deferred("legacy_reconcile");
            } else {
                retention_recovery_persist_latest_failure_best_effort(
                    pool,
                    "legacy_reconcile",
                    &error,
                )
                .await;
            }
            retention_recovery_log_event(
                tracing::Level::WARN,
                "legacy_reconcile",
                "legacy retention archive reconciliation deferred",
            );
        }
    }

    if should_stop_data_retention_maintenance(shutdown) {
        return Ok(summary);
    }

    if should_stop_data_retention_maintenance(shutdown) {
        return Ok(summary);
    }

    if dry_run {
        summary.model_route_rows_pruned =
            crate::upstream_accounts::count_expired_model_routes(pool).await? as usize;
    } else {
        let Some(admission) = acquire_retention_write_admission("model_route_purge").await else {
            return Ok(summary);
        };
        let execute_started = Instant::now();
        let candidate_limit = retention_candidate_limit(config, "model_route_purge");
        summary.model_route_rows_pruned =
            crate::upstream_accounts::purge_model_routes_bounded(pool, candidate_limit).await?
                as usize;
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
    }

    let payload_loss_days = config
        .invocation_success_full_days
        .min(config.invocation_max_days);
    let invocation_payload_retention_ready = dry_run
        || parallel_work_minute_coverage_ready_for_payload_retention(
            pool,
            shanghai_retention_cutoff(payload_loss_days).timestamp(),
        )
        .await
        .context("failed to verify parallel-work minute coverage before invocation retention")?;
    if !invocation_payload_retention_ready {
        summary.deferred = true;
        summary.wait_reason = Some("parallel_work_minute_coverage".to_string());
    }
    // The >90-day archive is the only stage that reduces the primary expired backlog. Run it
    // before the independent 30-90-day detail mirror pass so a large detail backlog cannot
    // starve the recovery-critical archive stage indefinitely.
    #[cfg(test)]
    retention_test_record_invocation_stage("invocation_archive");
    retention_recovery_clear_current_prepared_key();
    let invocation_archive = if invocation_payload_retention_ready {
        match archive_old_invocations_with_source_max(
            pool,
            config,
            raw_path_fallback_root,
            dry_run,
            &archive_cutoff,
            summary.source_max_invocation_id,
        )
        .await
        {
            Ok(archive) => archive,
            Err(error) => {
                if mark_archive_file_operation_budget_exhausted(&mut summary, &error) {
                    debug!(error = %error, "invocation archive hit the archive file operation budget");
                } else if retention_run_budget_expired() {
                    summary.deferred = true;
                    summary.wait_reason = Some("retention_work_budget".to_string());
                } else if is_retention_write_deferred(&error) {
                    retention_recovery_record_deferred("finalizing");
                    summary.deferred = true;
                    summary.wait_reason = Some("retention_write_admission".to_string());
                } else if crate::is_sqlite_lock_error(&error) {
                    retention_recovery_record_deferred("sqlite_busy");
                    summary.deferred = true;
                    summary.wait_reason =
                        Some(RETENTION_RECOVERY_DEFER_SQLITE_PRESSURE.to_string());
                } else if is_retention_recovery_failure_persisted(&error) {
                    retention_recovery_record_failure("finalizing", &error);
                    summary.deferred = true;
                    summary.recoverable_failure = true;
                    summary.wait_reason = Some("retention_recovery_failure".to_string());
                } else {
                    retention_recovery_persist_latest_failure_best_effort(
                        pool,
                        "finalizing",
                        &error,
                    )
                    .await;
                    summary.fatal_error = Some(format!(
                        "invocation archive failed; failure_fingerprint:{}",
                        retention_error_fingerprint(&error)
                    ));
                }
                retention_recovery_log_event(
                    tracing::Level::WARN,
                    "invocation_archive",
                    "invocation archive stage failed; continuing independent retention stages",
                );
                (0, 0, 0, std::collections::HashSet::new())
            }
        }
    } else {
        info!(
            payload_loss_days,
            "invocation archival deferred until parallel-work minute coverage catches up"
        );
        (0, 0, 0, std::collections::HashSet::new())
    };
    retention_recovery_clear_current_prepared_key();
    summary.invocation_rows_archived += invocation_archive.0;
    summary.archive_batches_touched += invocation_archive.1;
    summary.raw_files_removed += invocation_archive.2;

    let pool_attempt_archive = archive_timestamped_dataset(
        pool,
        config,
        archive_table_spec("pool_upstream_request_attempts"),
        "SELECT id, occurred_at AS timestamp_value FROM pool_upstream_request_attempts WHERE occurred_at < ?1 ORDER BY occurred_at ASC, id ASC LIMIT ?2",
        shanghai_local_cutoff_string(config.pool_upstream_request_attempts_retention_days),
        dry_run,
        config.database_path.parent(),
    )
    .await;
    let pool_attempt_archive = match pool_attempt_archive {
        Ok(archive) => archive,
        Err(error) => {
            if mark_archive_file_operation_budget_exhausted(&mut summary, &error) {
                debug!(error = %error, "attempt archive hit the archive file operation budget");
            } else if retention_run_budget_expired() {
                summary.deferred = true;
                summary.wait_reason = Some("retention_work_budget".to_string());
                debug!(error = %error, "attempt archive task timeout fallback");
            } else if is_retention_write_deferred(&error) || crate::is_sqlite_lock_error(&error) {
                summary.deferred = true;
                summary.wait_reason = Some(if crate::is_sqlite_lock_error(&error) {
                    "sqlite_pressure".to_string()
                } else {
                    "retention_write_admission".to_string()
                });
            } else {
                summary.fatal_error = Some(format!(
                    "pool upstream attempt archive failed; failure_fingerprint:{}",
                    retention_error_fingerprint(&error)
                ));
            }
            // Preserve already committed invocation batches in the immutable run result.
            return Ok(summary);
        }
    };
    summary.pool_upstream_request_attempt_rows_archived += pool_attempt_archive.0;
    summary.archive_batches_touched += pool_attempt_archive.1;
    summary.raw_files_removed += pool_attempt_archive.2;

    if should_stop_data_retention_maintenance(shutdown) {
        return Ok(summary);
    }

    let raw_compression = compress_cold_proxy_raw_payloads_with_budget(
        pool,
        config,
        raw_path_fallback_root,
        dry_run,
        retention_run_remaining_budget().or(Some(config.retention_catchup_budget)),
    )
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

    if should_stop_data_retention_maintenance(shutdown) {
        return Ok(summary);
    }

    if should_stop_data_retention_maintenance(shutdown) {
        return Ok(summary);
    }

    #[cfg(test)]
    retention_test_record_invocation_stage("invocation_detail_prune");
    let pruned = if invocation_payload_retention_ready {
        match prune_old_invocation_details(pool, config, raw_path_fallback_root, dry_run).await {
            Ok(pruned) => pruned,
            Err(error) => {
                if mark_archive_file_operation_budget_exhausted(&mut summary, &error) {
                    debug!(error = %error, "invocation detail pruning hit the archive file operation budget");
                } else if retention_run_budget_expired() {
                    summary.deferred = true;
                    summary.wait_reason = Some("retention_work_budget".to_string());
                } else if dry_run {
                    retention_recovery_record_failure("detail_prune", &error);
                    summary.fatal_error = Some(format!(
                        "invocation detail pruning failed; failure_fingerprint:{}",
                        retention_error_fingerprint(&error)
                    ));
                } else if is_retention_write_deferred(&error) {
                    retention_recovery_record_deferred("detail_prune");
                    summary.deferred = true;
                    summary.wait_reason = Some("retention_write_admission".to_string());
                } else if crate::is_sqlite_lock_error(&error) {
                    retention_recovery_record_deferred("sqlite_busy");
                    summary.deferred = true;
                    summary.wait_reason =
                        Some(RETENTION_RECOVERY_DEFER_SQLITE_PRESSURE.to_string());
                } else if is_retention_recovery_failure_persisted(&error) {
                    retention_recovery_record_failure("detail_prune", &error);
                    summary.deferred = true;
                    summary.recoverable_failure = true;
                    summary.wait_reason = Some("retention_recovery_failure".to_string());
                } else {
                    retention_recovery_persist_latest_failure_best_effort(
                        pool,
                        "detail_prune",
                        &error,
                    )
                    .await;
                    summary.fatal_error = Some(format!(
                        "invocation detail pruning failed; failure_fingerprint:{}",
                        retention_error_fingerprint(&error)
                    ));
                }
                retention_recovery_log_event(
                    tracing::Level::WARN,
                    "detail_prune",
                    "invocation detail pruning failed; continuing independent retention stages",
                );
                (0, 0, 0)
            }
        }
    } else {
        info!(
            payload_loss_days,
            "invocation detail pruning deferred until parallel-work minute coverage catches up"
        );
        (0, 0, 0)
    };
    summary.invocation_details_pruned += pruned.0;
    summary.archive_batches_touched += pruned.1;
    summary.raw_files_removed += pruned.2;
    retention_recovery_clear_current_prepared_key();
    if !dry_run {
        let preserve_recovery_stage = matches!(
            retention_recovery_health_snapshot().stage.as_deref(),
            Some("prepared_reconcile")
        );
        if !preserve_recovery_stage {
            retention_recovery_set_stage("status_refresh");
        }
        if let Err(error) = retention_recovery_refresh_counts(pool, config).await {
            retention_recovery_record_failure("status_refresh", &error);
            summary.deferred = true;
            summary.recoverable_failure = true;
            summary.wait_reason = Some("status_refresh".to_string());
            retention_recovery_log_event(
                tracing::Level::WARN,
                "status_refresh",
                "retention recovery status refresh failed; continuing independent retention stages",
            );
        } else {
            retention_recovery_log_event(
                tracing::Level::INFO,
                "status_refresh",
                "retention recovery status refreshed",
            );
        }
    }
    if !dry_run && (pruned.1 > 0 || invocation_archive.1 > 0) {
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
                StartupBackfillTask::LegacyDetailMirrors,
                StartupBackfillTask::HistoricalRollups,
            ],
            if pruned.1 > 0 {
                "invocation_detail_prune_archive_materialized"
            } else {
                "invocation_archive_materialized"
            },
        )
        .await?;
    }

    if should_stop_data_retention_maintenance(shutdown) {
        return Ok(summary);
    }

    let proxy_archive = archive_timestamped_dataset(
        pool,
        config,
        archive_table_spec("forward_proxy_attempts"),
        "SELECT id, occurred_at AS timestamp_value FROM forward_proxy_attempts WHERE occurred_at < ?1 ORDER BY occurred_at ASC, id ASC LIMIT ?2",
        shanghai_utc_cutoff_string(config.forward_proxy_attempts_retention_days),
        dry_run,
        config.database_path.parent(),
    )
    .await
    .context("failed to archive forward proxy attempts during retention");
    let proxy_archive = match proxy_archive {
        Ok(archive) => archive,
        Err(error) => {
            if mark_archive_file_operation_budget_exhausted(&mut summary, &error) {
                debug!(error = %error, "forward proxy archive hit the archive file operation budget");
            } else if retention_run_budget_expired() {
                summary.deferred = true;
                summary.wait_reason = Some("retention_work_budget".to_string());
            } else if is_retention_write_deferred(&error) || crate::is_sqlite_lock_error(&error) {
                summary.deferred = true;
                summary.wait_reason = Some(if crate::is_sqlite_lock_error(&error) {
                    RETENTION_RECOVERY_DEFER_SQLITE_PRESSURE.to_string()
                } else {
                    "retention_write_admission".to_string()
                });
            } else {
                summary.fatal_error = Some(format!(
                    "forward proxy archive failed; failure_fingerprint:{}",
                    retention_error_fingerprint(&error)
                ));
            }
            return Ok(summary);
        }
    };
    summary.forward_proxy_attempt_rows_archived += proxy_archive.0;
    summary.archive_batches_touched += proxy_archive.1;
    summary.raw_files_removed += proxy_archive.2;

    if should_stop_data_retention_maintenance(shutdown) {
        return Ok(summary);
    }

    let quota_archive = compact_old_quota_snapshots(pool, config, dry_run)
        .await
        .context("failed to compact old quota snapshots during retention");
    let quota_archive = match quota_archive {
        Ok(archive) => archive,
        Err(error) => {
            if mark_archive_file_operation_budget_exhausted(&mut summary, &error) {
                debug!(error = %error, "quota compaction hit the archive file operation budget");
            } else if retention_run_budget_expired() {
                summary.deferred = true;
                summary.wait_reason = Some("retention_work_budget".to_string());
            } else if is_retention_write_deferred(&error) || crate::is_sqlite_lock_error(&error) {
                summary.deferred = true;
                summary.wait_reason = Some(if crate::is_sqlite_lock_error(&error) {
                    RETENTION_RECOVERY_DEFER_SQLITE_PRESSURE.to_string()
                } else {
                    "retention_write_admission".to_string()
                });
            } else {
                summary.fatal_error = Some(format!(
                    "quota compaction failed; failure_fingerprint:{}",
                    retention_error_fingerprint(&error)
                ));
            }
            return Ok(summary);
        }
    };
    summary.quota_snapshot_rows_archived += quota_archive.0;
    summary.archive_batches_touched += quota_archive.1;

    if should_stop_data_retention_maintenance(shutdown) {
        return Ok(summary);
    }

    let archive_ttl_cleanup = cleanup_expired_archive_batches(pool, config, dry_run)
        .await
        .context("failed to clean up expired archive batches during retention")?;
    summary.archive_batches_deleted += archive_ttl_cleanup;

    if should_stop_data_retention_maintenance(shutdown) {
        return Ok(summary);
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

    info!(
        dry_run = summary.dry_run,
        ?summary,
        "data retention maintenance finished"
    );
    Ok(summary)
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
    let query_budget = retention_run_remaining_budget()
        .unwrap_or(RETENTION_SQLITE_MAINTENANCE_QUERY_BUDGET)
        .min(RETENTION_SQLITE_MAINTENANCE_QUERY_BUDGET);
    let deadline = Instant::now() + query_budget;
    let interrupted = Arc::new(AtomicBool::new(false));
    let shutdown = RETENTION_SHUTDOWN.try_with(Clone::clone).ok();
    let mut cleanup_failed = false;
    let result =
        async {
            // PRAGMA optimize can run ANALYZE even with SQLite's default sampling limit.
            // Use a dedicated connection so cancellation never returns live SQLite work or
            // a progress handler to the pool. Await execution and close before releasing P2.
            let options = pool
                .connect_options()
                .as_ref()
                .clone()
                .busy_timeout(query_budget);
            let mut connection =
                match tokio::time::timeout(query_budget, SqliteConnection::connect_with(&options))
                    .await
                {
                    Ok(connection) => connection?,
                    Err(_) => {
                        interrupted.store(true, Ordering::Release);
                        return Err(anyhow!(
                            "retention SQLite maintenance connection budget exceeded"
                        ));
                    }
                };
            {
                let interrupted = interrupted.clone();
                let query_shutdown = shutdown.clone();
                #[cfg(test)]
                let test_probe = RETENTION_TEST_SQLITE_MAINTENANCE_PROBE
                    .try_with(Arc::clone)
                    .ok();
                let mut handle = connection.lock_handle().await?;
                handle.set_progress_handler(1_000, move || {
                    #[cfg(test)]
                    let test_cancelled = test_probe.as_ref().is_some_and(|probe| {
                        probe.progress_callbacks.fetch_add(1, Ordering::AcqRel);
                        true
                    });
                    #[cfg(not(test))]
                    let test_cancelled = false;
                    let keep_running = Instant::now() < deadline
                        && !test_cancelled
                        && query_shutdown
                            .as_ref()
                            .is_none_or(|token| !token.is_cancelled());
                    if !keep_running {
                        interrupted.store(true, Ordering::Release);
                    }
                    keep_running
                });
            }
            let result = sqlx::query(sql).execute(&mut connection).await;
            let cleanup = connection.lock_handle().await.map(|mut handle| {
                handle.remove_progress_handler();
            });
            // Closing also rolls back any interrupted implicit ANALYZE transaction.
            let closed = connection.close().await;
            #[cfg(test)]
            let _ = RETENTION_TEST_SQLITE_MAINTENANCE_PROBE.try_with(|probe| {
                probe
                    .connection_closed
                    .store(closed.is_ok(), Ordering::Release);
            });
            cleanup_failed = cleanup.is_err() || closed.is_err();
            cleanup?;
            closed?;
            result.map_err(anyhow::Error::from)
        }
        .await
        .with_context(|| format!("failed to run {description}"));
    match result {
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
        Err(_) if interrupted.load(Ordering::Acquire) && !cleanup_failed => {
            let reason = if shutdown.as_ref().is_some_and(|token| token.is_cancelled()) {
                "shutdown"
            } else {
                "sqlite_maintenance_query_budget"
            };
            debug!(
                operation = "retention_pragma",
                reason,
                elapsed_ms = execute_started.elapsed().as_millis() as u64,
                description,
                "optional SQLite maintenance cancelled; archive commits remain complete"
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

#[cfg(test)]
mod archive_file_budget_classification_tests {
    use super::*;

    #[test]
    fn archive_file_budget_is_deferred_without_becoming_fatal() {
        let error = retention_archive_file_operation_budget_exhausted().context(
            "failed to decompress archive batch /tmp/archive.sqlite.gz into /tmp/archive.sqlite",
        );
        let mut summary = RetentionRunSummary::default();

        assert!(mark_archive_file_operation_budget_exhausted(
            &mut summary,
            &error
        ));
        assert!(summary.deferred);
        assert!(summary.budget_exhausted);
        assert_eq!(
            summary.wait_reason.as_deref(),
            Some(RETENTION_ARCHIVE_FILE_OPERATION_BUDGET_REASON)
        );
        assert!(summary.fatal_error.is_none());
    }
}

#[cfg(test)]
mod owner_fact_tests {
    use super::*;

    #[test]
    fn post_commit_owner_error_preserves_committed_facts() {
        let mut summary = RetentionOwnerRunSummary::new(
            "model_route_expiry",
            "model_routes:expiry",
            "model route rows",
        );
        let error = anyhow::anyhow!("follow-up inspection failed");

        apply_retention_owner_post_commit_error(&mut summary, 12, 5, &error);

        assert_eq!(summary.discovered, 12);
        assert_eq!(summary.selected, 5);
        assert_eq!(summary.committed, 5);
        assert_eq!(summary.has_more, Some(true));
        assert_eq!(summary.completion, "failed");
        assert!(summary.fatal_error.is_some());
    }

    #[test]
    fn dry_run_raw_file_facts_are_planned_not_removed() {
        let mut summary = RetentionOwnerRunSummary::new(
            "invocation_detail_prune",
            "codex_invocation_details",
            "rows",
        );

        set_retention_owner_raw_file_facts(&mut summary, 3, true);

        assert_eq!(summary.raw_files_removed, 0);
        assert_eq!(summary.raw_files_planned, 3);
        assert_eq!(summary.has_more, None);
    }

    #[test]
    fn no_progress_error_is_detected_through_context() {
        let error = retention_archive_file_operation_no_progress(128, Some(256))
            .context("archive identity preparation failed");

        assert!(is_retention_archive_file_operation_no_progress(&error));
    }
}
