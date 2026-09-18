pub(crate) async fn run_startup_persistent_prep_best_effort(
    state: &Arc<AppState>,
    prep_cli: &CliArgs,
) -> bool {
    if !should_run_startup_persistent_prep(prep_cli) {
        return true;
    }

    // The individual manifest and expiry writers acquire the maintenance permit themselves.
    // Holding the global background slot here would make those nested admissions reject every
    // startup pass before they can do any work.
    let defer_generation = super::retention::retention_defer_generation();
    match run_startup_persistent_prep_inner(&state.pool, &state.config, prep_cli, false).await {
        Ok(summary) => {
            let deferred = super::retention::retention_defer_generation() != defer_generation;
            info!(
                refreshed_manifest_batches = summary.refreshed_manifest_batches,
                refreshed_manifest_account_rows = summary.refreshed_manifest_account_rows,
                missing_manifest_files = summary.missing_manifest_files,
                backfilled_archive_expiries = summary.backfilled_archive_expiries,
                bootstrapped_hourly_rollups = summary.bootstrapped_hourly_rollups,
                pending_manifest_batches = summary.pending_manifest_batches,
                deferred,
                "startup background prep finished"
            );
            !deferred && summary.pending_manifest_batches == 0
        }
        Err(err) => {
            let pressure_error = crate::db_pressure::global_db_pressure_gate()
                .record_error("startup_persistent_prep", &err);
            warn!(error = %err, retry_soon = pressure_error, "startup background prep failed");
            !pressure_error
        }
    }
}

async fn run_startup_persistent_prep_best_effort_cancellable(
    state: &Arc<AppState>,
    prep_cli: &CliArgs,
    cancel: &CancellationToken,
) -> Option<bool> {
    tokio::select! {
        biased;
        _ = cancel.cancelled() => None,
        result = run_startup_persistent_prep_best_effort(state, prep_cli) => Some(result),
    }
}

pub(crate) async fn run_pressure_eligible_startup_backfill_tasks(
    state: Arc<AppState>,
    cancel: &CancellationToken,
    gate: &crate::db_pressure::DbPressureGate,
) {
    if cancel.is_cancelled() {
        return;
    }
    let tasks = STARTUP_BACKFILL_SCHEDULER.take_pressure_deferred_tasks();
    if tasks.is_empty() {
        return;
    }
    debug!(
        task_count = tasks.len(),
        "pressure eligibility changed; dispatching deferred startup backfill tasks"
    );
    run_startup_backfill_maintenance_pass_with_gate_inner(state, cancel, Some(&tasks), gate).await;
}

