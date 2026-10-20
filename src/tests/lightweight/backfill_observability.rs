use super::maintenance_loop::wake_startup_backfill_coverage_repair_inner;
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
async fn backfill_wake_retry_deadline_is_published_in_prometheus() {
    let runtime = ObservabilityRuntime::new(true);
    let scheduler = StartupBackfillScheduler::default();
    let task = StartupBackfillTask::ProxyUsage;

    let expected_next_due =
        crate::observability::contention::with_performance(runtime.clone(), async {
            let observation_ticket = scheduler
                .record_notification(task, WakeDecision::New)
                .expect("record wake notification");
            scheduler
                .accept_wake_obligation(
                    WakeObligationKey {
                        task,
                        control_generation: None,
                        progress_key: None,
                    },
                    observation_ticket,
                )
                .expect("accept wake obligation");
            let attempt = scheduler
                .claim_due_wake_obligations(Utc::now())
                .into_iter()
                .next()
                .expect("claim wake obligation");
            scheduler
                .fail_wake_obligation(attempt.ticket, Utc::now())
                .expect("schedule wake retry")
                .next_due
                .timestamp_millis() as f64
                / 1_000.0
        })
        .await;

    let text = runtime.render();
    let line = text
        .lines()
        .find(|line| {
            line.starts_with("cvm_backfill_wake_next_due_timestamp_seconds{")
                && line.contains(&format!("task=\"{}\"", task.name()))
        })
        .expect("retry deadline metric");
    let actual_next_due = line
        .split_whitespace()
        .last()
        .expect("metric value")
        .parse::<f64>()
        .expect("numeric metric value");
    assert_eq!(actual_next_due, expected_next_due);
}

#[test]
fn unavailable_wake_store_reschedules_due_obligations() {
    let scheduler = StartupBackfillScheduler::default();
    let task = StartupBackfillTask::ProxyUsage;
    let observation_ticket = scheduler
        .record_notification(task, WakeDecision::New)
        .expect("record wake notification");
    scheduler
        .accept_wake_obligation(
            WakeObligationKey {
                task,
                control_generation: None,
                progress_key: None,
            },
            observation_ticket,
        )
        .expect("accept wake obligation");

    let now = Utc::now();
    let failures = scheduler.defer_due_wake_obligations(now);
    assert_eq!(failures.len(), 1);
    assert!(failures[0].next_due > now);
    assert_eq!(scheduler.pending_wake_obligation_count(), 1);
    assert_eq!(
        scheduler.next_wake_obligation_due(),
        Some(failures[0].next_due)
    );
    assert!(scheduler.claim_due_wake_obligations(now).is_empty());
}

#[test]
fn wake_obligation_retries_follow_bounded_backoff_and_warning_limit() {
    let scheduler = StartupBackfillScheduler::default();
    let task = StartupBackfillTask::ProxyUsage;
    let observation_ticket = scheduler
        .record_notification(task, WakeDecision::New)
        .expect("record wake notification");
    scheduler
        .accept_wake_obligation(
            WakeObligationKey {
                task,
                control_generation: None,
                progress_key: None,
            },
            observation_ticket,
        )
        .expect("accept wake obligation");

    let base_delays_ms = [250_i64, 500, 1_000, 2_000, 5_000];
    let mut now = Utc::now();
    for (index, base_delay_ms) in base_delays_ms.into_iter().enumerate() {
        let attempt = scheduler
            .claim_due_wake_obligations(now)
            .into_iter()
            .next()
            .expect("claim due wake obligation");
        let failure = scheduler
            .fail_wake_obligation(attempt.ticket, now)
            .expect("schedule bounded wake retry");
        let delay_ms = (failure.next_due - now).num_milliseconds();
        assert!((base_delay_ms..=base_delay_ms * 6 / 5).contains(&delay_ms));
        assert_eq!(failure.should_warn, index == 0);
        now = failure.next_due;
    }
    assert_eq!(scheduler.pending_wake_obligation_count(), 1);
}

