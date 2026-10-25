use super::*;
use serde_json::json;
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
        .get(&topic_key)
        .expect("cached network topic")
        .cursor;
    assert_eq!(cursor_after_unchanged, cursor_after);
    assert_eq!(
        delivery_after.network_timeseries.materialization_count,
        delivery_before.network_timeseries.materialization_count,
        "unchanged network revisions must not rematerialize a topic",
    );
    assert_eq!(
        delivery_after.network_timeseries.serialization_count,
        delivery_before.network_timeseries.serialization_count,
        "unchanged network revisions must not serialize a topic",
    );
    assert_eq!(delivery_after.network_timeseries.payload_clone_count, 0);
}

#[tokio::test]
async fn initial_network_timeseries_materializes_live_bucket_at_fractional_boundary() {
    let state = crate::tests::test_state_with_openai_base(
        Url::parse("http://127.0.0.1:9").expect("valid test URL"),
    )
    .await;
    state
        .proxy_runtime_invocations
        .bind_dashboard_network_speed_cache(state.dashboard_network_speed_cache.clone())
        .expect("bind network cache");
    let hub = state.subscription_hub.clone();
    let now = Utc
        .with_ymd_and_hms(2026, 10, 25, 10, 50, 0)
        .single()
        .expect("valid boundary time")
        + ChronoDuration::milliseconds(451);
    state.dashboard_network_speed_cache.record_request_bytes(
        "fractional-boundary-network",
        "2026-10-25 18:50:00",
        None,
        Some("api.openai.com"),
        512,
        now,
    );
    let topic = SubscriptionTopic::DashboardNetworkTimeseriesWindow {
        range: "today".to_string(),
        time_zone: SUBSCRIPTION_DEFAULT_TIME_ZONE.to_string(),
        upstream_account_id: None,
    };
    let topic_key = topic.cache_key().expect("network topic key");
    let prepared = hub
        .prepare_connection_at(state.clone(), vec![topic.descriptor()], Vec::new(), now)
        .await
        .expect("prepare initial network subscription");
    let initial_payload = prepared
        .initial
        .first()
        .expect("initial network subscription frame")
        .frame
        .payload_value();
    let initial_live_point = initial_payload
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
        .expect("initial today frame should contain the current live bucket");
    assert_eq!(
        initial_live_point
            .get("bucketStart")
            .and_then(Value::as_str),
        Some("2026-10-25T10:50:00Z")
    );
    assert_eq!(
        initial_live_point
            .get("uploadBytes")
            .and_then(Value::as_i64),
        Some(512)
    );

    let (cursor_before, live_point) = {
        let guard = hub.state.lock().await;
        let cached = guard.topics.get(&topic_key).expect("cached network topic");
        let DashboardTopicMaterializer::NetworkTimeseries { base, .. } = cached
            .dashboard_materializer
            .as_ref()
            .expect("network materializer")
        else {
            panic!("expected network timeseries materializer");
        };
        let live_point = base
            .points
            .iter()
            .find(|point| point.is_live_bucket)
            .cloned()
            .expect("cached initial base should contain the current live bucket");
        (cached.cursor, live_point)
    };
    assert_eq!(live_point.bucket_start, "2026-10-25T10:50:00Z");
    assert_eq!(live_point.upload_bytes, 512);

    let _lease = hub
        .register_test_topic_name("dashboard.network-timeseries.window")
        .await;
    state.pool.close().await;
    let network = DashboardNetworkProjectionSlice {
        revision: 1,
        network_live_bucket: Some(live_point.clone()),
        network_realtime_rate: None,
        accounts: Vec::new(),
        recent: DashboardRecentNetworkWindowResponse {
            range_start: String::new(),
            range_end: String::new(),
            window_seconds: DASHBOARD_NETWORK_RECENT_WINDOW_SECONDS,
            sample_seconds: 1,
            is_warming_up: false,
            points: Vec::new(),
        },
        current_snapshot: DashboardActivityCurrentSnapshot::default(),
        current_snapshot_by_account: Default::default(),
    };
    hub.handle_internal_broadcast(
        state,
        BroadcastPayload::DashboardNetworkSlice {
            slice: Box::new(network),
        },
    )
    .await;
    let cached = hub
        .state
        .lock()
        .await
        .topics
        .get(&topic_key)
        .cloned()
        .expect("cached network topic after materialization");
    assert_eq!(cached.cursor, cursor_before + 1);
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
    let mut terminal = dashboard_runtime_topology_live_record(&occurred_at);
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

    let guard = state.subscription_hub.state.lock().await;
    let activity_cached = &guard.topics[&activity_key];
    let summary_cached = &guard.topics[&summary_key];
    assert_eq!(activity_cached.cursor, activity_cursor_before + 1);
    assert_eq!(summary_cached.cursor, summary_cursor_before);
    assert_eq!(
        activity_cached.snapshot_frame.payload_value()["summary"]["stats"]["totalCount"],
        json!(1)
    );
    assert_eq!(
        activity_cached.snapshot_frame.payload_value()["accounts"][0]["upstreamAccountId"],
        json!(42)
    );
    assert_eq!(
        activity_cached.snapshot_frame.payload_value()["accounts"][0]["totalTokens"],
        json!(42)
    );
    assert_eq!(
        activity_cached.snapshot_frame.payload_value()["summary"]["modelPerformance"]["total"]["cumulativeUsageDurationMs"],
        json!(3_450.0)
    );
    assert_eq!(
        activity_cached.snapshot_frame.payload_value()["accounts"][0]["firstTokenAvgMs"],
        json!(650.0)
    );
    assert_eq!(
        activity_cached.snapshot_frame.payload_value()["accounts"][0]["avgTotalMs"],
        json!(3_450.0)
    );
    assert_eq!(
        activity_cached.snapshot_frame.payload_value()["accounts"][0]["modelPerformance"]["models"]
            [0]["model"],
        json!("gpt-5")
    );
    assert_eq!(
        activity_cached.snapshot_frame.payload_value()["accounts"][0]["recentInvocations"][0]["invokeId"],
        json!("dashboard-runtime-terminal-materialization")
    );
    assert_eq!(
        summary_cached.snapshot_frame.payload_value()["totalCount"],
        json!(0)
    );
    drop(guard);

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
    let guard = state.subscription_hub.state.lock().await;
    assert_eq!(
        guard.topics[&activity_key].cursor,
        activity_cursor_before + 1,
        "an unchanged terminal revision must not advance the activity cursor",
    );
    assert_eq!(
        guard.topics[&summary_key].cursor, summary_cursor_before,
        "an unacknowledged terminal slice must not advance the summary cursor",
    );
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
async fn hydrated_summary_topic_degrades_for_terminal_and_source_gaps() {
    let state = crate::tests::test_state_with_openai_base(
        Url::parse("http://127.0.0.1:9").expect("valid test URL"),
    )
    .await;
    hydrate_summary_snapshots(state.as_ref())
        .await
        .expect("hydrate summary projection before recording the gap");
    let mut overlay = summary_projection_test_invocation();
    overlay.id = 913_101;
    overlay.invoke_id = "hydrated-summary-source-gap-overlay".to_string();
    overlay.occurred_at = format_naive(Utc::now().with_timezone(&Shanghai).naive_local());
    let mut delta = persisted_dashboard_activity_terminal_delta(&overlay);
    delta.terminal_sequence = 913_101;
    state
        .subscription_hub
        .acknowledge_replayed_summary_delta(delta)
        .await;
    state
        .subscription_hub
        .record_summary_source_change_gap(913_103)
        .await;
    state.pool.close().await;

    let summary = SubscriptionTopic::SummaryCurrent {
        window: "today".to_string(),
        time_zone: SUBSCRIPTION_DEFAULT_TIME_ZONE.to_string(),
        limit: None,
        upstream_account_id: Some(17),
    };
    let payload = summary
        .build_cached_payload(state.clone())
        .await
        .expect("source-journal Summary gap should retain the last-good payload")
        .serialize(None, None, None)
        .expect("serialize degraded Summary payload");
    let payload: Value = serde_json::from_slice(&payload).expect("Summary payload JSON");
    assert!(!payload["dataQuality"].is_null());
    let current = SubscriptionTopic::SummaryCurrent {
        window: "current".to_string(),
        time_zone: SUBSCRIPTION_DEFAULT_TIME_ZONE.to_string(),
        limit: Some(1),
        upstream_account_id: Some(17),
    };
    state
        .subscription_hub
        .record_summary_terminal_sequence_gap(913_102)
        .await;
    assert!(summary.build_cached_payload(state.clone()).await.is_ok());
    assert!(matches!(
        current.build_cached_payload(state.clone()).await,
        Err(ApiError::Unavailable(_))
    ));
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
    state
        .subscription_hub
        .materialize_dashboard_terminal_slice(DashboardTerminalProjectionSlice {
            revision: capture.revision,
            deltas: capture.deltas.clone(),
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

    let query = SummaryQuery {
        window: Some("today".to_string()),
        limit: None,
        time_zone: Some(SUBSCRIPTION_DEFAULT_TIME_ZONE.to_string()),
        upstream_account_id: None,
    };
    let mut response = projection
        .response_for_query(&query, state.config.list_limit_max as i64)
        .expect("old projection remains a valid immutable base");
    let summary_window = parse_summary_window(&query, state.config.list_limit_max as i64)
        .expect("parse Summary window");
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
    state
        .subscription_hub
        .materialize_dashboard_terminal_slice(DashboardTerminalProjectionSlice {
            revision: capture.revision,
            deltas: capture.deltas.clone(),
        })
        .await;

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

#[tokio::test]
async fn hydrated_summary_topic_recovers_after_terminal_overlay_overflow_is_durably_covered() {
    let state = crate::tests::test_state_with_openai_base(
        Url::parse("http://127.0.0.1:9").expect("valid test URL"),
    )
    .await;
    hydrate_summary_snapshots(state.as_ref())
        .await
        .expect("hydrate initial summary projection");
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
    state
        .subscription_hub
        .materialize_dashboard_terminal_slice(DashboardTerminalProjectionSlice {
            revision: capture.revision,
            deltas: capture.deltas.clone(),
        })
        .await;
    {
        let mut hub = state.subscription_hub.state.lock().await;
        hub.summary_delta_journal.overflowed_through_sequence = Some(delta.terminal_sequence);
        hub.summary_delta_journal.note_gap(&delta);
    }

    let summary = SubscriptionTopic::SummaryCurrent {
        window: "today".to_string(),
        time_zone: SUBSCRIPTION_DEFAULT_TIME_ZONE.to_string(),
        limit: None,
        upstream_account_id: None,
    };
    let payload = summary
        .build_cached_payload(state.clone())
        .await
        .expect("rolling terminal overflow must retain a degraded Summary payload")
        .serialize(
            None,
            None,
            Some(&DashboardTerminalProjectionSlice {
                revision: capture.revision,
                deltas: capture.deltas.clone(),
            }),
        )
        .expect("serialize the degraded Summary payload");
    let payload: Value = serde_json::from_slice(&payload).expect("summary payload JSON");
    assert_eq!(payload["totalCount"], json!(1));
    assert_eq!(payload["totalTokens"], json!(42));
    assert_eq!(payload["totalCost"], json!(0.25));
    assert_eq!(
        payload["dataQuality"],
        serde_json::to_value(StatsDataQualityResponse::summary_delta_journal_pending())
            .expect("serialize Summary data quality")
    );

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

    let payload = summary
        .build_cached_payload(state.clone())
        .await
        .expect("recovered Summary topic remains memory-only")
        .serialize(
            None,
            None,
            Some(&DashboardTerminalProjectionSlice {
                revision: capture.revision,
                deltas: capture.deltas,
            }),
        )
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
async fn rolling_overlay_overflow_stays_selection_local() {
    let state = crate::tests::test_state_with_openai_base(
        Url::parse("http://127.0.0.1:9").expect("valid test URL"),
    )
    .await;
    {
        let mut guard = state.subscription_hub.state.lock().await;
        guard.summary_delta_journal.overflowed_through_sequence = Some(42);
    }
    let projection = SummaryProjection::default();
    assert!(
        state
            .subscription_hub
            .summary_projection_terminal_overlay(&projection, false, None)
            .await
            .is_ok(),
        "rolling overflow must be reconciled by the selection-local gap proof"
    );
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

#[tokio::test]
async fn failed_runtime_window_rebase_retains_an_isolated_last_good_frame() {
    let state = crate::tests::test_state_with_openai_base(
        Url::parse("http://127.0.0.1:9").expect("valid test URL"),
    )
    .await;
    hydrate_summary_snapshots(state.as_ref())
        .await
        .expect("hydrate Summary baseline before failure isolation");
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
    {
        let mut guard = state.subscription_hub.state.lock().await;
        let DashboardTopicMaterializer::Summary { base, .. } = guard.topics[&summary_key]
            .dashboard_materializer
            .as_ref()
            .expect("typed summary materializer")
        else {
            panic!("expected summary materializer");
        };
        base.lock()
            .expect("summary materializer state lock")
            .range_start = Some(Utc::now() - ChronoDuration::days(2));
        // A runtime projection can be unavailable while a previously delivered frame is
        // still retained. Reconciliation must preserve that frame when it cannot rebuild.
        guard.summary_projection = None;
    }
    state.pool.close().await;

    state
        .subscription_hub
        .materialize_dashboard_terminal_slice(DashboardTerminalProjectionSlice {
            revision: 1,
            deltas: Vec::new(),
        })
        .await;

    assert!(
        state.subscription_hub.state.lock().await.topics[&summary_key].dirty,
        "terminal delivery must isolate a stale base without starting a database rebase",
    );

    state
        .subscription_hub
        .reconcile_dashboard_terminal_window_bases(state.clone())
        .await;
    let guard = state.subscription_hub.state.lock().await;
    let cached = &guard.topics[&summary_key];
    assert!(
        cached.dirty && cached.refresh_scheduled,
        "a failed rebase must preserve its last-good frame and isolate every slice",
    );
    assert!(
        cached
            .dashboard_materializer
            .as_ref()
            .expect("typed summary materializer")
            .requires_terminal_window_rebase(),
        "the next runtime reconcile must retry the unresolved window rebase",
    );
}

#[tokio::test]
async fn runtime_reconcile_rebases_stale_activity_and_summary_window_bases() {
    let state = crate::tests::test_state_with_openai_base(
        Url::parse("http://127.0.0.1:9").expect("valid test URL"),
    )
    .await;
    hydrate_summary_snapshots(state.as_ref())
        .await
        .expect("hydrate Summary baseline before runtime reconciliation");
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
    let summary_key = summary.cache_key().expect("summary topic key");
    let summary_cursor_before = {
        let mut guard = state.subscription_hub.state.lock().await;
        let activity_cached = guard
            .topics
            .get_mut(&activity.cache_key().expect("activity topic key"))
            .expect("activity cache entry");
        let DashboardTopicMaterializer::Activity { base, .. } = activity_cached
            .dashboard_materializer
            .as_ref()
            .expect("activity materializer")
        else {
            panic!("expected activity materializer");
        };
        base.lock()
            .expect("activity materializer state lock")
            .rebase_range_start = Some(Utc::now() - ChronoDuration::days(1));
        let summary_cached = guard
            .topics
            .get_mut(&summary_key)
            .expect("summary cache entry");
        let DashboardTopicMaterializer::Summary { base, .. } = summary_cached
            .dashboard_materializer
            .as_ref()
            .expect("summary materializer")
        else {
            panic!("expected summary materializer");
        };
        base.lock()
            .expect("summary materializer state lock")
            .range_start = Some(Utc::now() - ChronoDuration::days(2));
        summary_cached.cursor
    };

    state
        .subscription_hub
        .reconcile_dashboard_terminal_window_bases(state.clone())
        .await;
    let guard = state.subscription_hub.state.lock().await;
    for topic_key in [
        activity.cache_key().expect("activity topic key"),
        summary_key.clone(),
    ] {
        let cached = &guard.topics[&topic_key];
        assert!(
            !cached.dirty
                && !cached
                    .dashboard_materializer
                    .as_ref()
                    .expect("typed Dashboard materializer")
                    .requires_terminal_window_rebase(),
            "runtime reconciliation should replace stale terminal-window bases",
        );
    }
    assert_eq!(
        guard.topics[&summary_key].cursor, summary_cursor_before,
        "a byte-identical summary rebase should retain its shared frame cursor",
    );
}

#[tokio::test]
async fn runtime_reconcile_marks_inactive_stale_window_bases_dirty_for_reconnect() {
    let state = crate::tests::test_state_with_openai_base(
        Url::parse("http://127.0.0.1:9").expect("valid test URL"),
    )
    .await;
    hydrate_summary_snapshots(state.as_ref())
        .await
        .expect("hydrate Summary baseline before reconnect reconciliation");
    let summary = SubscriptionTopic::SummaryCurrent {
        window: "1d".to_string(),
        time_zone: SUBSCRIPTION_DEFAULT_TIME_ZONE.to_string(),
        limit: None,
        upstream_account_id: None,
    };
    let initial_lease = state
        .subscription_hub
        .register_topic_subscribers(std::slice::from_ref(&summary))
        .await
        .expect("register initial summary owner");
    state
        .subscription_hub
        .prepare_connection(state.clone(), vec![summary.descriptor()], Vec::new())
        .await
        .expect("prepare moving summary base");
    let summary_key = summary.cache_key().expect("summary topic key");
    {
        let guard = state.subscription_hub.state.lock().await;
        let DashboardTopicMaterializer::Summary { base, .. } = guard.topics[&summary_key]
            .dashboard_materializer
            .as_ref()
            .expect("typed summary materializer")
        else {
            panic!("expected summary materializer");
        };
        base.lock()
            .expect("summary materializer state lock")
            .range_start = Some(Utc::now() - ChronoDuration::days(2));
    }
    drop(initial_lease);

    tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            if !state
                .subscription_hub
                .state
                .lock()
                .await
                .active_subscribers
                .contains_key(&summary_key)
            {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("initial owner should release before the reconcile scan");

    state
        .subscription_hub
        .reconcile_dashboard_terminal_window_bases(state.clone())
        .await;
    {
        let guard = state.subscription_hub.state.lock().await;
        let cached = &guard.topics[&summary_key];
        assert!(
            cached.dirty && !cached.refresh_scheduled,
            "an inactive stale base must defer its authoritative rebuild to reconnect",
        );
    }

    let _returned_lease = state
        .subscription_hub
        .register_topic_subscribers(std::slice::from_ref(&summary))
        .await
        .expect("register returning summary owner");
    state
        .subscription_hub
        .prepare_connection(state.clone(), vec![summary.descriptor()], Vec::new())
        .await
        .expect("rebuild stale summary base on reconnect");
    let guard = state.subscription_hub.state.lock().await;
    let cached = &guard.topics[&summary_key];
    assert!(
        !cached.dirty
            && !cached
                .dashboard_materializer
                .as_ref()
                .expect("typed summary materializer")
                .requires_terminal_window_rebase(),
        "reconnecting must receive an authoritative moving-window base",
    );
}

#[tokio::test]
async fn summary_topic_base_preserves_pending_terminal_overlay() {
    let state = crate::tests::test_state_with_openai_base(
        Url::parse("http://127.0.0.1:9").expect("valid test URL"),
    )
    .await;
    let occurred_at = format_naive(Utc::now().with_timezone(&Shanghai).naive_local());
    let mut terminal = dashboard_runtime_topology_live_record(&occurred_at);
    terminal.id = 0;
    terminal.invoke_id = "dashboard-runtime-summary-pending-terminal".to_string();
    terminal.status = Some("success".to_string());
    terminal.live_phase = None;
    terminal.total_tokens = Some(42);
    terminal.output_tokens = Some(16);
    terminal.cost = Some(0.25);
    let delta = apply_dashboard_activity_terminal_record(state.as_ref(), &terminal)
        .await
        .terminal_delta
        .expect("accepted pending terminal delta");

    let query = SummaryQuery {
        window: Some("today".to_string()),
        limit: None,
        time_zone: Some(SUBSCRIPTION_DEFAULT_TIME_ZONE.to_string()),
        upstream_account_id: None,
    };
    let summary_window =
        parse_summary_window(&query, state.config.list_limit_max as i64).expect("summary window");
    let SummaryTopicTerminalConsistentBase {
        mut response,
        pending_terminal_deltas,
        terminal_sequence,
    } = build_summary_topic_terminal_consistent_base(state.as_ref(), &query)
        .await
        .expect("build terminal-consistent summary base");
    assert_eq!(terminal_sequence, delta.terminal_sequence);
    assert_eq!(pending_terminal_deltas.len(), 1);

    let initial_slice = DashboardTerminalProjectionSlice {
        revision: 0,
        deltas: pending_terminal_deltas,
    };
    let mut replayed_terminal_sequence = 0;
    apply_dashboard_terminal_slice_to_summary_response(
        &mut response,
        &mut replayed_terminal_sequence,
        &summary_window,
        Shanghai,
        InvocationSourceScope::ProxyOnly,
        None,
        &initial_slice,
    );
    let payload = DashboardTopicMaterializer::Summary {
        base: Arc::new(StdMutex::new(DashboardSummaryMaterializerState::new(
            response,
            terminal_sequence,
            summary_window_range(&summary_window, Shanghai, Utc::now())
                .expect("summary range")
                .map(|(start, _)| start),
        ))),
        window: summary_window,
        reporting_tz: Shanghai,
        source_scope: InvocationSourceScope::ProxyOnly,
        upstream_account_id: None,
    }
    .serialize(
        None,
        None,
        Some(&DashboardTerminalProjectionSlice {
            revision: 1,
            deltas: vec![delta],
        }),
    )
    .expect("serialize summary terminal base");
    let payload: Value = serde_json::from_slice(&payload).expect("summary payload JSON");
    assert_eq!(payload["totalCount"], json!(1));
    assert_eq!(payload["totalTokens"], json!(42));
}

#[tokio::test]
async fn timeseries_topic_base_preserves_pending_terminal_overlay() {
    let state = crate::tests::test_state_with_openai_base(
        Url::parse("http://127.0.0.1:9").expect("valid test URL"),
    )
    .await;
    let occurred_at = format_naive(Utc::now().with_timezone(&Shanghai).naive_local());
    let mut terminal = dashboard_runtime_topology_live_record(&occurred_at);
    terminal.id = 0;
    terminal.invoke_id = "dashboard-runtime-timeseries-pending-terminal".to_string();
    terminal.status = Some("success".to_string());
    terminal.live_phase = None;
    terminal.total_tokens = Some(42);
    terminal.output_tokens = Some(16);
    terminal.cost = Some(0.25);
    let delta = apply_dashboard_activity_terminal_record(state.as_ref(), &terminal)
        .await
        .terminal_delta
        .expect("accepted pending terminal delta");

    let timeseries = SubscriptionTopic::TimeseriesOpenWindow {
        range: "1d".to_string(),
        time_zone: SUBSCRIPTION_DEFAULT_TIME_ZONE.to_string(),
        bucket: Some("1m".to_string()),
        settlement_hour: None,
        upstream_account_id: None,
    };
    let materializer = timeseries
        .build_cached_payload(state)
        .await
        .expect("build terminal-consistent timeseries base")
        .dashboard_materializer()
        .expect("typed timeseries materializer");
    let initial_payload: Value = serde_json::from_slice(
        &materializer
            .serialize(None, None, None)
            .expect("serialize materialized timeseries base"),
    )
    .expect("timeseries payload JSON");

    assert_eq!(
        initial_payload["points"]
            .as_array()
            .expect("timeseries points")
            .iter()
            .map(|point| point["totalCount"].as_i64().unwrap_or_default())
            .sum::<i64>(),
        1,
        "the typed baseline must include the terminal delta before SQLite persistence",
    );

    let replayed_payload: Value = serde_json::from_slice(
        &materializer
            .serialize(
                None,
                None,
                Some(&DashboardTerminalProjectionSlice {
                    revision: 1,
                    deltas: vec![delta],
                }),
            )
            .expect("skip the terminal delta already folded into the base"),
    )
    .expect("timeseries replay payload JSON");
    assert_eq!(
        replayed_payload["points"]
            .as_array()
            .expect("timeseries points")
            .iter()
            .map(|point| point["totalCount"].as_i64().unwrap_or_default())
            .sum::<i64>(),
        1,
        "the sequence watermark must reject the pending terminal replay",
    );
}

#[tokio::test]
async fn summary_materializer_skips_terminal_delta_already_folded_into_base() {
    let state = crate::tests::test_state_with_openai_base(
        Url::parse("http://127.0.0.1:9").expect("valid test URL"),
    )
    .await;
    let summary = SubscriptionTopic::SummaryCurrent {
        window: "today".to_string(),
        time_zone: SUBSCRIPTION_DEFAULT_TIME_ZONE.to_string(),
        limit: None,
        upstream_account_id: None,
    };
    let occurred_at = format_naive(Utc::now().with_timezone(&Shanghai).naive_local());
    let mut terminal = dashboard_runtime_topology_live_record(&occurred_at);
    terminal.id = 748_002;
    terminal.invoke_id = "dashboard-runtime-summary-folded-terminal".to_string();
    terminal.status = Some("success".to_string());
    terminal.live_phase = None;
    terminal.total_tokens = Some(42);
    terminal.output_tokens = Some(16);
    terminal.cost = Some(0.25);
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
    let outcome = apply_dashboard_activity_terminal_record(state.as_ref(), &terminal).await;
    let delta = outcome
        .terminal_delta
        .expect("accepted persisted terminal delta");
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

    let payload = summary
        .build_cached_payload(state.clone())
        .await
        .expect("build typed summary base")
        .serialize(
            None,
            None,
            Some(&DashboardTerminalProjectionSlice {
                revision: capture.revision,
                deltas: capture.deltas,
            }),
        )
        .expect("serialize folded summary base");
    let payload: Value = serde_json::from_slice(&payload).expect("summary payload JSON");
    assert_eq!(payload["totalCount"], json!(1));
    assert_eq!(payload["totalTokens"], json!(42));
}

#[tokio::test]
async fn dashboard_materialization_rejects_out_of_order_network_revisions() {
    let state = crate::tests::test_state_with_openai_base(
        Url::parse("http://127.0.0.1:9").expect("valid test URL"),
    )
    .await;
    state
        .proxy_runtime_invocations
        .bind_dashboard_network_speed_cache(state.dashboard_network_speed_cache.clone())
        .expect("bind network cache");
    let hub = state.subscription_hub.clone();
    let topic = SubscriptionTopic::DashboardNetworkRecentCurrent;
    let topic_key = topic.cache_key().expect("network recent topic key");
    let _lease = hub
        .register_topic_subscribers(std::slice::from_ref(&topic))
        .await
        .expect("register network recent topic");
    hub.prepare_connection(state.clone(), vec![topic.descriptor()], Vec::new())
        .await
        .expect("prepare network recent topic");
    let cursor_before = hub
        .state
        .lock()
        .await
        .topics
        .get(&topic_key)
        .expect("cached network recent topic")
        .cursor;

    let older = state
        .proxy_runtime_invocations
        .capture_network_slice()
        .expect("capture older network slice")
        .slice;
    let mut newer = older.clone();
    newer.revision = older.revision.saturating_add(1);
    newer.recent.range_end = "2026-08-06T00:00:01.000Z".to_string();

    let (stale_pending, stale_payload) = {
        let mut guard = hub.state.lock().await;
        guard.dashboard_network_slice = Some(Arc::new(older.clone()));
        let pending = collect_pending_dashboard_topic_materializations(&mut guard)
            .into_iter()
            .find(|pending| pending.topic_key == topic_key)
            .expect("pending stale network revision");
        let payload = pending
            .materializer
            .serialize(
                guard.dashboard_current_slice.as_deref(),
                guard.dashboard_network_slice.as_deref(),
                guard.dashboard_terminal_slice.as_deref(),
            )
            .expect("serialize stale network revision");
        (pending, payload)
    };
    state.pool.close().await;

    hub.materialize_dashboard_network_slice(newer.clone()).await;
    hub.commit_dashboard_materialized_frame(stale_pending, stale_payload)
        .await
        .expect("stale materialization commit should be harmless");
    hub.materialize_dashboard_network_slice(older).await;

    let guard = hub.state.lock().await;
    assert_eq!(
        guard
            .dashboard_network_slice
            .as_ref()
            .map(|slice| slice.revision),
        Some(newer.revision),
        "a late slice must not regress the hub dependency revision",
    );
    let cached = guard
        .topics
        .get(&topic_key)
        .expect("cached network recent topic");
    assert_eq!(
        cached.dashboard_materialized_revision,
        Some(DashboardTopicRevision {
            base_revision: cached.dashboard_base_revision,
            current_revision: None,
            network_revision: Some(newer.revision),
            terminal_revision: None,
            routing_revision: 0,
        }),
        "a delayed frame must not replace the latest materialized revision",
    );
    assert_eq!(
        cached.cursor,
        cursor_before + 1,
        "only the newest revision should advance the SSE cursor",
    );
    assert_eq!(
        cached.snapshot_frame.payload_value()["rangeEnd"],
        json!(newer.recent.range_end),
    );
}