pub(crate) fn spawn_startup_backfill_maintenance(
    state: Arc<AppState>,
    cancel: CancellationToken,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        if cancel.is_cancelled() {
            info!("startup backfill maintenance skipped because shutdown is already in progress");
            return;
        }
        let prep_cli = CliArgs::default();
        let Some(prep_pending) =
            run_startup_persistent_prep_best_effort_cancellable(&state, &prep_cli, &cancel).await
        else {
            return;
        };
        let mut startup_prep_pending = prep_pending;
        let mut startup_prep_retry_at = startup_prep_pending
            .then(|| Instant::now() + Duration::from_secs(STARTUP_BACKFILL_ACTIVE_INTERVAL_SECS));
        run_startup_backfill_maintenance_pass(state.clone(), &cancel, None).await;
        // Register before either P2 supervisor is scheduled so long-term pruning cannot
        // reclaim a terminal event ahead of the minute projection consumer.
        state
            .terminal_projection_hub
            .activate_timeseries_consumer(0);
        spawn_long_term_projection_supervisor(state.clone(), cancel.clone());
        spawn_timeseries_minute_projection_supervisor(state.clone(), cancel.clone());

        let mut observed_generation = STARTUP_BACKFILL_SCHEDULER.generation();

        loop {
            let gate = crate::db_pressure::global_db_pressure_gate();
            let observed_pressure_eligibility = gate.eligibility_generation();
            let next_due = STARTUP_BACKFILL_SCHEDULER.next_due();
            let mut wait_for = startup_backfill_wait_duration(next_due);
            if let Some(retry_at) = startup_prep_retry_at {
                wait_for = wait_for.min(retry_at.saturating_duration_since(Instant::now()));
            }

            tokio::select! {
                _ = cancel.cancelled() => {
                    info!("startup backfill maintenance received shutdown");
                    break;
                }
                _ = STARTUP_BACKFILL_SCHEDULER.wait_for_wake(observed_generation) => {
                    observed_generation = STARTUP_BACKFILL_SCHEDULER.generation();
                    let tasks = STARTUP_BACKFILL_SCHEDULER.drain_woken_tasks();
                    if !tasks.is_empty() {
                        run_startup_backfill_maintenance_pass(state.clone(), &cancel, Some(&tasks)).await;
                    }
                }
                _ = gate.wait_for_eligibility_change(observed_pressure_eligibility) => {
                    run_pressure_eligible_startup_backfill_tasks(
                        state.clone(),
                        &cancel,
                        gate,
                    ).await;
                }
                _ = sleep(wait_for) => {
                    observed_generation = STARTUP_BACKFILL_SCHEDULER.generation();
                    STARTUP_BACKFILL_SCHEDULER.drain_woken_tasks();
                    if startup_prep_pending
                        && startup_prep_retry_at.is_none_or(|retry_at| retry_at <= Instant::now())
                    {
                        let Some(prep_pending) =
                            run_startup_persistent_prep_best_effort_cancellable(&state, &prep_cli, &cancel)
                                .await
                        else {
                            break;
                        };
                        startup_prep_pending = prep_pending;
                        startup_prep_retry_at = startup_prep_pending.then(|| {
                            Instant::now()
                                + Duration::from_secs(STARTUP_BACKFILL_ACTIVE_INTERVAL_SECS)
                        });
                    }
                    let due_tasks = STARTUP_BACKFILL_SCHEDULER.drain_due_tasks(Utc::now());
                    if !due_tasks.is_empty() {
                        run_startup_backfill_maintenance_pass(
                            state.clone(),
                            &cancel,
                            Some(&due_tasks),
                        )
                        .await;
                    }
                }
            }
        }
    })
}

#[cfg(test)]
mod startup_backfill_tests {
    use super::*;

    #[test]
    fn scheduler_health_tracks_wakes_due_work_and_active_outcomes() {
        let scheduler = StartupBackfillScheduler::default();
        scheduler.wake(StartupBackfillTask::HistoricalRollups);
        scheduler.record_noop_suppressed();

        let woken = scheduler.health_snapshot();
        assert_eq!(woken.state, "healthy");
        assert_eq!(woken.wake_count, 1);
        assert_eq!(woken.woken_task_count, 1);

        assert_eq!(
            scheduler.drain_due_tasks(Utc::now()),
            vec![StartupBackfillTask::HistoricalRollups]
        );
        scheduler.record_task_result(StartupBackfillTask::HistoricalRollups, false, true);
        let deferred = scheduler.health_snapshot();
        assert_eq!(deferred.state, "deferred");
        assert_eq!(deferred.due_dispatch_count, 1);
        assert_eq!(deferred.pressure_defer_count, 1);
        assert_eq!(deferred.noop_suppressed_count, 1);

        scheduler.record_task_result(StartupBackfillTask::HistoricalRollups, true, false);
        assert_eq!(scheduler.health_snapshot().state, "degraded");

        scheduler.record_task_result(StartupBackfillTask::HistoricalRollups, false, false);
        let recovered = scheduler.health_snapshot();
        assert_eq!(recovered.state, "healthy");
        assert_eq!(recovered.failure_count, 1);
        assert_eq!(recovered.failed_task_count, 0);
    }