#[test]
fn stale_prompt_control_generation_does_not_replace_or_cancel_newer_obligation() {
    let scheduler = StartupBackfillScheduler::default();
    let task = StartupBackfillTask::PromptCacheConversationsMaterialization;
    let first_observation = scheduler
        .record_notification(task, WakeDecision::New)
        .expect("record current wake notification");
    let first_ticket = scheduler
        .accept_wake_obligation(
            WakeObligationKey {
                task,
                control_generation: Some(3),
                progress_key: None,
            },
            first_observation,
        )
        .expect("accept current wake obligation");
    let stale_observation = scheduler
        .record_notification(task, WakeDecision::New)
        .expect("record stale wake notification");
    let stale_ticket = scheduler
        .accept_wake_obligation(
            WakeObligationKey {
                task,
                control_generation: Some(2),
                progress_key: None,
            },
            stale_observation,
        )
        .expect("accept stale wake notification");

    assert_eq!(stale_ticket, first_ticket);
    assert_eq!(scheduler.pending_wake_obligation_count(), 1);
    assert!(!scheduler.is_stale_wake_obligation_key(&WakeObligationKey {
        task,
        control_generation: Some(3),
        progress_key: None,
    }));
    scheduler.cancel_wake_obligation(task, Some(2));
    assert_eq!(scheduler.pending_wake_obligation_count(), 1);
    scheduler.cancel_wake_obligation(task, Some(3));
    assert_eq!(scheduler.pending_wake_obligation_count(), 0);
}

#[test]
fn stale_disable_does_not_clear_newer_wake_observation() {
    let scheduler = StartupBackfillScheduler::default();
    let task = StartupBackfillTask::PromptCacheConversationsMaterialization;
    let observation_ticket = scheduler
        .record_notification(task, WakeDecision::New)
        .expect("record current wake notification");
    scheduler
        .accept_wake_obligation(
            WakeObligationKey {
                task,
                control_generation: Some(3),
                progress_key: None,
            },
            observation_ticket,
        )
        .expect("accept current wake obligation");

    scheduler.record_and_cancel_wake_obligation(task, WakeDecision::Disabled, Some(2));

    assert_eq!(scheduler.pending_wake_obligation_count(), 1);
    assert!(
        scheduler
            .pending_since
            .lock()
            .expect("pending state lock")
            .contains_key(&task)
    );
}

#[test]
fn releasing_claimed_wake_batch_allows_every_obligation_to_retry() {
    let scheduler = StartupBackfillScheduler::default();
    let tasks = [
        StartupBackfillTask::ProxyUsage,
        StartupBackfillTask::ProxyCost,
    ];
    for task in tasks {
        let observation_ticket = scheduler
            .record_notification(task, WakeDecision::New)
            .expect("record wake notification");
        scheduler
            .accept_wake_obligation(
                WakeObligationKey {
                    task,
                    control_generation: None,
                    progress_key: None,
                },
                observation_ticket,
            )
            .expect("accept wake obligation");
    }
    let attempts = scheduler.claim_due_wake_obligations(Utc::now());
    assert_eq!(attempts.len(), tasks.len());

    scheduler.release_wake_obligation_claims(&attempts, Utc::now());

    assert_eq!(
        scheduler.claim_due_wake_obligations(Utc::now()).len(),
        tasks.len()
    );
}

#[test]
fn out_of_order_obligation_acceptance_keeps_latest_observation_ticket() {
    let scheduler = StartupBackfillScheduler::default();
    let task = StartupBackfillTask::ProxyUsage;
    let key = WakeObligationKey {
        task,
        control_generation: None,
        progress_key: None,
    };
    let first_observation = scheduler
        .record_notification(task, WakeDecision::New)
        .expect("record first wake notification");
    let second_observation = scheduler
        .record_notification(task, WakeDecision::New)
        .expect("record second wake notification");

    scheduler
        .accept_wake_obligation(key.clone(), second_observation)
        .expect("accept second wake notification");
    scheduler
        .accept_wake_obligation(key, first_observation)
        .expect("accept first wake notification after second");

    let attempt = scheduler
        .claim_due_wake_obligations(Utc::now())
        .into_iter()
        .next()
        .expect("claim accepted wake notification");
    let observation = scheduler
        .complete_wake_obligation(attempt.ticket)
        .expect("complete accepted wake obligation");
    assert_eq!(observation, second_observation);
    scheduler.confirm_observation(observation);
    assert!(
        !scheduler
            .pending_since
            .lock()
            .expect("pending state lock")
            .contains_key(&task)
    );
}

