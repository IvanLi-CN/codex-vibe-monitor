
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

fn seeded_cached_topic(
    topic: SubscriptionTopic,
    cursors: &[u64],
    emitted_at: DateTime<Utc>,
) -> CachedSubscriptionTopic {
    let descriptor = topic.descriptor();
    let schema_epoch = topic.schema_epoch();
    let replay_events = cursors
        .iter()
        .map(|cursor| test_replay_event(&descriptor, &schema_epoch, *cursor, emitted_at, 32))
        .collect::<VecDeque<_>>();
    let replay_bytes = replay_events.iter().map(|event| event.bytes).sum::<usize>();
    let cursor = cursors.last().copied().unwrap_or(0);

    let snapshot_payload = json!({ "cursor": cursor });
    let snapshot_frame = Arc::new(
        serialize_topic_frame(
            descriptor.clone(),
            topic.cache_key().expect("seeded topic key"),
            schema_epoch.clone(),
            cursor,
            serde_json::to_vec(&snapshot_payload).expect("seeded payload"),
        )
        .expect("seeded frame"),
    );
    CachedSubscriptionTopic {
        topic,
        descriptor,
        schema_epoch,
        cursor,
        snapshot_built_at: Instant::now(),
        refresh_scheduled: false,
        conversation_overview_refresh_scheduled: false,
        conversation_overview_refresh_in_flight: false,
        conversation_overview_refresh_pending: false,
        upstream_account_attempt_refresh_scheduled: false,
        upstream_account_attempt_refresh_in_flight: false,
        upstream_account_attempt_refresh_pending: false,
        upstream_account_attempt_refresh_generation: 0,
        dirty: false,
        runtime_topic_recovery_generation: 0,
        runtime_topic_recovery_retry_at: None,
        summary_refresh_scheduled: false,
        summary_refresh_in_flight: false,
        summary_pending_event_count: 0,
        summary_retry_backoff_ms: 0,
        parallel_work_refresh_scheduled: false,
        prompt_cache_refresh_scheduled: false,
        prompt_cache_reconcile_scheduled: false,
        prompt_cache_key_hydration_scheduled: false,
        prompt_cache_pending_records: BTreeMap::new(),
        prompt_cache_pending_key_hydrations: BTreeSet::new(),
        prompt_cache_candidate_refill_required: false,
        prompt_cache_applied_terminal_ids: HashSet::new(),
        prompt_cache_coalesced_event_count: 0,
        prompt_cache_full_hydration_count: 0,
        prompt_cache_bounded_key_hydration_count: 0,
        prompt_cache_baseline_at: None,
        prompt_cache_baseline_row_id: 0,
        prompt_cache_response_source: "memory",
        prompt_cache_reconcile_required: false,
        prompt_cache_pressure_deferred: false,
        latest_live_snapshot: None,
        calendar_anchor: None,
        continuity_reset_cursor: None,
        dashboard_materializer: None,
        dashboard_base_revision: cursor,
        dashboard_materialized_revision: None,
        snapshot_payload,
        snapshot_frame,
        snapshot_bytes: 32,
        replay_events,
        replay_bytes,
    }
}

fn test_replay_event(
    descriptor: &SubscriptionTopicDescriptor,
    schema_epoch: &str,
    cursor: u64,
    emitted_at: DateTime<Utc>,
    bytes: usize,
) -> ReplayableTopicEvent {
    let topic = SubscriptionTopic::from_descriptor(descriptor).expect("test topic");
    let frame = serialize_topic_frame(
        descriptor.clone(),
        topic.cache_key().expect("test topic key"),
        schema_epoch.to_string(),
        cursor,
        serde_json::to_vec(&json!({ "cursor": cursor })).expect("test payload"),
    )
    .expect("test frame");
    ReplayableTopicEvent {
        frame: Arc::new(frame),
        bytes,
        emitted_at,
    }
}

#[test]
fn descriptor_round_trip_canonicalizes_sorted_params() {
    let descriptor = SubscriptionTopicDescriptor {
        topic: "stats.summary.current".to_string(),
        params: BTreeMap::from([
            ("timeZone".to_string(), "Asia/Shanghai".to_string()),
            ("window".to_string(), "current".to_string()),
            ("limit".to_string(), "20".to_string()),
        ]),
    };

    let topic = SubscriptionTopic::from_descriptor(&descriptor).expect("topic should parse");
    let canonical = topic.descriptor();

    assert_eq!(canonical.topic, "stats.summary.current");
    assert_eq!(
        canonical.params.get("window").map(String::as_str),
        Some("current")
    );
    assert_eq!(
        canonical.params.get("timeZone").map(String::as_str),
        Some("Asia/Shanghai")
    );
    assert_eq!(
        canonical.params.get("limit").map(String::as_str),
        Some("20")
    );
}