    #[test]
    fn pressure_defer_uses_the_gate_absolute_deadline() {
        let gate = crate::db_pressure::DbPressureGate::new(1, Duration::from_secs(60));
        gate.record_pressure("test", "forced");
        let expected_deadline = gate
            .pressure_cooldown_deadline_epoch_ms()
            .expect("active pressure cooldown deadline");

        let retry_at = startup_backfill_pressure_retry_at(
            &gate,
            crate::db_pressure::DbPressureDenyReason::PressureCooldown { remaining_ms: 1 },
        );

        assert_eq!(retry_at.timestamp_millis() as u64, expected_deadline);
    }

    #[test]
    fn pressure_defer_schedules_one_deadline_and_dispatches_once() {
        let scheduler = StartupBackfillScheduler::default();
        let task = StartupBackfillTask::ReasoningEffort;
        let deadline = DateTime::<Utc>::from_timestamp_millis(1_800_000_000_750)
            .expect("valid fixed pressure deadline");

        scheduler.defer_for_pressure(task, deadline);
        assert!(
            scheduler
                .drain_due_tasks(deadline - ChronoDuration::milliseconds(1))
                .is_empty()
        );
        let waiting = scheduler.health_snapshot();
        assert_eq!(waiting.wake_count, 0);
        assert_eq!(waiting.due_dispatch_count, 0);
        assert_eq!(waiting.pressure_defer_count, 0);
        assert_eq!(waiting.scheduled_task_count, 1);

        assert_eq!(scheduler.drain_due_tasks(deadline), vec![task]);
        scheduler.record_task_result(task, false, true);
        assert!(scheduler.drain_due_tasks(deadline).is_empty());
        let deferred = scheduler.health_snapshot();
        assert_eq!(deferred.wake_count, 0);
        assert_eq!(deferred.due_dispatch_count, 1);
        assert_eq!(deferred.pressure_defer_count, 1);
        assert_eq!(deferred.scheduled_task_count, 0);
        assert_eq!(deferred.deferred_task_count, 1);
    }

