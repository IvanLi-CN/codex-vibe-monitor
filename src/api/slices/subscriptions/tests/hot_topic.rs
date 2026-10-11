use super::*;
use serde_json::json;
fn dashboard_delivery_reconnect_churn_is_degraded() {
    let counters = DashboardDeliveryTopologyCounters::default();
    counters.record_reconnect_churn("dashboard.working-conversations.current");

    assert!(counters.has_degraded_signal());
}

#[test]
fn dashboard_topic_classification_is_exhaustive_for_live_closed_and_cold_topics() {
    let hot = [
        SubscriptionTopic::DashboardActivityCurrent {
            range: "today".to_string(),
            time_zone: SUBSCRIPTION_DEFAULT_TIME_ZONE.to_string(),
            recent_limit: 16,
            include_accounts: true,
            include_recent: true,
        },
        SubscriptionTopic::DashboardWorkingConversationsCurrent {
            page_size: 16,
            recent_invocation_limit: 16,
            blocked_binding_upstream_account_id: None,
            blocked_binding_constraint_source: None,
        },
        SubscriptionTopic::ParallelWorkCurrent {
            range: "1d".to_string(),
            time_zone: SUBSCRIPTION_DEFAULT_TIME_ZONE.to_string(),
            bucket: Some("1m".to_string()),
            upstream_account_id: None,
        },
        SubscriptionTopic::TimeseriesOpenWindow {
            range: "1d".to_string(),
            time_zone: SUBSCRIPTION_DEFAULT_TIME_ZONE.to_string(),
            bucket: Some("1m".to_string()),
            settlement_hour: None,
            upstream_account_id: None,
        },
    ];
    for topic in hot {
        assert_eq!(topic.class(), SubscriptionTopicClass::HotProjection);
    }

    for topic in [
        SubscriptionTopic::DashboardActivityCurrent {
            range: "yesterday".to_string(),
            time_zone: SUBSCRIPTION_DEFAULT_TIME_ZONE.to_string(),
            recent_limit: 16,
            include_accounts: true,
            include_recent: true,
        },
        SubscriptionTopic::ParallelWorkCurrent {
            range: "yesterday".to_string(),
            time_zone: SUBSCRIPTION_DEFAULT_TIME_ZONE.to_string(),
            bucket: Some("1m".to_string()),
            upstream_account_id: None,
        },
    ] {
        assert_eq!(topic.class(), SubscriptionTopicClass::ClosedSnapshot);
    }

    assert_eq!(
        SubscriptionTopic::InvocationWindow {
            limit: 50,
            model: None,
            status: None,
        }
        .class(),
        SubscriptionTopicClass::BoundedColdHydrate,
    );
}

#[test]
fn dashboard_hot_topic_health_reports_fallback_db_cadence_and_churn() {
    let counters = DashboardDeliveryTopologyCounters::default();
    counters.record_materialization("stats.parallel-work.current", true);
    counters.record_reconnect_churn("dashboard.working-conversations.current");
    let projection = DashboardRuntimeTopologyCounterSnapshot {
        current: DashboardProjectionSliceCounterSnapshot {
            cadence_miss_count: 2,
            ..Default::default()
        },
        ..Default::default()
    };

    let health = counters.hot_topic_health(projection, DashboardHotTopicRecoveryHealth::default());

    assert_eq!(health.state, "degraded");
    assert_eq!(health.activity.cadence_miss_count, 2);
    assert_eq!(health.parallel_work.generic_fallback_build_count, 1);
    assert_eq!(health.parallel_work.live_path_db_read_count, 1);
    assert_eq!(health.parallel_work.state, "degraded");
    assert_eq!(health.working_conversations.reconnect_churn_count, 1);
    assert_eq!(health.working_conversations.state, "degraded");
    assert_eq!(health.timeseries.topic_class, "hot_projection");
}