#[test]
fn upstream_account_attempt_topic_descriptor_matches_rest_pagination_and_filters() {
    let descriptor = SubscriptionTopicDescriptor {
        topic: "upstream-account-attempts.window".to_string(),
        params: BTreeMap::from([
            ("accountId".to_string(), " 42 ".to_string()),
            ("page".to_string(), "0".to_string()),
            ("pageSize".to_string(), "17".to_string()),
            ("type".to_string(), "REMOTE_V2".to_string()),
            ("model".to_string(), " gpt-5.6-sol ".to_string()),
            ("stickyKey".to_string(), " sticky-a ".to_string()),
        ]),
    };
    let topic = SubscriptionTopic::from_descriptor(&descriptor)
        .expect("account attempt topic should parse");
    assert_eq!(
        topic.descriptor(),
        SubscriptionTopicDescriptor {
            topic: "upstream-account-attempts.window".to_string(),
            params: BTreeMap::from([
                ("accountId".to_string(), "42".to_string()),
                ("page".to_string(), "1".to_string()),
                ("pageSize".to_string(), "20".to_string()),
                ("type".to_string(), "remote_v2".to_string()),
                ("model".to_string(), "gpt-5.6-sol".to_string()),
                ("stickyKey".to_string(), "sticky-a".to_string()),
            ]),
        }
    );
    assert_eq!(topic.schema_epoch(), "upstream-account-attempts.window/v1");
    assert!(
        SubscriptionTopic::from_descriptor(&SubscriptionTopicDescriptor {
            topic: "upstream-account-attempts.window".to_string(),
            params: BTreeMap::from([(String::from("accountId"), String::from("0"))]),
        })
        .is_err()
    );
    assert!(
        SubscriptionTopic::from_descriptor(&SubscriptionTopicDescriptor {
            topic: "upstream-account-attempts.window".to_string(),
            params: BTreeMap::from([(String::from("accountId"), String::from("-1"))]),
        })
        .is_err()
    );
}

async fn insert_upstream_account_attempt_fixture(
    state: &AppState,
    account_id: i64,
    invoke_id: &str,
    sticky_key: Option<&str>,
    request_model: Option<&str>,
) -> ApiPoolUpstreamRequestAttempt {
    let occurred_at = crate::proxy::shanghai_now_string();
    crate::proxy::insert_pool_upstream_request_attempt(
        &state.pool,
        &crate::proxy::PoolUpstreamAttemptTraceContext {
            invoke_id: invoke_id.to_string(),
            occurred_at: occurred_at.clone(),
            endpoint: "/v1/responses".to_string(),
            sticky_key: sticky_key.map(ToOwned::to_owned),
            requester_ip: None,
            upstream_base_url_host: None,
            request_model: request_model.map(ToOwned::to_owned),
        },
        Some(account_id),
        Some("https://api.openai.com"),
        1,
        1,
        0,
        Some(&occurred_at),
        Some(&occurred_at),
        "success",
        Some("completed"),
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
    )
    .await
    .expect("insert upstream account attempt fixture");

    crate::api::slices::query_pool_attempt_records_from_live(&state.pool, invoke_id)
        .await
        .expect("load upstream account attempt fixture")
        .into_iter()
        .next()
        .expect("fixture attempt should exist")
}

#[tokio::test]
async fn upstream_account_attempt_topic_payload_matches_rest_reader() {
    let state = crate::tests::test_state_with_openai_base(
        Url::parse("http://127.0.0.1:9").expect("valid test URL"),
    )
    .await;
    insert_upstream_account_attempt_fixture(
        state.as_ref(),
        42,
        "account-attempt-topic-parity",
        Some("sticky-parity"),
        Some("gpt-5.6-sol"),
    )
    .await;
    let rest_query = ListUpstreamAccountAttemptsQuery {
        attempt_type: Some(" NORMAL ".to_string()),
        model: Some(" gpt-5.6-sol ".to_string()),
        sticky_key: Some(" sticky-parity ".to_string()),
        page: Some(0),
        page_size: Some(17),
    };
    let rest = load_upstream_account_attempt_page_from_query(state.as_ref(), 42, &rest_query)
        .await
        .expect("build REST account attempt page");
    let topic = SubscriptionTopic::from_descriptor(&SubscriptionTopicDescriptor {
        topic: "upstream-account-attempts.window".to_string(),
        params: BTreeMap::from([
            ("accountId".to_string(), "42".to_string()),
            ("page".to_string(), "0".to_string()),
            ("pageSize".to_string(), "17".to_string()),
            ("type".to_string(), " NORMAL ".to_string()),
            ("model".to_string(), " gpt-5.6-sol ".to_string()),
            ("stickyKey".to_string(), " sticky-parity ".to_string()),
        ]),
    })
    .expect("topic descriptor should normalize");
    let topic_payload = topic
        .build_cached_payload(state)
        .await
        .expect("build topic account attempt page")
        .serialize(None, None, None)
        .expect("serialize topic account attempt page");

    assert_eq!(
        serde_json::from_slice::<Value>(&topic_payload).expect("topic payload JSON"),
        serde_json::to_value(rest).expect("REST payload JSON"),
    );
}

