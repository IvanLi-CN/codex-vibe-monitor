//! Fixed, cached observations shared by retention, backfill and statistics.
//!
//! Callers supply events at existing boundaries. This module never reads a
//! database, changes scheduling, or registers labels supplied by a request.
use super::*;
use std::sync::OnceLock;

tokio::task_local! {
    static PERFORMANCE_RUNTIME: Arc<ObservabilityRuntime>;
}

/// Establish an explicit instance scope at an HTTP, worker or managed-task entry.
/// Spawned tasks must establish their own scope; no global recorder is shared.
pub(crate) async fn with_performance<F: Future>(
    metrics: Arc<ObservabilityRuntime>,
    future: F,
) -> F::Output {
    PERFORMANCE_RUNTIME.scope(metrics, future).await
}

pub(crate) fn current_performance() -> Option<Arc<ObservabilityRuntime>> {
    PERFORMANCE_RUNTIME.try_with(Arc::clone).ok()
}

pub(crate) fn spawn_performance<F>(
    metrics: Arc<ObservabilityRuntime>,
    future: F,
) -> JoinHandle<F::Output>
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    tokio::spawn(with_performance(metrics, future))
}

/// Observe one fixed SQL execution, including cancellation, without its text or parameters.
pub(crate) async fn observe_query<T, E>(
    query: StatsQuery,
    future: impl Future<Output = std::result::Result<T, E>>,
) -> std::result::Result<T, E> {
    let Some(metrics) = current_performance().filter(|metrics| metrics.enabled) else {
        return future.await;
    };
    let mut observation = QueryObservation {
        metrics,
        query,
        started: Instant::now(),
        outcome: QueryOutcome::Cancelled,
    };
    let result = future.await;
    observation.outcome = if result.is_ok() {
        QueryOutcome::Success
    } else {
        QueryOutcome::Error
    };
    result
}

struct QueryObservation {
    metrics: Arc<ObservabilityRuntime>,
    query: StatsQuery,
    started: Instant,
    outcome: QueryOutcome,
}
impl Drop for QueryObservation {
    fn drop(&mut self) {
        self.metrics
            .count_performance(PerformanceCounter::Query(self.query, self.outcome), 1);
        self.metrics.observe_performance(
            PerformanceHistogram::Query(self.query, self.outcome),
            self.started.elapsed(),
        );
    }
}

macro_rules! labels {
    ($name:ident { $($variant:ident => $label:literal),+ $(,)? }) => {
        #[derive(Clone, Copy, Debug)]
        pub(crate) enum $name { $($variant),+ }
        impl $name {
            pub(crate) const ALL: &'static [Self] = &[$(Self::$variant),+];
            pub(crate) fn label(self) -> &'static str {
                match self { $(Self::$variant => $label),+ }
            }
        }
    };
}

labels!(RetentionOperation {
    InvocationArchive => "invocation_archive", TimestampedArchive => "timestamped_archive",
    DetailPrune => "detail_prune"
});
labels!(RetentionPhase {
    Prepare => "prepare", FileVerify => "file_verify", AdmissionWait => "admission_wait",
    WriteHold => "write_hold", SourceVerify => "source_verify", Delete => "delete",
    Commit => "commit", Cleanup => "cleanup", Transaction => "transaction"
});
labels!(Outcome { Success => "success", Deferred => "deferred", Error => "error", Cancelled => "cancelled" });
labels!(WakeDecision { New => "new", Merged => "merged", Disabled => "disabled", Stale => "stale" });
labels!(WakeOutcome {
    Committed => "committed", Noop => "noop", Busy => "busy", Locked => "locked",
    Error => "error", Cancelled => "cancelled"
});
labels!(StoreOperation {
    Wake => "wake", Progress => "progress", Control => "control", Run => "run", Cleanup => "cleanup"
});
labels!(SummaryResult { Success => "success", Degraded => "degraded", Unavailable => "unavailable" });
labels!(SummaryReason {
    None => "none", Hydration => "hydration", SourceGap => "source_gap",
    CurrentRankUnproven => "current_rank_unproven", HistoricalCoverage => "historical_coverage",
    Other => "other"
});
labels!(StatsQuery {
    Summary => "summary",
    PromptCacheStatistics => "prompt_cache_statistics",
    LongTermOverlap => "long_term_overlap",
    InvocationTimeline => "invocation_timeline",
    SummarySnapshotCandidates => "summary_snapshot_candidates"
});
labels!(QueryOutcome { Success => "success", Error => "error", Cancelled => "cancelled" });
labels!(StatsProjection {
    SummaryRecent => "summary_recent", SummaryHistorical => "summary_historical",
    PromptCache => "prompt_cache", LongTerm => "long_term"
});
labels!(PendingKind { DirtyBuckets => "dirty_buckets", SourceGaps => "source_gaps", Rows => "rows" });
labels!(BacklogKind { Records => "records", OldestTimestamp => "oldest_timestamp" });
labels!(WakeState { Pending => "pending", OldestTimestamp => "oldest_timestamp", NextDueTimestamp => "next_due_timestamp" });