#[test]
fn dashboard_hot_topic_health_attributes_cadence_misses_to_each_materializer() {
    let counters = DashboardDeliveryTopologyCounters::default();
    counters.record_cadence_miss("dashboard.working-conversations.current");
    counters.record_cadence_miss("stats.parallel-work.current");
    let projection = DashboardRuntimeTopologyCounterSnapshot {
        current: DashboardProjectionSliceCounterSnapshot {
            cadence_miss_count: 2,
            ..Default::default()
        },
        terminal: DashboardProjectionSliceCounterSnapshot {
            cadence_miss_count: 3,
            ..Default::default()
        },
        ..Default::default()
    };

    let health = counters.hot_topic_health(projection, DashboardHotTopicRecoveryHealth::default());

    assert_eq!(health.working_conversations.cadence_miss_count, 1);
    assert_eq!(health.working_conversations.state, "degraded");
    assert_eq!(health.parallel_work.cadence_miss_count, 1);
    assert_eq!(health.parallel_work.state, "degraded");
    assert_eq!(health.timeseries.cadence_miss_count, 5);
    assert_eq!(health.timeseries.state, "degraded");
    assert_eq!(health.state, "degraded");
}

#[test]
fn dashboard_hot_topic_health_attributes_activity_and_summary_slice_dependencies() {
    let counters = DashboardDeliveryTopologyCounters::default();
    let projection = DashboardRuntimeTopologyCounterSnapshot {
        network: DashboardProjectionSliceCounterSnapshot {
            cadence_miss_count: 5,
            ..Default::default()
        },
        terminal: DashboardProjectionSliceCounterSnapshot {
            cadence_miss_count: 7,
            ..Default::default()
        },
        ..Default::default()
    };

    let health = counters.hot_topic_health(projection, DashboardHotTopicRecoveryHealth::default());

    assert_eq!(health.activity.cadence_miss_count, 12);
    assert_eq!(health.activity.state, "degraded");
    assert_eq!(health.summary.cadence_miss_count, 7);
    assert_eq!(health.summary.state, "degraded");
}

#[tokio::test]
async fn dashboard_hot_topic_health_marks_cursor_gap_last_good_as_degraded() {
    let state = crate::tests::test_state_with_openai_base(
        Url::parse("http://127.0.0.1:9").expect("valid test URL"),
    )
    .await;
    let hub = state.subscription_hub.clone();
    let topics = vec![
        SubscriptionTopic::DashboardWorkingConversationsCurrent {
            page_size: 20,
            recent_invocation_limit: 16,
            blocked_binding_upstream_account_id: None,
            blocked_binding_constraint_source: None,
        },
        SubscriptionTopic::ParallelWorkCurrent {
            range: "1d".to_string(),
            time_zone: SUBSCRIPTION_DEFAULT_TIME_ZONE.to_string(),
            bucket: Some("1m".to_string()),
            upstream_account_id: None,
        },
        SubscriptionTopic::TimeseriesOpenWindow {
            range: "1d".to_string(),
            time_zone: SUBSCRIPTION_DEFAULT_TIME_ZONE.to_string(),
            bucket: Some("1m".to_string()),
            settlement_hour: None,
            upstream_account_id: None,
        },
    ];
    {
        let mut guard = hub.state.lock().await;
        for topic in &topics {
            guard.topics.insert(
                topic.cache_key().expect("hot topic key"),
                seeded_cached_topic(topic.clone(), &[7], Utc::now()),
            );
        }
        // Keep the recovery worker parked so the health snapshot observes dirty last-good.
        guard.runtime_topic_recovery_running = true;
    }
    let lease = hub
        .register_topic_subscribers(&topics)
        .await
        .expect("register hot topic owners");

    hub.mark_runtime_mutation_gap_and_recover(state, 4, "cursor_gap")
        .await;
    let health = hub
        .dashboard_hot_topic_health(DashboardRuntimeTopologyCounterSnapshot::default())
        .await;

    assert_eq!(health.working_conversations.state, "degraded");
    assert_eq!(health.parallel_work.state, "degraded");
    assert_eq!(health.timeseries.state, "degraded");
    assert_eq!(health.state, "degraded");
    drop(lease);
}