#[test]
fn dashboard_working_conversations_descriptor_enforces_http_pagination_limits() {
    let descriptor = |page_size: &str, recent_invocation_limit: &str| SubscriptionTopicDescriptor {
        topic: "dashboard.working-conversations.current".to_string(),
        params: BTreeMap::from([
            ("pageSize".to_string(), page_size.to_string()),
            (
                "recentInvocationLimit".to_string(),
                recent_invocation_limit.to_string(),
            ),
        ]),
    };

    assert!(SubscriptionTopic::from_descriptor(&descriptor("0", "4")).is_err());
    assert!(SubscriptionTopic::from_descriptor(&descriptor("101", "4")).is_err());
    assert!(SubscriptionTopic::from_descriptor(&descriptor("20", "3")).is_err());
    assert!(SubscriptionTopic::from_descriptor(&descriptor("20", "17")).is_err());

    let valid = descriptor("100", "16");
    assert_eq!(
        SubscriptionTopic::from_descriptor(&valid)
            .expect("valid working conversation descriptor")
            .descriptor(),
        valid
    );
}

#[tokio::test]
async fn working_conversations_snapshot_builder_uses_one_transaction_snapshot() {
    let (state, temp_dir, _) = crate::tests::file_backed_test_state_with_busy_timeout(
        "working-conversations-snapshot",
        Duration::from_secs(1),
    )
    .await;
    sqlx::query("PRAGMA journal_mode = WAL")
        .execute(&state.pool)
        .await
        .expect("enable concurrent reader and writer fixture");
    let occurred_at = format_naive(Utc::now().with_timezone(&Shanghai).naive_local());
    let insert = |invoke_id: &str, prompt_cache_key: &str| {
        sqlx::query(
            r#"
                INSERT INTO codex_invocations (
                    invoke_id, occurred_at, source, status, payload, raw_response
                )
                VALUES (?1, ?2, 'proxy', 'success', ?3, '{}')
                "#,
        )
        .bind(invoke_id.to_string())
        .bind(occurred_at.clone())
        .bind(json!({ "promptCacheKey": prompt_cache_key }).to_string())
    };
    insert("baseline-key", "baseline-key")
        .execute(&state.pool)
        .await
        .expect("insert baseline working conversation");

    let mut transaction = state
        .pool
        .begin()
        .await
        .expect("begin snapshot transaction");
    let baseline_row_id =
        sqlx::query_scalar::<_, i64>("SELECT COALESCE(MAX(id), 0) FROM codex_invocations")
            .fetch_one(transaction.as_mut())
            .await
            .expect("read snapshot row ceiling");
    insert("post-snapshot-key", "post-snapshot-key")
        .execute(&state.pool)
        .await
        .expect("insert post-snapshot working conversation");

    let response = build_prompt_cache_conversations_response_for_request_on_connection(
        state.as_ref(),
        PromptCacheConversationsRequest {
            selection: PromptCacheConversationSelection::ActivityWindowMinutes(
                SUBSCRIPTION_DEFAULT_WORKING_CONVERSATIONS_ACTIVITY_MINUTES,
            ),
            detail_level: PromptCacheConversationDetailLevel::Full,
            recent_invocation_limit: Some(16),
            page_size: Some(20),
            cursor: None,
            snapshot_at: None,
            blocked_binding_filter: None,
        },
        transaction.as_mut(),
        Utc::now(),
        Some(baseline_row_id),
    )
    .await
    .expect("build transaction-pinned working response");

    let keys = response
        .conversations
        .iter()
        .map(|conversation| conversation.prompt_cache_key.as_str())
        .collect::<Vec<_>>();
    assert_eq!(keys, vec!["baseline-key"]);
    let cursor = response.conversations[0]
        .cursor
        .as_deref()
        .expect("working conversation cursor");
    assert_eq!(
        decode_prompt_cache_conversation_cursor(cursor)
            .expect("decode working conversation cursor")
            .3,
        Some(baseline_row_id)
    );
    transaction
        .commit()
        .await
        .expect("commit snapshot transaction");
    state.pool.close().await;
    let _ = fs::remove_dir_all(temp_dir);
}

