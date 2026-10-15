use super::*;

use crate::ObservabilityRuntime;
use crate::observability::contention::{Outcome, RetentionPhase, with_performance};
use chrono::{TimeZone, Utc};
use std::time::Duration;

fn has_metric(text: &str, name: &str, labels: &[&str], value: &str) -> bool {
    text.lines().any(|line| {
        line.starts_with(name)
            && labels.iter().all(|label| line.contains(label))
            && line.ends_with(value)
    })
}

#[tokio::test]
async fn retention_observability_records_outcomes_rows_phases_and_export() {
    let runtime = ObservabilityRuntime::new(true);
    with_performance(runtime.clone(), async {
        let mut success = BatchObservation::begin("codex_invocations", "synthetic", 3);
        for &phase in RetentionPhase::ALL {
            let guard = success.phase(phase);
            drop(guard);
        }
        success.committed(3, Duration::ZERO);
        success.finish(Outcome::Success);
        drop(success);

        let mut repeated = BatchObservation::begin("codex_invocations", "synthetic", 2);
        repeated.committed(2, Duration::ZERO);
        repeated.finish(Outcome::Success);
        drop(repeated);

        let mut failed = BatchObservation::begin("codex_invocations", "synthetic", 1);
        failed.finish(Outcome::Error);
        drop(failed);

        let mut unfinished_after_commit =
            BatchObservation::begin("codex_invocations", "synthetic", 1);
        unfinished_after_commit.committed(1, Duration::ZERO);
        drop(unfinished_after_commit);

        let mut deferred = BatchObservation::begin("codex_invocations", "synthetic", 1);
        deferred.finish(Outcome::Deferred);
        drop(deferred);

        let mut cancelled = BatchObservation::begin("codex_invocations", "synthetic", 1);
        cancelled.finish(Outcome::Cancelled);
        drop(cancelled);

        record_backlog_observation(
            7,
            Some(
                Utc.with_ymd_and_hms(2026, 10, 14, 12, 34, 56)
                    .single()
                    .expect("synthetic UTC timestamp"),
            ),
        );
    })
    .await;

    let text = runtime.render();
    assert!(has_metric(
        &text,
        "cvm_retention_batches_total",
        &["operation=\"invocation_archive\"", "outcome=\"success\""],
        "2"
    ));
    for (outcome, expected) in [("error", "2"), ("deferred", "1"), ("cancelled", "1")] {
        assert!(has_metric(
            &text,
            "cvm_retention_batches_total",
            &[
                "operation=\"invocation_archive\"",
                &format!("outcome=\"{outcome}\"")
            ],
            expected
        ));
    }
    assert!(has_metric(
        &text,
        "cvm_maintenance_processed_rows_total",
        &["operation=\"invocation_archive\""],
        "6"
    ));
    assert!(has_metric(
        &text,
        "cvm_maintenance_archived_rows_total",
        &["operation=\"invocation_archive\""],
        "6"
    ));
    for &phase in RetentionPhase::ALL {
        assert!(
            text.lines().any(|line| {
                line.starts_with("cvm_retention_phase_duration_seconds_count{")
                    && line.contains("operation=\"invocation_archive\"")
                    && line.contains(&format!("phase=\"{}\"", phase.label()))
                    && line.ends_with(" 1")
            }),
            "phase must be exported: {}",
            phase.label()
        );
    }
    assert!(has_metric(
        &text,
        "cvm_retention_backlog_records",
        &["operation=\"invocation_archive\""],
        "7"
    ));
    assert!(text.lines().any(|line| {
        line.starts_with("cvm_retention_backlog_oldest_timestamp_seconds{")
            && line.contains("operation=\"invocation_archive\"")
            && line.ends_with(" 1791981296")
    }));
}

#[tokio::test]
async fn retention_observability_keeps_disabled_and_unobserved_states_empty() {
    let disabled = ObservabilityRuntime::new(false);
    with_performance(disabled.clone(), async {
        let mut batch = BatchObservation::begin("codex_invocations", "synthetic", 1);
        batch.committed(1, Duration::ZERO);
        batch.finish(Outcome::Success);
        drop(batch);
        record_backlog_observation(1, None);
    })
    .await;
    assert!(!disabled.render().contains("cvm_retention_"));

    let unobserved = ObservabilityRuntime::new(true);
    assert!(!unobserved.render().contains("cvm_retention_backlog_"));
    with_performance(unobserved.clone(), async {
        record_backlog_observation(0, None);
    })
    .await;
    let text = unobserved.render();
    assert!(has_metric(
        &text,
        "cvm_retention_backlog_records",
        &["operation=\"invocation_archive\""],
        "0"
    ));
    assert!(has_metric(
        &text,
        "cvm_retention_backlog_oldest_timestamp_seconds",
        &["operation=\"invocation_archive\""],
        "0"
    ));
}