#[test]
fn catalog_identity_change_preserves_wake_backoff() {
    let scheduler = StartupBackfillScheduler::default();
    let task = StartupBackfillTask::ProxyCost;
    let first_observation = scheduler
        .record_notification(task, WakeDecision::New)
        .expect("record first catalog notification");
    scheduler
        .accept_wake_obligation(
            WakeObligationKey {
                task,
                control_generation: None,
                progress_key: Some("catalog-v1".to_owned()),
            },
            first_observation,
        )
        .expect("accept first catalog notification");
    let attempt = scheduler
        .claim_due_wake_obligations(Utc::now())
        .into_iter()
        .next()
        .expect("claim first catalog notification");
    let retry_observed_at = Utc::now();
    let failure = scheduler
        .fail_wake_obligation(attempt.ticket, retry_observed_at)
        .expect("schedule catalog retry");

    let second_observation = scheduler
        .record_notification(task, WakeDecision::New)
        .expect("record replacement catalog notification");
    scheduler
        .accept_wake_obligation(
            WakeObligationKey {
                task,
                control_generation: None,
                progress_key: Some("catalog-v2".to_owned()),
            },
            second_observation,
        )
        .expect("accept replacement catalog notification");

    assert_eq!(scheduler.next_wake_obligation_due(), Some(failure.next_due));
    assert!(
        scheduler
            .claim_due_wake_obligations(retry_observed_at)
            .is_empty()
    );
}

#[test]
fn in_flight_catalog_replacement_waits_for_current_attempt() {
    let scheduler = StartupBackfillScheduler::default();
    let task = StartupBackfillTask::ProxyCost;
    let first_observation = scheduler
        .record_notification(task, WakeDecision::New)
        .expect("record first catalog notification");
    let first_ticket = scheduler
        .accept_wake_obligation(
            WakeObligationKey {
                task,
                control_generation: None,
                progress_key: Some("catalog-v1".to_owned()),
            },
            first_observation,
        )
        .expect("accept first catalog notification");
    let first_attempt = scheduler
        .claim_due_wake_obligations(Utc::now())
        .into_iter()
        .next()
        .expect("claim first catalog notification");

    let second_observation = scheduler
        .record_notification(task, WakeDecision::New)
        .expect("record replacement catalog notification");
    let queued_ticket = scheduler
        .accept_wake_obligation(
            WakeObligationKey {
                task,
                control_generation: None,
                progress_key: Some("catalog-v2".to_owned()),
            },
            second_observation,
        )
        .expect("queue replacement catalog notification");
    assert_eq!(queued_ticket, first_ticket);

    let completed_observation = scheduler
        .complete_wake_obligation(first_attempt.ticket)
        .expect("complete first catalog notification");
    scheduler.confirm_observation(completed_observation);
    let second_attempt = scheduler
        .claim_due_wake_obligations(Utc::now())
        .into_iter()
        .next()
        .expect("claim queued catalog notification");
    assert_eq!(
        second_attempt.key.progress_key.as_deref(),
        Some("catalog-v2")
    );
}

