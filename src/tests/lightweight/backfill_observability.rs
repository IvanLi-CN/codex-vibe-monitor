use super::*;

#[tokio::test]
async fn backfill_notifications_preserve_oldest_pending_state_in_prometheus() {
    let runtime = ObservabilityRuntime::new(true);
    let scheduler = StartupBackfillScheduler::default();
    let task = StartupBackfillTask::ProxyUsage;

    crate::observability::contention::with_performance(runtime.clone(), async {
        let _ = scheduler.record_notification(task, WakeDecision::New);
        let first_pending_since = scheduler
            .pending_since
            .lock()
            .expect("pending state lock")
            .get(&task)
            .map(|observation| observation.since)
            .expect("first pending timestamp");
        tokio::time::sleep(Duration::from_millis(2)).await;
        let _ = scheduler.record_notification(task, WakeDecision::New);
        scheduler.wake(task);

        assert_eq!(
            scheduler
                .pending_since
                .lock()
                .expect("pending state lock")
                .get(&task)
                .map(|observation| observation.since),
            Some(first_pending_since)
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
        let _ = scheduler.record_notification(task, WakeDecision::New);
        scheduler.wake(task);
        observe_wake_attempt(task, async { Ok::<((), bool), anyhow::Error>(((), true)) })
            .await
            .expect("disabled wake attempt remains successful");
    })
    .await;

    assert!(runtime.render().is_empty());
}

#[tokio::test]
async fn noop_wake_preserves_business_schedule_with_observation_enabled_or_disabled() {
    let task = StartupBackfillTask::ProxyUsage;
    let task_name = task.name();
    for (enabled, wake_reason) in [
        (false, "noop_observation_regression"),
        (true, "noop_observation_regression"),
        (false, "operator_disabled"),
        (true, "operator_disabled"),
    ] {
        let pool = startup_backfill_tests::control_test_pool().await;
        sqlx::query(
            "INSERT INTO managed_tasks (task_key,enabled) VALUES ('startup_backfill.proxy_usage',1)",
        )
        .execute(&pool)
        .await
        .expect("enable managed task");
        sqlx::query("INSERT INTO startup_backfill_progress (task_name,enabled) VALUES (?,0)")
            .bind(task.name())
            .execute(&pool)
            .await
            .expect("retain independently disabled progress row");

        clear_startup_backfill_task_scheduler_for_test(task);
        STARTUP_BACKFILL_SCHEDULER.wake(task);
        let retry_at = Utc::now() + ChronoDuration::hours(1);
        STARTUP_BACKFILL_SCHEDULER.defer_for_pressure(task, retry_at);
        let before = STARTUP_BACKFILL_SCHEDULER.health_snapshot();
        let runtime = ObservabilityRuntime::new(enabled);

        let woken = crate::observability::contention::with_performance(runtime.clone(), async {
            wake_startup_backfill_tasks(&pool, &[task], wake_reason).await
        })
        .await
        .expect("complete no-op wake");

        assert_eq!(woken, 0);
        assert_eq!(STARTUP_BACKFILL_SCHEDULER.health_snapshot(), before);
        assert!(STARTUP_BACKFILL_SCHEDULER.has_pending_wake(task));
        assert!(STARTUP_BACKFILL_SCHEDULER.has_future_pressure_deadline(task, Utc::now()));
        assert_eq!(
            STARTUP_BACKFILL_SCHEDULER
                .next_due
                .lock()
                .expect("retry schedule lock")
                .get(&task),
            Some(&retry_at)
        );
        assert!(
            !STARTUP_BACKFILL_SCHEDULER
                .pending_since
                .lock()
                .expect("observation lock")
                .contains_key(&task)
        );
        if enabled {
            let text = runtime.render();
            assert!(text.lines().any(|line| {
                line.starts_with("cvm_backfill_wake_attempts_total{")
                    && line.contains(&format!("task=\"{task_name}\""))
                    && line.contains("outcome=\"noop\"")
                    && line.ends_with(" 1")
            }));
            assert!(text.contains(&format!(
                "cvm_backfill_wake_pending_tasks{{task=\"{task_name}\"}} 0"
            )));
        } else {
            assert!(runtime.render().is_empty());
        }
        clear_startup_backfill_task_scheduler_for_test(task);
        pool.close().await;
    }
}

#[tokio::test]
async fn coverage_wake_without_maintenance_pool_does_not_confirm_attempt() {
    let runtime = ObservabilityRuntime::new(true);
    let task = StartupBackfillTask::AccountActivityV2Coverage;
    clear_startup_backfill_task_scheduler_for_test(task);

    crate::observability::contention::with_performance(runtime.clone(), async {
        let ticket = STARTUP_BACKFILL_SCHEDULER
            .record_notification(task, WakeDecision::New)
            .expect("new coverage notification ticket");
        assert_eq!(
            wake_startup_backfill_coverage_repair_inner(
                None,
                "maintenance_store_unavailable",
                Some(ticket),
            )
            .await
            .expect("scheduler-only coverage wake"),
            0
        );
        assert!(
            STARTUP_BACKFILL_SCHEDULER
                .pending_since
                .lock()
                .expect("pending state lock")
                .contains_key(&task)
        );
    })
    .await;

    let text = runtime.render();
    assert!(text.lines().any(|line| {
        line.starts_with("cvm_backfill_wake_notifications_total{")
            && line.contains(&format!("task=\"{}\"", task.name()))
            && line.contains("reason=\"new\"")
            && line.ends_with(" 1")
    }));
    assert!(!text.lines().any(|line| {
        line.starts_with("cvm_backfill_wake_attempts_total{")
            && line.contains(&format!("task=\"{}\"", task.name()))
    }));
    clear_startup_backfill_task_scheduler_for_test(task);
}

#[test]
fn business_task_results_do_not_create_pending_wake_state() {
    let scheduler = StartupBackfillScheduler::default();
    let task = StartupBackfillTask::ProxyUsage;

    scheduler.record_task_result_with_actionable(task, false, false, true);
    scheduler.record_task_result_with_actionable(task, false, true, false);
    scheduler.record_task_result_with_actionable(task, true, false, false);

    assert!(
        !scheduler
            .pending_since
            .lock()
            .expect("pending state lock")
            .contains_key(&task)
    );
}

#[tokio::test]
async fn durable_wake_confirmation_clears_current_observation() {
    let runtime = ObservabilityRuntime::new(true);
    let scheduler = StartupBackfillScheduler::default();
    let task = StartupBackfillTask::ProxyUsage;

    crate::observability::contention::with_performance(runtime.clone(), async {
        let ticket = scheduler
            .record_notification(task, WakeDecision::New)
            .expect("new wake notification ticket");
        observe_wake_attempt(task, async { Ok::<((), bool), anyhow::Error>(((), true)) })
            .await
            .expect("durable wake attempt");
        confirm_scheduler_wake_observation(&scheduler, ticket);
        scheduler.record_task_result_with_actionable(task, false, false, true);
        assert!(
            !scheduler
                .pending_since
                .lock()
                .expect("pending state lock")
                .contains_key(&task)
        );
    })
    .await;

    let task_name = task.name();
    let text = runtime.render();
    assert!(text.contains(&format!(
        "cvm_backfill_wake_pending_tasks{{task=\"{task_name}\"}} 0"
    )));
    assert!(text.contains(&format!(
        "cvm_backfill_wake_oldest_timestamp_seconds{{task=\"{task_name}\"}} 0"
    )));
    assert!(text.lines().any(|line| {
        line.starts_with("cvm_backfill_wake_attempts_total{")
            && line.contains(&format!("task=\"{task_name}\""))
            && line.contains("outcome=\"committed\"")
            && line.ends_with(" 1")
    }));
}

#[tokio::test]
async fn durable_wake_failure_retains_current_observation() {
    let runtime = ObservabilityRuntime::new(true);
    let scheduler = StartupBackfillScheduler::default();
    let task = StartupBackfillTask::ProxyUsage;

    crate::observability::contention::with_performance(runtime.clone(), async {
        let _ticket = scheduler
            .record_notification(task, WakeDecision::New)
            .expect("new wake notification ticket");
        assert!(
            observe_wake_attempt(task, async {
                Err::<((), bool), _>(anyhow::anyhow!("durable wake failure"))
            })
            .await
            .is_err()
        );
        assert!(
            scheduler
                .pending_since
                .lock()
                .expect("pending state lock")
                .contains_key(&task)
        );
    })
    .await;

    let task_name = task.name();
    let text = runtime.render();
    assert!(text.contains(&format!(
        "cvm_backfill_wake_pending_tasks{{task=\"{task_name}\"}} 1"
    )));
    assert!(text.lines().any(|line| {
        line.starts_with("cvm_backfill_wake_attempts_total{")
            && line.contains(&format!("task=\"{task_name}\""))
            && line.contains("outcome=\"error\"")
            && line.ends_with(" 1")
    }));
}

#[test]
fn stale_wake_confirmation_does_not_clear_new_notification() {
    let scheduler = StartupBackfillScheduler::default();
    let task = StartupBackfillTask::ProxyUsage;

    let old_ticket = scheduler
        .record_notification(task, WakeDecision::New)
        .expect("old wake notification ticket");
    let first_pending_since = scheduler
        .pending_since
        .lock()
        .expect("pending state lock")
        .get(&task)
        .map(|observation| observation.since)
        .expect("first pending timestamp");
    let new_ticket = scheduler
        .record_notification(task, WakeDecision::New)
        .expect("new wake notification ticket");

    assert_ne!(old_ticket, new_ticket);
    assert_eq!(
        scheduler
            .pending_since
            .lock()
            .expect("pending state lock")
            .get(&task)
            .map(|observation| observation.since),
        Some(first_pending_since)
    );

    confirm_scheduler_wake_observation(&scheduler, old_ticket);
    assert_eq!(
        scheduler
            .pending_since
            .lock()
            .expect("pending state lock")
            .get(&task)
            .map(|observation| observation.ticket),
        Some(new_ticket)
    );

    confirm_scheduler_wake_observation(&scheduler, new_ticket);
    assert!(
        !scheduler
            .pending_since
            .lock()
            .expect("pending state lock")
            .contains_key(&task)
    );
}
