use super::*;

#[tokio::test]
async fn performance_metrics_query_results_and_cancellation_are_counted_once() {
    let runtime = ObservabilityRuntime::new(true);
    with_performance(runtime.clone(), async {
        assert_eq!(
            observe_query(StatsQuery::Summary, async { Ok::<_, ()>(7) }).await,
            Ok(7)
        );
        assert_eq!(
            observe_query(StatsQuery::Summary, async { Err::<(), _>("fixture") }).await,
            Err("fixture")
        );
    })
    .await;
    let mut pending = Box::pin(with_performance(
        runtime.clone(),
        observe_query(
            StatsQuery::Summary,
            std::future::pending::<std::result::Result<(), ()>>(),
        ),
    ));
    assert!(futures_util::FutureExt::now_or_never(pending.as_mut()).is_none());
    drop(pending);
    let text = runtime.render();
    for outcome in ["success", "error", "cancelled"] {
        for family in [
            "cvm_stats_queries_total",
            "cvm_stats_query_duration_seconds_count",
        ] {
            assert!(
                text.lines().any(|line| {
                    line.starts_with(&format!("{family}{{"))
                        && line.contains(&format!("outcome=\"{outcome}\""))
                        && line.contains("query=\"summary\"")
                        && line.ends_with(" 1")
                }),
                "each result must be counted exactly once: {outcome}"
            );
        }
    }
}

#[tokio::test]
async fn performance_metrics_scope_is_instance_local_and_not_implicitly_spawned() {
    assert!(current_performance().is_none());
    let first = ObservabilityRuntime::new(true);
    let second = ObservabilityRuntime::new(true);
    with_performance(first.clone(), async {
        assert!(Arc::ptr_eq(&current_performance().unwrap(), &first));
        with_performance(second.clone(), async {
            assert!(Arc::ptr_eq(&current_performance().unwrap(), &second));
        })
        .await;
        assert!(Arc::ptr_eq(&current_performance().unwrap(), &first));
        assert!(
            tokio::spawn(async { current_performance().is_none() })
                .await
                .unwrap()
        );
    })
    .await;
    assert!(current_performance().is_none());
}

#[test]
fn performance_metrics_fixed_combinations_fit_budget_and_do_not_alias() {
    let runtime = ObservabilityRuntime::new(true);
    let mut counters = HashSet::new();
    let mut histograms = HashSet::new();
    let mut gauges = HashSet::new();
    let mut count = |event: PerformanceCounter| {
        assert!(
            counters.insert(event.index()),
            "counter slots must not alias"
        );
        runtime.count_performance(event, 1);
    };
    let mut observe = |event: PerformanceHistogram| {
        assert!(
            histograms.insert(event.index()),
            "histogram slots must not alias"
        );
        runtime.observe_performance(event, Duration::from_secs(16));
    };
    let mut set = |event: PerformanceGauge| {
        assert!(gauges.insert(event.index()), "gauge slots must not alias");
        runtime.set_performance(event, 1.0);
    };
    assert_eq!(StartupBackfillTask::ordered_tasks().len(), TASK_COUNT);
    for &op in RetentionOperation::ALL {
        for &outcome in Outcome::ALL {
            count(PerformanceCounter::RetentionBatch(op, outcome));
        }
        count(PerformanceCounter::ProcessedRows(op));
        count(PerformanceCounter::ArchivedRows(op));
        for &phase in RetentionPhase::ALL {
            observe(PerformanceHistogram::RetentionPhase(op, phase));
        }
        for &kind in BacklogKind::ALL {
            set(PerformanceGauge::RetentionBacklog(op, kind));
        }
    }
    for &task in StartupBackfillTask::ordered_tasks() {
        for &decision in WakeDecision::ALL {
            count(PerformanceCounter::WakeNotification(task, decision));
        }
        for &outcome in WakeOutcome::ALL {
            count(PerformanceCounter::WakeAttempt(task, outcome));
        }
        for &state in WakeState::ALL {
            set(PerformanceGauge::Wake(task, state));
        }
    }
    for &op in StoreOperation::ALL {
        for &outcome in WakeOutcome::ALL {
            count(PerformanceCounter::StoreWrite(op, outcome));
        }
        observe(PerformanceHistogram::StoreWrite(op));
    }
    for &result in SummaryResult::ALL {
        for &reason in SummaryReason::ALL {
            count(PerformanceCounter::SummaryResponse(result, reason));
        }
        observe(PerformanceHistogram::SummaryResponse(result));
    }
    for &query in StatsQuery::ALL {
        for &outcome in QueryOutcome::ALL {
            count(PerformanceCounter::Query(query, outcome));
            observe(PerformanceHistogram::Query(query, outcome));
        }
    }
    for &projection in StatsProjection::ALL {
        set(PerformanceGauge::ProjectionLastGood(projection));
        for &kind in PendingKind::ALL {
            set(PerformanceGauge::ProjectionPending(projection, kind));
        }
    }
    assert_eq!(counters.len(), COUNTERS);
    assert_eq!(histograms.len(), HISTOGRAMS);
    assert_eq!(gauges.len(), GAUGES);
    assert_eq!(runtime.series.dropped(), 0);
    let text = runtime.render();
    let series = text
        .lines()
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .count();
    assert_eq!(
        series,
        COUNTERS + GAUGES + HISTOGRAMS * (MAINTENANCE_BUCKETS.len() + 3)
    );
    assert!(
        series < 3996,
        "new fixed combinations share the existing application budget"
    );
    assert!(text.contains("le=\"0.2\""));
    assert!(text.contains("le=\"0.25\""));
    assert!(text.contains("le=\"300\""));
}