#[tokio::test]
async fn working_conversations_paginated_snapshot_keeps_pre_snapshot_runtime_after_update() {
    let state = crate::tests::test_state_with_openai_base(
        Url::parse("http://127.0.0.1:9").expect("valid test URL"),
    )
    .await;
    let snapshot_at = Utc::now();
    let before_snapshot = format_naive(
        (snapshot_at - ChronoDuration::seconds(1))
            .with_timezone(&Shanghai)
            .naive_local(),
    );

    let mut running = dashboard_runtime_topology_live_record(&before_snapshot);
    running.id = 0;
    running.invoke_id = "working-runtime-pre-snapshot-update".to_string();
    running.status = Some("running".to_string());
    running.live_phase = Some("streaming".to_string());
    running.prompt_cache_key = Some("working-runtime-pre-snapshot-update-key".to_string());
    running.created_at = format_utc_iso_precise(snapshot_at - ChronoDuration::seconds(1));
    state.proxy_runtime_invocations.upsert(running.clone());

    let mut terminal_update = running;
    terminal_update.status = Some("success".to_string());
    terminal_update.live_phase = None;
    terminal_update.total_tokens = Some(23);
    terminal_update.created_at = format_utc_iso_precise(snapshot_at + ChronoDuration::minutes(1));
    state
        .proxy_runtime_invocations
        .upsert_terminal(terminal_update);

    let mut post_snapshot = dashboard_runtime_topology_live_record(&format_naive(
        (snapshot_at + ChronoDuration::minutes(1))
            .with_timezone(&Shanghai)
            .naive_local(),
    ));
    post_snapshot.id = 0;
    post_snapshot.invoke_id = "working-runtime-post-snapshot".to_string();
    post_snapshot.status = Some("success".to_string());
    post_snapshot.live_phase = None;
    post_snapshot.prompt_cache_key = Some("working-runtime-post-snapshot-key".to_string());
    post_snapshot.created_at = format_utc_iso_precise(snapshot_at + ChronoDuration::minutes(1));
    state.proxy_runtime_invocations.upsert(post_snapshot);

    let response = build_prompt_cache_conversations_response_for_request(
        state.as_ref(),
        PromptCacheConversationsRequest {
            selection: PromptCacheConversationSelection::ActivityWindowMinutes(
                SUBSCRIPTION_DEFAULT_WORKING_CONVERSATIONS_ACTIVITY_MINUTES,
            ),
            detail_level: PromptCacheConversationDetailLevel::Full,
            recent_invocation_limit: Some(16),
            page_size: Some(20),
            cursor: None,
            snapshot_at: Some(format_utc_iso_precise(snapshot_at)),
            blocked_binding_filter: None,
        },
    )
    .await
    .expect("build frozen working conversations page");

    assert_eq!(response.total_matched, Some(1));
    assert_eq!(response.conversations.len(), 1);
    let conversation = &response.conversations[0];
    assert_eq!(
        conversation.prompt_cache_key,
        "working-runtime-pre-snapshot-update-key"
    );
    assert_eq!(conversation.request_count, 1);
    assert_eq!(conversation.total_tokens, 23);
    assert_eq!(conversation.recent_invocations.len(), 1);
}

#[tokio::test]
async fn working_conversations_initial_baseline_preserves_prebaseline_key_for_hydration() {
    let state = crate::tests::test_state_with_openai_base(
        Url::parse("http://127.0.0.1:9").expect("valid test URL"),
    )
    .await;
    let topic = SubscriptionTopic::DashboardWorkingConversationsCurrent {
        page_size: 20,
        recent_invocation_limit: 16,
        blocked_binding_upstream_account_id: None,
        blocked_binding_constraint_source: None,
    };
    let occurred_at = format_naive(Utc::now().with_timezone(&Shanghai).naive_local());
    let mut record = dashboard_runtime_topology_live_record(&occurred_at);
    record.id = 0;
    record.invoke_id = "prebaseline-working-key".to_string();
    record.status = Some("success".to_string());
    record.live_phase = None;
    record.prompt_cache_key = Some("prebaseline-working-key".to_string());
    record.total_tokens = Some(37);
    let delta = PromptCacheTopicDelta::from_record(&record)
        .expect("build prebaseline working delta")
        .expect("working delta");
    let topic_key = topic.cache_key().expect("working conversation topic key");
    state
        .subscription_hub
        .state
        .lock()
        .await
        .prompt_cache_prebaseline_records
        .entry(topic_key)
        .or_default()
        .insert(delta.identity.clone(), delta.clone());

    let cached = state
        .subscription_hub
        .refresh_topic(state.clone(), topic, false)
        .await
        .expect("initial baseline must defer rather than drop a runtime-only key");

    assert!(
        cached
            .prompt_cache_pending_records
            .contains_key(&delta.identity)
    );
    assert!(
        cached
            .prompt_cache_pending_key_hydrations
            .contains("prebaseline-working-key")
    );
    assert!(cached.prompt_cache_key_hydration_scheduled);
}

#[tokio::test]
async fn working_conversations_initial_baseline_dedupes_runtime_overlay_terminal_replay() {
    let state = crate::tests::test_state_with_openai_base(
        Url::parse("http://127.0.0.1:9").expect("valid test URL"),
    )
    .await;
    let topic = SubscriptionTopic::DashboardWorkingConversationsCurrent {
        page_size: 20,
        recent_invocation_limit: 16,
        blocked_binding_upstream_account_id: None,
        blocked_binding_constraint_source: None,
    };
    let occurred_at = format_naive(Utc::now().with_timezone(&Shanghai).naive_local());
    let mut record = dashboard_runtime_topology_live_record(&occurred_at);
    record.id = 0;
    record.invoke_id = "runtime-overlay-pending-terminal".to_string();
    record.status = Some("success".to_string());
    record.live_phase = None;
    record.prompt_cache_key = Some("runtime-overlay-pending-key".to_string());
    record.total_tokens = Some(37);
    record.cost = Some(0.75);
    state.proxy_runtime_invocations.upsert(record.clone());
    let delta = PromptCacheTopicDelta::from_record(&record)
        .expect("build prebaseline runtime terminal delta")
        .expect("working runtime terminal delta");
    let topic_key = topic.cache_key().expect("working conversation topic key");
    state
        .subscription_hub
        .state
        .lock()
        .await
        .prompt_cache_prebaseline_records
        .entry(topic_key)
        .or_default()
        .insert(delta.identity.clone(), delta);

    let cached = state
        .subscription_hub
        .refresh_topic(state.clone(), topic, false)
        .await
        .expect("initial baseline must not double-count its runtime overlay");
    let DashboardTopicMaterializer::WorkingConversations {
        state: materializer,
    } = cached
        .dashboard_materializer
        .as_ref()
        .expect("typed working materializer")
    else {
        panic!("expected typed working conversations materializer");
    };
    let materializer = materializer
        .lock()
        .expect("working conversations materializer state lock");
    let conversation = materializer
        .response
        .conversations
        .iter()
        .find(|conversation| conversation.prompt_cache_key == "runtime-overlay-pending-key")
        .expect("runtime overlay conversation is present");
    assert_eq!(conversation.request_count, 1);
    assert_eq!(conversation.total_tokens, 37);
    assert!((conversation.total_cost - 0.75).abs() < f64::EPSILON);
    assert_eq!(conversation.last24h_requests.len(), 1);
    assert_eq!(conversation.recent_invocations.len(), 1);
}