// A diagnostic is one observed boundary, not a replacement for committed-work
// counters. Keeping valid combinations in one enum avoids dynamic label pairs.
labels!(RecoveryDiagnostic {
    RangeFirstInitialization => "first_initialization", RangeCacheAdmission => "cache_admission",
    RangeRefillWait => "refill_wait", RangeSequenceExhausted => "sequence_exhausted",
    RangeReady => "ready", RangeUnavailable => "unavailable",
    RangePrewarmReady => "prewarm_ready", RangePrewarmDeferred => "prewarm_deferred",
    RetentionNoWork => "no_work", RetentionBudgetExhausted => "budget_exhausted",
    RetentionPressureDeferred => "pressure_deferred", RetentionProofUnavailable => "proof_unavailable",
    RetentionCancelled => "cancelled", RetentionFailed => "failed", RetentionCommitted => "committed",
    IndexPresent => "index_present", IndexMissing => "index_missing",
    IndexDefinitionConflict => "index_definition_conflict", IndexUnavailable => "index_unavailable",
    StatisticsLiveTail => "live_tail", StatisticsHistoricalCoverage => "historical_coverage",
    StatisticsPublicationConflict => "publication_conflict",
    SummaryRecoveryScannedKeys => "summary_recovery_scanned_keys",
    SummaryRecoveryCommittedTerminalRows => "summary_recovery_committed_terminal_rows",
    SummaryRecoveryCursorAdvanced => "summary_recovery_cursor_advanced",
    SummaryRecoveryBudgetExhausted => "summary_recovery_budget_exhausted",
    SummaryRecoveryLiveBoundaryDeferred => "summary_recovery_live_boundary_deferred",
    SummaryRecoveryPressureDeferred => "summary_recovery_pressure_deferred",
    SummaryRecoveryContentionDeferred => "summary_recovery_contention_deferred",
    SummaryRecoveryRetry => "summary_recovery_retry",
    SummaryRecoveryPublication => "summary_recovery_publication",
    QueryPlanIndexed => "query_plan_indexed", QueryPlanFallback => "query_plan_fallback",
    QueryPlanUnavailable => "query_plan_unavailable",
    SummaryReorderBuffered => "summary_reorder_buffered",
    SummaryReorderAdvanced => "summary_reorder_advanced"
});
impl RecoveryDiagnostic {
    fn operation(self) -> &'static str {
        match self {
            Self::RangeFirstInitialization
            | Self::RangeCacheAdmission
            | Self::RangeRefillWait
            | Self::RangeSequenceExhausted
            | Self::RangeReady
            | Self::RangeUnavailable
            | Self::RangePrewarmReady
            | Self::RangePrewarmDeferred => "invocation_range",
            Self::SummaryReorderBuffered | Self::SummaryReorderAdvanced => "summary",
            Self::RetentionNoWork
            | Self::RetentionBudgetExhausted
            | Self::RetentionPressureDeferred
            | Self::RetentionProofUnavailable
            | Self::RetentionCancelled
            | Self::RetentionFailed
            | Self::RetentionCommitted => "retention",
            _ => "statistics",
        }
    }
}
labels!(InvocationRangePhase {
    CacheAdmission => "cache_admission", WriteAdmission => "write_admission",
    Connection => "connection", Transaction => "transaction"
});

