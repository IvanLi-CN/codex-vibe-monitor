use super::*;
use serde_json::json;
pub(crate) fn summary_topic() -> SubscriptionTopic {
    SubscriptionTopic::SummaryCurrent {
        window: "current".to_string(),
        time_zone: SUBSCRIPTION_DEFAULT_TIME_ZONE.to_string(),
        limit: Some(20),
        upstream_account_id: None,
    }
}

pub(crate) fn dashboard_runtime_topology_descriptors() -> Vec<SubscriptionTopicDescriptor> {
    vec![
        SubscriptionTopicDescriptor {
            topic: "dashboard.activity.current".to_string(),
            params: BTreeMap::from([
                ("range".to_string(), "today".to_string()),
                (
                    "timeZone".to_string(),
                    SUBSCRIPTION_DEFAULT_TIME_ZONE.to_string(),
                ),
                ("recentLimit".to_string(), "16".to_string()),
                ("includeAccounts".to_string(), "true".to_string()),
                ("includeRecent".to_string(), "true".to_string()),
            ]),
        },
        SubscriptionTopicDescriptor {
            topic: "stats.summary.current".to_string(),
            params: BTreeMap::from([
                ("window".to_string(), "current".to_string()),
                (
                    "timeZone".to_string(),
                    SUBSCRIPTION_DEFAULT_TIME_ZONE.to_string(),
                ),
            ]),
        },
        SubscriptionTopicDescriptor {
            topic: "dashboard.network-timeseries.window".to_string(),
            params: BTreeMap::from([
                ("range".to_string(), "today".to_string()),
                (
                    "timeZone".to_string(),
                    SUBSCRIPTION_DEFAULT_TIME_ZONE.to_string(),
                ),
            ]),
        },
        SubscriptionTopicDescriptor {
            topic: "dashboard.network-recent.current".to_string(),
            params: BTreeMap::new(),
        },
        SubscriptionTopicDescriptor {
            topic: "dashboard.working-conversations.current".to_string(),
            params: BTreeMap::from([
                ("pageSize".to_string(), "20".to_string()),
                ("recentInvocationLimit".to_string(), "16".to_string()),
            ]),
        },
        SubscriptionTopicDescriptor {
            topic: "stats.parallel-work.current".to_string(),
            params: BTreeMap::from([
                ("range".to_string(), "1d".to_string()),
                ("bucket".to_string(), "1m".to_string()),
                (
                    "timeZone".to_string(),
                    SUBSCRIPTION_DEFAULT_TIME_ZONE.to_string(),
                ),
            ]),
        },
        SubscriptionTopicDescriptor {
            topic: "stats.timeseries.open-window".to_string(),
            params: BTreeMap::from([
                ("range".to_string(), "1d".to_string()),
                ("bucket".to_string(), "1m".to_string()),
                (
                    "timeZone".to_string(),
                    SUBSCRIPTION_DEFAULT_TIME_ZONE.to_string(),
                ),
            ]),
        },
    ]
}