#[test]
fn hot_fallback_metrics_exclude_closed_dashboard_snapshots() {
    let closed_activity = SubscriptionTopic::DashboardActivityCurrent {
        range: "yesterday".to_string(),
        time_zone: SUBSCRIPTION_DEFAULT_TIME_ZONE.to_string(),
        recent_limit: 16,
        include_accounts: true,
        include_recent: true,
    };
    let closed_summary = SubscriptionTopic::SummaryCurrent {
        window: "yesterday".to_string(),
        time_zone: SUBSCRIPTION_DEFAULT_TIME_ZONE.to_string(),
        limit: None,
        upstream_account_id: None,
    };
    let closed_parallel = SubscriptionTopic::ParallelWorkCurrent {
        range: "yesterday".to_string(),
        time_zone: SUBSCRIPTION_DEFAULT_TIME_ZONE.to_string(),
        bucket: Some("1m".to_string()),
        upstream_account_id: None,
    };
    let closed_timeseries = SubscriptionTopic::TimeseriesOpenWindow {
        range: "yesterday".to_string(),
        time_zone: SUBSCRIPTION_DEFAULT_TIME_ZONE.to_string(),
        bucket: Some("1m".to_string()),
        settlement_hour: None,
        upstream_account_id: None,
    };
    let open_parallel = SubscriptionTopic::ParallelWorkCurrent {
        range: "1d".to_string(),
        time_zone: SUBSCRIPTION_DEFAULT_TIME_ZONE.to_string(),
        bucket: Some("1m".to_string()),
        upstream_account_id: None,
    };
    let open_timeseries = SubscriptionTopic::TimeseriesOpenWindow {
        range: "1d".to_string(),
        time_zone: SUBSCRIPTION_DEFAULT_TIME_ZONE.to_string(),
        bucket: Some("1m".to_string()),
        settlement_hour: None,
        upstream_account_id: None,
    };
    let mutation = RuntimeMutation::invocation(
        &dashboard_runtime_topology_live_record("2026-08-11 12:00:00"),
        RuntimeMutationKind::RuntimeUpsert,
    );

    for topic in [
        &closed_activity,
        &closed_summary,
        &closed_parallel,
        &closed_timeseries,
    ] {
        assert!(!topic.is_unmigrated_dashboard_hot_projection());
    }
    assert!(!open_parallel.is_unmigrated_dashboard_hot_projection());
    assert!(!open_timeseries.is_unmigrated_dashboard_hot_projection());
    assert!(!closed_parallel.is_affected_by_runtime_mutation(&mutation));
    assert!(!closed_timeseries.is_affected_by_runtime_mutation(&mutation));
    assert!(open_parallel.is_affected_by_runtime_mutation(&mutation));
    assert!(open_timeseries.is_affected_by_runtime_mutation(&mutation));
}

#[tokio::test]
async fn inactive_topics_clear_dirty_when_rebuilt_payload_is_unchanged() {
    let state = crate::tests::test_state_with_openai_base(
        Url::parse("http://127.0.0.1:9").expect("valid test URL"),
    )
    .await;
    let hub = SubscriptionHub::new();
    let topic = SubscriptionTopic::AppVersion;
    let descriptor = topic.descriptor();
    let topic_key = topic.cache_key().expect("topic key");

    let initial = hub
        .prepare_connection(state.clone(), vec![descriptor.clone()], Vec::new())
        .await
        .expect("prepare initial app version topic");
    let initial_cursor = initial.outcomes[0].cursor;
    hub.handle_internal_broadcast(
        state.clone(),
        BroadcastPayload::Version {
            version: "next".to_string(),
        },
    )
    .await;
    {
        let guard = hub.state.lock().await;
        assert!(
            guard
                .topics
                .get(&topic_key)
                .is_some_and(|cached| cached.dirty)
        );
    }

    let prepared = hub
        .prepare_connection(
            state,
            vec![descriptor],
            vec![SubscriptionResumeCursor {
                topic_key: topic_key.clone(),
                cursor: initial_cursor,
                schema_epoch: topic.schema_epoch(),
            }],
        )
        .await
        .expect("reconnect should rebuild dirty topic");
    assert!(prepared.initial.is_empty());
    assert_eq!(
        prepared.outcomes[0].disposition,
        TopicInitDisposition::ResumeCaughtUp
    );
    assert_eq!(prepared.outcomes[0].cursor, initial_cursor);
    assert_eq!(prepared.outcomes[0].miss_reason, None);
    let guard = hub.state.lock().await;
    assert!(!guard.topics.get(&topic_key).expect("cached topic").dirty);
    assert!(
        guard
            .topics
            .get(&topic_key)
            .expect("cached topic")
            .replay_events
            .is_empty()
    );
}

