
#[tokio::test]
async fn dashboard_runtime_topology_materializes_shared_frames_without_business_payloads() {
    let (state, descriptors, topics) = dashboard_runtime_topology_setup().await;
    let (first_response, second_response) =
        open_dashboard_runtime_topology_streams(&state, &topics).await;
    let occurred_at = format_naive(Utc::now().with_timezone(&Shanghai).naive_local());
    publish_dashboard_runtime_updates(&state, &occurred_at).await;
    let fallback = publish_parallel_work_runtime_updates(&state).await;
    let parallel_topic = SubscriptionTopic::ParallelWorkCurrent {
        range: "1d".to_string(),
        time_zone: SUBSCRIPTION_DEFAULT_TIME_ZONE.to_string(),
        bucket: Some("1m".to_string()),
        upstream_account_id: None,
    };
    assert_parallel_work_runtime_projection(&state, &parallel_topic, &fallback).await;
    let (first_frames, second_frames) = reconnect_and_collect_dashboard_topology(
        &state,
        &topics,
        descriptors,
        first_response,
        second_response,
    )
    .await;
    assert_dashboard_topology_frames(&state, first_frames, second_frames).await;
    assert_dashboard_topology_counters(&state);
    state.shutdown.cancel();
}

async fn dashboard_runtime_topology_setup() -> (
    Arc<AppState>,
    Vec<SubscriptionTopicDescriptor>,
    Vec<SubscriptionTopic>,
) {
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
    (state, descriptors, topics)
}

async fn open_dashboard_runtime_topology_streams(
    state: &Arc<AppState>,
    topics: &[SubscriptionTopic],
) -> (Response, Response) {
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
    assert_dashboard_runtime_topology_owners(state, topics).await;
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
    (first_response, second_response)
}

