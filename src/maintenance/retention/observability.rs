use crate::ObservabilityRuntime;
use crate::observability::contention::{
    BacklogKind, Outcome, PerformanceCounter, PerformanceGauge, PerformanceHistogram,
    RetentionOperation, RetentionPhase, current_performance,
};
use chrono::{DateTime, Utc};
use std::sync::Arc;
use std::time::Instant;

pub(crate) struct RetentionBatchObservation {
    metrics: Option<Arc<ObservabilityRuntime>>,
    operation: RetentionOperation,
    outcome: Option<Outcome>,
}

impl RetentionBatchObservation {
    pub(crate) fn new(operation: RetentionOperation) -> Self {
        Self {
            metrics: current_performance().filter(|metrics| metrics.enabled),
            operation,
            outcome: None,
        }
    }

    pub(crate) fn phase(&self, phase: RetentionPhase) -> RetentionPhaseGuard {
        RetentionPhaseGuard {
            metrics: self.metrics.clone(),
            operation: self.operation,
            phase,
            started: Instant::now(),
        }
    }

    pub(crate) fn committed(&self, rows: usize) {
        let Some(metrics) = &self.metrics else {
            return;
        };
        let rows = u64::try_from(rows).unwrap_or(u64::MAX);
        if rows == 0 {
            return;
        }
        metrics.count_performance(PerformanceCounter::ProcessedRows(self.operation), rows);
        metrics.count_performance(PerformanceCounter::ArchivedRows(self.operation), rows);
    }

    pub(crate) fn finish(&mut self, outcome: Outcome) {
        self.outcome = Some(outcome);
    }

    pub(crate) fn has_outcome(&self) -> bool {
        self.outcome.is_some()
    }
}

impl Drop for RetentionBatchObservation {
    fn drop(&mut self) {
        if let (Some(metrics), Some(outcome)) = (&self.metrics, self.outcome) {
            metrics.count_performance(
                PerformanceCounter::RetentionBatch(self.operation, outcome),
                1,
            );
        }
    }
}

pub(crate) struct RetentionPhaseGuard {
    metrics: Option<Arc<ObservabilityRuntime>>,
    operation: RetentionOperation,
    phase: RetentionPhase,
    started: Instant,
}

impl Drop for RetentionPhaseGuard {
    fn drop(&mut self) {
        if let Some(metrics) = &self.metrics {
            metrics.observe_performance(
                PerformanceHistogram::RetentionPhase(self.operation, self.phase),
                self.started.elapsed(),
            );
        }
    }
}

pub(crate) fn record_backlog_observation(records: i64, oldest_timestamp: Option<DateTime<Utc>>) {
    let Some(metrics) = current_performance().filter(|metrics| metrics.enabled) else {
        return;
    };
    let records = records.max(0);
    metrics.set_performance(
        PerformanceGauge::RetentionBacklog(
            RetentionOperation::InvocationArchive,
            BacklogKind::Records,
        ),
        records as f64,
    );
    if records == 0 {
        metrics.set_performance(
            PerformanceGauge::RetentionBacklog(
                RetentionOperation::InvocationArchive,
                BacklogKind::OldestTimestamp,
            ),
            0.0,
        );
    } else if let Some(oldest_timestamp) = oldest_timestamp {
        metrics.set_performance(
            PerformanceGauge::RetentionBacklog(
                RetentionOperation::InvocationArchive,
                BacklogKind::OldestTimestamp,
            ),
            oldest_timestamp.timestamp() as f64
                + f64::from(oldest_timestamp.timestamp_subsec_nanos()) / 1_000_000_000.0,
        );
    }
}