#[tokio::test]
async fn releasing_the_last_subscriber_marks_cached_topic_dirty() {
    let hub = SubscriptionHub::new();
    let topic = SubscriptionTopic::InvocationWindow {
        limit: 20,
        model: None,
        status: None,
    };
    let topic_key = topic.cache_key().expect("invocation topic key");
    hub.state.lock().await.topics.insert(
        topic_key.clone(),
        seeded_cached_topic(topic, &[], Utc::now()),
    );
    {
        let mut guard = hub.state.lock().await;
        guard.active_topics.insert(
            topic_key.clone(),
            SubscriptionTopic::InvocationWindow {
                limit: 20,
                model: None,
                status: None,
            },
        );
        guard.active_subscribers.insert(topic_key.clone(), 1);
    }

    hub.release_topic_subscribers(
        vec![topic_key.clone()],
        vec!["invocations.window".to_string()],
        false,
    )
    .await;

    assert!(
        hub.state
            .lock()
            .await
            .topics
            .get(&topic_key)
            .is_some_and(|cached| cached.dirty),
        "a reconnect must rebuild cached data changed while no owner was subscribed"
    );
}

#[tokio::test]
async fn topic_name_dirty_marker_invalidates_cached_quota_snapshot() {
    let hub = SubscriptionHub::new();
    let topic = SubscriptionTopic::QuotaCurrent;
    let topic_key = topic.cache_key().expect("quota topic key");
    hub.state.lock().await.topics.insert(
        topic_key.clone(),
        seeded_cached_topic(topic, &[], Utc::now()),
    );

    hub.mark_topic_name_dirty("quota.current").await;

    let guard = hub.state.lock().await;
    assert!(
        guard
            .topics
            .get(&topic_key)
            .is_some_and(|cached| cached.dirty)
    );
}

#[tokio::test]
async fn summary_live_overlay_updates_only_changed_fields() {
    let hub = Arc::new(SubscriptionHub::new());
    let topic = summary_topic();
    let topic_key = topic.cache_key().expect("topic key");
    let mut cached = seeded_cached_topic(topic.clone(), &[], Utc::now());
    cached.snapshot_payload = json!({
        "inProgressConversationCount": 0,
        "inProgressRetryConversationCount": 0,
        "inProgressAvgWaitMs": null,
        "inProgressPhaseCounts": {"queued": 0, "requesting": 0, "responding": 0}
    });
    hub.state
        .lock()
        .await
        .topics
        .insert(topic_key.clone(), cached);
    let mut receiver = hub.subscribe();
    let live = DashboardActivityLiveSnapshot {
        revision: 1,
        generated_at: "2026-07-24T00:00:00.000Z".to_string(),
        in_progress_invocation_count: 2,
        in_progress_phase_counts: InvocationPhaseCountsResponse {
            queued: 1,
            requesting: 1,
            responding: 0,
        },
        retry_invocation_count: 1,
        in_progress_wait_sum_ms: 80.0,
        in_progress_wait_sample_count: 2,
        network_live_bucket: None,
        network_realtime_rate: None,
        accounts: Vec::new(),
    };

    hub.apply_summary_live_overlay(&topic, live.clone())
        .await
        .expect("apply summary live overlay");
    let dispatch = receiver
        .recv()
        .await
        .expect("summary overlay should dispatch a changed payload");
    assert_eq!(hub.serialization_count(), 1);
    {
        let guard = hub.state.lock().await;
        let cached = guard.topics.get(&topic_key).expect("cached summary topic");
        let replay = cached.replay_events.back().expect("summary replay frame");
        assert!(Arc::ptr_eq(&cached.snapshot_frame, &replay.frame));
        assert!(Arc::ptr_eq(&cached.snapshot_frame, &dispatch.frame));
        assert_eq!(
            cached.snapshot_frame.fingerprint,
            dispatch.frame.fingerprint
        );
    }
    let dispatch_payload = dispatch.frame.payload_value();
    assert_eq!(dispatch_payload["inProgressConversationCount"], json!(2));
    assert_eq!(
        dispatch_payload["inProgressRetryConversationCount"],
        json!(1)
    );
    assert_eq!(dispatch_payload["inProgressAvgWaitMs"], json!(40.0));

    hub.apply_summary_live_overlay(&topic, live)
        .await
        .expect("reapply summary live overlay");
    assert!(
        tokio::time::timeout(Duration::from_millis(20), receiver.recv())
            .await
            .is_err()
    );
    let guard = hub.state.lock().await;
    assert_eq!(
        guard.topics.get(&topic_key).expect("cached topic").cursor,
        1
    );
    assert_eq!(hub.serialization_count(), 1);
}