pub(crate) fn record_recovery(diagnostic: RecoveryDiagnostic) {
    if let Some(metrics) = current_performance() {
        metrics.count_performance(PerformanceCounter::Recovery(diagnostic), 1);
    }
}

const TASK_COUNT: usize = 16;
const RETENTION_COUNT: usize = RetentionOperation::ALL.len();
const OUTCOMES: usize = Outcome::ALL.len();
const WAKE_DECISIONS: usize = WakeDecision::ALL.len();
const WAKE_OUTCOMES: usize = WakeOutcome::ALL.len();
const STORE_COUNT: usize = StoreOperation::ALL.len();
const RESULTS: usize = SummaryResult::ALL.len();
const REASONS: usize = SummaryReason::ALL.len();
const QUERIES: usize = StatsQuery::ALL.len();
const QUERY_OUTCOMES: usize = QueryOutcome::ALL.len();
const COUNTERS: usize = RETENTION_COUNT * (OUTCOMES + 2)
    + TASK_COUNT * (WAKE_DECISIONS + WAKE_OUTCOMES)
    + STORE_COUNT * WAKE_OUTCOMES
    + RESULTS * REASONS
    + QUERIES * QUERY_OUTCOMES
    + RecoveryDiagnostic::ALL.len();
const HISTOGRAMS: usize = RETENTION_COUNT * RetentionPhase::ALL.len()
    + STORE_COUNT
    + RESULTS
    + QUERIES * QUERY_OUTCOMES
    + InvocationRangePhase::ALL.len();
const GAUGES: usize = RETENTION_COUNT * BacklogKind::ALL.len()
    + TASK_COUNT * WakeState::ALL.len()
    + StatsProjection::ALL.len() * (PendingKind::ALL.len() + 1)
    + 1;

