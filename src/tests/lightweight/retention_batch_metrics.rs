use super::*;

#[test]
fn retention_final_summary_preserves_specific_stop_causes_and_budget_precedence() {
    use crate::maintenance::RetentionRunSummary;

    for reason in ["parallel_work_minute_coverage", "retention_work_budget"] {
        let mut summary = RetentionRunSummary {
            wait_reason: Some(reason.to_string()),
            ..RetentionRunSummary::default()
        };
        summary.apply_recorded_defer_reason(Some("archive_directory_lock_busy".to_string()));
        assert_eq!(summary.wait_reason.as_deref(), Some(reason));
        assert!(summary.deferred);
    }
    let mut summary = RetentionRunSummary {
        budget_exhausted: true,
        ..RetentionRunSummary::default()
    };
    summary.apply_recorded_defer_reason(Some("archive_directory_lock_busy".to_string()));
    assert_eq!(
        summary.wait_reason.as_deref(),
        Some("retention_work_budget")
    );
    let mut summary = RetentionRunSummary {
        deferred: true,
        wait_reason: Some("retention_write_admission".to_string()),
        ..RetentionRunSummary::default()
    };
    summary.apply_recorded_defer_reason(None);
    assert_eq!(
        summary.wait_reason.as_deref(),
        Some("retention_write_admission")
    );
}

#[tokio::test]
async fn retention_batch_timeout_metrics_attribute_only_unfinished_deadline_work() {
    use crate::maintenance::{
        BatchObservation, retention_test_with_batch_metrics, retention_test_with_work_budget,
    };

    let (_, metrics) = retention_test_with_batch_metrics(async {
        // No deadline: an admission deferral or error is not a timeout.
        drop(BatchObservation::begin(
            "codex_invocations",
            "2026-09",
            1000,
        ));
        retention_test_with_work_budget(Duration::ZERO, async {
            let mut partial =
                BatchObservation::begin("pool_upstream_request_attempts", "2026-09", 1000);
            partial.committed(64, Duration::ZERO);
            drop(partial);
            let mut complete = BatchObservation::begin("codex_invocations", "2026-08", 512);
            complete.committed(512, Duration::ZERO);
            drop(complete);
        })
        .await;
    })
    .await;

    assert_eq!(metrics.len(), 3);
    assert_eq!(metrics[0].timeout_count, 0);
    assert_eq!(metrics[1].dataset, "pool_upstream_request_attempts");
    assert_eq!(metrics[1].timeout_count, 1);
    assert_eq!(metrics[1].committed_rows, 64);
    assert_eq!(metrics[2].timeout_count, 0);
    let serialized = serde_json::to_value(&metrics).unwrap();
    assert_eq!(serialized[1]["timeoutCount"], 1);
    assert_eq!(serialized[2]["timeoutCount"], 0);
}