#[test]
fn in_flight_catalog_replacement_keeps_latest_observation_ticket_queued() {
    let scheduler = StartupBackfillScheduler::default();
    let task = StartupBackfillTask::ProxyCost;
    let first_observation = scheduler
        .record_notification(task, WakeDecision::New)
        .expect("record first catalog notification");
    scheduler
        .accept_wake_obligation(
            WakeObligationKey {
                task,
                control_generation: None,
                progress_key: Some("catalog-v1".to_owned()),
            },
            first_observation,
        )
        .expect("accept first catalog notification");
    let first_attempt = scheduler
        .claim_due_wake_obligations(Utc::now())
        .into_iter()
        .next()
        .expect("claim first catalog notification");

    let second_observation = scheduler
        .record_notification(task, WakeDecision::New)
        .expect("record replacement catalog notification");
    scheduler
        .accept_wake_obligation(
            WakeObligationKey {
                task,
                control_generation: None,
                progress_key: Some("catalog-v2".to_owned()),
            },
            second_observation,
        )
        .expect("queue replacement catalog notification");
    let latest_observation = scheduler
        .record_notification(task, WakeDecision::New)
        .expect("record latest catalog notification");
    scheduler
        .accept_wake_obligation(
            WakeObligationKey {
                task,
                control_generation: None,
                progress_key: Some("catalog-v1".to_owned()),
            },
            latest_observation,
        )
        .expect("queue latest catalog notification");

    let completed_observation = scheduler
        .complete_wake_obligation(first_attempt.ticket)
        .expect("complete first catalog notification");
    assert_eq!(completed_observation, first_observation);
    scheduler.confirm_observation(completed_observation);
    assert!(
        scheduler
            .pending_since
            .lock()
            .expect("pending state lock")
            .contains_key(&task)
    );

    let second_attempt = scheduler
        .claim_due_wake_obligations(Utc::now())
        .into_iter()
        .next()
        .expect("claim queued latest catalog notification");
    assert_eq!(
        second_attempt.key.progress_key.as_deref(),
        Some("catalog-v1")
    );
    let completed_observation = scheduler
        .complete_wake_obligation(second_attempt.ticket)
        .expect("complete latest catalog notification");
    assert_eq!(completed_observation, latest_observation);
    scheduler.confirm_observation(completed_observation);
    assert!(
        !scheduler
            .pending_since
            .lock()
            .expect("pending state lock")
            .contains_key(&task)
    );
}

#[test]
fn stale_prompt_generation_keeps_latest_observation_ticket_queued() {
    let scheduler = StartupBackfillScheduler::default();
    let task = StartupBackfillTask::PromptCacheConversationsMaterialization;
    let first_observation = scheduler
        .record_notification(task, WakeDecision::New)
        .expect("record current prompt-cache notification");
    scheduler
        .accept_wake_obligation(
            WakeObligationKey {
                task,
                control_generation: Some(3),
                progress_key: None,
            },
            first_observation,
        )
        .expect("accept current prompt-cache notification");
    let first_attempt = scheduler
        .claim_due_wake_obligations(Utc::now())
        .into_iter()
        .next()
        .expect("claim current prompt-cache notification");

    let second_observation = scheduler
        .record_notification(task, WakeDecision::New)
        .expect("record newer prompt-cache notification");
    scheduler
        .accept_wake_obligation(
            WakeObligationKey {
                task,
                control_generation: Some(3),
                progress_key: None,
            },
            second_observation,
        )
        .expect("queue newer prompt-cache notification");
    let stale_observation = scheduler
        .record_notification(task, WakeDecision::New)
        .expect("record stale prompt-cache notification");
    scheduler
        .accept_wake_obligation(
            WakeObligationKey {
                task,
                control_generation: Some(1),
                progress_key: None,
            },
            stale_observation,
        )
        .expect("queue stale prompt-cache notification");

    let completed_observation = scheduler
        .complete_wake_obligation(first_attempt.ticket)
        .expect("complete current prompt-cache notification");
    assert_eq!(completed_observation, first_observation);
    scheduler.confirm_observation(completed_observation);
    assert!(
        scheduler
            .pending_since
            .lock()
            .expect("pending state lock")
            .contains_key(&task)
    );

    let second_attempt = scheduler
        .claim_due_wake_obligations(Utc::now())
        .into_iter()
        .next()
        .expect("claim queued prompt-cache notification");
    assert_eq!(second_attempt.key.control_generation, Some(3));
    let completed_observation = scheduler
        .complete_wake_obligation(second_attempt.ticket)
        .expect("complete queued prompt-cache notification");
    assert_eq!(completed_observation, stale_observation);
    scheduler.confirm_observation(completed_observation);
    assert!(
        !scheduler
            .pending_since
            .lock()
            .expect("pending state lock")
            .contains_key(&task)
    );
}