#[tokio::test]
async fn working_conversations_initial_baseline_dedupes_persisted_runtime_overlay() {
    let state = crate::tests::test_state_with_openai_base(
        Url::parse("http://127.0.0.1:9").expect("valid test URL"),
    )
    .await;
    let topic = SubscriptionTopic::DashboardWorkingConversationsCurrent {
        page_size: 20,
        recent_invocation_limit: 16,
        blocked_binding_upstream_account_id: None,
        blocked_binding_constraint_source: None,
    };
    let occurred_at = format_naive(Utc::now().with_timezone(&Shanghai).naive_local());
    let mut record = dashboard_runtime_topology_live_record(&occurred_at);
    record.id = 0;
    record.invoke_id = "persisted-runtime-overlay-baseline".to_string();
    record.status = Some("success".to_string());
    record.live_phase = None;
    record.prompt_cache_key = Some("persisted-runtime-overlay-key".to_string());
    record.upstream_account_id = Some(77);
    record.upstream_account_name = Some("Persisted Overlay Account".to_string());
    record.total_tokens = Some(37);
    record.cost = Some(0.75);
    state.proxy_runtime_invocations.upsert(record.clone());
    sqlx::query(
        r#"
            INSERT INTO codex_invocations (
                invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response
            ) VALUES (?1, ?2, 'proxy', 'success', 37, 0.75, ?3, '{}')
            "#,
    )
    .bind(&record.invoke_id)
    .bind(&record.occurred_at)
    .bind(
        json!({
            "promptCacheKey": "persisted-runtime-overlay-key",
            "upstreamAccountId": 77,
            "upstreamAccountName": "Persisted Overlay Account",
        })
        .to_string(),
    )
    .execute(&state.pool)
    .await
    .expect("persist terminal before runtime overlay acknowledgement");

    let cached = state
        .subscription_hub
        .refresh_topic(state.clone(), topic, false)
        .await
        .expect("initial baseline must dedupe a persisted runtime overlay");
    let DashboardTopicMaterializer::WorkingConversations {
        state: materializer,
    } = cached
        .dashboard_materializer
        .as_ref()
        .expect("typed working materializer")
    else {
        panic!("expected typed working conversations materializer");
    };
    let materializer = materializer
        .lock()
        .expect("working conversations materializer state lock");
    let conversation = materializer
        .response
        .conversations
        .iter()
        .find(|conversation| conversation.prompt_cache_key == "persisted-runtime-overlay-key")
        .expect("persisted runtime overlay conversation is present");
    assert_eq!(conversation.request_count, 1);
    assert_eq!(conversation.total_tokens, 37);
    assert!((conversation.total_cost - 0.75).abs() < f64::EPSILON);
    assert_eq!(conversation.last24h_requests.len(), 1);
    assert_eq!(conversation.recent_invocations.len(), 1);
    assert_eq!(conversation.upstream_accounts.len(), 1);
    assert_eq!(conversation.upstream_accounts[0].request_count, 1);
}