pub(crate) fn dashboard_runtime_topology_live_record(occurred_at: &str) -> ApiInvocation {
    ApiInvocation {
        id: 748_001,
        invoke_id: "dashboard-runtime-topology-live".to_string(),
        occurred_at: occurred_at.to_string(),
        source: SOURCE_PROXY.to_string(),
        proxy_display_name: None,
        model: Some("gpt-5".to_string()),
        request_model: None,
        response_model: None,
        input_tokens: None,
        output_tokens: None,
        cache_input_tokens: None,
        reported_cache_write_tokens: None,
        reasoning_tokens: None,
        reasoning_effort: None,
        total_tokens: None,
        cost: None,
        cost_input: None,
        cost_cache_write: None,
        cost_cache_read: None,
        cost_output: None,
        cost_reasoning: None,
        cache_write_tokens: None,
        status: Some("running".to_string()),
        live_phase: Some("requesting".to_string()),
        error_message: None,
        downstream_status_code: None,
        failure_kind: None,
        blocked_binding: None,
        blocked_binding_json: None,
        stream_terminal_event: None,
        upstream_error_code: None,
        upstream_error_message: None,
        downstream_error_message: None,
        upstream_request_id: None,
        failure_class: None,
        is_actionable: None,
        endpoint: Some("/v1/responses".to_string()),
        compaction_request_kind: None,
        compaction_response_kind: None,
        image_intent: None,
        requester_ip: None,
        prompt_cache_key: None,
        sticky_key: None,
        route_mode: None,
        upstream_account_id: Some(42),
        upstream_account_name: Some("Topology Account".to_string()),
        response_content_encoding: None,
        request_compression_algorithm: None,
        transport: None,
        pool_attempt_count: None,
        pool_distinct_account_count: None,
        pool_attempt_terminal_reason: None,
        requested_service_tier: None,
        service_tier: None,
        billing_service_tier: None,
        proxy_weight_delta: None,
        cost_estimated: None,
        price_version: None,
        cost_audit: None,
        request_raw_path: None,
        request_raw_size: None,
        request_raw_truncated: None,
        request_raw_truncated_reason: None,
        response_raw_path: None,
        response_raw_size: None,
        response_raw_truncated: None,
        response_raw_truncated_reason: None,
        detail_level: DETAIL_LEVEL_FULL.to_string(),
        detail_pruned_at: None,
        detail_prune_reason: None,
        t_total_ms: None,
        t_req_read_ms: None,
        t_req_parse_ms: None,
        t_upstream_connect_ms: None,
        t_upstream_ttfb_ms: None,
        first_token_ms: None,
        t_upstream_stream_ms: None,
        t_resp_parse_ms: None,
        t_persist_ms: None,
        created_at: occurred_at.to_string(),
    }
}

pub(crate) fn dashboard_runtime_topology_stream_query(attempt: u64) -> SubscriptionStreamQuery {
    SubscriptionStreamQuery {
        topics: Some(
            serde_json::to_string(&dashboard_runtime_topology_descriptors())
                .expect("serialize dashboard topology topics"),
        ),
        resume: None,
        attempt: Some(attempt),
        reason: Some(DASHBOARD_RUNTIME_TOPOLOGY_CONTRACT_REASON.to_string()),
    }
}

pub(crate) async fn collect_dashboard_runtime_topology_sse_events(
    response: Response,
    required_working_keys: &[&str],
) -> BTreeMap<String, Value> {
    let expected = [
        "dashboard.activity.current",
        "stats.summary.current",
        "dashboard.network-timeseries.window",
        "dashboard.network-recent.current",
        "dashboard.working-conversations.current",
        "stats.parallel-work.current",
        "stats.timeseries.open-window",
    ];
    // Shared CI can delay the network materializer behind the other Dashboard topics. Keep
    // this bounded, but allow the same async projection window used by the runtime assertion.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
    let mut stream = response.into_body().into_data_stream();
    let mut buffered = Vec::new();
    let mut events: BTreeMap<String, Value> = BTreeMap::new();
    loop {
        let working_keys_ready = events
            .get("dashboard.working-conversations.current")
            .and_then(|envelope| envelope.pointer("/payload/conversations"))
            .and_then(Value::as_array)
            .map(|conversations| {
                required_working_keys.iter().all(|required_key| {
                    conversations.iter().any(|conversation| {
                        conversation["promptCacheKey"].as_str() == Some(required_key)
                    })
                })
            })
            .unwrap_or(required_working_keys.is_empty());
        if events.len() == expected.len() && working_keys_ready {
            break;
        }
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        assert!(
            !remaining.is_zero(),
            "dashboard topology SSE live event timeout; received={:?}",
            events.keys()
        );
        let chunk = tokio::time::timeout(remaining, stream.next())
            .await
            .unwrap_or_else(|_| {
                panic!(
                    "dashboard topology SSE live event timeout; received={:?}",
                    events.keys()
                )
            })
            .expect("dashboard topology SSE stream closed")
            .expect("dashboard topology SSE stream chunk");
        buffered.extend_from_slice(&chunk);
        while let Some(event_end) = buffered.windows(2).position(|window| window == b"\n\n") {
            let event = buffered.drain(..event_end + 2).collect::<Vec<_>>();
            let Some(payload) = event
                .strip_prefix(b"data: ")
                .and_then(|payload| payload.strip_suffix(b"\n\n"))
            else {
                continue;
            };
            let envelope: Value = serde_json::from_slice(payload).expect("Dashboard SSE envelope");
            let Some(topic) = envelope
                .pointer("/topic/topic")
                .and_then(Value::as_str)
                .filter(|topic| expected.contains(topic))
            else {
                continue;
            };
            if envelope.get("type").and_then(Value::as_str) == Some("live") {
                events.insert(topic.to_string(), envelope);
            }
        }
    }
    events
}