#[tokio::test]
async fn prompt_cache_obligation_respects_pressure_retry_deadline() {
    let store = startup_backfill_tests::prompt_cache_materialization_test_store().await;
    let task = StartupBackfillTask::PromptCacheConversationsMaterialization;
    let scheduler = StartupBackfillScheduler::default();
    let retry_at = Utc::now() + ChronoDuration::hours(1);
    scheduler.defer_for_pressure(task, retry_at);
    let observation_ticket = scheduler
        .record_notification(task, WakeDecision::New)
        .expect("record prompt-cache wake notification");
    scheduler
        .accept_wake_obligation(
            WakeObligationKey {
                task,
                control_generation: None,
                progress_key: None,
            },
            observation_ticket,
        )
        .expect("accept prompt-cache wake obligation");
    let attempt = scheduler
        .claim_due_wake_obligations(Utc::now())
        .into_iter()
        .next()
        .expect("claim prompt-cache wake obligation");

    assert!(matches!(
        confirm_startup_backfill_wake_obligation(
            &store.pool,
            &tokio_util::sync::CancellationToken::new(),
            &attempt,
            &scheduler,
        )
        .await
        .expect("confirm pressure-deferred wake obligation"),
        StartupBackfillWakeConfirmation::Deferred { next_due: _ }
    ));
    assert!(scheduler.defer_wake_obligation(attempt.ticket, retry_at));
    assert_eq!(scheduler.pending_wake_obligation_count(), 1);
    assert!(scheduler.claim_due_wake_obligations(Utc::now()).is_empty());
    assert_eq!(
        load_startup_backfill_progress_from_pool(&store.pool, task.name())
            .await
            .expect("read unchanged pressure-deferred progress")
            .wake_generation,
        0
    );
    store.pool.close().await;
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
async fn prompt_cache_noop_confirms_observation_without_changing_queued_work() {
    let task = StartupBackfillTask::PromptCacheConversationsMaterialization;
    let task_name = task.name();
    for enabled in [false, true] {
        let store = startup_backfill_tests::prompt_cache_materialization_test_store().await;
        let scheduler = StartupBackfillScheduler::default();
        scheduler.wake(task);
        let retry_at = Utc::now() + ChronoDuration::hours(1);
        scheduler.record_next_due(task, retry_at);
        let before = scheduler.health_snapshot();
        let runtime = ObservabilityRuntime::new(enabled);

        let changed = crate::observability::contention::with_performance(runtime.clone(), async {
            wake_prompt_cache_materialization_with_scheduler(
                &store,
                "noop_observation_regression",
                &scheduler,
            )
            .await
        })
        .await
        .expect("confirm an already queued prompt-cache wake");

        assert_eq!(changed, 0);
        assert_eq!(scheduler.health_snapshot(), before);
        assert!(scheduler.has_pending_wake(task));
        assert_eq!(scheduler.next_due_for(task), Some(retry_at));
        assert_eq!(
            load_startup_backfill_progress_from_pool(&store.pool, task_name)
                .await
                .expect("read unchanged prompt-cache progress")
                .wake_generation,
            0
        );
        assert!(
            !scheduler
                .pending_since
                .lock()
                .expect("observation lock")
                .contains_key(&task)
        );
        let text = runtime.render();
        if enabled {
            assert!(text.lines().any(|line| {
                line.starts_with("cvm_backfill_wake_attempts_total{")
                    && line.contains(&format!("task=\"{task_name}\""))
                    && line.contains("outcome=\"noop\"")
                    && line.ends_with(" 1")
            }));
            for metric in [
                "cvm_backfill_wake_pending_tasks",
                "cvm_backfill_wake_oldest_timestamp_seconds",
            ] {
                assert!(text.contains(&format!("{metric}{{task=\"{task_name}\"}} 0")));
            }
        } else {
            assert!(text.is_empty());
        }
        store.pool.close().await;
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

#[tokio::test]
async fn wake_obligation_confirmation_preserves_failed_deadline() {
    let pool = startup_backfill_tests::control_test_pool().await;
    let task = StartupBackfillTask::ProxyUsage;
    let task_key = "startup_backfill.proxy_usage";
    let retry_at = Utc::now() + ChronoDuration::hours(1);
    let retry_after = format_utc_iso(retry_at);
    sqlx::query("INSERT INTO managed_tasks (task_key,enabled) VALUES (?,1)")
        .bind(task_key)
        .execute(&pool)
        .await
        .expect("insert enabled task control");
    sqlx::query(
        "INSERT INTO startup_backfill_progress (task_name,next_run_after,last_status,enabled) VALUES (?,?,?,1)",
    )
    .bind(task.name())
    .bind(&retry_after)
    .bind(STARTUP_BACKFILL_STATUS_FAILED)
    .execute(&pool)
    .await
    .expect("insert failed progress");

    let scheduler = StartupBackfillScheduler::default();
    let observation_ticket = scheduler
        .record_notification(task, WakeDecision::New)
        .expect("record wake notification");
    scheduler
        .accept_wake_obligation(
            WakeObligationKey {
                task,
                control_generation: None,
                progress_key: None,
            },
            observation_ticket,
        )
        .expect("accept wake obligation");
    let attempt = scheduler
        .claim_due_wake_obligations(Utc::now())
        .into_iter()
        .next()
        .expect("claim wake obligation");

    let confirmation = confirm_startup_backfill_wake_obligation(
        &pool,
        &tokio_util::sync::CancellationToken::new(),
        &attempt,
        &scheduler,
    )
    .await
    .expect("confirm wake obligation");
    assert!(matches!(
        confirmation,
        StartupBackfillWakeConfirmation::Confirmed {
            wake_task: false,
            next_due: Some(_),
        }
    ));
    let stored = sqlx::query_as::<_, (i64, String, Option<String>)>(
        "SELECT wake_generation,last_status,next_run_after FROM startup_backfill_progress WHERE task_name=?",
    )
    .bind(task.name())
    .fetch_one(&pool)
    .await
    .expect("read confirmed progress");
    assert_eq!(stored.0, 1);
    assert_eq!(stored.1, STARTUP_BACKFILL_STATUS_FAILED);
    assert_eq!(stored.2.as_deref(), Some(retry_after.as_str()));
    pool.close().await;
}

#[tokio::test]
async fn claimed_wake_obligation_queues_replacement_until_current_attempt_finishes() {
    let pool = startup_backfill_tests::control_test_pool().await;
    let task = StartupBackfillTask::ProxyUsage;
    sqlx::query(
        "INSERT INTO managed_tasks (task_key,enabled) VALUES ('startup_backfill.proxy_usage',1)",
    )
    .execute(&pool)
    .await
    .expect("insert enabled task control");
    sqlx::query("INSERT INTO startup_backfill_progress (task_name,enabled) VALUES (?,1)")
        .bind(task.name())
        .execute(&pool)
        .await
        .expect("insert progress row");

    let scheduler = StartupBackfillScheduler::default();
    let old_observation = scheduler
        .record_notification(task, WakeDecision::New)
        .expect("record old wake notification");
    scheduler
        .accept_wake_obligation(
            WakeObligationKey {
                task,
                control_generation: None,
                progress_key: None,
            },
            old_observation,
        )
        .expect("accept old wake obligation");
    let old_attempt = scheduler
        .claim_due_wake_obligations(Utc::now())
        .into_iter()
        .next()
        .expect("claim old wake obligation");

    let new_observation = scheduler
        .record_notification(task, WakeDecision::New)
        .expect("record replacement wake notification");
    scheduler
        .accept_wake_obligation(
            WakeObligationKey {
                task,
                control_generation: None,
                progress_key: Some("proxy_usage:new-catalog".to_string()),
            },
            new_observation,
        )
        .expect("accept replacement wake obligation");

    assert!(matches!(
        confirm_startup_backfill_wake_obligation(
            &pool,
            &tokio_util::sync::CancellationToken::new(),
            &old_attempt,
            &scheduler,
        )
        .await
        .expect("confirm current wake obligation before promotion"),
        StartupBackfillWakeConfirmation::Confirmed { .. }
    ));
    let old_observation = scheduler
        .complete_wake_obligation(old_attempt.ticket)
        .expect("complete current wake obligation");
    scheduler.confirm_observation(old_observation);
    let replacement_attempt = scheduler
        .claim_due_wake_obligations(Utc::now())
        .into_iter()
        .next()
        .expect("claim queued replacement wake obligation");
    assert_eq!(
        replacement_attempt.key.progress_key.as_deref(),
        Some("proxy_usage:new-catalog")
    );
    scheduler.release_wake_obligation_claims(&[replacement_attempt], Utc::now());
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT wake_generation FROM startup_backfill_progress WHERE task_name=?",
        )
        .bind(task.name())
        .fetch_one(&pool)
        .await
        .expect("read stale progress row"),
        1
    );
    pool.close().await;
}

#[tokio::test]
async fn wake_obligation_confirmation_supports_legacy_pool_without_controls() {
    let pool = sqlx::SqlitePool::connect("sqlite::memory:")
        .await
        .expect("connect legacy wake pool");
    sqlx::query(
        "CREATE TABLE startup_backfill_progress (
            task_name TEXT PRIMARY KEY,
            cursor_id INTEGER NOT NULL DEFAULT 0,
            next_run_after TEXT,
            zero_update_streak INTEGER NOT NULL DEFAULT 0,
            last_started_at TEXT,
            last_finished_at TEXT,
            last_scanned INTEGER NOT NULL DEFAULT 0,
            last_updated INTEGER NOT NULL DEFAULT 0,
            last_status TEXT NOT NULL DEFAULT 'idle',
            suspension_reason TEXT,
            next_probe_at TEXT,
            wake_generation INTEGER NOT NULL DEFAULT 0,
            enabled INTEGER NOT NULL DEFAULT 1
        )",
    )
    .execute(&pool)
    .await
    .expect("create legacy progress table");

    let task = StartupBackfillTask::ReasoningEffort;
    let scheduler = StartupBackfillScheduler::default();
    let observation_ticket = scheduler
        .record_notification(task, WakeDecision::New)
        .expect("record legacy wake notification");
    scheduler
        .accept_wake_obligation(
            WakeObligationKey {
                task,
                control_generation: None,
                progress_key: None,
            },
            observation_ticket,
        )
        .expect("accept legacy wake obligation");
    let attempt = scheduler
        .claim_due_wake_obligations(Utc::now())
        .into_iter()
        .next()
        .expect("claim legacy wake obligation");

    assert!(matches!(
        confirm_startup_backfill_wake_obligation(
            &pool,
            &tokio_util::sync::CancellationToken::new(),
            &attempt,
            &scheduler,
        )
        .await
        .expect("confirm legacy wake obligation"),
        StartupBackfillWakeConfirmation::Confirmed {
            wake_task: true,
            next_due: None,
        }
    ));
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT wake_generation FROM startup_backfill_progress WHERE task_name=?",
        )
        .bind(task.name())
        .fetch_one(&pool)
        .await
        .expect("read legacy wake generation"),
        1
    );
    pool.close().await;
}