#[tokio::test]
async fn working_conversations_bounded_hydration_restores_reentered_key_contract() {
    let state = crate::tests::test_state_with_openai_base(
        Url::parse("http://127.0.0.1:9").expect("valid test URL"),
    )
    .await;
    let topic = SubscriptionTopic::DashboardWorkingConversationsCurrent {
        page_size: 20,
        recent_invocation_limit: 16,
        blocked_binding_upstream_account_id: None,
        blocked_binding_constraint_source: None,
    };
    let _lease = state
        .subscription_hub
        .register_topic_subscribers(std::slice::from_ref(&topic))
        .await
        .expect("register working conversations topic");
    state
        .subscription_hub
        .prepare_connection(state.clone(), vec![topic.descriptor()], Vec::new())
        .await
        .expect("build empty working conversations baseline");

    let now = Utc::now();
    let occurred_at = format_naive(now.with_timezone(&Shanghai).naive_local());
    seed_reentered_working_history(&state, now, &occurred_at).await;

    let topic_key = topic.cache_key().expect("working conversation topic key");
    {
        let mut guard = state.subscription_hub.state.lock().await;
        let cached = guard
            .topics
            .get_mut(&topic_key)
            .expect("cached working conversations topic");
        cached
            .prompt_cache_pending_key_hydrations
            .insert("reentered-key".to_string());
        cached.prompt_cache_key_hydration_scheduled = true;
    }
    state
        .subscription_hub
        .hydrate_dashboard_working_conversation_keys(state.clone(), &topic)
        .await
        .expect("hydrate reentered key without a full working-window rebuild");

    let guard = state.subscription_hub.state.lock().await;
    let cached = guard
        .topics
        .get(&topic_key)
        .expect("hydrated working conversations topic");
    assert_reentered_working_key(cached);
}

async fn seed_reentered_working_history(
    state: &Arc<AppState>,
    now: DateTime<Utc>,
    occurred_at: &str,
) {
    for (invoke_id, total_tokens) in [("reentered-older", 11_i64), ("reentered-newer", 31_i64)] {
        sqlx::query(
                r#"INSERT INTO codex_invocations
                   (invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response)
                   VALUES (?1, ?2, 'proxy', 'success', ?3, 0.25, ?4, '{}')"#,
            )
            .bind(invoke_id)
            .bind(occurred_at)
            .bind(total_tokens)
            .bind(json!({
                "promptCacheKey": "reentered-key",
                "upstreamAccountId": 42,
                "upstreamAccountName": "Hydrated Account",
            }).to_string())
            .execute(&state.pool)
            .await
            .expect("persist reentered working conversation history");
    }
    sqlx::query(
        r#"INSERT INTO prompt_cache_upstream_account_hourly
               (bucket_start_epoch, source, prompt_cache_key, upstream_account_key,
                upstream_account_id, upstream_account_name, request_count, success_count,
                failure_count, total_tokens, total_cost, first_seen_at, last_seen_at, updated_at)
               VALUES (?1, 'proxy', 'reentered-key', 'id:42|name:Hydrated Account', 42,
                       'Hydrated Account', 2, 2, 0, 42, 0.5, ?2, ?2, datetime('now'))"#,
    )
    .bind(now.timestamp().div_euclid(3_600) * 3_600)
    .bind(occurred_at)
    .execute(&state.pool)
    .await
    .expect("materialize reentered working conversation account history");
    sqlx::query(
            "INSERT INTO prompt_cache_conversation_bindings (prompt_cache_key, binding_kind, group_name, upstream_account_id, created_at, updated_at) VALUES ('reentered-key', 'group', 'Bounded Group', NULL, datetime('now'), datetime('now'))",
        )
        .execute(&state.pool)
        .await
        .expect("persist reentered key manual binding");
}

fn assert_reentered_working_key(cached: &CachedSubscriptionTopic) {
    let DashboardTopicMaterializer::WorkingConversations {
        state: materializer,
    } = cached
        .dashboard_materializer
        .as_ref()
        .expect("typed working materializer")
    else {
        panic!("expected typed working conversations materializer");
    };
    let materializer = materializer
        .lock()
        .expect("working conversations materializer state lock");
    let conversation = materializer
        .response
        .conversations
        .iter()
        .find(|conversation| conversation.prompt_cache_key == "reentered-key")
        .expect("bounded hydrate must restore the reentered key");
    assert_eq!(conversation.request_count, 2);
    assert_eq!(conversation.total_tokens, 42);
    assert_eq!(conversation.last24h_requests.len(), 2);
    assert_eq!(conversation.recent_invocations.len(), 2);
    assert_eq!(
        conversation
            .manual_binding
            .as_ref()
            .and_then(|binding| binding.group_name.as_deref()),
        Some("Bounded Group")
    );
    assert_eq!(
        conversation.upstream_accounts[0].upstream_account_id,
        Some(42)
    );
    assert_eq!(cached.prompt_cache_full_hydration_count, 1);
    assert_eq!(cached.prompt_cache_bounded_key_hydration_count, 1);
    assert!(cached.prompt_cache_pending_key_hydrations.is_empty());
    assert!(!cached.prompt_cache_reconcile_required);
}