    #[tokio::test]
    async fn pressure_eligibility_change_dispatches_a_deferred_task_before_its_deadline() {
        let scheduler = Arc::new(StartupBackfillScheduler::default());
        let gate = Arc::new(crate::db_pressure::DbPressureGate::new(
            1,
            Duration::from_secs(30),
        ));
        let permit = gate
            .try_begin_background("test-holder")
            .expect("occupy the sole background slot");
        let observed_eligibility = gate.eligibility_generation();
        let task = StartupBackfillTask::ReasoningEffort;
        let deadline = Utc::now() + ChronoDuration::minutes(5);

        scheduler.defer_for_pressure(task, deadline);
        scheduler.record_task_result(task, false, true);

        assert!(
            scheduler.drain_due_tasks(Utc::now()).is_empty(),
            "the in-memory fallback deadline must not be due yet"
        );

        let wake_gate = gate.clone();
        let wake_scheduler = scheduler.clone();
        let wake = tokio::spawn(async move {
            wake_gate
                .wait_for_eligibility_change(observed_eligibility)
                .await;
            wake_scheduler.take_pressure_deferred_tasks()
        });
        tokio::task::yield_now().await;
        assert!(
            !wake.is_finished(),
            "the task must wait for an eligibility-clear event before its fallback deadline"
        );

        drop(permit);
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(1), wake)
                .await
                .expect("permit release must wake the deferred task before its deadline")
                .expect("eligibility waiter must not panic"),
            vec![task]
        );
        assert_eq!(scheduler.next_due(), None);
    }

    #[test]
    fn sqlite_busy_and_locked_are_actual_backfill_failures() {
        for error in [
            anyhow::anyhow!("database is busy"),
            anyhow::anyhow!("database table is locked"),
        ] {
            assert_eq!(
                startup_backfill_failure_kind(&error),
                StartupBackfillFailureKind::SqliteBusyOrLocked
            );
        }
        assert_eq!(
            startup_backfill_failure_kind(&anyhow::anyhow!("source unavailable")),
            StartupBackfillFailureKind::Operation
        );

        let scheduler = StartupBackfillScheduler::default();
        scheduler.record_task_result(StartupBackfillTask::ReasoningEffort, true, false);
        let health = scheduler.health_snapshot();
        assert_eq!(health.state, "degraded");
        assert_eq!(health.failure_count, 1);
        assert_eq!(health.pressure_defer_count, 0);
        assert_eq!(health.failed_task_count, 1);
        assert_eq!(health.deferred_task_count, 0);
    }

    #[test]
    fn coverage_repair_health_is_independent_from_historical_rollups() {
        let scheduler = StartupBackfillScheduler::default();
        scheduler.record_task_result(StartupBackfillTask::HistoricalRollups, true, false);

        scheduler.record_task_result(StartupBackfillTask::AccountActivityV2Coverage, false, true);

        let health = scheduler.health_snapshot();
        assert_eq!(health.state, "degraded");
        assert_eq!(health.failed_task_count, 1);
        assert_eq!(health.pressure_defer_count, 1);
    }

    #[test]
    fn coverage_repair_does_not_repeat_its_planner_in_the_following_hourly_refresh() {
        assert_eq!(
            startup_backfill_hourly_rollup_refresh_scope(),
            HourlyRollupRefreshScope::SkipActiveAccountActivityV2CoverageRepair
        );
    }

    #[test]
    fn actionable_no_progress_backoff_caps_at_fifteen_minutes() {
        let run = StartupBackfillRunState {
            scanned: 2,
            updated: 0,
            hit_scan_limit: true,
            ..StartupBackfillRunState::default()
        };
        assert_eq!(
            startup_backfill_next_delay(&run, 1),
            Duration::from_secs(15)
        );
        assert_eq!(
            startup_backfill_next_delay(&run, 2),
            Duration::from_secs(60)
        );
        assert_eq!(
            startup_backfill_next_delay(&run, 3),
            Duration::from_secs(5 * 60)
        );
        assert_eq!(
            startup_backfill_next_delay(&run, 4),
            Duration::from_secs(15 * 60)
        );
        assert_eq!(
            startup_backfill_next_delay(&run, 99),
            Duration::from_secs(15 * 60)
        );
    }

    #[test]
    fn historical_rollup_cursor_advance_after_budget_exhaustion_retries_without_a_task_run() {
        let run = StartupBackfillRunState {
            next_cursor_id: 12,
            scanned: 1,
            retry_soon: true,
            ..StartupBackfillRunState::default()
        };

        assert_eq!(
            startup_backfill_next_delay(&run, 1),
            Duration::from_secs(15)
        );
        assert!(!startup_backfill_run_is_actionable(&run));
    }

    #[test]
    fn historical_rollup_budget_retry_stays_short_after_cursor_wrap() {
        let retry_soon = historical_rollup_should_retry_soon(true, 1);
        assert!(retry_soon);
        assert!(historical_rollup_should_retry_soon(true, 32));
        assert!(!historical_rollup_should_retry_soon(true, 0));
        assert!(!historical_rollup_should_retry_soon(false, 1));

        let run = StartupBackfillRunState {
            retry_soon,
            ..StartupBackfillRunState::default()
        };
        assert_eq!(
            startup_backfill_next_delay(&run, 0),
            Duration::from_secs(15)
        );
        assert!(!startup_backfill_run_is_actionable(&run));
    }

    #[test]
    fn overdue_backfill_deadline_runs_without_an_idle_sleep() {
        assert_eq!(
            startup_backfill_wait_duration(Some(Utc::now() - ChronoDuration::seconds(1))),
            Duration::ZERO
        );
    }

    #[test]
    fn scheduler_drains_only_tasks_with_an_expired_deadline() {
        let scheduler = StartupBackfillScheduler::default();
        let future_due = Utc::now() + ChronoDuration::hours(1);
        scheduler.record_next_due(
            StartupBackfillTask::HistoricalRollups,
            Utc::now() - ChronoDuration::seconds(1),
        );
        scheduler.record_next_due(StartupBackfillTask::ReasoningEffort, future_due);

        assert_eq!(
            scheduler.drain_due_tasks(Utc::now()),
            vec![StartupBackfillTask::HistoricalRollups]
        );
        assert_eq!(scheduler.next_due(), Some(future_due));
    }

    #[test]
    fn only_progress_or_non_idle_backlog_triggers_rollup_refresh() {
        assert!(startup_backfill_run_is_actionable(
            &StartupBackfillRunState {
                updated: 1,
                ..StartupBackfillRunState::default()
            }
        ));
        assert!(startup_backfill_run_is_actionable(
            &StartupBackfillRunState {
                hit_scan_limit: true,
                ..StartupBackfillRunState::default()
            }
        ));
        assert!(!startup_backfill_run_is_actionable(
            &StartupBackfillRunState {
                scanned: 1,
                force_idle: true,
                ..StartupBackfillRunState::default()
            }
        ));
        assert!(!startup_backfill_run_is_actionable(
            &StartupBackfillRunState::default()
        ));
    }

    #[test]
    fn terminal_payload_input_wakes_only_missing_field_repairs() {
        let mut record = crate::tests::test_proxy_capture_record(
            "startup-backfill-terminal-wake",
            "2026-08-09 12:00:00",
        );
        record.usage.total_tokens = None;
        record.cost = None;
        record.payload = Some("{}".to_string());
        record.req_raw.path = Some("/tmp/request.raw".to_string());
        record.resp_raw.path = Some("/tmp/response.raw".to_string());

        let tasks =
            startup_backfill_tasks_for_terminal(&api_invocation_from_runtime_record(&record));

        assert_eq!(
            tasks,
            vec![
                StartupBackfillTask::ProxyUsage,
                StartupBackfillTask::PromptCacheKey,
                StartupBackfillTask::RequestedServiceTier,
                StartupBackfillTask::ReasoningEffort,
                StartupBackfillTask::InvocationServiceTier,
            ]
        );

        let complete =
            api_invocation_from_runtime_record(&crate::tests::test_proxy_capture_record(
                "startup-backfill-terminal-complete",
                "2026-08-09 12:01:00",
            ));
        assert!(startup_backfill_tasks_for_terminal(&complete).is_empty());
    }

    #[test]
    fn source_unavailable_probe_uses_one_shared_budget() {
        assert_eq!(startup_backfill_scan_limit(true), 100);
        assert_eq!(startup_backfill_run_budget(true), Duration::from_secs(2));
        assert_eq!(
            startup_backfill_scan_limit(false),
            STARTUP_BACKFILL_SCAN_LIMIT
        );
        assert_eq!(
            startup_backfill_run_budget(false),
            Duration::from_secs(STARTUP_BACKFILL_RUN_BUDGET_SECS)
        );
    }

    #[test]
    fn historical_rollup_backfill_run_state_backs_off_when_only_blocked_archives_remain() {
        let before = HistoricalRollupBackfillSnapshot {
            pending_buckets: 2,
            legacy_archive_pending: 1,
            pending_usage_breakdown_batches: 1,
            last_materialized_hour: None,
            alert_level: HistoricalRollupBackfillAlertLevel::Critical,
        };
        let after = before.clone();
        let summary = HistoricalRollupMaterializationSummary {
            scanned_archive_batches: 1,
            blocked_archive_batches: 1,
            ..HistoricalRollupMaterializationSummary::default()
        };

        let run =
            historical_rollup_startup_backfill_run_state(7, 0, &before, &after, &summary, 1, 1);

        assert_eq!(run.next_cursor_id, 8);
        assert_eq!(run.scanned, 1);
        assert_eq!(run.updated, 0);
        assert!(!run.hit_scan_limit);
        assert!(run.force_idle);
    }

    #[test]
    fn historical_rollup_backfill_run_state_stays_active_while_catching_up() {
        let before = HistoricalRollupBackfillSnapshot {
            pending_buckets: 8,
            legacy_archive_pending: 3,
            pending_usage_breakdown_batches: 3,
            last_materialized_hour: None,
            alert_level: HistoricalRollupBackfillAlertLevel::Critical,
        };
        let after = HistoricalRollupBackfillSnapshot {
            pending_buckets: 4,
            legacy_archive_pending: 2,
            pending_usage_breakdown_batches: 2,
            last_materialized_hour: None,
            alert_level: HistoricalRollupBackfillAlertLevel::Warn,
        };
        let summary = HistoricalRollupMaterializationSummary {
            scanned_archive_batches: 1,
            materialized_archive_batches: 1,
            materialized_invocation_batches: 1,
            ..HistoricalRollupMaterializationSummary::default()
        };

        let run =
            historical_rollup_startup_backfill_run_state(11, 0, &before, &after, &summary, 3, 2);

        assert_eq!(run.next_cursor_id, 12);
        assert_eq!(run.scanned, 1);
        assert_eq!(run.updated, 4);
        assert!(run.hit_scan_limit);
        assert!(!run.force_idle);
    }

    #[test]
    fn historical_rollup_backfill_run_state_stays_active_when_partial_scan_found_only_blocked_work()
    {
        let before = HistoricalRollupBackfillSnapshot {
            pending_buckets: 8,
            legacy_archive_pending: 3,
            pending_usage_breakdown_batches: 3,
            last_materialized_hour: None,
            alert_level: HistoricalRollupBackfillAlertLevel::Critical,
        };
        let after = before.clone();
        let summary = HistoricalRollupMaterializationSummary {
            scanned_archive_batches: 1,
            blocked_archive_batches: 1,
            ..HistoricalRollupMaterializationSummary::default()
        };

        let run =
            historical_rollup_startup_backfill_run_state(5, 0, &before, &after, &summary, 3, 3);

        assert_eq!(run.next_cursor_id, 6);
        assert_eq!(run.scanned, 1);
        assert_eq!(run.updated, 0);
        assert!(run.hit_scan_limit);
        assert!(!run.force_idle);
    }

    #[test]
    fn historical_rollup_backfill_run_state_does_not_back_off_when_only_blocked_archive_was_after_skip()
     {
        let before = HistoricalRollupBackfillSnapshot {
            pending_buckets: 8,
            legacy_archive_pending: 2,
            pending_usage_breakdown_batches: 2,
            last_materialized_hour: None,
            alert_level: HistoricalRollupBackfillAlertLevel::Critical,
        };
        let after = before.clone();
        let summary = HistoricalRollupMaterializationSummary {
            scanned_archive_batches: 2,
            skipped_archive_batches: 1,
            blocked_archive_batches: 1,
            ..HistoricalRollupMaterializationSummary::default()
        };

        let run =
            historical_rollup_startup_backfill_run_state(9, 0, &before, &after, &summary, 2, 2);

        assert_eq!(run.next_cursor_id, 10);
        assert_eq!(run.scanned, 2);
        assert_eq!(run.updated, 0);
        assert!(run.hit_scan_limit);
        assert!(!run.force_idle);
    }

    #[test]
    fn historical_rollup_backfill_run_state_backs_off_after_blocked_cycle_across_multiple_passes() {
        let before = HistoricalRollupBackfillSnapshot {
            pending_buckets: 8,
            legacy_archive_pending: 2,
            pending_usage_breakdown_batches: 2,
            last_materialized_hour: None,
            alert_level: HistoricalRollupBackfillAlertLevel::Critical,
        };
        let after = before.clone();
        let summary = HistoricalRollupMaterializationSummary {
            scanned_archive_batches: 2,
            skipped_archive_batches: 1,
            blocked_archive_batches: 1,
            ..HistoricalRollupMaterializationSummary::default()
        };

        let run =
            historical_rollup_startup_backfill_run_state(9, 1, &before, &after, &summary, 2, 2);

        assert_eq!(run.next_cursor_id, 10);
        assert_eq!(run.scanned, 2);
        assert_eq!(run.updated, 0);
        assert!(!run.hit_scan_limit);
        assert!(run.force_idle);
    }
}
