use super::*;
use serde_json::json;
#[tokio::test]
async fn first_dashboard_owner_starts_pending_runtime_projection_producer() {
    let state = crate::tests::test_state_with_openai_base(
        Url::parse("http://127.0.0.1:9").expect("valid test URL"),
    )
    .await;
    hydrate_summary_snapshots(state.as_ref())
        .await
        .expect("hydrate Summary baseline before Dashboard subscription setup");
    let topic = SubscriptionTopic::DashboardActivityCurrent {
        range: "today".to_string(),
        time_zone: SUBSCRIPTION_DEFAULT_TIME_ZONE.to_string(),
        recent_limit: 16,
        include_accounts: true,
        include_recent: true,
    };
    let selected_topics = vec![topic.clone(), summary_topic()];
    state.dashboard_network_speed_cache.record_request_bytes(
        "network-before-owner",
        "2026-08-04 12:00:00",
        Some(42),
        Some("api.openai.com"),
        128,
        Utc::now(),
    );
    schedule_dashboard_activity_live_snapshot(state.as_ref());
    assert!(
        state
            .proxy_runtime_invocations
            .pending_dashboard_deadline()
            .is_some()
    );
    let _lease = state
        .subscription_hub
        .register_topic_subscribers(&selected_topics)
        .await
        .expect("register first dashboard owner");
    assert!(
        state
            .subscription_hub
            .has_active_dashboard_activity_live_topic()
            .await
    );
    assert_eq!(
        state
            .subscription_hub
            .dashboard_activity_live_subscriber_count()
            .await,
        1
    );
    let prepared = state
        .subscription_hub
        .prepare_connection(
            state.clone(),
            selected_topics
                .iter()
                .map(SubscriptionTopic::descriptor)
                .collect(),
            Vec::new(),
        )
        .await
        .expect("prepare first dashboard owner connection");
    assert!(!prepared.initial.is_empty());
    ensure_dashboard_activity_live_snapshot_producer(state.as_ref());
}
#[tokio::test]
async fn dashboard_runtime_topology_materializes_shared_frames_without_business_payloads() {
    let state = crate::tests::test_state_with_openai_base(
        Url::parse("http://127.0.0.1:9").expect("valid test URL"),
    )
    .await;
    hydrate_summary_snapshots(state.as_ref())
        .await
        .expect("hydrate Summary baseline before Dashboard topology setup");
    state
        .proxy_runtime_invocations
        .bind_dashboard_network_speed_cache(state.dashboard_network_speed_cache.clone())
        .expect("bind dashboard network cache");
    state
        .proxy_runtime_invocations
        .capture_memory_snapshot()
        .expect("establish in-memory dashboard projection");
    if let Some(window) = state
        .proxy_runtime_invocations
        .pending_dashboard_publish_window()
    {
        let consumed = state
            .proxy_runtime_invocations
            .begin_dashboard_publish_window(window)
            .expect("consume initial dashboard projection window");
        state
            .proxy_runtime_invocations
            .complete_dashboard_publish_window(consumed);
    }
    let descriptors = dashboard_runtime_topology_descriptors();
    let topics = descriptors
        .iter()
        .map(SubscriptionTopic::from_descriptor)
        .collect::<Result<Vec<_>, _>>()
        .expect("dashboard topology topics");
    let first_response = topic_sse_stream(
        State(state.clone()),
        Query(dashboard_runtime_topology_stream_query(1)),
    )
    .await
    .expect("open first full Dashboard SSE topology");
    let second_response = topic_sse_stream(
        State(state.clone()),
        Query(dashboard_runtime_topology_stream_query(2)),
    )
    .await
    .expect("open second full Dashboard SSE topology");
    for topic in &topics {
        assert_eq!(
            state
                .subscription_hub
                .active_topic_subscriber_count(topic.name())
                .await,
            2,
            "SSE entrypoint should retain both Dashboard owners for {}",
            topic.name(),
        );
    }
    for topic in &topics {
        let expected_typed_materializer = matches!(
            topic.name(),
            "dashboard.activity.current"
                | "stats.summary.current"
                | "dashboard.network-timeseries.window"
                | "dashboard.network-recent.current"
                | "stats.timeseries.open-window"
                | "stats.parallel-work.current"
                | "dashboard.working-conversations.current"
        );
        assert_eq!(
            state
                .subscription_hub
                .dashboard_topic_uses_typed_materializer(topic)
                .await,
            expected_typed_materializer,
            "typed materializer seam must report the current implementation for {}",
            topic.name(),
        );
    }
    {
        let guard = state.subscription_hub.state.lock().await;
        let working_topic_key = topics
            .iter()
            .find(|topic| topic.name() == "dashboard.working-conversations.current")
            .expect("working conversations topic")
            .cache_key()
            .expect("working conversations topic key");
        assert_eq!(
            guard
                .server_push_subscribers
                .get(&working_topic_key)
                .copied(),
            Some(2),
            "working conversations owns its bounded expiry and reconcile cadence",
        );
        assert!(
            guard.server_push_tasks.contains(&working_topic_key),
            "working conversations must retain its bounded expiry task",
        );
    }
    spawn_subscription_broadcast_listener(state.clone());
    let process_started_epoch_second = state
        .dashboard_network_speed_cache
        .process_started_at_utc()
        .timestamp();
    while Utc::now().timestamp() <= process_started_epoch_second.saturating_add(1) {
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    state
        .proxy_runtime_invocations
        .reset_dashboard_topology_counters();
    state.subscription_hub.reset_dashboard_topology_counters();
    state
        .subscription_hub
        .reset_dashboard_topology_sse_frame_observations()
        .await;
    let occurred_at = format_naive(Utc::now().with_timezone(&Shanghai).naive_local());
    state
        .proxy_runtime_invocations
        .upsert(dashboard_runtime_topology_live_record(&occurred_at));
    let current_capture = state
        .proxy_runtime_invocations
        .capture_memory_snapshot()
        .expect("capture deterministic Dashboard current slice");
    assert!(current_capture.changed);
    state
        .subscription_hub
        .handle_internal_broadcast(
            state.clone(),
            BroadcastPayload::DashboardCurrentSlice {
                slice: Box::new(DashboardCurrentProjectionSlice::from(
                    &current_capture.snapshot,
                )),
            },
        )
        .await;
    let parallel_topic = SubscriptionTopic::ParallelWorkCurrent {
        range: "1d".to_string(),
        time_zone: SUBSCRIPTION_DEFAULT_TIME_ZONE.to_string(),
        bucket: Some("1m".to_string()),
        upstream_account_id: None,
    };
    let parallel_base = {
        let guard = state.subscription_hub.state.lock().await;
        let key = parallel_topic.cache_key().expect("parallel-work topic key");
        match guard.topics[&key].dashboard_materializer.as_ref() {
            Some(DashboardTopicMaterializer::ParallelWork { base }) => base.clone(),
            _ => panic!("expected parallel-work materializer"),
        }
    };
    {
        let mut parallel_base = parallel_base
            .lock()
            .expect("parallel-work materializer state lock");
        parallel_base.response.current.range_start =
            format_utc_iso(Utc::now() - ChronoDuration::minutes(2));
        assert!(
            parallel_base.requires_rolling_rebase(),
            "parallel materializer fixture must cross the rolling rebase boundary",
        );
    }
    let mut terminal = dashboard_runtime_topology_live_record(&occurred_at);
    terminal.id = 748_003;
    terminal.invoke_id = "dashboard-runtime-topology-terminal".to_string();
    terminal.status = Some("success".to_string());
    terminal.live_phase = None;
    terminal.total_tokens = Some(42);
    terminal.output_tokens = Some(16);
    terminal.cost = Some(0.25);
    terminal.t_total_ms = Some(3_450.0);
    terminal.t_req_read_ms = Some(10.0);
    terminal.t_req_parse_ms = Some(12.0);
    terminal.t_upstream_connect_ms = Some(18.0);
    terminal.t_upstream_ttfb_ms = Some(508.0);
    terminal.first_token_ms = Some(650.0);
    terminal.t_upstream_stream_ms = Some(2_800.0);
    let terminal_delta = apply_dashboard_activity_terminal_record(state.as_ref(), &terminal)
        .await
        .terminal_delta
        .expect("accept terminal projection delta");
    state
        .proxy_runtime_invocations
        .record_dashboard_terminal_delta(terminal_delta);
    let terminal_capture = state
        .proxy_runtime_invocations
        .capture_terminal_slice()
        .expect("capture one terminal projection slice");
    state
        .subscription_hub
        .materialize_dashboard_terminal_slice(DashboardTerminalProjectionSlice {
            revision: terminal_capture.revision,
            deltas: terminal_capture.deltas,
        })
        .await;
    // Match the production boundary rebase before applying later live mutations.
    tokio::time::timeout(
        Duration::from_secs(5),
        state
            .subscription_hub
            .reconcile_dashboard_terminal_window_bases(state.clone()),
    )
    .await
    .expect("dashboard terminal window rebase timed out");
    let guard = state.subscription_hub.state.lock().await;
    let parallel_cached =
        &guard.topics[&parallel_topic.cache_key().expect("parallel-work topic key")];
    assert!(
        !guard.topics.values().any(|cached| cached.dirty)
            && parallel_cached
                .dashboard_materializer
                .as_ref()
                .is_some_and(|materializer| !materializer.requires_terminal_window_rebase()),
        "terminal rebase must clear dirty bases and refresh the parallel materializer",
    );
    drop(guard);
    let network_observed_at = Utc::now();
    state.dashboard_network_speed_cache.record_request_bytes(
        "dashboard-runtime-topology-network",
        &occurred_at,
        Some(42),
        Some("api.openai.com"),
        4096,
        network_observed_at,
    );
    let network_capture = state
        .proxy_runtime_invocations
        .capture_network_slice()
        .expect("capture deterministic Dashboard network slice");
    assert!(network_capture.changed);
    state
        .subscription_hub
        .handle_internal_broadcast(
            state.clone(),
            BroadcastPayload::DashboardNetworkSlice {
                slice: Box::new(network_capture.slice),
            },
        )
        .await;
    let mut fallback = terminal.clone();
    fallback.id = 748_004;
    fallback.invoke_id = "dashboard-runtime-topology-fallback".to_string();
    fallback.occurred_at = format_naive(
        (Utc::now() - ChronoDuration::minutes(1))
            .with_timezone(&Shanghai)
            .naive_local(),
    );
    fallback.created_at = fallback.occurred_at.clone();
    fallback.prompt_cache_key = Some("dashboard-runtime-topology-fallback".to_string());
    sqlx::query(
            r#"
            INSERT INTO codex_invocations (
                id, invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
            "#,
        )
        .bind(fallback.id)
        .bind(fallback.invoke_id.as_str())
        .bind(fallback.occurred_at.as_str())
        .bind(fallback.source.as_str())
        .bind("success")
        .bind(42_i64)
        .bind(0.25_f64)
        .bind(
            json!({
                "promptCacheKey": fallback.prompt_cache_key.as_deref(),
                "upstreamAccountId": fallback.upstream_account_id,
            })
            .to_string(),
        )
        .bind("{}")
        .execute(&state.pool)
        .await
        .expect("persist generic fallback benchmark invocation");
    let mut second_parallel_work = fallback.clone();
    second_parallel_work.id = 748_005;
    second_parallel_work.invoke_id = "dashboard-runtime-topology-second-parallel-work".to_string();
    second_parallel_work.prompt_cache_key = Some("dashboard-runtime-topology-second".to_string());
    sqlx::query(
            r#"
            INSERT INTO codex_invocations (
                id, invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
            "#,
        )
        .bind(second_parallel_work.id)
        .bind(second_parallel_work.invoke_id.as_str())
        .bind(second_parallel_work.occurred_at.as_str())
        .bind(second_parallel_work.source.as_str())
        .bind("success")
        .bind(42_i64)
        .bind(0.25_f64)
        .bind(
            json!({
                "promptCacheKey": second_parallel_work.prompt_cache_key.as_deref(),
                "upstreamAccountId": second_parallel_work.upstream_account_id,
            })
            .to_string(),
        )
        .bind("{}")
        .execute(&state.pool)
        .await
        .expect("persist second parallel-work benchmark invocation");
    crate::tests::complete_prompt_cache_conversation_materialization_for_test(&state.pool).await;
    let mut parallel_work_mutations = Vec::with_capacity(10_000);
    parallel_work_mutations.push(SequencedRuntimeMutation {
        sequence: 1,
        mutation: RuntimeMutation::invocation(&fallback, RuntimeMutationKind::RuntimeUpsert),
    });
    parallel_work_mutations.push(SequencedRuntimeMutation {
        sequence: 2,
        mutation: RuntimeMutation::invocation(
            &second_parallel_work,
            RuntimeMutationKind::RuntimeUpsert,
        ),
    });
    parallel_work_mutations.extend((3..=10_000).map(|sequence| SequencedRuntimeMutation {
        sequence,
        mutation: RuntimeMutation::invocation(&fallback, RuntimeMutationKind::RuntimeUpsert),
    }));
    state.proxy_runtime_invocations.upsert(fallback.clone());
    state
        .subscription_hub
        .handle_runtime_mutation_batch(state.clone(), parallel_work_mutations)
        .await;
    let exact_parallel = load_parallel_work_stats_response(
        &state,
        ParallelWorkStatsQuery {
            range: "1d".to_string(),
            bucket: Some("1m".to_string()),
            time_zone: Some(SUBSCRIPTION_DEFAULT_TIME_ZONE.to_string()),
            upstream_account_id: None,
        },
    )
    .await
    .expect("build exact parallel-work response");
    let fallback_occurred_at =
        parse_to_utc_datetime(&fallback.occurred_at).expect("parse fallback timestamp");
    let fallback_bucket_start = format_utc_iso(
        Utc.timestamp_opt(fallback_occurred_at.timestamp().div_euclid(60) * 60, 0)
            .single()
            .expect("construct fallback minute"),
    );
    let exact_point = exact_parallel
        .current
        .points
        .iter()
        .find(|point| point.bucket_start == fallback_bucket_start)
        .expect("exact fallback point");
    let projection_deadline = tokio::time::Instant::now() + Duration::from_secs(15);
    let projected_parallel = loop {
        let projected_parallel = {
            let guard = state.subscription_hub.state.lock().await;
            guard.topics[&parallel_topic.cache_key().expect("parallel-work topic key")]
                .snapshot_frame
                .payload_value()
        };
        let materialized_point =
            projected_parallel["current"]["points"]
                .as_array()
                .and_then(|points| {
                    points.iter().find(|point| {
                        point["bucketStart"].as_str() == Some(fallback_bucket_start.as_str())
                    })
                });
        if materialized_point
            .is_some_and(|point| point["parallelCount"] == json!(exact_point.parallel_count))
        {
            break projected_parallel;
        }
        let remaining = projection_deadline.saturating_duration_since(tokio::time::Instant::now());
        assert!(
            !remaining.is_zero(),
            "parallel-work projection did not materialize the expected bucket"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    };
    let projected_point = projected_parallel["current"]["points"]
        .as_array()
        .and_then(|points| {
            points
                .iter()
                .find(|point| point["bucketStart"].as_str() == Some(fallback_bucket_start.as_str()))
        })
        .expect("projected fallback point");
    assert_eq!(
        projected_point["parallelCount"],
        json!(exact_point.parallel_count),
        "runtime projection must preserve the exact distinct-key bucket count"
    );
    let projected_conversation = projected_parallel["current"]["conversations"]
        .as_array()
        .and_then(|conversations| {
            conversations.iter().find(|conversation| {
                conversation["conversationId"].as_str() == fallback.prompt_cache_key.as_deref()
            })
        })
        .expect("projected fallback conversation");
    let exact_conversation = exact_parallel
        .current
        .conversations
        .iter()
        .find(|conversation| {
            Some(conversation.conversation_id.as_str()) == fallback.prompt_cache_key.as_deref()
        })
        .expect("exact fallback conversation");
    assert_eq!(
        projected_conversation,
        &serde_json::to_value(exact_conversation).expect("serialize exact conversation"),
        "runtime projection must preserve the exact conversation span"
    );
    let delivery_before_reconnect = state.subscription_hub.dashboard_topology_counters();
    for topic in [
        delivery_before_reconnect.activity,
        delivery_before_reconnect.summary,
        delivery_before_reconnect.network_timeseries,
        delivery_before_reconnect.network_recent,
        delivery_before_reconnect.working_conversations,
        delivery_before_reconnect.parallel_work,
        delivery_before_reconnect.timeseries,
    ] {
        assert_eq!(
            topic.active_subscriber_count, 2,
            "the live Dashboard topology must retain two subscribers before reconnect"
        );
        assert_eq!(
            topic.reconnect_churn_count, 0,
            "runtime mutations must not create reconnect churn"
        );
    }
    let resume = {
        let guard = state.subscription_hub.state.lock().await;
        topics
            .iter()
            .map(|topic| {
                let topic_key = topic.cache_key().expect("Dashboard topic key");
                let cached = guard
                    .topics
                    .get(&topic_key)
                    .expect("cached Dashboard topic");
                SubscriptionResumeCursor {
                    topic_key,
                    cursor: cached.cursor,
                    schema_epoch: cached.schema_epoch.clone(),
                }
            })
            .collect::<Vec<_>>()
    };
    let resumed = state
        .subscription_hub
        .prepare_connection(state.clone(), descriptors, resume)
        .await
        .expect("reconnect Dashboard topology with cursors");
    assert!(resumed.initial.is_empty());
    assert!(
        resumed
            .outcomes
            .iter()
            .all(|outcome| outcome.disposition == TopicInitDisposition::ResumeCaughtUp),
        "same-cursor reconnect must not rebuild the Dashboard bundle"
    );
    let (first_frames, second_frames) = tokio::join!(
        collect_dashboard_runtime_topology_sse_events(
            first_response,
            &[
                "dashboard-runtime-topology-fallback",
                "dashboard-runtime-topology-second",
            ],
        ),
        collect_dashboard_runtime_topology_sse_events(
            second_response,
            &[
                "dashboard-runtime-topology-fallback",
                "dashboard-runtime-topology-second",
            ],
        ),
    );
    state.shutdown.cancel();
    assert_eq!(
        first_frames, second_frames,
        "both SSE owners should receive the same serialized live frames",
    );
    let working_payload = first_frames
        .get("dashboard.working-conversations.current")
        .and_then(|envelope| envelope.get("payload"))
        .expect("working conversations live payload");
    assert_eq!(working_payload["totalMatched"], json!(2));
    let working_keys = working_payload["conversations"]
        .as_array()
        .expect("working conversations array")
        .iter()
        .filter_map(|conversation| conversation["promptCacheKey"].as_str())
        .collect::<HashSet<_>>();
    assert!(working_keys.contains("dashboard-runtime-topology-fallback"));
    assert!(
        working_keys.contains("dashboard-runtime-topology-second"),
        "a durable terminal missing from runtime store must hydrate its selected key"
    );
    let first_observations = state
        .subscription_hub
        .dashboard_topology_sse_frame_observations(1)
        .await;
    let second_observations = state
        .subscription_hub
        .dashboard_topology_sse_frame_observations(2)
        .await;
    for topic in [
        "dashboard.activity.current",
        "stats.summary.current",
        "dashboard.network-timeseries.window",
        "dashboard.network-recent.current",
        "dashboard.working-conversations.current",
        "stats.parallel-work.current",
        "stats.timeseries.open-window",
    ] {
        let first_owner_frames = first_observations
            .get(topic)
            .expect("first owner should observe each Dashboard live frame");
        let second_owner_frames = second_observations
            .get(topic)
            .expect("second owner should observe each Dashboard live frame");
        assert!(
            first_owner_frames
                .iter()
                .any(|first_frame| second_owner_frames
                    .iter()
                    .any(|second_frame| Arc::ptr_eq(first_frame, second_frame))),
            "topic {topic} should reuse one serialized frame across SSE owners",
        );
    }
    let topic = "dashboard.activity.current";
    let terminal_total = json!(1);
    let first_owner_frames = first_observations
        .get(topic)
        .expect("first owner should observe terminal Dashboard frame");
    let second_owner_frames = second_observations
        .get(topic)
        .expect("second owner should observe terminal Dashboard frame");
    let terminal_frame = first_owner_frames
        .iter()
        .find(|frame| frame.payload_value()["summary"]["stats"]["totalCount"] == terminal_total)
        .expect("first owner should observe the terminal slice frame");
    assert!(
        second_owner_frames
            .iter()
            .any(|frame| Arc::ptr_eq(terminal_frame, frame)),
        "terminal topic {topic} must reuse one frame across two SSE owners",
    );
    let projection = state
        .proxy_runtime_invocations
        .dashboard_topology_counters();
    assert_eq!(projection.current.build_count, 1);
    assert_eq!(projection.current.revision_count, 1);
    assert!(projection.current.cadence_miss_count <= 1);
    assert_eq!(projection.network.build_count, 1);
    assert_eq!(projection.network.revision_count, 1);
    assert_eq!(projection.network.cadence_miss_count, 0);
    assert_eq!(projection.terminal.build_count, 1);
    assert_eq!(projection.terminal.revision_count, 1);
    assert_eq!(projection.terminal.cadence_miss_count, 0);
    assert_eq!(
        state
            .proxy_runtime_invocations
            .health_snapshot(2)
            .live_path_db_read_count,
        0,
        "active Dashboard producer must remain on the in-memory live path",
    );
    let delivery = state.subscription_hub.dashboard_topology_counters();
    for topic in [
        delivery.activity,
        delivery.summary,
        delivery.network_timeseries,
        delivery.network_recent,
        delivery.working_conversations,
        delivery.parallel_work,
        delivery.timeseries,
    ] {
        assert_eq!(
            topic.business_payload_count, 0,
            "new Dashboard delivery must not fan out business payloads"
        );
        assert_eq!(
            topic.json_overlay_count, 0,
            "new Dashboard delivery must not mutate generic JSON payloads"
        );
        assert!(
            topic.materialization_count >= 1,
            "each active Dashboard topic should materialize an immutable frame"
        );
        assert_eq!(topic.builder_count, topic.materialization_count);
        assert!(topic.frame_bytes_count > 0);
        assert!(topic.serialization_count >= topic.cursor_advanced);
        assert!(topic.frame_reused > 0);
        assert_eq!(topic.lagged_count, 0);
        assert_eq!(topic.skipped_count, 0);
        assert_eq!(
            topic.reconnect_churn_count, 0,
            "a cursor-continuous resume is not reconnect churn"
        );
    }
    for topic in [delivery.activity, delivery.summary] {
        assert_eq!(
            topic.serialization_count, topic.materialization_count,
            "each materialized topic revision should serialize exactly once"
        );
        assert_eq!(
            topic.payload_clone_count, 0,
            "activity and summary must mutate typed bases without complete payload clones"
        );
    }
    for topic in [delivery.network_timeseries, delivery.network_recent] {
        assert_eq!(
            topic.serialization_count, topic.materialization_count,
            "each materialized network topic revision should serialize exactly once"
        );
        assert_eq!(
            topic.payload_clone_count, 0,
            "network topic materialization must borrow its typed base"
        );
    }
    assert!(
        delivery.timeseries.serialization_count >= delivery.timeseries.materialization_count,
        "each materialized timeseries revision retains one full frame and may add a bounded delta frame"
    );
    assert_eq!(
        delivery.timeseries.payload_clone_count, 0,
        "timeseries topic materialization must retain typed aggregates"
    );
    for topic in [
        delivery.activity,
        delivery.summary,
        delivery.network_timeseries,
        delivery.network_recent,
        delivery.timeseries,
        delivery.working_conversations,
        delivery.parallel_work,
    ] {
        assert_eq!(topic.generic_fallback_build_count, 0);
        assert_eq!(topic.live_path_db_read_count, 0);
    }
    assert!(
        !state
            .subscription_hub
            .dashboard_delivery_has_degraded_signal(),
        "typed Dashboard topics must remain healthy on the in-memory live path"
    );
}
