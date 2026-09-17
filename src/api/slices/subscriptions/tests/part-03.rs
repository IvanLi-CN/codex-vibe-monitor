
fn terminal_for_activity_materialization(occurred_at: &str) -> ApiInvocation {
    let mut terminal = dashboard_runtime_topology_live_record(occurred_at);
    terminal.invoke_id = "dashboard-runtime-terminal-materialization".to_string();
    terminal.status = Some("success".to_string());
    terminal.live_phase = None;
    terminal.total_tokens = Some(42);
    terminal.output_tokens = Some(16);
    terminal.cost = Some(0.25);
    terminal.prompt_cache_key = Some("terminal-materialized-recent".to_string());
    terminal.request_model = Some("gpt-5".to_string());
    terminal.response_model = Some("gpt-5".to_string());
    terminal.t_total_ms = Some(3_450.0);
    terminal.t_req_read_ms = Some(10.0);
    terminal.t_req_parse_ms = Some(12.0);
    terminal.t_upstream_connect_ms = Some(18.0);
    terminal.t_upstream_ttfb_ms = Some(508.0);
    terminal.first_token_ms = Some(650.0);
    terminal.t_upstream_stream_ms = Some(2_800.0);
    terminal
}

async fn assert_materialized_activity_payload(
    state: &Arc<AppState>,
    activity_key: &str,
    summary_key: &str,
    activity_cursor_before: u64,
    summary_cursor_before: u64,
) {
    let guard = state.subscription_hub.state.lock().await;
    let activity_cached = &guard.topics[activity_key];
    let summary_cached = &guard.topics[summary_key];
    assert_eq!(activity_cached.cursor, activity_cursor_before + 1);
    assert_eq!(summary_cached.cursor, summary_cursor_before);
    let activity = activity_cached.snapshot_frame.payload_value();
    assert_eq!(activity["summary"]["stats"]["totalCount"], json!(1));
    assert_eq!(activity["accounts"][0]["upstreamAccountId"], json!(42));
    assert_eq!(activity["accounts"][0]["totalTokens"], json!(42));
    assert_eq!(
        activity["summary"]["modelPerformance"]["total"]["cumulativeUsageDurationMs"],
        json!(3_450.0)
    );
    assert_eq!(activity["accounts"][0]["firstTokenAvgMs"], json!(650.0));
    assert_eq!(activity["accounts"][0]["avgTotalMs"], json!(3_450.0));
    assert_eq!(
        activity["accounts"][0]["modelPerformance"]["models"][0]["model"],
        json!("gpt-5")
    );
    assert_eq!(
        activity["accounts"][0]["recentInvocations"][0]["invokeId"],
        json!("dashboard-runtime-terminal-materialization")
    );
    assert_eq!(
        summary_cached.snapshot_frame.payload_value()["totalCount"],
        json!(0)
    );
}