#[tokio::test]
async fn working_conversations_bounded_hydration_refills_active_page_candidates() {
    let state = crate::tests::test_state_with_openai_base(
        Url::parse("http://127.0.0.1:9").expect("valid test URL"),
    )
    .await;
    let topic = SubscriptionTopic::DashboardWorkingConversationsCurrent {
        page_size: 1,
        recent_invocation_limit: 16,
        blocked_binding_upstream_account_id: None,
        blocked_binding_constraint_source: None,
    };
    let _lease = state
        .subscription_hub
        .register_topic_subscribers(std::slice::from_ref(&topic))
        .await
        .expect("register working conversations topic");
    state
        .subscription_hub
        .prepare_connection(state.clone(), vec![topic.descriptor()], Vec::new())
        .await
        .expect("build empty working conversations baseline");

    let now = Utc::now();
    for (invoke_id, prompt_cache_key, occurred_at) in [
        (
            "candidate-older",
            "candidate-older",
            format_naive(
                (now - ChronoDuration::minutes(2))
                    .with_timezone(&Shanghai)
                    .naive_local(),
            ),
        ),
        (
            "candidate-newer",
            "candidate-newer",
            format_naive(
                (now - ChronoDuration::minutes(1))
                    .with_timezone(&Shanghai)
                    .naive_local(),
            ),
        ),
    ] {
        sqlx::query(
            r#"
                INSERT INTO codex_invocations (
                    invoke_id, occurred_at, source, status, total_tokens, payload, raw_response
                ) VALUES (?1, ?2, 'proxy', 'success', 1, ?3, '{}')
                "#,
        )
        .bind(invoke_id)
        .bind(occurred_at)
        .bind(json!({ "promptCacheKey": prompt_cache_key }).to_string())
        .execute(&state.pool)
        .await
        .expect("persist candidate conversation");
    }

    let topic_key = topic.cache_key().expect("working conversation topic key");
    {
        let mut guard = state.subscription_hub.state.lock().await;
        let cached = guard
            .topics
            .get_mut(&topic_key)
            .expect("cached working conversations topic");
        cached.prompt_cache_candidate_refill_required = true;
        cached.prompt_cache_key_hydration_scheduled = true;
    }
    state
        .subscription_hub
        .hydrate_dashboard_working_conversation_keys(state.clone(), &topic)
        .await
        .expect("refill active page candidates with bounded hydration");

    let guard = state.subscription_hub.state.lock().await;
    let cached = guard
        .topics
        .get(&topic_key)
        .expect("hydrated working conversations topic");
    assert_refilled_working_page(cached);
}

fn assert_refilled_working_page(cached: &CachedSubscriptionTopic) {
    let DashboardTopicMaterializer::WorkingConversations {
        state: materializer,
    } = cached
        .dashboard_materializer
        .as_ref()
        .expect("typed working materializer")
    else {
        panic!("expected typed working conversations materializer");
    };
    let materializer = materializer
        .lock()
        .expect("working conversations materializer state lock");
    assert_eq!(materializer.response.total_matched, Some(2));
    assert!(materializer.response.has_more);
    assert_eq!(materializer.response.conversations.len(), 1);
    assert_eq!(
        materializer.response.conversations[0].prompt_cache_key,
        "candidate-newer"
    );
    assert!(!cached.prompt_cache_candidate_refill_required);
    assert_eq!(cached.prompt_cache_bounded_key_hydration_count, 1);
    assert_eq!(cached.prompt_cache_full_hydration_count, 1);
    assert!(!cached.prompt_cache_reconcile_required);
}

#[tokio::test]
async fn working_conversations_hydration_keeps_an_unresolved_eligible_delta_for_recovery() {
    let state = crate::tests::test_state_with_openai_base(
        Url::parse("http://127.0.0.1:9").expect("valid test URL"),
    )
    .await;
    let topic = SubscriptionTopic::DashboardWorkingConversationsCurrent {
        page_size: 20,
        recent_invocation_limit: 16,
        blocked_binding_upstream_account_id: None,
        blocked_binding_constraint_source: None,
    };
    let _lease = state
        .subscription_hub
        .register_topic_subscribers(std::slice::from_ref(&topic))
        .await
        .expect("register working conversations topic");
    state
        .subscription_hub
        .prepare_connection(state.clone(), vec![topic.descriptor()], Vec::new())
        .await
        .expect("build empty working conversations baseline");

    let occurred_at = format_naive(Utc::now().with_timezone(&Shanghai).naive_local());
    let mut record = dashboard_runtime_topology_live_record(&occurred_at);
    record.invoke_id = "unresolved-working-hydration".to_string();
    record.status = Some("success".to_string());
    record.live_phase = None;
    record.prompt_cache_key = Some("unresolved-working-hydration".to_string());
    let delta = PromptCacheTopicDelta::from_record(&record)
        .expect("build unresolved working delta")
        .expect("working delta");
    let topic_key = topic.cache_key().expect("working conversation topic key");
    {
        let mut guard = state.subscription_hub.state.lock().await;
        let cached = guard
            .topics
            .get_mut(&topic_key)
            .expect("cached working conversations topic");
        cached
            .prompt_cache_pending_records
            .insert(delta.identity.clone(), delta.clone());
        cached
            .prompt_cache_pending_key_hydrations
            .insert("unresolved-working-hydration".to_string());
        cached.prompt_cache_key_hydration_scheduled = true;
    }

    state
        .subscription_hub
        .hydrate_dashboard_working_conversation_keys(state.clone(), &topic)
        .await
        .expect("attempt bounded key hydration");

    let guard = state.subscription_hub.state.lock().await;
    let cached = guard
        .topics
        .get(&topic_key)
        .expect("cached working conversations topic");
    assert!(
        cached.dirty,
        "an eligible delta absent from the bounded hydration result must enter recovery"
    );
    assert!(cached.prompt_cache_reconcile_required);
    assert!(
        cached
            .prompt_cache_pending_records
            .contains_key(&delta.identity),
        "recovery must retain the unresolved eligible delta instead of silently dropping it"
    );
}

