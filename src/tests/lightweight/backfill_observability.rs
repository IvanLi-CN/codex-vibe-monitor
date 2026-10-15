use super::*;

#[tokio::test]
async fn backfill_notifications_preserve_oldest_pending_state_in_prometheus() {
    let runtime = ObservabilityRuntime::new(true);
    let scheduler = StartupBackfillScheduler::default();
    let task = StartupBackfillTask::ProxyUsage;

    crate::observability::contention::with_performance(runtime.clone(), async {
        scheduler.record_notification(task, WakeDecision::New);
        let first_pending = scheduler
            .pending_since
            .lock()
            .expect("pending state lock")
            .get(&task)
            .copied()
            .expect("first pending timestamp");
        tokio::time::sleep(Duration::from_millis(2)).await;
        scheduler.record_notification(task, WakeDecision::New);
        scheduler.wake(task);

        assert_eq!(
            scheduler
                .pending_since
                .lock()
                .expect("pending state lock")
                .get(&task)
                .copied(),
            Some(first_pending)
        );
    })
    .await;

    let text = runtime.render();
    let task_name = task.name();
    for reason in ["new", "merged"] {
        assert!(text.lines().any(|line| {
            line.starts_with("cvm_backfill_wake_notifications_total{")
                && line.contains(&format!("task=\"{task_name}\""))
                && line.contains(&format!("reason=\"{reason}\""))
                && line.ends_with(" 1")
        }));
    }
    assert!(text.lines().any(|line| {
        line.starts_with("cvm_backfill_wake_pending_tasks{")
            && line.contains(&format!("task=\"{task_name}\""))
            && line.ends_with(" 1")
    }));
    assert!(text.contains(&format!(
        "cvm_backfill_wake_oldest_timestamp_seconds{{task=\"{task_name}\"}}"
    )));
    assert!(text.contains(&format!(
        "cvm_backfill_wake_next_due_timestamp_seconds{{task=\"{task_name}\"}}"
    )));
}

#[tokio::test]
async fn backfill_wake_attempts_record_outcomes_and_cancellation() {
    let runtime = ObservabilityRuntime::new(true);
    let task = StartupBackfillTask::ProxyUsage;

    crate::observability::contention::with_performance(runtime.clone(), async {
        assert_eq!(
            observe_wake_attempt(task, async { Ok::<(u64, bool), anyhow::Error>((7, true)) })
                .await
                .expect("committed wake attempt"),
            7
        );
        assert_eq!(
            observe_wake_attempt(task, async { Ok::<(u64, bool), anyhow::Error>((0, false)) })
                .await
                .expect("noop wake attempt"),
            0
        );
        assert!(
            observe_wake_attempt(task, async {
                Err::<(u64, bool), _>(anyhow::anyhow!("fixture wake failure"))
            })
            .await
            .is_err()
        );
    })
    .await;

    let mut pending = Box::pin(crate::observability::contention::with_performance(
        runtime.clone(),
        observe_wake_attempt(task, std::future::pending::<anyhow::Result<(u64, bool)>>()),
    ));
    assert!(futures_util::FutureExt::now_or_never(pending.as_mut()).is_none());
    drop(pending);

    let text = runtime.render();
    let task_name = task.name();
    for outcome in ["committed", "noop", "error", "cancelled"] {
        assert!(
            text.lines().any(|line| {
                line.starts_with("cvm_backfill_wake_attempts_total{")
                    && line.contains(&format!("outcome=\"{outcome}\""))
                    && line.contains(&format!("task=\"{task_name}\""))
                    && line.ends_with(" 1")
            }),
            "missing wake outcome {outcome}: {text}"
        );
    }
}

#[tokio::test]
async fn disabled_backfill_observability_does_not_register_series() {
    let runtime = ObservabilityRuntime::new(false);
    let scheduler = StartupBackfillScheduler::default();
    let task = StartupBackfillTask::ProxyUsage;

    crate::observability::contention::with_performance(runtime.clone(), async {
        scheduler.record_notification(task, WakeDecision::New);
        scheduler.wake(task);
        observe_wake_attempt(task, async { Ok::<((), bool), anyhow::Error>(((), true)) })
            .await
            .expect("disabled wake attempt remains successful");
    })
    .await;

    assert!(runtime.render().is_empty());
}

#[test]
fn source_unavailable_result_retains_pending_backfill_state() {
    let scheduler = StartupBackfillScheduler::default();
    let task = StartupBackfillTask::ProxyUsage;

    scheduler.record_notification(task, WakeDecision::New);
    scheduler.record_task_result_with_actionable(task, false, false, false, true);

    assert!(
        scheduler
            .pending_since
            .lock()
            .expect("pending state lock")
            .contains_key(&task)
    );
}