#[tokio::test]
async fn additional_topic_owners_reuse_the_committed_serialized_frame() {
    let hub = Arc::new(SubscriptionHub::new());
    let topic = summary_topic();
    let topic_key = topic.cache_key().expect("topic key");
    hub.state.lock().await.topics.insert(
        topic_key.clone(),
        seeded_cached_topic(topic.clone(), &[], Utc::now()),
    );

    let first = hub
        .register_topic_subscribers(std::slice::from_ref(&topic))
        .await
        .expect("first owner");
    let second = hub
        .register_topic_subscribers(std::slice::from_ref(&topic))
        .await
        .expect("second owner");

    let guard = hub.state.lock().await;
    assert_eq!(guard.active_subscribers.get(&topic_key), Some(&2));
    assert_eq!(hub.serialization_count(), 0);
    drop(guard);
    drop(first);
    drop(second);
}

#[tokio::test]
async fn closed_summary_topics_do_not_keep_live_snapshot_worker_active() {
    let hub = Arc::new(SubscriptionHub::new());
    let closed_topic = SubscriptionTopic::SummaryCurrent {
        window: "previous7d".to_string(),
        time_zone: "Asia/Shanghai".to_string(),
        limit: None,
        upstream_account_id: None,
    };
    let closed_key = closed_topic.cache_key().expect("closed topic key");
    hub.state.lock().await.topics.insert(
        closed_key,
        seeded_cached_topic(closed_topic, &[], Utc::now()),
    );
    let _closed_lease = hub.register_test_topic_name("stats.summary.current").await;

    assert!(!hub.has_active_dashboard_activity_live_topic().await);
    assert!(!hub.has_active_dashboard_activity_live_topic_sync());

    let open_topic = summary_topic();
    let open_key = open_topic.cache_key().expect("open topic key");
    hub.state
        .lock()
        .await
        .topics
        .insert(open_key, seeded_cached_topic(open_topic, &[], Utc::now()));
    let _open_lease = hub.register_test_topic_name("stats.summary.current").await;

    assert!(hub.has_active_dashboard_activity_live_topic().await);
    assert!(hub.has_active_dashboard_activity_live_topic_sync());
}

#[test]
fn closed_summary_topics_use_calendar_rollover_push_cadence() {
    for window in ["yesterday", "previous7d"] {
        let topic = SubscriptionTopic::SummaryCurrent {
            window: window.to_string(),
            time_zone: SUBSCRIPTION_DEFAULT_TIME_ZONE.to_string(),
            limit: None,
            upstream_account_id: None,
        };

        assert!(topic.uses_server_push_cadence(RuntimeProjectionMode::Auto));
        assert!(subscription_calendar_rollover_delay(&topic) <= Duration::from_secs(24 * 60 * 60));
    }
}

#[test]
fn closed_summary_rollover_uses_next_local_midnight_across_dst() {
    let topic = SubscriptionTopic::SummaryCurrent {
        window: "previous7d".to_string(),
        time_zone: "America/New_York".to_string(),
        limit: None,
        upstream_account_id: None,
    };
    let reporting_tz = parse_reporting_tz(Some("America/New_York")).expect("valid timezone");
    let before_fall_back = Utc
        .with_ymd_and_hms(2026, 11, 1, 5, 30, 0)
        .single()
        .expect("valid UTC instant");

    assert_eq!(
        subscription_calendar_rollover_delay_at(&topic, before_fall_back, reporting_tz),
        Duration::from_secs(23 * 60 * 60 + 30 * 60)
    );
}