#[tokio::test]
async fn working_conversations_bounded_hydration_keeps_runtime_terminal_details() {
    let state = crate::tests::test_state_with_openai_base(
        Url::parse("http://127.0.0.1:9").expect("valid test URL"),
    )
    .await;
    let range_end = Utc::now();
    let occurred_at = format_naive(range_end.with_timezone(&Shanghai).naive_local());
    let mut record = dashboard_runtime_topology_live_record(&occurred_at);
    record.id = 0;
    record.invoke_id = "runtime-only-working-hydration".to_string();
    record.status = Some("success".to_string());
    record.live_phase = None;
    record.prompt_cache_key = Some("runtime-only-working-hydration".to_string());
    record.upstream_account_id = Some(77);
    record.upstream_account_name = Some("Runtime Hydration Account".to_string());
    record.total_tokens = Some(37);
    record.cost = Some(0.75);
    state.proxy_runtime_invocations.upsert(record);

    let source_scope = resolve_default_source_scope(&state.pool)
        .await
        .expect("resolve default source scope");
    let range_start_bound = db_occurred_at_lower_bound(
        range_end
            - ChronoDuration::minutes(SUBSCRIPTION_DEFAULT_WORKING_CONVERSATIONS_ACTIVITY_MINUTES),
    );
    let conversation = hydrate_working_prompt_cache_conversation_for_key(
        state.as_ref(),
        source_scope,
        "runtime-only-working-hydration",
        range_end,
        &range_start_bound,
        16,
        None,
    )
    .await
    .expect("hydrate runtime-only working conversation")
    .expect("runtime-only terminal key is selected");

    assert_eq!(conversation.request_count, 1);
    assert_eq!(conversation.total_tokens, 37);
    assert_eq!(conversation.last24h_requests.len(), 1);
    assert_eq!(conversation.last24h_requests[0].request_tokens, 37);
    assert_eq!(conversation.upstream_accounts.len(), 1);
    assert_eq!(
        conversation.upstream_accounts[0].upstream_account_id,
        Some(77)
    );
    assert_eq!(conversation.upstream_accounts[0].request_count, 1);
}

#[tokio::test]
async fn working_conversations_bounded_hydration_dedupes_persisted_runtime_overlay() {
    let state = crate::tests::test_state_with_openai_base(
        Url::parse("http://127.0.0.1:9").expect("valid test URL"),
    )
    .await;
    let range_end = Utc::now();
    let occurred_at = format_naive(range_end.with_timezone(&Shanghai).naive_local());
    let mut record = dashboard_runtime_topology_live_record(&occurred_at);
    record.id = 0;
    record.invoke_id = "persisted-runtime-overlay-hydration".to_string();
    record.status = Some("success".to_string());
    record.live_phase = None;
    record.prompt_cache_key = Some("persisted-runtime-overlay-hydration-key".to_string());
    record.upstream_account_id = Some(78);
    record.upstream_account_name = Some("Persisted Hydration Account".to_string());
    record.total_tokens = Some(41);
    record.cost = Some(0.5);
    state.proxy_runtime_invocations.upsert(record.clone());
    sqlx::query(
        r#"
            INSERT INTO codex_invocations (
                invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response
            ) VALUES (?1, ?2, 'proxy', 'success', 41, 0.5, ?3, '{}')
            "#,
    )
    .bind(&record.invoke_id)
    .bind(&record.occurred_at)
    .bind(
        json!({
            "promptCacheKey": "persisted-runtime-overlay-hydration-key",
            "upstreamAccountId": 78,
            "upstreamAccountName": "Persisted Hydration Account",
        })
        .to_string(),
    )
    .execute(&state.pool)
    .await
    .expect("persist terminal before bounded hydration acknowledgement");

    let source_scope = resolve_default_source_scope(&state.pool)
        .await
        .expect("resolve default source scope");
    let range_start_bound = db_occurred_at_lower_bound(
        range_end
            - ChronoDuration::minutes(SUBSCRIPTION_DEFAULT_WORKING_CONVERSATIONS_ACTIVITY_MINUTES),
    );
    let conversation = hydrate_working_prompt_cache_conversation_for_key(
        state.as_ref(),
        source_scope,
        "persisted-runtime-overlay-hydration-key",
        range_end,
        &range_start_bound,
        16,
        None,
    )
    .await
    .expect("hydrate persisted runtime overlay working conversation")
    .expect("persisted runtime overlay key is selected");

    assert_eq!(conversation.request_count, 1);
    assert_eq!(conversation.total_tokens, 41);
    assert!((conversation.total_cost - 0.5).abs() < f64::EPSILON);
    assert_eq!(conversation.last24h_requests.len(), 1);
    assert_eq!(conversation.recent_invocations.len(), 1);
    assert_eq!(conversation.upstream_accounts.len(), 1);
    assert_eq!(conversation.upstream_accounts[0].request_count, 1);
}