pub(crate) fn parallel_work_materializer_state(
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

pub(crate) fn seeded_cached_topic(
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

pub(crate) fn test_replay_event(
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

pub(crate) async fn insert_upstream_account_attempt_fixture(
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

pub(crate) fn phase_summary_test_state(
    key: &str,
) -> DashboardWorkingConversationsMaterializerState {
    let now = Utc::now();
    DashboardWorkingConversationsMaterializerState::new(
        PromptCacheConversationsResponse {
            range_start: format_utc_iso(now - ChronoDuration::minutes(5)),
            range_end: format_utc_iso(now),
            snapshot_at: None,
            selection_mode: PromptCacheConversationSelectionMode::ActivityWindow,
            selected_limit: None,
            selected_activity_hours: None,
            selected_activity_minutes: Some(5),
            implicit_filter: PromptCacheConversationImplicitFilter {
                kind: None,
                filtered_count: 0,
            },
            total_matched: Some(1),
            has_more: false,
            next_cursor: None,
            conversations: vec![PromptCacheConversationResponse {
                prompt_cache_key: key.to_string(),
                request_count: 0,
                total_tokens: 0,
                total_cost: 0.0,
                created_at: format_utc_iso(now),
                last_activity_at: format_utc_iso(now),
                last_terminal_at: None,
                last_in_flight_at: None,
                conversation_id: None,
                success_count: None,
                failure_count: None,
                input_tokens: None,
                output_tokens: None,
                cache_input_tokens: None,
                reported_cache_write_tokens: None,
                reasoning_tokens: None,
                cost_input: None,
                cost_cache_write: None,
                cost_cache_read: None,
                cost_output: None,
                cost_reasoning: None,
                first_invocation_at: None,
                last_invocation_at: None,
                in_flight_phase_counts: InvocationPhaseCountsResponse::default(),
                cursor: None,
                has_encrypted_session_owner: false,
                encrypted_owner_account_id: None,
                encrypted_owner_account_name: None,
                encrypted_owner_group_name: None,
                manual_binding: None,
                blocked_binding: None,
                upstream_accounts: Vec::new(),
                recent_invocations: Vec::new(),
                last24h_requests: Vec::new(),
            }],
        },
        20,
        16,
        None,
    )
}

pub(crate) fn phase_summary_test_delta(
    identity: &str,
    key: Option<&str>,
    status: &str,
    removed: bool,
) -> PromptCacheTopicDelta {
    PromptCacheTopicDelta {
        row_id: 0,
        identity: identity.to_string(),
        invoke_id: identity.to_string(),
        prompt_cache_key: key.map(str::to_string),
        sticky_key: None,
        occurred_at: "2026-09-03T10:00:00Z".to_string(),
        is_runtime_removed: removed,
        status: status.to_string(),
        is_terminal: status == "success",
        is_success: status == "success",
        request_tokens: 0,
        cost: 0.0,
        input_tokens: None,
        output_tokens: None,
        cache_input_tokens: None,
        reported_cache_write_tokens: None,
        reasoning_tokens: None,
        cost_input: None,
        cost_cache_write: None,
        cost_cache_read: None,
        cost_output: None,
        cost_reasoning: None,
        upstream_account_id: None,
        upstream_account_name: None,
        preview: None,
    }
}