async fn assert_unchanged_terminal_revision(
    state: &Arc<AppState>,
    activity_key: &str,
    summary_key: &str,
    activity_cursor_before: u64,
    summary_cursor_before: u64,
) {
    let guard = state.subscription_hub.state.lock().await;
    assert_eq!(
        guard.topics[activity_key].cursor,
        activity_cursor_before + 1,
        "an unchanged terminal revision must not advance the activity cursor",
    );
    assert_eq!(
        guard.topics[summary_key].cursor, summary_cursor_before,
        "an unacknowledged terminal slice must not advance the summary cursor",
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

#[tokio::test]
async fn network_timeseries_topics_keep_live_snapshot_worker_active() {
    let hub = Arc::new(SubscriptionHub::new());
    let topic = SubscriptionTopic::DashboardNetworkTimeseriesWindow {
        range: "today".to_string(),
        time_zone: SUBSCRIPTION_DEFAULT_TIME_ZONE.to_string(),
        upstream_account_id: None,
    };
    let topic_key = topic.cache_key().expect("network topic key");
    hub.state
        .lock()
        .await
        .topics
        .insert(topic_key, seeded_cached_topic(topic, &[], Utc::now()));
    let _lease = hub
        .register_test_topic_name("dashboard.network-timeseries.window")
        .await;

    assert!(hub.has_active_dashboard_activity_live_topic().await);
    assert!(hub.has_active_dashboard_activity_live_topic_sync());
}

#[tokio::test]
async fn cold_network_only_slice_materializes_without_sqlite() {
    let state = crate::tests::test_state_with_openai_base(
        Url::parse("http://127.0.0.1:9").expect("valid test URL"),
    )
    .await;
    state
        .proxy_runtime_invocations
        .bind_dashboard_network_speed_cache(state.dashboard_network_speed_cache.clone())
        .expect("bind network cache");
    let hub = state.subscription_hub.clone();
    let topic = SubscriptionTopic::DashboardNetworkTimeseriesWindow {
        range: "today".to_string(),
        time_zone: SUBSCRIPTION_DEFAULT_TIME_ZONE.to_string(),
        upstream_account_id: None,
    };
    let topic_key = topic.cache_key().expect("network topic key");
    hub.prepare_connection(state.clone(), vec![topic.descriptor()], Vec::new())
        .await
        .expect("prepare network topic");
    let _lease = hub
        .register_test_topic_name("dashboard.network-timeseries.window")
        .await;
    let cursor_before = hub
        .state
        .lock()
        .await
        .topics
        .get(&topic_key)
        .expect("cached network topic")
        .cursor;
    state.dashboard_network_speed_cache.record_request_bytes(
        "cold-network-only",
        &crate::proxy::shanghai_now_string(),
        None,
        Some("api.openai.com"),
        512,
        Utc::now(),
    );
    state.dashboard_network_speed_cache.record_request_bytes(
        "cold-network-account",
        &crate::proxy::shanghai_now_string(),
        Some(42),
        Some("api.openai.com"),
        256,
        Utc::now(),
    );
    let slice = state
        .proxy_runtime_invocations
        .capture_network_slice()
        .expect("capture network slice")
        .slice;
    state.pool.close().await;

    hub.handle_internal_broadcast(
        state.clone(),
        BroadcastPayload::DashboardNetworkSlice {
            slice: Box::new(slice.clone()),
        },
    )
    .await;

    let cursor_after = hub
        .state
        .lock()
        .await
        .topics
        .get(&topic_key)
        .expect("cached network topic")
        .cursor;
    assert_eq!(cursor_after, cursor_before + 1);
    let guard = hub.state.lock().await;
    let payload = guard
        .topics
        .get(&topic_key)
        .map(|cached| cached.snapshot_frame.payload_value())
        .expect("cached network frame");
    let live_point = payload
        .get("points")
        .and_then(Value::as_array)
        .and_then(|points| {
            points.iter().find(|point| {
                point
                    .get("isLiveBucket")
                    .and_then(Value::as_bool)
                    .unwrap_or(false)
            })
        })
        .expect("global live point");
    assert_eq!(
        live_point.get("uploadBytes").and_then(Value::as_i64),
        Some(768)
    );
    drop(guard);

    assert_network_slice_reuse(&hub, state, slice, &topic_key, cursor_after).await;
}

async fn assert_network_slice_reuse(
    hub: &SubscriptionHub,
    state: Arc<AppState>,
    slice: DashboardNetworkProjectionSlice,
    topic_key: &str,
    cursor_after: u64,
) {
    let delivery_before = hub.dashboard_topology_counters();
    hub.handle_internal_broadcast(
        state,
        BroadcastPayload::DashboardNetworkSlice {
            slice: Box::new(slice),
        },
    )
    .await;
    let delivery_after = hub.dashboard_topology_counters();
    let cursor_after_unchanged = hub
        .state
        .lock()
        .await
        .topics
        .get(topic_key)
        .expect("cached network topic")
        .cursor;
    assert_eq!(cursor_after_unchanged, cursor_after);
    assert_eq!(
        delivery_after.network_timeseries.materialization_count,
        delivery_before.network_timeseries.materialization_count,
    );
    assert_eq!(
        delivery_after.network_timeseries.serialization_count,
        delivery_before.network_timeseries.serialization_count,
    );
    assert_eq!(delivery_after.network_timeseries.payload_clone_count, 0);
}

#[tokio::test]
async fn terminal_slice_materializes_activity_without_speculative_summary() {
    let state = crate::tests::test_state_with_openai_base(
        Url::parse("http://127.0.0.1:9").expect("valid test URL"),
    )
    .await;
    hydrate_summary_snapshots(state.as_ref())
        .await
        .expect("hydrate Summary baseline before closing SQLite");
    let activity = SubscriptionTopic::DashboardActivityCurrent {
        range: "today".to_string(),
        time_zone: SUBSCRIPTION_DEFAULT_TIME_ZONE.to_string(),
        recent_limit: 16,
        include_accounts: true,
        include_recent: true,
    };
    let summary = SubscriptionTopic::SummaryCurrent {
        window: "today".to_string(),
        time_zone: SUBSCRIPTION_DEFAULT_TIME_ZONE.to_string(),
        limit: None,
        upstream_account_id: None,
    };
    let topics = vec![activity.clone(), summary.clone()];
    let _lease = state
        .subscription_hub
        .register_topic_subscribers(&topics)
        .await
        .expect("register active Dashboard topics");
    state
        .subscription_hub
        .prepare_connection(
            state.clone(),
            topics.iter().map(SubscriptionTopic::descriptor).collect(),
            Vec::new(),
        )
        .await
        .expect("prepare Dashboard topic bases");

    let activity_key = activity.cache_key().expect("activity topic key");
    let summary_key = summary.cache_key().expect("summary topic key");
    let (activity_cursor_before, summary_cursor_before) = {
        let guard = state.subscription_hub.state.lock().await;
        (
            guard.topics[&activity_key].cursor,
            guard.topics[&summary_key].cursor,
        )
    };
    let occurred_at = format_naive(Utc::now().with_timezone(&Shanghai).naive_local());
    let terminal = terminal_for_activity_materialization(&occurred_at);
    let outcome = apply_dashboard_activity_terminal_record(state.as_ref(), &terminal).await;
    let delta = outcome.terminal_delta.expect("accepted terminal delta");
    state.pool.close().await;

    state
        .subscription_hub
        .handle_internal_broadcast(
            state.clone(),
            BroadcastPayload::DashboardTerminalSlice {
                slice: Box::new(DashboardTerminalProjectionSlice {
                    revision: 1,
                    deltas: vec![delta],
                }),
            },
        )
        .await;

    assert_materialized_activity_payload(
        &state,
        &activity_key,
        &summary_key,
        activity_cursor_before,
        summary_cursor_before,
    )
    .await;

    state
        .subscription_hub
        .handle_internal_broadcast(
            state.clone(),
            BroadcastPayload::DashboardTerminalSlice {
                slice: Box::new(DashboardTerminalProjectionSlice {
                    revision: 1,
                    deltas: Vec::new(),
                }),
            },
        )
        .await;
    assert_unchanged_terminal_revision(
        &state,
        &activity_key,
        &summary_key,
        activity_cursor_before,
        summary_cursor_before,
    )
    .await;
}

#[tokio::test]
async fn activity_materializer_skips_terminal_delta_already_folded_into_base() {
    let state = crate::tests::test_state_with_openai_base(
        Url::parse("http://127.0.0.1:9").expect("valid test URL"),
    )
    .await;
    let activity = SubscriptionTopic::DashboardActivityCurrent {
        range: "today".to_string(),
        time_zone: SUBSCRIPTION_DEFAULT_TIME_ZONE.to_string(),
        recent_limit: 16,
        include_accounts: true,
        include_recent: true,
    };
    let occurred_at = format_naive(Utc::now().with_timezone(&Shanghai).naive_local());
    let mut terminal = dashboard_runtime_topology_live_record(&occurred_at);
    terminal.invoke_id = "dashboard-runtime-folded-terminal".to_string();
    terminal.status = Some("success".to_string());
    terminal.live_phase = None;
    terminal.total_tokens = Some(42);
    terminal.output_tokens = Some(16);
    terminal.cost = Some(0.25);
    let outcome = apply_dashboard_activity_terminal_record(state.as_ref(), &terminal).await;
    let delta = outcome.terminal_delta.expect("accepted terminal delta");

    let payload = activity
        .build_cached_payload(state.clone())
        .await
        .expect("build typed activity base")
        .serialize(
            None,
            None,
            Some(&DashboardTerminalProjectionSlice {
                revision: 1,
                deltas: vec![delta],
            }),
        )
        .expect("serialize folded activity base");
    let payload: Value = serde_json::from_slice(&payload).expect("activity payload JSON");
    assert_eq!(payload["summary"]["stats"]["totalCount"], json!(1));
    assert_eq!(payload["summary"]["stats"]["totalTokens"], json!(42));
}

#[tokio::test]
async fn activity_materializer_skips_terminal_delta_already_persisted_in_base() {
    let state = crate::tests::test_state_with_openai_base(
        Url::parse("http://127.0.0.1:9").expect("valid test URL"),
    )
    .await;
    let occurred_at = format_naive(Utc::now().with_timezone(&Shanghai).naive_local());
    let mut terminal = dashboard_runtime_topology_live_record(&occurred_at);
    terminal.id = 0;
    terminal.invoke_id = "dashboard-runtime-persisted-activity-terminal".to_string();
    terminal.status = Some("success".to_string());
    terminal.live_phase = None;
    terminal.total_tokens = Some(42);
    terminal.output_tokens = Some(16);
    terminal.cost = Some(0.25);
    let delta = apply_dashboard_activity_terminal_record(state.as_ref(), &terminal)
        .await
        .terminal_delta
        .expect("accepted pending terminal delta");

    terminal.id = 748_004;
    sqlx::query(
        r#"
            INSERT INTO codex_invocations (
                id, invoke_id, occurred_at, source, model, total_tokens, output_tokens, cost,
                status, payload, raw_response
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, '{}', '{}')
            "#,
    )
    .bind(terminal.id)
    .bind(&terminal.invoke_id)
    .bind(&terminal.occurred_at)
    .bind(&terminal.source)
    .bind(terminal.model.as_deref())
    .bind(terminal.total_tokens)
    .bind(terminal.output_tokens)
    .bind(terminal.cost)
    .bind(terminal.status.as_deref())
    .execute(&state.pool)
    .await
    .expect("persist terminal baseline row");

    let base = build_dashboard_activity_topic_materialized_base(
        state.as_ref(),
        &DashboardActivityQuery {
            range: "today".to_string(),
            recent_limit: Some(16),
            time_zone: Some(SUBSCRIPTION_DEFAULT_TIME_ZONE.to_string()),
            include_accounts: true,
            include_recent: Some(true),
        },
    )
    .await
    .expect("build typed activity base");
    assert_eq!(base.response().terminal_sequence, delta.terminal_sequence);
    let payload = DashboardTopicMaterializer::Activity {
        base: Arc::new(StdMutex::new(DashboardActivityMaterializerState::new(base))),
        reporting_tz: Shanghai,
        source_scope: InvocationSourceScope::ProxyOnly,
    }
    .serialize(
        None,
        None,
        Some(&DashboardTerminalProjectionSlice {
            revision: 1,
            deltas: vec![delta],
        }),
    )
    .expect("serialize persisted activity base");
    let payload: Value = serde_json::from_slice(&payload).expect("activity payload JSON");
    assert_eq!(payload["summary"]["stats"]["totalCount"], json!(1));
    assert_eq!(payload["summary"]["stats"]["totalTokens"], json!(42));
}

#[tokio::test]
async fn activity_materializer_emits_routing_only_revision_without_live_slice() {
    let state = crate::tests::test_state_with_openai_base(
        Url::parse("http://127.0.0.1:9").expect("valid test URL"),
    )
    .await;
    let base = build_dashboard_activity_topic_materialized_base(
        state.as_ref(),
        &DashboardActivityQuery {
            range: "today".to_string(),
            recent_limit: Some(16),
            time_zone: Some(SUBSCRIPTION_DEFAULT_TIME_ZONE.to_string()),
            include_accounts: true,
            include_recent: Some(true),
        },
    )
    .await
    .expect("build typed activity base");
    let materializer = DashboardTopicMaterializer::Activity {
        base: Arc::new(StdMutex::new(DashboardActivityMaterializerState::new(base))),
        reporting_tz: Shanghai,
        source_scope: InvocationSourceScope::All,
    };
    let DashboardTopicMaterializer::Activity { base, .. } = &materializer else {
        unreachable!("constructed activity materializer");
    };
    base.lock()
        .expect("activity materializer state lock")
        .routing_revision = 1;

    assert_eq!(
        materializer
            .revision(0, None, None, None)
            .expect("routing-only revision")
            .routing_revision,
        1
    );
}

#[tokio::test]
async fn activity_materializer_handles_maximum_distinct_account_terminal_slice() {
    let delta_state = crate::tests::test_state_with_openai_base(
        Url::parse("http://127.0.0.1:9").expect("valid test URL"),
    )
    .await;
    let base_state = crate::tests::test_state_with_openai_base(
        Url::parse("http://127.0.0.1:9").expect("valid test URL"),
    )
    .await;
    let mut base = build_dashboard_activity_topic_materialized_base(
        base_state.as_ref(),
        &DashboardActivityQuery {
            range: "today".to_string(),
            recent_limit: Some(16),
            time_zone: Some(SUBSCRIPTION_DEFAULT_TIME_ZONE.to_string()),
            include_accounts: true,
            include_recent: Some(false),
        },
    )
    .await
    .expect("build typed activity base");
    let occurred_at = format_naive(Utc::now().with_timezone(&Shanghai).naive_local());
    let mut deltas = Vec::with_capacity(10_000);
    for account_id in 1..=10_000_i64 {
        let mut terminal = dashboard_runtime_topology_live_record(&occurred_at);
        terminal.id = 0;
        terminal.invoke_id = format!("dashboard-runtime-terminal-account-{account_id}");
        terminal.upstream_account_id = Some(account_id);
        terminal.upstream_account_name = Some(format!("Account {account_id}"));
        terminal.status = Some("success".to_string());
        terminal.live_phase = None;
        terminal.total_tokens = Some(1);
        terminal.output_tokens = Some(1);
        deltas.push(
            apply_dashboard_activity_terminal_record(delta_state.as_ref(), &terminal)
                .await
                .terminal_delta
                .expect("accepted distinct terminal delta"),
        );
    }

    base.apply_terminal_slice(
        Shanghai,
        InvocationSourceScope::ProxyOnly,
        &DashboardTerminalProjectionSlice {
            revision: 1,
            deltas,
        },
    );

    assert_eq!(base.response().summary.stats.total_count, 10_000);
    assert_eq!(
        base.response().accounts.as_ref().map(Vec::len),
        Some(10_000),
        "each distinct account should be materialized once",
    );
}

#[tokio::test]
async fn terminal_materializers_detect_stale_moving_window_bases() {
    let state = crate::tests::test_state_with_openai_base(
        Url::parse("http://127.0.0.1:9").expect("valid test URL"),
    )
    .await;
    hydrate_summary_snapshots(state.as_ref())
        .await
        .expect("hydrate Summary baseline before materializer construction");
    let activity = SubscriptionTopic::DashboardActivityCurrent {
        range: "today".to_string(),
        time_zone: SUBSCRIPTION_DEFAULT_TIME_ZONE.to_string(),
        recent_limit: 16,
        include_accounts: true,
        include_recent: true,
    };
    let summary = SubscriptionTopic::SummaryCurrent {
        window: "1d".to_string(),
        time_zone: SUBSCRIPTION_DEFAULT_TIME_ZONE.to_string(),
        limit: None,
        upstream_account_id: None,
    };
    let timeseries = SubscriptionTopic::TimeseriesOpenWindow {
        range: "1d".to_string(),
        time_zone: SUBSCRIPTION_DEFAULT_TIME_ZONE.to_string(),
        bucket: Some("1m".to_string()),
        settlement_hour: None,
        upstream_account_id: None,
    };
    let activity_materializer = activity
        .build_cached_payload(state.clone())
        .await
        .expect("build activity base")
        .dashboard_materializer()
        .expect("activity typed materializer");
    let summary_materializer = summary
        .build_cached_payload(state.clone())
        .await
        .expect("build summary base")
        .dashboard_materializer()
        .expect("summary typed materializer");
    let timeseries_materializer = timeseries
        .build_cached_payload(state)
        .await
        .expect("build timeseries base")
        .dashboard_materializer()
        .expect("timeseries typed materializer");

    assert!(
        !activity_materializer.requires_terminal_window_rebase(),
        "a freshly built activity base must remain on the terminal delivery path",
    );
    assert!(
        !summary_materializer.requires_terminal_window_rebase(),
        "a freshly built duration base must wait for the reconcile boundary",
    );
    assert!(
        !timeseries_materializer.requires_terminal_window_rebase(),
        "a freshly built timeseries base must wait for the reconcile boundary",
    );

    let DashboardTopicMaterializer::Activity { base, .. } = &activity_materializer else {
        panic!("expected activity materializer");
    };
    base.lock()
        .expect("activity materializer state lock")
        .rebase_range_start = Some(Utc::now() - ChronoDuration::days(1));
    let DashboardTopicMaterializer::Summary { base, .. } = &summary_materializer else {
        panic!("expected summary materializer");
    };
    base.lock()
        .expect("summary materializer state lock")
        .range_start = Some(Utc::now() - ChronoDuration::days(2));
    let DashboardTopicMaterializer::Timeseries { base, .. } = &timeseries_materializer else {
        panic!("expected timeseries materializer");
    };
    base.lock()
        .expect("timeseries materializer state lock")
        .set_range_start_for_test(
            Utc::now()
                - ChronoDuration::days(1)
                - ChronoDuration::seconds(DASHBOARD_ACTIVITY_SNAPSHOT_CACHE_TTL_SECS as i64),
        );

    assert!(activity_materializer.requires_terminal_window_rebase());
    assert!(summary_materializer.requires_terminal_window_rebase());
    assert!(timeseries_materializer.requires_terminal_window_rebase());
}

#[tokio::test]
async fn hydrated_summary_topics_materialize_without_sqlite_for_open_and_closed_windows() {
    let state = crate::tests::test_state_with_openai_base(
        Url::parse("http://127.0.0.1:9").expect("valid test URL"),
    )
    .await;
    hydrate_summary_snapshots(state.as_ref())
        .await
        .expect("hydrate summary projection before topic materialization");
    state.pool.close().await;

    for window in ["1d", "previous7d"] {
        let summary = SubscriptionTopic::SummaryCurrent {
            window: window.to_string(),
            time_zone: SUBSCRIPTION_DEFAULT_TIME_ZONE.to_string(),
            limit: None,
            upstream_account_id: None,
        };
        let payload = summary
            .build_cached_payload(state.clone())
            .await
            .expect("hydrated Summary topic must not rebuild from SQLite");
        assert!(
            matches!(
                payload,
                BuiltSubscriptionTopicPayload::Dashboard(
                    DashboardTopicMaterializer::Summary { .. }
                )
            ),
            "{window} must use the memory-backed Summary materializer"
        );
    }
}

#[tokio::test]
async fn hydrated_summary_topic_does_not_replay_unacknowledged_terminal_without_sqlite() {
    let state = crate::tests::test_state_with_openai_base(
        Url::parse("http://127.0.0.1:9").expect("valid test URL"),
    )
    .await;
    hydrate_summary_snapshots(state.as_ref())
        .await
        .expect("hydrate summary projection before the terminal delta");
    let occurred_at = format_naive(Utc::now().with_timezone(&Shanghai).naive_local());
    let mut terminal = dashboard_runtime_topology_live_record(&occurred_at);
    terminal.id = 0;
    terminal.invoke_id = "hydrated-summary-pending-terminal".to_string();
    terminal.status = Some("success".to_string());
    terminal.live_phase = None;
    terminal.total_tokens = Some(42);
    terminal.output_tokens = Some(16);
    terminal.cost = Some(0.25);
    let delta = apply_dashboard_activity_terminal_record(state.as_ref(), &terminal)
        .await
        .terminal_delta
        .expect("accept pending terminal delta");
    state.pool.close().await;

    let summary = SubscriptionTopic::SummaryCurrent {
        window: "today".to_string(),
        time_zone: SUBSCRIPTION_DEFAULT_TIME_ZONE.to_string(),
        limit: None,
        upstream_account_id: None,
    };
    let payload = summary
        .build_cached_payload(state.clone())
        .await
        .expect("hydrated Summary topic must keep the pre-commit baseline in memory")
        .serialize(
            None,
            None,
            Some(&DashboardTerminalProjectionSlice {
                revision: 1,
                deltas: vec![delta],
            }),
        )
        .expect("serialize the memory-backed Summary topic");
    let payload: Value = serde_json::from_slice(&payload).expect("summary payload JSON");
    assert_eq!(payload["totalCount"], json!(0));
    assert_eq!(payload["totalTokens"], json!(0));
    assert_eq!(payload["totalCost"], json!(0.0));
}

#[tokio::test]
async fn cold_summary_topic_materialization_is_unavailable_without_sqlite() {
    let state = crate::tests::test_state_with_openai_base(
        Url::parse("http://127.0.0.1:9").expect("valid test URL"),
    )
    .await;
    let summary = SubscriptionTopic::SummaryCurrent {
        window: "1d".to_string(),
        time_zone: SUBSCRIPTION_DEFAULT_TIME_ZONE.to_string(),
        limit: None,
        upstream_account_id: None,
    };

    state.pool.close().await;

    assert!(matches!(
        summary.build_cached_payload(state).await,
        Err(ApiError::Unavailable(_))
    ));
}

#[tokio::test]
async fn hydrated_summary_topic_replays_acked_terminal_until_projection_catches_up() {
    let state = crate::tests::test_state_with_openai_base(
        Url::parse("http://127.0.0.1:9").expect("valid test URL"),
    )
    .await;
    hydrate_summary_snapshots(state.as_ref())
        .await
        .expect("hydrate summary projection before the terminal write");
    let occurred_at = format_naive(Utc::now().with_timezone(&Shanghai).naive_local());
    let mut terminal = dashboard_runtime_topology_live_record(&occurred_at);
    terminal.id = 842_001;
    terminal.invoke_id = "hydrated-summary-acked-terminal".to_string();
    terminal.status = Some("success".to_string());
    terminal.live_phase = None;
    terminal.total_tokens = Some(42);
    terminal.output_tokens = Some(16);
    terminal.cost = Some(0.25);
    sqlx::query(
        r#"
            INSERT INTO codex_invocations (
                id, invoke_id, occurred_at, source, status, total_tokens, output_tokens, cost,
                payload, raw_response
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
            "#,
    )
    .bind(terminal.id)
    .bind(terminal.invoke_id.as_str())
    .bind(terminal.occurred_at.as_str())
    .bind(terminal.source.as_str())
    .bind("success")
    .bind(terminal.total_tokens)
    .bind(terminal.output_tokens)
    .bind(terminal.cost)
    .bind(json!({ "upstreamAccountId": terminal.upstream_account_id }).to_string())
    .bind("{}")
    .execute(&state.pool)
    .await
    .expect("persist terminal after the previous projection revision");
    let delta = apply_dashboard_activity_terminal_record(state.as_ref(), &terminal)
        .await
        .terminal_delta
        .expect("accept persisted terminal delta");
    assert_eq!(delta.persisted_row_id, Some(terminal.id));
    state
        .proxy_runtime_invocations
        .record_dashboard_terminal_delta(delta);
    let capture = state
        .proxy_runtime_invocations
        .capture_terminal_slice()
        .expect("capture persisted terminal overlay");
    let slice = DashboardTerminalProjectionSlice {
        revision: capture.revision,
        deltas: capture.deltas.clone(),
    };
    state
        .subscription_hub
        .materialize_dashboard_terminal_slice(DashboardTerminalProjectionSlice {
            revision: slice.revision,
            deltas: slice.deltas.clone(),
        })
        .await;
    state.pool.close().await;

    let summary = SubscriptionTopic::SummaryCurrent {
        window: "today".to_string(),
        time_zone: SUBSCRIPTION_DEFAULT_TIME_ZONE.to_string(),
        limit: None,
        upstream_account_id: None,
    };
    let payload = summary
        .build_cached_payload(state.clone())
        .await
        .expect("acked terminal remains in the memory-only Summary overlay")
        .serialize(
            None,
            None,
            Some(&DashboardTerminalProjectionSlice {
                revision: capture.revision,
                deltas: capture.deltas,
            }),
        )
        .expect("serialize the memory-backed Summary topic");
    let payload: Value = serde_json::from_slice(&payload).expect("summary payload JSON");
    assert_eq!(payload["totalCount"], json!(1));
    assert_eq!(payload["totalTokens"], json!(42));
    assert_eq!(payload["totalCost"], json!(0.25));
}

#[tokio::test]
async fn summary_current_snapshot_keeps_overlay_across_projection_refresh_swap() {
    let state = crate::tests::test_state_with_openai_base(
        Url::parse("http://127.0.0.1:9").expect("valid test URL"),
    )
    .await;
    hydrate_summary_snapshots(state.as_ref())
        .await
        .expect("publish initial Summary projection");
    let occurred_at = format_naive(Utc::now().with_timezone(&Shanghai).naive_local());
    let mut terminal = dashboard_runtime_topology_live_record(&occurred_at);
    terminal.id = 842_101;
    terminal.invoke_id = "summary-current-refresh-swap-terminal".to_string();
    terminal.status = Some("success".to_string());
    terminal.live_phase = None;
    terminal.total_tokens = Some(42);
    terminal.output_tokens = Some(16);
    terminal.cost = Some(0.25);
    sqlx::query(
        r#"
            INSERT INTO codex_invocations (
                id, invoke_id, occurred_at, source, status, total_tokens, output_tokens, cost,
                payload, raw_response
            ) VALUES (?1, ?2, ?3, ?4, 'success', ?5, ?6, ?7, ?8, '{}')
            "#,
    )
    .bind(terminal.id)
    .bind(terminal.invoke_id.as_str())
    .bind(terminal.occurred_at.as_str())
    .bind(terminal.source.as_str())
    .bind(terminal.total_tokens)
    .bind(terminal.output_tokens)
    .bind(terminal.cost)
    .bind(json!({ "upstreamAccountId": terminal.upstream_account_id }).to_string())
    .execute(&state.pool)
    .await
    .expect("persist terminal after the initial projection");
    let delta = apply_dashboard_activity_terminal_record(state.as_ref(), &terminal)
        .await
        .terminal_delta
        .expect("accept persisted terminal delta");
    state
        .subscription_hub
        .materialize_dashboard_terminal_slice(DashboardTerminalProjectionSlice {
            revision: 1,
            deltas: vec![delta],
        })
        .await;

    // This is the former split-read interleaving: capture the old projection and its overlay,
    // then let refresh publish the new projection that consumes and clears that overlay.
    let (projection, pending_terminal_deltas, initial_terminal_slice_suppressions, _gaps) = state
        .subscription_hub
        .summary_projection_with_terminal_overlay(false, None)
        .await
        .expect("capture coherent Summary snapshot")
        .expect("initial Summary projection is available");
    assert_eq!(pending_terminal_deltas.len(), 1);

    hydrate_summary_snapshots(state.as_ref())
        .await
        .expect("refresh projection with the durable terminal");
    assert!(
        state
            .subscription_hub
            .state
            .lock()
            .await
            .summary_delta_journal
            .entries
            .is_empty(),
        "the refreshed projection must consume the prior terminal overlay"
    );
    state.pool.close().await;

    assert_summary_snapshot_overlay(
        projection,
        pending_terminal_deltas,
        initial_terminal_slice_suppressions,
        state.config.list_limit_max,
    );
}

fn assert_summary_snapshot_overlay(
    projection: Arc<SummaryProjection>,
    pending_terminal_deltas: Vec<DashboardActivityTerminalDelta>,
    initial_terminal_slice_suppressions: HashSet<u64>,
    list_limit_max: usize,
) {
    let query = SummaryQuery {
        window: Some("today".to_string()),
        limit: None,
        time_zone: Some(SUBSCRIPTION_DEFAULT_TIME_ZONE.to_string()),
        upstream_account_id: None,
    };
    let mut response = projection
        .response_for_query(&query, list_limit_max as i64)
        .expect("old projection remains a valid immutable base");
    let summary_window =
        parse_summary_window(&query, list_limit_max as i64).expect("parse Summary window");
    let mut replayed_terminal_sequence = 0;
    apply_dashboard_terminal_slice_to_summary_response(
        &mut response,
        &mut replayed_terminal_sequence,
        &summary_window,
        Shanghai,
        InvocationSourceScope::All,
        None,
        &DashboardTerminalProjectionSlice {
            revision: 0,
            deltas: pending_terminal_deltas,
        },
    );
    let materializer = DashboardSummaryMaterializerState::from_summary_projection(
        response,
        initial_terminal_slice_suppressions,
        None,
    );
    assert_summary_overlay_totals(&materializer);
}

fn assert_summary_overlay_totals(materializer: &DashboardSummaryMaterializerState) {
    assert_eq!(materializer.response.total_count, 1);
    assert_eq!(materializer.response.total_tokens, 42);
    assert_eq!(materializer.response.total_cost, 0.25);
}

#[tokio::test]
async fn hydrated_summary_topic_does_not_replay_terminal_included_by_projection() {
    let state = crate::tests::test_state_with_openai_base(
        Url::parse("http://127.0.0.1:9").expect("valid test URL"),
    )
    .await;
    hydrate_summary_snapshots(state.as_ref())
        .await
        .expect("hydrate initial summary projection");
    let occurred_at = format_naive(Utc::now().with_timezone(&Shanghai).naive_local());
    let mut terminal = dashboard_runtime_topology_live_record(&occurred_at);
    terminal.id = 842_002;
    terminal.invoke_id = "hydrated-summary-projection-terminal".to_string();
    terminal.status = Some("success".to_string());
    terminal.live_phase = None;
    terminal.total_tokens = Some(42);
    terminal.output_tokens = Some(16);
    terminal.cost = Some(0.25);
    sqlx::query(
        r#"
            INSERT INTO codex_invocations (
                id, invoke_id, occurred_at, source, status, total_tokens, output_tokens, cost,
                payload, raw_response
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
            "#,
    )
    .bind(terminal.id)
    .bind(terminal.invoke_id.as_str())
    .bind(terminal.occurred_at.as_str())
    .bind(terminal.source.as_str())
    .bind("success")
    .bind(terminal.total_tokens)
    .bind(terminal.output_tokens)
    .bind(terminal.cost)
    .bind(json!({ "upstreamAccountId": terminal.upstream_account_id }).to_string())
    .bind("{}")
    .execute(&state.pool)
    .await
    .expect("persist terminal before the refreshed projection");
    let delta = apply_dashboard_activity_terminal_record(state.as_ref(), &terminal)
        .await
        .terminal_delta
        .expect("accept persisted terminal delta");
    state
        .proxy_runtime_invocations
        .record_dashboard_terminal_delta(delta);
    let capture = state
        .proxy_runtime_invocations
        .capture_terminal_slice()
        .expect("capture persisted terminal overlay");
    state
        .subscription_hub
        .materialize_dashboard_terminal_slice(DashboardTerminalProjectionSlice {
            revision: capture.revision,
            deltas: capture.deltas.clone(),
        })
        .await;
    hydrate_summary_snapshots(state.as_ref())
        .await
        .expect("refresh summary projection with the durable terminal");
    state.pool.close().await;

    let summary = SubscriptionTopic::SummaryCurrent {
        window: "today".to_string(),
        time_zone: SUBSCRIPTION_DEFAULT_TIME_ZONE.to_string(),
        limit: None,
        upstream_account_id: None,
    };
    let payload = summary
        .build_cached_payload(state.clone())
        .await
        .expect("refreshed Summary topic remains memory-only")
        .serialize(
            None,
            None,
            Some(&DashboardTerminalProjectionSlice {
                revision: capture.revision,
                deltas: capture.deltas,
            }),
        )
        .expect("serialize the refreshed Summary topic");
    let payload: Value = serde_json::from_slice(&payload).expect("summary payload JSON");
    assert_eq!(payload["totalCount"], json!(1));
    assert_eq!(payload["totalTokens"], json!(42));
    assert_eq!(payload["totalCost"], json!(0.25));
}

#[tokio::test]
async fn hydrated_summary_projection_deduplicates_global_terminal_once_when_account_rollup_lags() {
    let state = crate::tests::test_state_with_openai_base(
        Url::parse("http://127.0.0.1:9").expect("valid test URL"),
    )
    .await;
    hydrate_summary_snapshots(state.as_ref())
        .await
        .expect("hydrate initial summary projection");

    let capture = prepare_lagging_summary_fixture(&state).await;

    hydrate_summary_snapshots(state.as_ref())
        .await
        .expect("global durable proof must refresh the projection");
    {
        let hub = state.subscription_hub.state.lock().await;
        assert!(
            hub.summary_delta_journal.entries.is_empty(),
            "global proof must consume the terminal exactly once from the shared overlay"
        );
    }
    state.pool.close().await;

    let global = SubscriptionTopic::SummaryCurrent {
        window: "7d".to_string(),
        time_zone: SUBSCRIPTION_DEFAULT_TIME_ZONE.to_string(),
        limit: None,
        upstream_account_id: None,
    };
    let payload = global
        .build_cached_payload(state.clone())
        .await
        .expect("global 7d Summary stays memory-only")
        .serialize(
            None,
            None,
            Some(&DashboardTerminalProjectionSlice {
                revision: capture.revision,
                deltas: capture.deltas.clone(),
            }),
        )
        .expect("serialize global Summary without a duplicate terminal");
    let payload: Value = serde_json::from_slice(&payload).expect("global Summary payload");
    assert_eq!(
        payload["totalCount"],
        json!(state.config.list_limit_max + 2),
        "the durable global rollup contributes once and the replay slice cannot add it again"
    );

    let account = SubscriptionTopic::SummaryCurrent {
        window: "7d".to_string(),
        time_zone: SUBSCRIPTION_DEFAULT_TIME_ZONE.to_string(),
        limit: None,
        upstream_account_id: Some(42),
    };
    assert!(matches!(
        account.build_cached_payload(state).await,
        Err(ApiError::Unavailable(_))
    ));
}

async fn prepare_lagging_summary_fixture(
    state: &Arc<AppState>,
) -> DashboardTerminalProjectionSlice {
    let occurred_at = format_naive(
        (Utc::now() - ChronoDuration::days(3))
            .with_timezone(&Shanghai)
            .naive_local(),
    );
    let mut terminal = dashboard_runtime_topology_live_record(&occurred_at);
    terminal.id = 1;
    terminal.invoke_id = "hydrated-summary-account-lag-terminal".to_string();
    terminal.status = Some("success".to_string());
    terminal.live_phase = None;
    terminal.upstream_account_id = Some(42);
    terminal.total_tokens = Some(17);
    terminal.output_tokens = Some(17);
    terminal.cost = Some(1.25);
    sqlx::query(
        r#"
            INSERT INTO codex_invocations (
                id, invoke_id, occurred_at, source, status, total_tokens, output_tokens, cost,
                payload, raw_response
            ) VALUES (?1, ?2, ?3, ?4, 'success', ?5, ?6, ?7, ?8, '{}')
            "#,
    )
    .bind(terminal.id)
    .bind(terminal.invoke_id.as_str())
    .bind(terminal.occurred_at.as_str())
    .bind(terminal.source.as_str())
    .bind(terminal.total_tokens)
    .bind(terminal.output_tokens)
    .bind(terminal.cost)
    .bind(json!({ "upstreamAccountId": terminal.upstream_account_id }).to_string())
    .execute(&state.pool)
    .await
    .expect("persist historical terminal");
    sqlx::query(
            "WITH RECURSIVE rows(value) AS ( \
                 SELECT 1 UNION ALL SELECT value + 1 FROM rows WHERE value < ?1 \
             ) \
             INSERT INTO codex_invocations \
             (invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response, detail_level) \
             SELECT 'summary-account-lag-current-' || value, datetime('now'), 'proxy', 'running', 0, 0, '{\"upstreamAccountId\":42}', '', 'full' \
             FROM rows",
        )
        .bind((state.config.list_limit_max + 1) as i64)
        .execute(&state.pool)
        .await
        .expect("fill the bounded current prefix ahead of the historical terminal");
    let terminal_at =
        parse_to_utc_datetime(&terminal.occurred_at).expect("parse historical terminal timestamp");
    let bucket = align_bucket_epoch(terminal_at.timestamp(), 3_600, 0);
    sqlx::query(
            "INSERT INTO invocation_rollup_hourly \
             (bucket_start_epoch, source, total_count, success_count, failure_count, total_tokens, total_cost, non_success_cost) \
             VALUES (?1, 'proxy', 1, 1, 0, 17, 1.25, 0)",
        )
        .bind(bucket)
        .execute(&state.pool)
        .await
        .expect("insert global terminal totals proof");
    sqlx::query(
            "INSERT INTO upstream_account_usage_breakdown_hourly \
             (bucket_start_epoch, source, upstream_account_key, normalized_model, normalized_reasoning_effort, \
              request_count, output_tokens, cost_output, has_cost) \
             VALUES (?1, 'proxy', '42', 'gpt-5', 'high', 1, 17, 1.25, 1)",
        )
        .bind(bucket)
        .execute(&state.pool)
        .await
        .expect("insert global terminal usage proof");
    sqlx::query(
            "INSERT INTO hourly_rollup_live_progress (dataset, cursor_id, updated_at) \
             VALUES ('codex_invocations_summary_rollup_v2_live_cursor', 1, datetime('now')) \
             ON CONFLICT(dataset) DO UPDATE SET cursor_id = excluded.cursor_id, updated_at = datetime('now')",
        )
        .execute(&state.pool)
        .await
        .expect("advance only the global coverage cursor");
    let delta = apply_dashboard_activity_terminal_record(state.as_ref(), &terminal)
        .await
        .terminal_delta
        .expect("accept persisted historical terminal delta");
    state
        .proxy_runtime_invocations
        .record_dashboard_terminal_delta(delta);
    let capture = state
        .proxy_runtime_invocations
        .capture_terminal_slice()
        .expect("capture historical terminal overlay");
    materialize_terminal_projection_capture(state, &capture).await;
    DashboardTerminalProjectionSlice {
        revision: capture.revision,
        deltas: capture.deltas,
    }
}

async fn materialize_terminal_projection_capture(
    state: &Arc<AppState>,
    capture: &DashboardTerminalProjectionCapture,
) {
    state
        .subscription_hub
        .materialize_dashboard_terminal_slice(DashboardTerminalProjectionSlice {
            revision: capture.revision,
            deltas: capture.deltas.clone(),
        })
        .await;
}

#[tokio::test]
async fn hydrated_summary_topic_recovers_after_terminal_overlay_overflow_is_durably_covered() {
    let state = crate::tests::test_state_with_openai_base(
        Url::parse("http://127.0.0.1:9").expect("valid test URL"),
    )
    .await;
    hydrate_summary_snapshots(state.as_ref())
        .await
        .expect("hydrate initial summary projection");
    let (summary, capture) = prepare_summary_overflow_recovery(&state).await;
    assert!(matches!(
        summary.build_cached_payload(state.clone()).await,
        Err(ApiError::Unavailable(_))
    ));

    hydrate_summary_snapshots(state.as_ref())
        .await
        .expect("durable projection must recover the bounded overlay");
    assert!(
        state
            .subscription_hub
            .state
            .lock()
            .await
            .summary_delta_journal
            .overflowed_through_sequence
            .is_none(),
        "the durable watermark must clear an overflow it fully covers",
    );
    state.pool.close().await;

    assert_recovered_summary_payload(&summary, state, capture).await;
}

async fn prepare_summary_overflow_recovery(
    state: &Arc<AppState>,
) -> (SubscriptionTopic, DashboardTerminalProjectionSlice) {
    let occurred_at = format_naive(Utc::now().with_timezone(&Shanghai).naive_local());
    let mut terminal = dashboard_runtime_topology_live_record(&occurred_at);
    terminal.id = 842_003;
    terminal.invoke_id = "hydrated-summary-overflow-recovery-terminal".to_string();
    terminal.status = Some("success".to_string());
    terminal.live_phase = None;
    terminal.total_tokens = Some(42);
    terminal.output_tokens = Some(16);
    terminal.cost = Some(0.25);
    sqlx::query(
        r#"
            INSERT INTO codex_invocations (
                id, invoke_id, occurred_at, source, status, total_tokens, output_tokens, cost,
                payload, raw_response
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
            "#,
    )
    .bind(terminal.id)
    .bind(terminal.invoke_id.as_str())
    .bind(terminal.occurred_at.as_str())
    .bind(terminal.source.as_str())
    .bind("success")
    .bind(terminal.total_tokens)
    .bind(terminal.output_tokens)
    .bind(terminal.cost)
    .bind(json!({ "upstreamAccountId": terminal.upstream_account_id }).to_string())
    .bind("{}")
    .execute(&state.pool)
    .await
    .expect("persist terminal before the recovery projection");
    let delta = apply_dashboard_activity_terminal_record(state.as_ref(), &terminal)
        .await
        .terminal_delta
        .expect("accept persisted terminal delta");
    assert_eq!(delta.persisted_row_id, Some(terminal.id));
    state
        .proxy_runtime_invocations
        .record_dashboard_terminal_delta(delta.clone());
    let capture = state
        .proxy_runtime_invocations
        .capture_terminal_slice()
        .expect("capture persisted terminal overlay");
    let slice = DashboardTerminalProjectionSlice {
        revision: capture.revision,
        deltas: capture.deltas.clone(),
    };
    state
        .subscription_hub
        .materialize_dashboard_terminal_slice(DashboardTerminalProjectionSlice {
            revision: slice.revision,
            deltas: slice.deltas.clone(),
        })
        .await;
    let mut hub = state.subscription_hub.state.lock().await;
    hub.summary_delta_journal.overflowed_through_sequence = Some(delta.terminal_sequence);
    hub.summary_delta_journal.note_gap(&delta);
    drop(hub);
    (
        SubscriptionTopic::SummaryCurrent {
            window: "today".to_string(),
            time_zone: SUBSCRIPTION_DEFAULT_TIME_ZONE.to_string(),
            limit: None,
            upstream_account_id: None,
        },
        slice,
    )
}

async fn assert_recovered_summary_payload(
    summary: &SubscriptionTopic,
    state: Arc<AppState>,
    capture: DashboardTerminalProjectionSlice,
) {
    let payload = summary
        .build_cached_payload(state)
        .await
        .expect("recovered Summary topic remains memory-only")
        .serialize(None, None, Some(&capture))
        .expect("serialize the recovered Summary topic");
    let payload: Value = serde_json::from_slice(&payload).expect("summary payload JSON");
    assert_eq!(payload["totalCount"], json!(1));
    assert_eq!(payload["totalTokens"], json!(42));
    assert_eq!(payload["totalCost"], json!(0.25));
}

#[tokio::test]
async fn all_time_overlay_overflow_does_not_block_rolling_summary() {
    let state = crate::tests::test_state_with_openai_base(
        Url::parse("http://127.0.0.1:9").expect("valid test URL"),
    )
    .await;
    {
        let mut guard = state.subscription_hub.state.lock().await;
        guard.summary_terminal_overlay_all_time_overflowed_through_sequence = Some(42);
    }
    let projection = SummaryProjection::default();
    assert!(
        state
            .subscription_hub
            .summary_projection_terminal_overlay(&projection, false, None)
            .await
            .is_ok(),
        "an all-time replay budget exhaustion must not make rolling Summary unavailable"
    );
    assert!(matches!(
        state
            .subscription_hub
            .summary_projection_terminal_overlay(&projection, true, None)
            .await,
        Err(ApiError::Unavailable(_))
    ));
    state.pool.close().await;
}

#[tokio::test]
async fn account_all_time_overlay_overflow_remains_scoped_until_account_proof() {
    let state = crate::tests::test_state_with_openai_base(
        Url::parse("http://127.0.0.1:9").expect("valid test URL"),
    )
    .await;
    {
        let mut guard = state.subscription_hub.state.lock().await;
        guard
            .summary_terminal_overlay_all_time_overflowed_through_account
            .insert(42, 7);
    }

    let projection = SummaryProjection::default();
    assert!(matches!(
        state
            .subscription_hub
            .summary_projection_terminal_overlay(&projection, true, Some(42))
            .await,
        Err(ApiError::Unavailable(_))
    ));
    assert!(
        state
            .subscription_hub
            .summary_projection_terminal_overlay(&projection, true, None)
            .await
            .is_ok(),
        "an account-scoped overflow must not make the global all-time topic unavailable"
    );

    state
        .subscription_hub
        .store_summary_projection(projection)
        .await;
    assert!(
        state
            .subscription_hub
            .state
            .lock()
            .await
            .summary_terminal_overlay_all_time_overflowed_through_account
            .contains_key(&42)
    );
    state.pool.close().await;
}

#[tokio::test]
async fn terminal_slice_keeps_fresh_moving_summary_base_off_sqlite() {
    let state = crate::tests::test_state_with_openai_base(
        Url::parse("http://127.0.0.1:9").expect("valid test URL"),
    )
    .await;
    hydrate_summary_snapshots(state.as_ref())
        .await
        .expect("hydrate Summary baseline before closing SQLite");
    let summary = SubscriptionTopic::SummaryCurrent {
        window: "1d".to_string(),
        time_zone: SUBSCRIPTION_DEFAULT_TIME_ZONE.to_string(),
        limit: None,
        upstream_account_id: None,
    };
    let _lease = state
        .subscription_hub
        .register_topic_subscribers(std::slice::from_ref(&summary))
        .await
        .expect("register active summary topic");
    state
        .subscription_hub
        .prepare_connection(state.clone(), vec![summary.descriptor()], Vec::new())
        .await
        .expect("prepare moving summary base");
    let summary_key = summary.cache_key().expect("summary topic key");
    state.pool.close().await;

    state
        .subscription_hub
        .materialize_dashboard_terminal_slice(DashboardTerminalProjectionSlice {
            revision: 1,
            deltas: Vec::new(),
        })
        .await;

    let guard = state.subscription_hub.state.lock().await;
    let cached = &guard.topics[&summary_key];
    assert!(
        !cached.dirty,
        "a fresh duration base must not schedule a SQLite rebase from terminal delivery",
    );
    assert!(
        !cached
            .dashboard_materializer
            .as_ref()
            .expect("typed summary materializer")
            .requires_terminal_window_rebase(),
    );
}

#[tokio::test]
async fn terminal_slice_keeps_fresh_rolling_activity_base_off_sqlite() {
    let state = crate::tests::test_state_with_openai_base(
        Url::parse("http://127.0.0.1:9").expect("valid test URL"),
    )
    .await;
    let activity = SubscriptionTopic::DashboardActivityCurrent {
        range: "1d".to_string(),
        time_zone: SUBSCRIPTION_DEFAULT_TIME_ZONE.to_string(),
        recent_limit: 16,
        include_accounts: true,
        include_recent: true,
    };
    let _lease = state
        .subscription_hub
        .register_topic_subscribers(std::slice::from_ref(&activity))
        .await
        .expect("register active rolling activity topic");
    state
        .subscription_hub
        .prepare_connection(state.clone(), vec![activity.descriptor()], Vec::new())
        .await
        .expect("prepare rolling activity base");
    let activity_key = activity.cache_key().expect("activity topic key");
    {
        let guard = state.subscription_hub.state.lock().await;
        let DashboardTopicMaterializer::Activity { base, .. } = guard.topics[&activity_key]
            .dashboard_materializer
            .as_ref()
            .expect("typed activity materializer")
        else {
            panic!("expected activity materializer");
        };
        let mut base = base.lock().expect("activity materializer state lock");
        let current_start = base.rebase_range_start.expect("activity base range start");
        base.rebase_range_start = Some(current_start - ChronoDuration::seconds(1));
    }
    state.pool.close().await;

    state
        .subscription_hub
        .materialize_dashboard_terminal_slice(DashboardTerminalProjectionSlice {
            revision: 1,
            deltas: Vec::new(),
        })
        .await;

    let guard = state.subscription_hub.state.lock().await;
    let cached = &guard.topics[&activity_key];
    assert!(
        !cached.dirty,
        "a fresh rolling activity base must not schedule a SQLite rebase from terminal delivery",
    );
    assert!(
        !cached
            .dashboard_materializer
            .as_ref()
            .expect("typed activity materializer")
            .requires_terminal_window_rebase(),
    );
}

#[tokio::test]
async fn rolling_activity_terminal_slice_preserves_rebase_anchor() {
    let state = crate::tests::test_state_with_openai_base(
        Url::parse("http://127.0.0.1:9").expect("valid test URL"),
    )
    .await;
    let activity = SubscriptionTopic::DashboardActivityCurrent {
        range: "1d".to_string(),
        time_zone: SUBSCRIPTION_DEFAULT_TIME_ZONE.to_string(),
        recent_limit: 16,
        include_accounts: true,
        include_recent: true,
    };
    let materializer = activity
        .build_cached_payload(state.clone())
        .await
        .expect("build rolling activity base")
        .dashboard_materializer()
        .expect("typed activity materializer");
    let DashboardTopicMaterializer::Activity { base, .. } = &materializer else {
        panic!("expected activity materializer");
    };
    let stale_anchor = resolve_dashboard_activity_cached_range("1d", Shanghai)
        .expect("rolling activity range")
        .start
        - ChronoDuration::seconds(DASHBOARD_ACTIVITY_SNAPSHOT_CACHE_TTL_SECS as i64);
    base.lock()
        .expect("activity materializer state lock")
        .rebase_range_start = Some(stale_anchor);

    let occurred_at = format_naive(Utc::now().with_timezone(&Shanghai).naive_local());
    let mut terminal = dashboard_runtime_topology_live_record(&occurred_at);
    terminal.id = 0;
    terminal.invoke_id = "dashboard-runtime-rolling-activity-anchor".to_string();
    terminal.status = Some("success".to_string());
    terminal.live_phase = None;
    terminal.total_tokens = Some(42);
    terminal.output_tokens = Some(16);
    let delta = apply_dashboard_activity_terminal_record(state.as_ref(), &terminal)
        .await
        .terminal_delta
        .expect("accepted terminal delta");

    materializer
        .serialize(
            None,
            None,
            Some(&DashboardTerminalProjectionSlice {
                revision: 1,
                deltas: vec![delta.clone()],
            }),
        )
        .expect("apply nonempty terminal slice");

    let base = base.lock().expect("activity materializer state lock");
    assert_eq!(base.rebase_range_start, Some(stale_anchor));
    assert_eq!(
        base.base.response().terminal_sequence,
        delta.terminal_sequence
    );
    drop(base);
    assert!(
        materializer.requires_terminal_window_rebase(),
        "terminal delivery must not reset a rolling activity rebase boundary",
    );
}