#[derive(Clone, Copy)]
pub(crate) enum PerformanceCounter {
    RetentionBatch(RetentionOperation, Outcome),
    ProcessedRows(RetentionOperation),
    ArchivedRows(RetentionOperation),
    WakeNotification(StartupBackfillTask, WakeDecision),
    WakeAttempt(StartupBackfillTask, WakeOutcome),
    StoreWrite(StoreOperation, WakeOutcome),
    SummaryResponse(SummaryResult, SummaryReason),
    Query(StatsQuery, QueryOutcome),
    Recovery(RecoveryDiagnostic),
}
impl PerformanceCounter {
    fn index(self) -> usize {
        let rows = RETENTION_COUNT * OUTCOMES;
        let wake = rows + RETENTION_COUNT * 2;
        let attempts = wake + TASK_COUNT * WAKE_DECISIONS;
        let store = attempts + TASK_COUNT * WAKE_OUTCOMES;
        let summary = store + STORE_COUNT * WAKE_OUTCOMES;
        let query = summary + RESULTS * REASONS;
        match self {
            Self::RetentionBatch(op, result) => op as usize * OUTCOMES + result as usize,
            Self::ProcessedRows(op) => rows + op as usize,
            Self::ArchivedRows(op) => rows + RETENTION_COUNT + op as usize,
            Self::WakeNotification(task, decision) => {
                wake + task as usize * WAKE_DECISIONS + decision as usize
            }
            Self::WakeAttempt(task, outcome) => {
                attempts + task as usize * WAKE_OUTCOMES + outcome as usize
            }
            Self::StoreWrite(op, outcome) => store + op as usize * WAKE_OUTCOMES + outcome as usize,
            Self::SummaryResponse(result, reason) => {
                summary + result as usize * REASONS + reason as usize
            }
            Self::Query(id, outcome) => query + id as usize * QUERY_OUTCOMES + outcome as usize,
            Self::Recovery(diagnostic) => query + QUERIES * QUERY_OUTCOMES + diagnostic as usize,
        }
    }
    fn key(self) -> Key {
        let (name, labels) = match self {
            Self::RetentionBatch(op, outcome) => (
                "cvm_retention_batches_total",
                vec![("operation", op.label()), ("outcome", outcome.label())],
            ),
            Self::ProcessedRows(op) => (
                "cvm_maintenance_processed_rows_total",
                vec![("operation", op.label())],
            ),
            Self::ArchivedRows(op) => (
                "cvm_maintenance_archived_rows_total",
                vec![("operation", op.label())],
            ),
            Self::WakeNotification(task, decision) => (
                "cvm_backfill_wake_notifications_total",
                vec![("task", task.name()), ("reason", decision.label())],
            ),
            Self::WakeAttempt(task, outcome) => (
                "cvm_backfill_wake_attempts_total",
                vec![("task", task.name()), ("outcome", outcome.label())],
            ),
            Self::StoreWrite(op, outcome) => (
                "cvm_maintenance_store_writes_total",
                vec![("operation", op.label()), ("outcome", outcome.label())],
            ),
            Self::SummaryResponse(result, reason) => (
                "cvm_summary_responses_total",
                vec![("outcome", result.label()), ("reason", reason.label())],
            ),
            Self::Query(query, outcome) => (
                "cvm_stats_queries_total",
                vec![("query", query.label()), ("outcome", outcome.label())],
            ),
            Self::Recovery(diagnostic) => (
                "cvm_runtime_recovery_diagnostics_total",
                vec![
                    ("operation", diagnostic.operation()),
                    ("reason", diagnostic.label()),
                ],
            ),
        };
        ObservabilityRuntime::key(name, &labels)
    }
}

#[derive(Clone, Copy)]
pub(crate) enum PerformanceHistogram {
    RetentionPhase(RetentionOperation, RetentionPhase),
    StoreWrite(StoreOperation),
    SummaryResponse(SummaryResult),
    Query(StatsQuery, QueryOutcome),
    InvocationRange(InvocationRangePhase),
}
impl PerformanceHistogram {
    fn index(self) -> usize {
        let store = RETENTION_COUNT * RetentionPhase::ALL.len();
        match self {
            Self::RetentionPhase(op, phase) => {
                op as usize * RetentionPhase::ALL.len() + phase as usize
            }
            Self::StoreWrite(op) => store + op as usize,
            Self::SummaryResponse(result) => store + STORE_COUNT + result as usize,
            Self::Query(query, outcome) => {
                store + STORE_COUNT + RESULTS + query as usize * QUERY_OUTCOMES + outcome as usize
            }
            Self::InvocationRange(phase) => {
                store + STORE_COUNT + RESULTS + QUERIES * QUERY_OUTCOMES + phase as usize
            }
        }
    }
    fn key(self) -> Key {
        let (name, labels) = match self {
            Self::RetentionPhase(op, phase) => (
                "cvm_retention_phase_duration_seconds",
                vec![("operation", op.label()), ("phase", phase.label())],
            ),
            Self::StoreWrite(op) => (
                "cvm_maintenance_store_write_duration_seconds",
                vec![("operation", op.label())],
            ),
            Self::SummaryResponse(result) => (
                "cvm_summary_response_duration_seconds",
                vec![("outcome", result.label())],
            ),
            Self::Query(query, outcome) => (
                "cvm_stats_query_duration_seconds",
                vec![("query", query.label()), ("outcome", outcome.label())],
            ),
            Self::InvocationRange(phase) => (
                "cvm_invocation_range_phase_duration_seconds",
                vec![("phase", phase.label())],
            ),
        };
        ObservabilityRuntime::key(name, &labels)
    }
}