async fn assert_dashboard_runtime_topology_owners(
    state: &Arc<AppState>,
    topics: &[SubscriptionTopic],
) {
    for topic in topics {
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
    for topic in topics {
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

async fn publish_dashboard_runtime_updates(state: &Arc<AppState>, occurred_at: &str) {
    state
        .proxy_runtime_invocations
        .upsert(dashboard_runtime_topology_live_record(occurred_at));
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
    let mut terminal = dashboard_runtime_topology_live_record(occurred_at);
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
    state.dashboard_network_speed_cache.record_request_bytes(
        "dashboard-runtime-topology-network",
        occurred_at,
        Some(42),
        Some("api.openai.com"),
        4096,
        Utc::now() - ChronoDuration::seconds(1),
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
}

async fn persist_dashboard_topology_invocation(state: &Arc<AppState>, invocation: &ApiInvocation) {
    sqlx::query(
            r#"
            INSERT INTO codex_invocations (
                id, invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
            "#,
        )
        .bind(invocation.id)
        .bind(invocation.invoke_id.as_str())
        .bind(invocation.occurred_at.as_str())
        .bind(invocation.source.as_str())
        .bind("success")
        .bind(42_i64)
        .bind(0.25_f64)
        .bind(
            json!({
                "promptCacheKey": invocation.prompt_cache_key.as_deref(),
                "upstreamAccountId": invocation.upstream_account_id,
            })
            .to_string(),
        )
        .bind("{}")
        .execute(&state.pool)
        .await
        .expect("persist Dashboard topology benchmark invocation");
}

async fn publish_parallel_work_runtime_updates(state: &Arc<AppState>) -> ApiInvocation {
    let mut fallback = dashboard_runtime_topology_live_record(&format_naive(
        (Utc::now() - ChronoDuration::minutes(1))
            .with_timezone(&Shanghai)
            .naive_local(),
    ));
    fallback.id = 748_004;
    fallback.invoke_id = "dashboard-runtime-topology-fallback".to_string();
    fallback.status = Some("success".to_string());
    fallback.live_phase = None;
    fallback.total_tokens = Some(42);
    fallback.cost = Some(0.25);
    fallback.prompt_cache_key = Some("dashboard-runtime-topology-fallback".to_string());
    let mut second_parallel_work = fallback.clone();
    second_parallel_work.id = 748_005;
    second_parallel_work.invoke_id = "dashboard-runtime-topology-second-parallel-work".to_string();
    second_parallel_work.prompt_cache_key = Some("dashboard-runtime-topology-second".to_string());
    persist_dashboard_topology_invocation(state, &fallback).await;
    persist_dashboard_topology_invocation(state, &second_parallel_work).await;
    let mut mutations = Vec::with_capacity(10_000);
    mutations.push(SequencedRuntimeMutation {
        sequence: 1,
        mutation: RuntimeMutation::invocation(&fallback, RuntimeMutationKind::RuntimeUpsert),
    });
    mutations.push(SequencedRuntimeMutation {
        sequence: 2,
        mutation: RuntimeMutation::invocation(
            &second_parallel_work,
            RuntimeMutationKind::RuntimeUpsert,
        ),
    });
    mutations.extend((3..=10_000).map(|sequence| SequencedRuntimeMutation {
        sequence,
        mutation: RuntimeMutation::invocation(&fallback, RuntimeMutationKind::RuntimeUpsert),
    }));
    state.proxy_runtime_invocations.upsert(fallback.clone());
    state
        .subscription_hub
        .handle_runtime_mutation_batch(state.clone(), mutations)
        .await;
    fallback
}

async fn assert_parallel_work_runtime_projection(
    state: &Arc<AppState>,
    parallel_topic: &SubscriptionTopic,
    fallback: &ApiInvocation,
) {
    let exact_parallel = load_parallel_work_stats_response(
        state,
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
}

async fn reconnect_and_collect_dashboard_topology(
    state: &Arc<AppState>,
    topics: &[SubscriptionTopic],
    descriptors: Vec<SubscriptionTopicDescriptor>,
    first_response: Response,
    second_response: Response,
) -> (BTreeMap<String, Value>, BTreeMap<String, Value>) {
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
        assert_eq!(topic.active_subscriber_count, 2);
        assert_eq!(topic.reconnect_churn_count, 0);
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
            .all(|outcome| { outcome.disposition == TopicInitDisposition::ResumeCaughtUp })
    );
    tokio::join!(
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
    )
}

async fn assert_dashboard_topology_frames(
    state: &Arc<AppState>,
    first_frames: BTreeMap<String, Value>,
    second_frames: BTreeMap<String, Value>,
) {
    assert_eq!(first_frames, second_frames);
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
    assert!(working_keys.contains("dashboard-runtime-topology-second"));
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
        let first_owner_frames = first_observations.get(topic).expect("first owner frames");
        let second_owner_frames = second_observations.get(topic).expect("second owner frames");
        assert!(first_owner_frames.iter().any(|first_frame| {
            second_owner_frames
                .iter()
                .any(|second_frame| Arc::ptr_eq(first_frame, second_frame))
        }));
    }
    let first_owner_frames = first_observations
        .get("dashboard.activity.current")
        .expect("first owner should observe terminal Dashboard frame");
    let second_owner_frames = second_observations
        .get("dashboard.activity.current")
        .expect("second owner should observe terminal Dashboard frame");
    let terminal_frame = first_owner_frames
        .iter()
        .find(|frame| frame.payload_value()["summary"]["stats"]["totalCount"] == json!(1))
        .expect("first owner should observe the terminal slice frame");
    assert!(
        second_owner_frames
            .iter()
            .any(|frame| Arc::ptr_eq(terminal_frame, frame))
    );
}

fn assert_dashboard_topology_counters(state: &Arc<AppState>) {
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
        0
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
        assert_eq!(topic.business_payload_count, 0);
        assert_eq!(topic.json_overlay_count, 0);
        assert!(topic.materialization_count >= 1);
        assert_eq!(topic.builder_count, topic.materialization_count);
        assert!(topic.frame_bytes_count > 0);
        assert_eq!(topic.cursor_advanced, topic.serialization_count);
        assert!(topic.frame_reused > 0);
        assert_eq!(topic.lagged_count, 0);
        assert_eq!(topic.skipped_count, 0);
        assert_eq!(topic.reconnect_churn_count, 0);
    }
    for topic in [delivery.activity, delivery.summary] {
        assert_eq!(topic.serialization_count, topic.materialization_count);
        assert_eq!(topic.payload_clone_count, 0);
    }
    for topic in [delivery.network_timeseries, delivery.network_recent] {
        assert_eq!(topic.serialization_count, topic.materialization_count);
        assert_eq!(topic.payload_clone_count, 0);
    }
    assert_eq!(
        delivery.timeseries.serialization_count,
        delivery.timeseries.materialization_count
    );
    assert_eq!(delivery.timeseries.payload_clone_count, 0);
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
            .dashboard_delivery_has_degraded_signal()
    );
}

#[test]
fn parallel_work_projection_extends_rolling_window_with_live_bucket() {
    let current_bucket = Utc::now().timestamp().div_euclid(60) * 60;
    let initial_start = Utc
        .timestamp_opt(current_bucket - 120, 0)
        .single()
        .expect("construct initial point start");
    let initial_end = Utc
        .timestamp_opt(current_bucket - 60, 0)
        .single()
        .expect("construct initial point end");
    let live_end = Utc
        .timestamp_opt(current_bucket + 60, 0)
        .single()
        .expect("construct live point end");
    let mut window = ParallelWorkWindowResponse {
        range_start: format_utc_iso(initial_start),
        range_end: format_utc_iso(initial_end),
        bucket_seconds: 60,
        complete_bucket_count: 1,
        active_bucket_count: 0,
        active_minute_count: Some(0),
        min_count: Some(0),
        max_count: Some(0),
        avg_count: None,
        effective_time_zone: chrono_tz::UTC.to_string(),
        time_zone_fallback: false,
        points: vec![ParallelWorkPoint {
            bucket_start: format_utc_iso(initial_start),
            bucket_end: format_utc_iso(initial_end),
            parallel_count: 0,
        }],
        conversations: Vec::new(),
    };
    let bucket_keys = BTreeMap::from([(current_bucket, HashSet::from(["live-key".to_string()]))]);

    assert!(refresh_parallel_work_points(
        &mut window,
        current_bucket,
        chrono_tz::UTC,
        &bucket_keys,
    ));
    assert_eq!(window.range_end, format_utc_iso(live_end));
    assert_eq!(window.complete_bucket_count, 3);
    assert_eq!(
        window.points.last().map(|point| point.parallel_count),
        Some(1)
    );
}

fn parallel_work_materializer_state(
    range: &str,
    range_start: DateTime<Utc>,
    baseline_row_id: i64,
    upstream_account_id: Option<i64>,
) -> DashboardParallelWorkMaterializerState {
    let range_end = range_start + ChronoDuration::minutes(1);
    let window = ParallelWorkWindowResponse {
        range_start: format_utc_iso(range_start),
        range_end: format_utc_iso(range_end),
        bucket_seconds: 60,
        complete_bucket_count: 1,
        active_bucket_count: 0,
        active_minute_count: Some(0),
        min_count: Some(0),
        max_count: Some(0),
        avg_count: None,
        effective_time_zone: chrono_tz::UTC.to_string(),
        time_zone_fallback: false,
        points: vec![ParallelWorkPoint {
            bucket_start: format_utc_iso(range_start),
            bucket_end: format_utc_iso(range_end),
            parallel_count: 0,
        }],
        conversations: vec![ParallelWorkConversation {
            conversation_id: "baseline-key".to_string(),
            start: format_utc_iso(range_start),
            end: format_utc_iso(range_end),
            request_count: 1,
        }],
    };
    let response = ParallelWorkStatsResponse {
        current: window.clone(),
        minute7d: window.clone(),
        hour30d: window.clone(),
        day_all: window,
    };
    let range_start_epoch = range_start.timestamp();
    DashboardParallelWorkMaterializerState {
        baseline_response: response.clone(),
        response,
        baseline_bucket_keys: BTreeMap::new(),
        bucket_keys: BTreeMap::new(),
        baseline_minute_keys: BTreeMap::new(),
        minute_keys: BTreeMap::new(),
        baseline_active_minute_stats: ParallelWorkActiveMinuteStats::default(),
        active_minute_stats: ParallelWorkActiveMinuteStats::default(),
        baseline_complete_minute_start_epoch: if range_start_epoch.rem_euclid(60) == 0 {
            range_start_epoch
        } else {
            range_start_epoch.div_euclid(60) * 60 + 60
        },
        baseline_complete_minute_end_epoch: range_end.timestamp().div_euclid(60) * 60,
        baseline_row_id,
        range: range.to_string(),
        reporting_tz: chrono_tz::UTC,
        upstream_account_id,
        conversations_enabled: true,
        baseline_identities: HashSet::new(),
        applied_identities: HashSet::new(),
        runtime_mutations: BTreeMap::new(),
        revision: 0,
    }
}

#[test]
fn parallel_work_projection_excludes_incomplete_current_minute_from_average() {
    let now = Utc::now();
    let current_minute_start = now.timestamp().div_euclid(60) * 60;
    let occurred_at = Utc
        .timestamp_opt(current_minute_start + 1, 0)
        .single()
        .expect("construct current incomplete minute");
    let mut state =
        parallel_work_materializer_state("1d", occurred_at - ChronoDuration::minutes(2), 0, None);
    let active_minute_stats = ParallelWorkActiveMinuteStats {
        active_minute_count: Some(4),
        parallel_count_sum: 12,
    };
    state.baseline_active_minute_stats = active_minute_stats;
    state.active_minute_stats = active_minute_stats;
    for window in [
        &mut state.response.current,
        &mut state.response.minute7d,
        &mut state.response.hour30d,
        &mut state.response.day_all,
    ] {
        window.active_minute_count = Some(4);
        window.avg_count = Some(3.0);
    }

    assert!(state.apply_runtime_overlay_at(
        &RuntimeInvocationMutation {
            identity: RuntimeInvocationIdentity::new(
                "current-minute-invoke",
                format_utc_iso(occurred_at),
            ),
            kind: RuntimeMutationKind::RuntimeUpsert,
            row_id: None,
            is_terminal: false,
            prompt_cache_key: Some("current-minute-key".to_string()),
            sticky_key: None,
            upstream_account_id: None,
        },
        now,
    ));
    assert_eq!(state.active_minute_stats, active_minute_stats);
    assert_eq!(state.response.current.active_minute_count, Some(4));
    assert_eq!(state.response.current.avg_count, Some(3.0));
}

#[test]
fn parallel_work_projection_promotes_closed_runtime_minutes_on_next_overlay() {
    let minute_start = Utc
        .timestamp_opt(1_700_000_000, 0)
        .single()
        .expect("construct minute start");
    let mut state =
        parallel_work_materializer_state("1d", minute_start - ChronoDuration::minutes(2), 0, None);
    let active_minute_stats = ParallelWorkActiveMinuteStats {
        active_minute_count: Some(4),
        parallel_count_sum: 12,
    };
    state.baseline_active_minute_stats = active_minute_stats;
    state.active_minute_stats = active_minute_stats;
    for window in [
        &mut state.response.current,
        &mut state.response.minute7d,
        &mut state.response.hour30d,
        &mut state.response.day_all,
    ] {
        window.active_minute_count = Some(4);
        window.avg_count = Some(3.0);
    }

    let first = RuntimeInvocationMutation {
        identity: RuntimeInvocationIdentity::new(
            "completed-minute-invoke",
            format_utc_iso(minute_start + ChronoDuration::seconds(1)),
        ),
        kind: RuntimeMutationKind::RuntimeUpsert,
        row_id: None,
        is_terminal: false,
        prompt_cache_key: Some("completed-minute-key".to_string()),
        sticky_key: None,
        upstream_account_id: None,
    };
    assert!(state.apply_runtime_overlay_at(&first, minute_start + ChronoDuration::seconds(30)));
    assert_eq!(state.active_minute_stats, active_minute_stats);

    let next_minute = RuntimeInvocationMutation {
        identity: RuntimeInvocationIdentity::new(
            "next-minute-invoke",
            format_utc_iso(minute_start + ChronoDuration::seconds(61)),
        ),
        kind: RuntimeMutationKind::RuntimeUpsert,
        row_id: None,
        is_terminal: false,
        prompt_cache_key: Some("next-minute-key".to_string()),
        sticky_key: None,
        upstream_account_id: None,
    };
    assert!(
        state.apply_runtime_overlay_at(&next_minute, minute_start + ChronoDuration::seconds(90))
    );
    assert_eq!(
        state.active_minute_stats,
        ParallelWorkActiveMinuteStats {
            active_minute_count: Some(5),
            parallel_count_sum: 13,
        }
    );
    assert_eq!(state.response.current.avg_count, Some(2.6));
}

#[test]
fn parallel_work_projection_promotes_persisted_current_minute_after_boundary() {
    let minute_start = Utc
        .timestamp_opt(1_700_000_000, 0)
        .single()
        .expect("construct minute start");
    let mut state =
        parallel_work_materializer_state("1d", minute_start - ChronoDuration::minutes(2), 0, None);
    let active_minute_stats = ParallelWorkActiveMinuteStats {
        active_minute_count: Some(4),
        parallel_count_sum: 12,
    };
    state.baseline_active_minute_stats = active_minute_stats;
    state.active_minute_stats = active_minute_stats;
    state.baseline_complete_minute_end_epoch = minute_start.timestamp();
    state.baseline_minute_keys.insert(
        minute_start.timestamp(),
        HashSet::from(["persisted-current-minute-key".to_string()]),
    );
    state.minute_keys = state.baseline_minute_keys.clone();
    for window in [
        &mut state.response.current,
        &mut state.response.minute7d,
        &mut state.response.hour30d,
        &mut state.response.day_all,
    ] {
        window.active_minute_count = Some(4);
        window.avg_count = Some(3.0);
    }

    let next_minute = RuntimeInvocationMutation {
        identity: RuntimeInvocationIdentity::new(
            "next-minute-invoke",
            format_utc_iso(minute_start + ChronoDuration::seconds(61)),
        ),
        kind: RuntimeMutationKind::RuntimeUpsert,
        row_id: None,
        is_terminal: false,
        prompt_cache_key: Some("next-minute-key".to_string()),
        sticky_key: None,
        upstream_account_id: None,
    };
    assert!(
        state.apply_runtime_overlay_at(&next_minute, minute_start + ChronoDuration::seconds(90))
    );
    assert_eq!(
        state.active_minute_stats,
        ParallelWorkActiveMinuteStats {
            active_minute_count: Some(5),
            parallel_count_sum: 13,
        }
    );
    assert_eq!(state.response.current.avg_count, Some(2.6));
}

#[test]
fn parallel_work_projection_skips_rows_already_in_its_cold_baseline() {
    let occurred_at = Utc::now() - ChronoDuration::seconds(30);
    let mut state = parallel_work_materializer_state("1d", occurred_at, 42, None);
    let outcome = state.apply_runtime_mutation(&RuntimeInvocationMutation {
        identity: RuntimeInvocationIdentity::new("baseline-invoke", format_utc_iso(occurred_at)),
        kind: RuntimeMutationKind::TerminalCommitted,
        row_id: Some(42),
        is_terminal: true,
        prompt_cache_key: Some("baseline-key".to_string()),
        sticky_key: None,
        upstream_account_id: None,
    });

    assert!(!outcome.changed);
    assert_eq!(state.response.current.conversations[0].request_count, 1);
}

#[test]
fn parallel_work_projection_requires_typed_reconcile_for_unknown_account_fallback() {
    let occurred_at = Utc::now() - ChronoDuration::seconds(30);
    let mut state = parallel_work_materializer_state("1d", occurred_at, 0, Some(77));
    let outcome = state.apply_runtime_mutation(&RuntimeInvocationMutation {
        identity: RuntimeInvocationIdentity::new("fallback-invoke", format_utc_iso(occurred_at)),
        kind: RuntimeMutationKind::RuntimeUpsert,
        row_id: Some(43),
        is_terminal: false,
        prompt_cache_key: Some("account-fallback-key".to_string()),
        sticky_key: None,
        upstream_account_id: None,
    });

    assert!(!outcome.changed);
    assert!(outcome.needs_account_reconcile);
}

#[test]
fn parallel_work_projection_rebases_moving_ranges() {
    let state =
        parallel_work_materializer_state("1d", Utc::now() - ChronoDuration::minutes(2), 0, None);

    assert!(state.requires_rolling_rebase());
}

#[test]
fn parallel_work_projection_removes_runtime_only_work_immediately() {
    let occurred_at = Utc::now();
    let mut state =
        parallel_work_materializer_state("1d", occurred_at - ChronoDuration::minutes(2), 0, None);
    let identity =
        RuntimeInvocationIdentity::new("runtime-only-invoke", format_utc_iso(occurred_at));
    let upsert = RuntimeInvocationMutation {
        identity: identity.clone(),
        kind: RuntimeMutationKind::RuntimeUpsert,
        row_id: None,
        is_terminal: false,
        prompt_cache_key: Some("runtime-only-key".to_string()),
        sticky_key: None,
        upstream_account_id: None,
    };
    assert!(state.apply_runtime_mutation(&upsert).changed);
    assert!(
        state
            .response
            .current
            .conversations
            .iter()
            .any(|conversation| { conversation.conversation_id == "runtime-only-key" })
    );

    let removed = state.apply_runtime_mutation(&RuntimeInvocationMutation {
        identity,
        kind: RuntimeMutationKind::RuntimeRemoved,
        row_id: None,
        is_terminal: false,
        prompt_cache_key: None,
        sticky_key: None,
        upstream_account_id: None,
    });
    assert!(removed.changed);
    assert!(
        !state
            .response
            .current
            .conversations
            .iter()
            .any(|conversation| { conversation.conversation_id == "runtime-only-key" })
    );
    assert_eq!(
        state
            .response
            .current
            .points
            .iter()
            .map(|point| point.parallel_count)
            .sum::<i64>(),
        0
    );
}

#[test]
fn parallel_work_projection_replays_runtime_entries_after_rebase() {
    let occurred_at = Utc::now();
    let mut old =
        parallel_work_materializer_state("1d", occurred_at - ChronoDuration::minutes(2), 0, None);
    let mutation = RuntimeInvocationMutation {
        identity: RuntimeInvocationIdentity::new("rebase-race-invoke", format_utc_iso(occurred_at)),
        kind: RuntimeMutationKind::RuntimeUpsert,
        row_id: None,
        is_terminal: false,
        prompt_cache_key: Some("rebase-race-key".to_string()),
        sticky_key: None,
        upstream_account_id: None,
    };
    assert!(old.apply_runtime_mutation(&mutation).changed);

    let mut rebased =
        parallel_work_materializer_state("1d", occurred_at - ChronoDuration::minutes(1), 0, None);
    assert!(rebased.replay_runtime_mutations(&old.runtime_mutations));
    assert!(
        rebased
            .response
            .current
            .conversations
            .iter()
            .any(|conversation| { conversation.conversation_id == "rebase-race-key" })
    );
}

#[test]
fn parallel_work_projection_skips_runtime_entries_already_in_rebased_baseline() {
    let occurred_at = Utc::now();
    let mut old =
        parallel_work_materializer_state("1d", occurred_at - ChronoDuration::minutes(2), 0, None);
    let mutation = RuntimeInvocationMutation {
        identity: RuntimeInvocationIdentity::new(
            "persisted-rebase-invoke",
            format_utc_iso(occurred_at),
        ),
        kind: RuntimeMutationKind::RuntimeUpsert,
        row_id: None,
        is_terminal: false,
        prompt_cache_key: Some("persisted-rebase-key".to_string()),
        sticky_key: None,
        upstream_account_id: None,
    };
    assert!(old.apply_runtime_mutation(&mutation).changed);

    let mut rebased =
        parallel_work_materializer_state("1d", occurred_at - ChronoDuration::minutes(1), 0, None);
    for window in [
        &mut rebased.response.current,
        &mut rebased.response.minute7d,
        &mut rebased.response.hour30d,
        &mut rebased.response.day_all,
    ] {
        window.conversations = vec![ParallelWorkConversation {
            conversation_id: "persisted-rebase-key".to_string(),
            start: format_utc_iso(occurred_at),
            end: format_utc_iso(occurred_at + ChronoDuration::minutes(1)),
            request_count: 1,
        }];
    }
    rebased.baseline_response = rebased.response.clone();
    rebased.baseline_identities.insert(format!(
        "{}\0{}",
        mutation.identity.invoke_id, mutation.identity.occurred_at
    ));

    assert!(!rebased.replay_runtime_mutations(&old.runtime_mutations));
    assert_eq!(rebased.response.current.conversations[0].request_count, 1);
}

#[tokio::test]
async fn parallel_work_reconcile_failure_schedules_runtime_recovery_retry() {
    let state = crate::tests::test_state_with_openai_base(
        Url::parse("http://127.0.0.1:9").expect("valid test URL"),
    )
    .await;
    let hub = state.subscription_hub.clone();
    let topic = SubscriptionTopic::ParallelWorkCurrent {
        range: "1d".to_string(),
        time_zone: SUBSCRIPTION_DEFAULT_TIME_ZONE.to_string(),
        bucket: Some("1m".to_string()),
        upstream_account_id: Some(77),
    };
    let topic_key = topic.cache_key().expect("parallel-work topic key");
    let lease = hub
        .register_topic_subscribers(std::slice::from_ref(&topic))
        .await
        .expect("register parallel-work owner");
    hub.prepare_connection(state.clone(), vec![topic.descriptor()], Vec::new())
        .await
        .expect("build initial parallel-work baseline");
    {
        let mut guard = hub.state.lock().await;
        let cached = guard.topics.get_mut(&topic_key).expect("cached topic");
        cached.dirty = true;
        cached.parallel_work_refresh_scheduled = true;
    }

    hub.schedule_parallel_work_reconcile_retry(state.clone(), &topic)
        .await;

    let guard = hub.state.lock().await;
    let cached = guard.topics.get(&topic_key).expect("cached topic");
    assert!(cached.dirty);
    assert!(!cached.parallel_work_refresh_scheduled);
    assert!(cached.runtime_topic_recovery_retry_at.is_some());
    assert!(guard.runtime_topic_recovery_running);
    drop(guard);
    drop(lease);
}

#[tokio::test]
async fn parallel_work_projection_replays_mutations_buffered_before_cold_baseline() {
    let state = crate::tests::test_state_with_openai_base(
        Url::parse("http://127.0.0.1:9").expect("valid test URL"),
    )
    .await;
    let hub = state.subscription_hub.clone();
    let topic = SubscriptionTopic::ParallelWorkCurrent {
        range: "1d".to_string(),
        time_zone: SUBSCRIPTION_DEFAULT_TIME_ZONE.to_string(),
        bucket: Some("1m".to_string()),
        upstream_account_id: None,
    };
    let topic_key = topic.cache_key().expect("parallel-work topic key");
    let lease = hub
        .register_topic_subscribers(std::slice::from_ref(&topic))
        .await
        .expect("register parallel-work owner");
    let occurred_at = format_utc_iso(Utc::now());
    let mutations = [SequencedRuntimeMutation {
        sequence: 1,
        mutation: RuntimeMutation::Invocation(RuntimeInvocationMutation {
            identity: RuntimeInvocationIdentity::new("prebaseline-invoke", occurred_at),
            kind: RuntimeMutationKind::RuntimeUpsert,
            row_id: None,
            is_terminal: false,
            prompt_cache_key: Some("prebaseline-key".to_string()),
            sticky_key: None,
            upstream_account_id: None,
        }),
    }];

    hub.schedule_parallel_work_topic_projection(state.clone(), topic.clone(), &mutations)
        .await
        .expect("buffer mutation before initial baseline");
    assert!(
        hub.state
            .lock()
            .await
            .parallel_work_prebaseline_mutations
            .contains_key(&topic_key)
    );

    hub.prepare_connection(state, vec![topic.descriptor()], Vec::new())
        .await
        .expect("build and replay parallel-work baseline");
    let guard = hub.state.lock().await;
    let Some(DashboardTopicMaterializer::ParallelWork { base }) =
        guard.topics[&topic_key].dashboard_materializer.as_ref()
    else {
        panic!("parallel-work topic must use a typed materializer");
    };
    assert!(
        base.lock()
            .expect("parallel-work materializer state lock")
            .response
            .current
            .conversations
            .iter()
            .any(|conversation| conversation.conversation_id == "prebaseline-key")
    );
    drop(guard);
    drop(lease);
}

#[tokio::test]
async fn parallel_work_projection_discards_prebaseline_mutations_after_last_owner_leaves() {
    let state = crate::tests::test_state_with_openai_base(
        Url::parse("http://127.0.0.1:9").expect("valid test URL"),
    )
    .await;
    let hub = state.subscription_hub.clone();
    let topic = SubscriptionTopic::ParallelWorkCurrent {
        range: "1d".to_string(),
        time_zone: SUBSCRIPTION_DEFAULT_TIME_ZONE.to_string(),
        bucket: Some("1m".to_string()),
        upstream_account_id: None,
    };
    let topic_key = topic.cache_key().expect("parallel-work topic key");
    let mut lease = hub
        .register_topic_subscribers(std::slice::from_ref(&topic))
        .await
        .expect("register parallel-work owner");
    let mutations = [SequencedRuntimeMutation {
        sequence: 1,
        mutation: RuntimeMutation::Invocation(RuntimeInvocationMutation {
            identity: RuntimeInvocationIdentity::new(
                "released-prebaseline-invoke",
                format_utc_iso(Utc::now()),
            ),
            kind: RuntimeMutationKind::RuntimeUpsert,
            row_id: None,
            is_terminal: false,
            prompt_cache_key: Some("released-prebaseline-key".to_string()),
            sticky_key: None,
            upstream_account_id: None,
        }),
    }];

    hub.schedule_parallel_work_topic_projection(state.clone(), topic.clone(), &mutations)
        .await
        .expect("buffer mutation before initial baseline");
    assert!(
        hub.state
            .lock()
            .await
            .parallel_work_prebaseline_mutations
            .contains_key(&topic_key)
    );

    let topic_keys = std::mem::take(&mut lease.topic_keys);
    let topic_names = std::mem::take(&mut lease.topic_names);
    hub.release_topic_subscribers(topic_keys, topic_names, lease.owns_dashboard_live)
        .await;
    drop(lease);

    assert!(
        !hub.state
            .lock()
            .await
            .parallel_work_prebaseline_mutations
            .contains_key(&topic_key)
    );

    hub.schedule_parallel_work_topic_projection(state, topic, &mutations)
        .await
        .expect("ignore mutation after final owner release");
    assert!(
        !hub.state
            .lock()
            .await
            .parallel_work_prebaseline_mutations
            .contains_key(&topic_key)
    );
}

#[tokio::test]
async fn parallel_work_projection_retains_unknown_account_mutation_through_reconcile() {
    let state = crate::tests::test_state_with_openai_base(
        Url::parse("http://127.0.0.1:9").expect("valid test URL"),
    )
    .await;
    let hub = state.subscription_hub.clone();
    let topic = SubscriptionTopic::ParallelWorkCurrent {
        range: "1d".to_string(),
        time_zone: SUBSCRIPTION_DEFAULT_TIME_ZONE.to_string(),
        bucket: Some("1m".to_string()),
        upstream_account_id: Some(77),
    };
    let topic_key = topic.cache_key().expect("parallel-work topic key");
    let lease = hub
        .register_topic_subscribers(std::slice::from_ref(&topic))
        .await
        .expect("register parallel-work owner");
    hub.prepare_connection(state.clone(), vec![topic.descriptor()], Vec::new())
        .await
        .expect("build initial parallel-work baseline");
    let mutation = RuntimeInvocationMutation {
        identity: RuntimeInvocationIdentity::new(
            "unknown-account-invoke",
            format_utc_iso(Utc::now()),
        ),
        kind: RuntimeMutationKind::RuntimeUpsert,
        row_id: None,
        is_terminal: false,
        prompt_cache_key: Some("unknown-account-key".to_string()),
        sticky_key: None,
        upstream_account_id: None,
    };
    let mutations = [SequencedRuntimeMutation {
        sequence: 1,
        mutation: RuntimeMutation::Invocation(mutation.clone()),
    }];

    hub.schedule_parallel_work_topic_projection(state.clone(), topic.clone(), &mutations)
        .await
        .expect("schedule unknown-account reconcile");
    hub.refresh_topic_if_active(state.clone(), topic.clone(), true)
        .await
        .expect("reconcile unknown-account baseline");

    assert!(
        hub.state
            .lock()
            .await
            .parallel_work_prebaseline_mutations
            .get(&topic_key)
            .is_some_and(|pending| pending.contains_key(&format!(
                "{}\0{}",
                mutation.identity.invoke_id, mutation.identity.occurred_at
            )))
    );

    let removed = RuntimeInvocationMutation {
        identity: mutation.identity.clone(),
        kind: RuntimeMutationKind::RuntimeRemoved,
        row_id: None,
        is_terminal: false,
        prompt_cache_key: None,
        sticky_key: None,
        upstream_account_id: None,
    };
    let removals = [SequencedRuntimeMutation {
        sequence: 2,
        mutation: RuntimeMutation::Invocation(removed),
    }];
    hub.schedule_parallel_work_topic_projection(state.clone(), topic, &removals)
        .await
        .expect("discard removed unknown-account mutation");

    assert!(
        !hub.state
            .lock()
            .await
            .parallel_work_prebaseline_mutations
            .contains_key(&topic_key)
    );
    drop(lease);
}

#[tokio::test]
async fn parallel_work_projection_replays_mutations_buffered_while_rebasing() {
    let state = crate::tests::test_state_with_openai_base(
        Url::parse("http://127.0.0.1:9").expect("valid test URL"),
    )
    .await;
    let hub = state.subscription_hub.clone();
    let topic = SubscriptionTopic::ParallelWorkCurrent {
        range: "1d".to_string(),
        time_zone: SUBSCRIPTION_DEFAULT_TIME_ZONE.to_string(),
        bucket: Some("1m".to_string()),
        upstream_account_id: None,
    };
    let topic_key = topic.cache_key().expect("parallel-work topic key");
    let lease = hub
        .register_topic_subscribers(std::slice::from_ref(&topic))
        .await
        .expect("register parallel-work owner");
    hub.prepare_connection(state.clone(), vec![topic.descriptor()], Vec::new())
        .await
        .expect("build initial parallel-work baseline");
    {
        let mut guard = hub.state.lock().await;
        guard
            .topics
            .get_mut(&topic_key)
            .expect("cached topic")
            .dirty = true;
    }

    let mutations = [SequencedRuntimeMutation {
        sequence: 1,
        mutation: RuntimeMutation::Invocation(RuntimeInvocationMutation {
            identity: RuntimeInvocationIdentity::new(
                "rebase-buffered-invoke",
                format_utc_iso(Utc::now()),
            ),
            kind: RuntimeMutationKind::RuntimeUpsert,
            row_id: None,
            is_terminal: false,
            prompt_cache_key: Some("rebase-buffered-key".to_string()),
            sticky_key: None,
            upstream_account_id: None,
        }),
    }];
    hub.schedule_parallel_work_topic_projection(state.clone(), topic.clone(), &mutations)
        .await
        .expect("buffer mutation while rebase is in flight");
    assert!(
        hub.state
            .lock()
            .await
            .parallel_work_prebaseline_mutations
            .contains_key(&topic_key)
    );

    hub.refresh_topic_if_active(state, topic, true)
        .await
        .expect("rebuild parallel-work baseline")
        .expect("active owner receives rebuilt topic");
    let guard = hub.state.lock().await;
    let Some(DashboardTopicMaterializer::ParallelWork { base }) =
        guard.topics[&topic_key].dashboard_materializer.as_ref()
    else {
        panic!("parallel-work topic must retain a typed materializer");
    };
    assert!(
        base.lock()
            .expect("parallel-work materializer state lock")
            .response
            .current
            .conversations
            .iter()
            .any(|conversation| conversation.conversation_id == "rebase-buffered-key")
    );
    drop(guard);
    drop(lease);
}

#[test]
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
