use super::*;

#[derive(Debug, Clone, Default)]
pub(crate) struct StartupBackfillRunState {
    pub(crate) next_cursor_id: i64,
    pub(crate) scanned: u64,
    pub(crate) updated: u64,
    pub(crate) hit_scan_limit: bool,
    pub(crate) retry_soon: bool,
    pub(crate) force_idle: bool,
    pub(crate) source_unavailable: bool,
    pub(crate) deferred: bool,
    pub(crate) defer_reason: Option<&'static str>,
    pub(crate) samples: Vec<String>,
}

pub(crate) fn startup_backfill_next_delay(
    run: &StartupBackfillRunState,
    zero_update_streak: u32,
) -> Duration {
    if run.source_unavailable {
        Duration::from_secs(24 * 60 * 60)
    } else if run.force_idle {
        Duration::from_secs(STARTUP_BACKFILL_IDLE_INTERVAL_SECS)
    } else if run.retry_soon {
        Duration::from_secs(15)
    } else if run.updated > 0 {
        Duration::from_secs(STARTUP_BACKFILL_ACTIVE_INTERVAL_SECS)
    } else if run.hit_scan_limit && run.scanned > 0 {
        Duration::from_secs(match zero_update_streak {
            0 | 1 => 15,
            2 => 60,
            3 => 5 * 60,
            _ => 15 * 60,
        })
    } else if run.hit_scan_limit {
        Duration::from_secs(STARTUP_BACKFILL_ACTIVE_INTERVAL_SECS)
    } else if run.scanned == 0 || zero_update_streak > 0 {
        Duration::from_secs(STARTUP_BACKFILL_IDLE_INTERVAL_SECS)
    } else {
        Duration::from_secs(STARTUP_BACKFILL_ACTIVE_INTERVAL_SECS)
    }
}

pub(crate) fn historical_rollup_should_retry_soon(
    hit_budget: bool,
    candidate_count: usize,
) -> bool {
    hit_budget && candidate_count > 0
}

pub(crate) fn startup_backfill_next_run_after(
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

pub(crate) fn historical_rollup_startup_backfill_run_state(
    cursor_id: i64,
    zero_update_streak: u32,
    before: &HistoricalRollupBackfillSnapshot,
    after: &HistoricalRollupBackfillSnapshot,
    summary: &HistoricalRollupMaterializationSummary,
    pending_before: u64,
    pending_after: u64,
) -> StartupBackfillRunState {
    let archive_progress = before
        .legacy_archive_pending
        .saturating_sub(after.legacy_archive_pending);
    let usage_breakdown_progress = before
        .pending_usage_breakdown_batches
        .saturating_sub(after.pending_usage_breakdown_batches);
    let bucket_progress = before.pending_buckets.saturating_sub(after.pending_buckets);
    let selected_backlog_progress = pending_before.saturating_sub(pending_after);
    let attempted_archive_batches = summary
        .scanned_archive_batches
        .saturating_sub(summary.skipped_archive_batches);
    let scanned_all_pending_archives = attempted_archive_batches as u64 >= pending_before;
    let exhausted_blocked_cycle = summary.blocked_archive_batches == attempted_archive_batches
        && selected_backlog_progress == 0
        && bucket_progress == 0
        && zero_update_streak.saturating_add(attempted_archive_batches as u32) as u64
            >= pending_before;
    let permanently_blocked = summary.blocked_archive_batches > 0
        && selected_backlog_progress == 0
        && bucket_progress == 0
        && (scanned_all_pending_archives || exhausted_blocked_cycle);

    StartupBackfillRunState {
        next_cursor_id: cursor_id.saturating_add(attempted_archive_batches as i64),
        scanned: summary.scanned_archive_batches as u64,
        updated: archive_progress
            .max(usage_breakdown_progress)
            .max(selected_backlog_progress)
            .max(bucket_progress),
        hit_scan_limit: pending_after > 0 && !permanently_blocked,
        retry_soon: false,
        force_idle: pending_after == 0 || permanently_blocked,
        source_unavailable: permanently_blocked,
        deferred: false,
        defer_reason: None,
        samples: Vec::new(),
    }
}

#[derive(Debug, Clone)]
pub(crate) struct BackfillBatchOutcome<T> {
    pub(crate) summary: T,
    pub(crate) next_cursor_id: i64,
    pub(crate) hit_budget: bool,
    pub(crate) samples: Vec<String>,
}

#[derive(Debug)]
pub(crate) struct BackfillPartialFailure {
    pub(crate) source: anyhow::Error,
    pub(crate) next_cursor_id: i64,
    pub(crate) scanned: u64,
    pub(crate) updated: u64,
}

impl std::fmt::Display for BackfillPartialFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.source.fmt(formatter)
    }
}

impl std::error::Error for BackfillPartialFailure {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(self.source.root_cause())
    }
}

pub(crate) fn startup_backfill_query_limit(scanned: u64, scan_limit: Option<u64>) -> i64 {
    let remaining = scan_limit
        .map(|limit| limit.saturating_sub(scanned))
        .unwrap_or(BACKFILL_BATCH_SIZE as u64);
    remaining.min(BACKFILL_BATCH_SIZE as u64).max(1) as i64
}

pub(crate) fn startup_backfill_budget_reached(
    started_at: Instant,
    scanned: u64,
    scan_limit: Option<u64>,
    max_elapsed: Option<Duration>,
) -> bool {
    let hit_scan_limit = scan_limit.is_some_and(|limit| scanned >= limit);
    let hit_elapsed_limit = max_elapsed.is_some_and(|limit| started_at.elapsed() >= limit);
    crate::maintenance::migration_hold_signal_reached() || hit_scan_limit || hit_elapsed_limit
}

pub(crate) fn startup_backfill_samples_text(samples: &[String]) -> String {
    if samples.is_empty() {
        "-".to_string()
    } else {
        samples.join(" | ")
    }
}

pub(crate) fn startup_backfill_scan_limit(source_unavailable_probe: bool) -> u64 {
    if source_unavailable_probe {
        100
    } else {
        STARTUP_BACKFILL_SCAN_LIMIT
    }
}

pub(crate) fn startup_backfill_run_budget(source_unavailable_probe: bool) -> Duration {
    if source_unavailable_probe {
        Duration::from_secs(2)
    } else {
        Duration::from_secs(STARTUP_BACKFILL_RUN_BUDGET_SECS)
    }
}