#[derive(Clone, Copy)]
pub(crate) enum PerformanceGauge {
    RetentionBacklog(RetentionOperation, BacklogKind),
    Wake(StartupBackfillTask, WakeState),
    ProjectionLastGood(StatsProjection),
    ProjectionPending(StatsProjection, PendingKind),
    SummaryRecoveryRetryDelay,
}
impl PerformanceGauge {
    fn index(self) -> usize {
        let wake = RETENTION_COUNT * BacklogKind::ALL.len();
        let projection = wake + TASK_COUNT * WakeState::ALL.len();
        match self {
            Self::RetentionBacklog(op, kind) => {
                op as usize * BacklogKind::ALL.len() + kind as usize
            }
            Self::Wake(task, kind) => wake + task as usize * WakeState::ALL.len() + kind as usize,
            Self::ProjectionLastGood(id) => projection + id as usize,
            Self::ProjectionPending(id, kind) => {
                projection
                    + StatsProjection::ALL.len()
                    + id as usize * PendingKind::ALL.len()
                    + kind as usize
            }
            Self::SummaryRecoveryRetryDelay => {
                projection + StatsProjection::ALL.len() * (PendingKind::ALL.len() + 1)
            }
        }
    }
    fn key(self) -> Key {
        let (name, labels) = match self {
            Self::RetentionBacklog(op, kind) => (
                match kind {
                    BacklogKind::Records => "cvm_retention_backlog_records",
                    BacklogKind::OldestTimestamp => {
                        "cvm_retention_backlog_oldest_timestamp_seconds"
                    }
                },
                vec![("operation", op.label())],
            ),
            Self::Wake(task, kind) => (
                match kind {
                    WakeState::Pending => "cvm_backfill_wake_pending_tasks",
                    WakeState::OldestTimestamp => "cvm_backfill_wake_oldest_timestamp_seconds",
                    WakeState::NextDueTimestamp => "cvm_backfill_wake_next_due_timestamp_seconds",
                },
                vec![("task", task.name())],
            ),
            Self::ProjectionLastGood(id) => (
                "cvm_stats_projection_last_good_timestamp_seconds",
                vec![("query", id.label())],
            ),
            Self::ProjectionPending(id, kind) => (
                "cvm_stats_projection_pending_items",
                vec![("query", id.label()), ("phase", kind.label())],
            ),
            Self::SummaryRecoveryRetryDelay => (
                "cvm_summary_recovery_retry_delay_seconds",
                vec![("scope", "dashboard")],
            ),
        };
        ObservabilityRuntime::key(name, &labels)
    }
}

pub(super) struct ContentionMetrics {
    counters: [OnceLock<metrics::Counter>; COUNTERS],
    histograms: [OnceLock<metrics::Histogram>; HISTOGRAMS],
    gauges: [OnceLock<metrics::Gauge>; GAUGES],
}
impl Default for ContentionMetrics {
    fn default() -> Self {
        Self {
            counters: std::array::from_fn(|_| OnceLock::new()),
            histograms: std::array::from_fn(|_| OnceLock::new()),
            gauges: std::array::from_fn(|_| OnceLock::new()),
        }
    }
}
impl ObservabilityRuntime {
    pub(crate) fn count_performance(&self, event: PerformanceCounter, count: u64) {
        if self.enabled {
            self.contention.counters[event.index()]
                .get_or_init(|| self.register_counter(event.key()))
                .increment(count);
        }
    }
    pub(crate) fn observe_performance(&self, event: PerformanceHistogram, duration: Duration) {
        if self.enabled {
            self.contention.histograms[event.index()]
                .get_or_init(|| self.register_histogram(event.key()))
                .record(duration.as_secs_f64());
        }
    }
    pub(crate) fn set_performance(&self, event: PerformanceGauge, value: f64) {
        if self.enabled && value.is_finite() && value >= 0.0 {
            self.contention.gauges[event.index()]
                .get_or_init(|| self.register_gauge(event.key()))
                .set(value);
        }
    }
}

#[cfg(test)]
#[path = "../tests/lightweight/performance_metrics_contract.rs"]
mod contract_tests;