#[tokio::test]
async fn prompt_cache_wake_obligation_uses_the_same_conditional_confirmation() {
    let pool = startup_backfill_tests::control_test_pool().await;
    let task = StartupBackfillTask::PromptCacheConversationsMaterialization;
    let prompt_cache_control =
        crate::maintenance_store::PromptCacheMaterializationControl::default();
    let control_generation = prompt_cache_control.initialize(true).generation;
    sqlx::query(
        "INSERT INTO managed_tasks (task_key,enabled) VALUES ('startup_backfill.prompt_cache_conversations_materialization',1)",
    )
    .execute(&pool)
    .await
    .expect("insert prompt-cache task control");
    sqlx::query("INSERT INTO startup_backfill_progress (task_name,enabled) VALUES (?,1)")
        .bind(task.name())
        .execute(&pool)
        .await
        .expect("insert prompt-cache progress");

    let scheduler = StartupBackfillScheduler::default();
    let observation_ticket = scheduler
        .record_notification(task, WakeDecision::New)
        .expect("record prompt-cache wake notification");
    scheduler
        .accept_wake_obligation(
            WakeObligationKey {
                task,
                control_generation: Some(control_generation),
                progress_key: None,
            },
            observation_ticket,
        )
        .expect("accept prompt-cache wake obligation");
    let attempt = scheduler
        .claim_due_wake_obligations(Utc::now())
        .into_iter()
        .next()
        .expect("claim prompt-cache wake obligation");

    assert!(matches!(
        confirm_startup_backfill_wake_obligation_for_test(
            &pool,
            &tokio_util::sync::CancellationToken::new(),
            &attempt,
            &scheduler,
            &prompt_cache_control,
        )
        .await
        .expect("confirm prompt-cache wake obligation"),
        StartupBackfillWakeConfirmation::Confirmed {
            wake_task: true,
            next_due: None,
        }
    ));
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT wake_generation FROM startup_backfill_progress WHERE task_name=?",
        )
        .bind(task.name())
        .fetch_one(&pool)
        .await
        .expect("read prompt-cache wake generation"),
        1
    );
    pool.close().await;
}