#[test]
fn performance_metrics_preserve_results_and_unknown_until_observed() {
    let runtime = ObservabilityRuntime::new(true);
    assert!(!runtime.render().contains("cvm_summary_responses_total"));
    let result =
        PerformanceCounter::SummaryResponse(SummaryResult::Unavailable, SummaryReason::Hydration);
    runtime.count_performance(result, 1);
    runtime.count_performance(result, 2);
    runtime.observe_performance(
        PerformanceHistogram::SummaryResponse(SummaryResult::Unavailable),
        Duration::from_millis(2),
    );
    runtime.observe_performance(
        PerformanceHistogram::SummaryResponse(SummaryResult::Success),
        Duration::from_secs(12),
    );
    let text = runtime.render();
    assert!(
        text.contains(
            "cvm_summary_responses_total{outcome=\"unavailable\",reason=\"hydration\"} 3"
        )
    );
    assert!(text.contains("cvm_summary_response_duration_seconds_sum{outcome=\"success\"} 12"));
    assert!(!text.contains("cvm_maintenance_archived_rows_total"));
    assert!(!text.contains("cvm_stats_projection_pending_items"));
    let disabled = ObservabilityRuntime::new(false);
    disabled.count_performance(result, 1);
    disabled.observe_performance(
        PerformanceHistogram::SummaryResponse(SummaryResult::Success),
        Duration::from_secs(1),
    );
    disabled.set_performance(
        PerformanceGauge::ProjectionLastGood(StatsProjection::SummaryRecent),
        123.0,
    );
    assert!(!disabled.render().contains("cvm_"));
}

#[test]
fn performance_metrics_long_tail_buckets_use_actual_series_weight() {
    let runtime = ObservabilityRuntime::new(true);
    for i in 0..200 {
        let key = Key::from_parts(
            "cvm_stats_query_duration_seconds",
            vec![Label::new("synthetic", i.to_string())],
        );
        runtime.register_histogram(key).record(250.0);
    }
    assert!(runtime.series.dropped() > 0);
    let text = runtime.render();
    assert!(
        text.lines()
            .filter(|line| !line.is_empty() && !line.starts_with('#'))
            .count()
            <= 4000
    );
    assert_eq!(runtime.state(), "degraded");
}
