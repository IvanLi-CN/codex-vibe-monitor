use super::*;
use serde_json::json;
#[tokio::test]
async fn working_conversations_bounded_hydration_keeps_old_in_flight_off_the_chart() {
    let state = crate::tests::test_state_with_openai_base(
        Url::parse("http://127.0.0.1:9").expect("valid test URL"),
    )
    .await;
    let range_end = Utc::now();
    let occurred_at = format_naive(
        (range_end - ChronoDuration::hours(PROMPT_CACHE_CONVERSATION_CHART_MAX_HOURS + 1))
            .with_timezone(&Shanghai)
            .naive_local(),
    );
    let mut record = dashboard_runtime_topology_live_record(&occurred_at);
    record.id = 0;
    record.invoke_id = "old-runtime-working-hydration".to_string();
    record.prompt_cache_key = Some("old-runtime-working-hydration".to_string());
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
        "old-runtime-working-hydration",
        range_end,
        &range_start_bound,
        16,
        None,
    )
    .await
    .expect("hydrate old in-flight working conversation")
    .expect("old in-flight key remains selected");

    assert!(conversation.last_in_flight_at.is_some());
    assert!(conversation.last24h_requests.is_empty());
}

#[test]
fn working_conversations_hydration_detects_pending_delta_changes_after_snapshot() {
    let occurred_at = format_naive(Utc::now().with_timezone(&Shanghai).naive_local());
    let delta = |invoke_id: &str| {
        let mut record = dashboard_runtime_topology_live_record(&occurred_at);
        record.id = 0;
        record.invoke_id = invoke_id.to_string();
        record.status = Some("success".to_string());
        record.live_phase = None;
        record.prompt_cache_key = Some("hydration-race-key".to_string());
        PromptCacheTopicDelta::from_record(&record)
            .expect("build hydration race delta")
            .expect("typed hydration race delta")
    };
    let initial = delta("hydration-race-initial");
    let before = BTreeMap::from([(initial.identity.clone(), initial.clone())]);
    let hydration_keys = vec!["hydration-race-key".to_string()];

    assert!(
        prompt_cache_hydration_changed_pending_keys(&before, &before, &hydration_keys).is_empty(),
        "a stable pending snapshot must not force recovery",
    );

    let later = delta("hydration-race-later");
    let mut with_later_identity = before.clone();
    with_later_identity.insert(later.identity.clone(), later);
    assert_eq!(
        prompt_cache_hydration_changed_pending_keys(&before, &with_later_identity, &hydration_keys,),
        BTreeSet::from(["hydration-race-key".to_string()]),
        "a delta added after the hydrate snapshot must retry only its bounded key",
    );

    let mut transitioned = initial.clone();
    transitioned.status = "running".to_string();
    transitioned.is_terminal = false;
    transitioned.preview.as_mut().expect("typed preview").status = "running".to_string();
    let with_replaced_identity = BTreeMap::from([(transitioned.identity.clone(), transitioned)]);
    assert_eq!(
        prompt_cache_hydration_changed_pending_keys(
            &before,
            &with_replaced_identity,
            &hydration_keys,
        ),
        BTreeSet::from(["hydration-race-key".to_string()]),
        "a same-identity state transition after the hydrate snapshot must retry its key",
    );
}

#[test]
fn dashboard_network_recent_topic_uses_empty_descriptor_and_stable_schema_epoch() {
    let descriptor = SubscriptionTopicDescriptor {
        topic: "dashboard.network-recent.current".to_string(),
        params: BTreeMap::new(),
    };

    let topic = SubscriptionTopic::from_descriptor(&descriptor).expect("recent topic should parse");

    assert_eq!(topic.descriptor(), descriptor);
    assert_eq!(topic.name(), "dashboard.network-recent.current");
    assert_eq!(topic.schema_epoch(), "dashboard.network-recent.current/v1");
    assert!(
        topic.is_affected_by_runtime_mutation(&RuntimeMutation::Invocation(
            RuntimeInvocationMutation {
                identity: RuntimeInvocationIdentity::new("network", "2026-07-20 00:00:00"),
                kind: RuntimeMutationKind::RuntimeUpsert,
                row_id: None,
                is_terminal: false,
                prompt_cache_key: None,
                sticky_key: None,
                upstream_account_id: None,
            }
        ))
    );
    assert!(
        !topic.is_affected_by(&BroadcastPayload::DashboardActivityLive {
            snapshot: Box::new(DashboardActivityLiveSnapshot {
                revision: 1,
                generated_at: "2026-07-20T00:00:00.000Z".to_string(),
                in_progress_invocation_count: 0,
                in_progress_phase_counts: InvocationPhaseCountsResponse::default(),
                retry_invocation_count: 0,
                in_progress_wait_sum_ms: 0.0,
                in_progress_wait_sample_count: 0,
                network_live_bucket: None,
                network_realtime_rate: None,
                accounts: Vec::new(),
            }),
        })
    );
}

#[test]
fn managed_task_observation_changes_refresh_only_the_matching_sse_topic() {
    let runtime = SubscriptionTopic::ManagedTaskRuntime;
    let timeline = SubscriptionTopic::ManagedTaskTimeline;
    let timeline_v2 = SubscriptionTopic::ManagedTaskTimelineV2;
    let catalog = SubscriptionTopic::ManagedTaskCatalog;
    let detail = SubscriptionTopic::ManagedTaskDetail {
        task_key: "retention_archive".to_string(),
    };
    let workload = SubscriptionTopic::ManagedTaskWorkload {
        task_key: "retention_archive".to_string(),
        window_hours: 24,
        limit: 200,
    };
    use crate::task_timeline::TaskObservationChange::{Runtime, Timeline};

    assert!(managed_task_change_matches_topic(&Runtime, &runtime));
    assert!(!managed_task_change_matches_topic(&Runtime, &timeline));
    assert!(managed_task_change_matches_topic(&Runtime, &catalog));
    assert!(!managed_task_change_matches_topic(&Timeline, &runtime));
    assert!(managed_task_change_matches_topic(&Timeline, &timeline));
    assert!(managed_task_change_matches_topic(&Timeline, &timeline_v2));
    assert!(managed_task_change_matches_topic(&Timeline, &catalog));
    assert!(managed_task_change_matches_topic(
        &crate::task_timeline::TaskObservationChange::Workload("retention_archive".to_string()),
        &detail,
    ));
    assert!(managed_task_change_matches_topic(
        &crate::task_timeline::TaskObservationChange::Workload("retention_archive".to_string()),
        &catalog,
    ));
    assert!(managed_task_change_matches_topic(
        &crate::task_timeline::TaskObservationChange::Workload("retention_archive".to_string()),
        &workload,
    ));
    assert!(!managed_task_change_matches_topic(
        &crate::task_timeline::TaskObservationChange::Workload("another_task".to_string()),
        &detail,
    ));
    assert!(!managed_task_change_matches_topic(
        &crate::task_timeline::TaskObservationChange::Workload("another_task".to_string()),
        &workload,
    ));
}

#[tokio::test]
async fn managed_task_refresh_generation_covers_every_task_topic() {
    let hub = SubscriptionHub::new();
    let topics = [
        SubscriptionTopic::ManagedTaskCatalog,
        SubscriptionTopic::ManagedTaskRuntime,
        SubscriptionTopic::ManagedTaskTimeline,
        SubscriptionTopic::ManagedTaskTimelineV2,
        SubscriptionTopic::ManagedTaskDetail {
            task_key: "retention_archive".to_string(),
        },
        SubscriptionTopic::ManagedTaskWorkload {
            task_key: "retention_archive".to_string(),
            window_hours: 24,
            limit: 200,
        },
    ];

    for topic in topics {
        assert!(is_managed_task_refresh_topic(&topic));
        assert!(hub.mark_managed_task_topic_dirty(&topic).await);
        assert!(hub.mark_managed_task_topic_dirty(&topic).await);
        let topic_key = topic.cache_key().expect("managed task topic key");
        let generation = hub
            .state
            .lock()
            .await
            .managed_task_refresh_generations
            .get(&topic_key)
            .copied();
        assert_eq!(generation, Some(2));
    }
}

#[tokio::test]
async fn failed_managed_task_timeline_refresh_releases_coalescing_latch() {
    let state = crate::tests::test_state_with_openai_base(
        Url::parse("http://127.0.0.1:9").expect("valid test URL"),
    )
    .await;
    let hub = state.subscription_hub.clone();
    let topic = SubscriptionTopic::ManagedTaskTimelineV2;
    let topic_key = topic.cache_key().expect("managed task timeline key");
    let mut cached = seeded_cached_topic(topic.clone(), &[], Utc::now());
    cached.dirty = true;
    cached.snapshot_payload = json!({"watermark": 1});
    {
        let mut guard = hub.state.lock().await;
        guard.topics.insert(topic_key.clone(), cached);
        guard.active_subscribers.insert(topic_key.clone(), 1);
    }

    let result = hub.refresh_topic_if_active(state, topic, true).await;
    assert!(
        result.is_err(),
        "malformed cached timeline must fail closed"
    );

    let guard = hub.state.lock().await;
    let cached = guard
        .topics
        .get(&topic_key)
        .expect("cached managed task timeline topic");
    assert!(cached.dirty, "failed refresh must retain the last-good gap");
    assert!(
        !cached.refresh_scheduled,
        "failed refresh must release the retry latch"
    );
}

#[test]
fn model_routing_live_sse_topic_is_bounded_and_refreshes_only_on_route_updates() {
    let descriptor = SubscriptionTopicDescriptor {
        topic: "pool.model-routing-live".to_string(),
        params: BTreeMap::from([
            ("window".to_string(), "1h".to_string()),
            ("model".to_string(), "gpt-5.5".to_string()),
            ("state".to_string(), "cooling_down".to_string()),
            ("limit".to_string(), "100".to_string()),
        ]),
    };
    let topic = SubscriptionTopic::from_descriptor(&descriptor)
        .expect("model routing topic descriptor should parse");

    assert_eq!(topic.descriptor(), descriptor);
    assert_eq!(topic.class(), SubscriptionTopicClass::BoundedColdHydrate);
    assert_eq!(topic.schema_epoch(), "pool.model-routing-live/v1");
    assert_eq!(
        topic.runtime_topic_dependencies(),
        vec![RuntimeTopicDependency::ModelRouting]
    );
    assert!(topic.is_affected_by_runtime_mutation(&RuntimeMutation::ModelRoutingChanged));
    assert!(
        !topic.is_affected_by_runtime_mutation(&RuntimeMutation::AttemptChanged {
            invoke_id: "unrelated".to_string(),
        })
    );
    assert!(
        SubscriptionTopic::from_descriptor(&SubscriptionTopicDescriptor {
            topic: "pool.model-routing-live".to_string(),
            params: BTreeMap::from([("window".to_string(), "48h".to_string())]),
        })
        .is_err()
    );
}

#[test]
fn conversation_detail_topics_require_an_unambiguous_scope() {
    let prompt_cache_descriptor = SubscriptionTopicDescriptor {
        topic: "invocation-history.window".to_string(),
        params: BTreeMap::from([("promptCacheKey".to_string(), "pck-1".to_string())]),
    };
    let sticky_descriptor = SubscriptionTopicDescriptor {
        topic: "prompt-cache.conversation-operations.window".to_string(),
        params: BTreeMap::from([
            ("stickyKey".to_string(), "sticky-1".to_string()),
            ("upstreamAccountId".to_string(), "42".to_string()),
            ("infoType".to_string(), "routing".to_string()),
        ]),
    };

    let prompt_cache_topic = SubscriptionTopic::from_descriptor(&prompt_cache_descriptor)
        .expect("prompt cache scope should parse");
    let sticky_topic =
        SubscriptionTopic::from_descriptor(&sticky_descriptor).expect("sticky scope should parse");

    assert_eq!(prompt_cache_topic.descriptor(), prompt_cache_descriptor);
    assert_eq!(sticky_topic.descriptor(), sticky_descriptor);
    assert!(
        SubscriptionTopic::from_descriptor(&SubscriptionTopicDescriptor {
            topic: "invocation-history.overview".to_string(),
            params: BTreeMap::new(),
        })
        .is_err()
    );
    assert!(
        SubscriptionTopic::from_descriptor(&SubscriptionTopicDescriptor {
            topic: "invocation-history.overview".to_string(),
            params: BTreeMap::from([
                ("promptCacheKey".to_string(), "pck-1".to_string()),
                ("stickyKey".to_string(), "sticky-1".to_string()),
                ("upstreamAccountId".to_string(), "42".to_string()),
            ]),
        })
        .is_err()
    );
}

#[test]
fn typed_runtime_binding_events_only_refresh_binding_and_operations_topics() {
    let scope_params = BTreeMap::from([("promptCacheKey".to_string(), "pck-1".to_string())]);
    let calls = SubscriptionTopic::from_descriptor(&SubscriptionTopicDescriptor {
        topic: "invocation-history.window".to_string(),
        params: scope_params.clone(),
    })
    .expect("calls topic should parse");
    let overview = SubscriptionTopic::from_descriptor(&SubscriptionTopicDescriptor {
        topic: "invocation-history.overview".to_string(),
        params: scope_params.clone(),
    })
    .expect("overview topic should parse");
    let binding = SubscriptionTopic::from_descriptor(&SubscriptionTopicDescriptor {
        topic: "prompt-cache.conversation-binding.current".to_string(),
        params: scope_params.clone(),
    })
    .expect("binding topic should parse");
    let operations = SubscriptionTopic::from_descriptor(&SubscriptionTopicDescriptor {
        topic: "prompt-cache.conversation-operations.window".to_string(),
        params: scope_params,
    })
    .expect("operations topic should parse");
    let event = RuntimeMutation::PromptCacheBindingChanged {
        prompt_cache_key: "pck-1".to_string(),
    };

    assert!(!calls.is_affected_by_runtime_mutation(&event));
    assert!(!overview.is_affected_by_runtime_mutation(&event));
    assert!(binding.is_affected_by_runtime_mutation(&event));
    assert!(operations.is_affected_by_runtime_mutation(&event));
    assert!(!binding.is_affected_by_runtime_mutation(
        &RuntimeMutation::PromptCacheBindingChanged {
            prompt_cache_key: "pck-2".to_string(),
        }
    ));
    assert!(
        !binding.is_affected_by_runtime_mutation(&RuntimeMutation::AttemptChanged {
            invoke_id: "other".to_string(),
        })
    );
    assert!(
        !operations.is_affected_by_runtime_mutation(&RuntimeMutation::AttemptChanged {
            invoke_id: "other".to_string(),
        })
    );
}

#[test]
fn sticky_route_changes_refresh_only_the_previous_and_current_history_scopes() {
    let topic_for = |upstream_account_id| SubscriptionTopic::InvocationHistoryWindow {
        scope: ConversationSubscriptionScope::StickyKey {
            sticky_key: "sticky-1".to_string(),
            upstream_account_id,
        },
    };
    let prompt_cache_topic = SubscriptionTopic::InvocationHistoryOverview {
        scope: ConversationSubscriptionScope::PromptCacheKey("sticky-1".to_string()),
    };
    let event = RuntimeMutation::StickyRouteChanged {
        sticky_key: "sticky-1".to_string(),
        previous_upstream_account_id: 41,
        upstream_account_id: 42,
    };

    assert!(topic_for(41).is_affected_by_runtime_mutation(&event));
    assert!(topic_for(42).is_affected_by_runtime_mutation(&event));
    assert!(!topic_for(43).is_affected_by_runtime_mutation(&event));
    assert!(prompt_cache_topic.is_affected_by_runtime_mutation(&event));
}

#[tokio::test]
async fn conversation_overview_refresh_marks_events_arriving_during_rebuild_for_rerun() {
    let state = crate::tests::test_state_with_openai_base(
        Url::parse("http://127.0.0.1:9").expect("valid test URL"),
    )
    .await;
    let hub = SubscriptionHub::new();
    let topic = SubscriptionTopic::InvocationHistoryOverview {
        scope: ConversationSubscriptionScope::PromptCacheKey("pck-1".to_string()),
    };
    let topic_key = topic.cache_key().expect("conversation overview topic key");

    {
        let mut guard = hub.state.lock().await;
        let mut cached = seeded_cached_topic(topic.clone(), &[], Utc::now());
        cached.conversation_overview_refresh_scheduled = true;
        cached.conversation_overview_refresh_in_flight = true;
        guard.topics.insert(topic_key.clone(), cached);
        guard.active_subscribers.insert(topic_key.clone(), 1);
    }

    hub.schedule_conversation_overview_topic_refresh(state, topic)
        .await
        .expect("in-flight conversation overview refresh should accept a pending event");

    let guard = hub.state.lock().await;
    assert!(
        guard
            .topics
            .get(&topic_key)
            .is_some_and(|cached| cached.conversation_overview_refresh_pending)
    );
}

#[tokio::test]
async fn upstream_account_attempt_refresh_coalesces_events_during_rebuild() {
    let state = crate::tests::test_state_with_openai_base(
        Url::parse("http://127.0.0.1:9").expect("valid test URL"),
    )
    .await;
    let hub = state.subscription_hub.clone();
    let topic = SubscriptionTopic::UpstreamAccountAttemptsWindow {
        account_id: 42,
        page: 1,
        page_size: 50,
        attempt_type: None,
        model: None,
        sticky_key: None,
    };
    let topic_key = topic.cache_key().expect("account attempt topic key");

    {
        let mut guard = hub.state.lock().await;
        let mut cached = seeded_cached_topic(topic.clone(), &[], Utc::now());
        cached.upstream_account_attempt_refresh_scheduled = true;
        cached.upstream_account_attempt_refresh_in_flight = true;
        guard.topics.insert(topic_key.clone(), cached);
        guard.active_subscribers.insert(topic_key.clone(), 1);
    }

    hub.schedule_upstream_account_attempt_topic_refresh(state, topic.clone())
        .await
        .expect("in-flight account attempt refresh should accept a pending event");

    {
        let guard = hub.state.lock().await;
        assert!(
            guard
                .topics
                .get(&topic_key)
                .is_some_and(|cached| cached.upstream_account_attempt_refresh_pending)
        );
    }
    assert!(
        hub.finish_upstream_account_attempt_topic_refresh(&topic, 0)
            .await,
        "an event during the build must schedule exactly one follow-up frame"
    );
    assert!(
        hub.rearm_upstream_account_attempt_topic_refresh(&topic, 0)
            .await,
        "the active topic must retain the fixed-window follow-up owner"
    );
    let guard = hub.state.lock().await;
    let cached = guard.topics.get(&topic_key).expect("cached account topic");
    assert!(cached.upstream_account_attempt_refresh_scheduled);
    assert!(!cached.upstream_account_attempt_refresh_in_flight);
    assert!(!cached.upstream_account_attempt_refresh_pending);
    assert_eq!(
        UPSTREAM_ACCOUNT_ATTEMPTS_TOPIC_REFRESH_DEBOUNCE,
        Duration::from_millis(250)
    );
}

#[tokio::test]
async fn upstream_account_attempt_refresh_reconnect_invalidates_stale_debounce_task() {
    let hub = SubscriptionHub::new();
    let topic = SubscriptionTopic::UpstreamAccountAttemptsWindow {
        account_id: 42,
        page: 1,
        page_size: 50,
        attempt_type: None,
        model: None,
        sticky_key: None,
    };
    let topic_key = topic.cache_key().expect("account attempt topic key");
    let stale_generation = 7;

    {
        let mut guard = hub.state.lock().await;
        let mut cached = seeded_cached_topic(topic.clone(), &[], Utc::now());
        cached.upstream_account_attempt_refresh_generation = stale_generation;
        cached.upstream_account_attempt_refresh_scheduled = true;
        guard.topics.insert(topic_key.clone(), cached);
        guard.active_subscribers.insert(topic_key.clone(), 1);
    }

    hub.release_topic_subscribers(vec![topic_key.clone()], Vec::new(), false)
        .await;

    let current_generation = {
        let mut guard = hub.state.lock().await;
        let current_generation = {
            let cached = guard
                .topics
                .get_mut(&topic_key)
                .expect("cached account attempt topic");
            assert!(
                cached.upstream_account_attempt_refresh_generation > stale_generation,
                "disconnect must invalidate the queued task"
            );
            cached.upstream_account_attempt_refresh_generation = cached
                .upstream_account_attempt_refresh_generation
                .saturating_add(1);
            cached.upstream_account_attempt_refresh_scheduled = true;
            cached.upstream_account_attempt_refresh_generation
        };
        guard.active_subscribers.insert(topic_key.clone(), 1);
        current_generation
    };

    assert!(
        !hub.begin_upstream_account_attempt_topic_refresh(&topic, stale_generation)
            .await,
        "the disconnected task must not claim the reconnected refresh lease"
    );
    assert!(
        hub.begin_upstream_account_attempt_topic_refresh(&topic, current_generation)
            .await,
        "the reconnected task must claim the current refresh lease"
    );

    {
        let mut guard = hub.state.lock().await;
        let cached = guard
            .topics
            .get_mut(&topic_key)
            .expect("cached account attempt topic");
        cached.upstream_account_attempt_refresh_pending = true;
    }
    assert!(
        !hub.finish_upstream_account_attempt_topic_refresh(&topic, stale_generation)
            .await,
        "a stale task must not clear the current task's pending follow-up"
    );
    {
        let guard = hub.state.lock().await;
        let cached = guard
            .topics
            .get(&topic_key)
            .expect("cached account attempt topic");
        assert!(cached.upstream_account_attempt_refresh_scheduled);
        assert!(cached.upstream_account_attempt_refresh_in_flight);
        assert!(cached.upstream_account_attempt_refresh_pending);
    }
    assert!(
        hub.finish_upstream_account_attempt_topic_refresh(&topic, current_generation)
            .await,
        "the current task must retain its exactly-once follow-up"
    );
}

#[tokio::test]
async fn upstream_account_attempt_refresh_reconnect_rejects_stale_worker_after_begin() {
    let state = crate::tests::test_state_with_openai_base(
        Url::parse("http://127.0.0.1:9").expect("valid test URL"),
    )
    .await;
    let hub = state.subscription_hub.clone();
    let topic = SubscriptionTopic::UpstreamAccountAttemptsWindow {
        account_id: 42,
        page: 1,
        page_size: 50,
        attempt_type: None,
        model: None,
        sticky_key: None,
    };
    let topic_key = topic.cache_key().expect("account attempt topic key");
    let stale_generation = 7;

    {
        let mut guard = hub.state.lock().await;
        let mut cached = seeded_cached_topic(topic.clone(), &[7], Utc::now());
        cached.upstream_account_attempt_refresh_generation = stale_generation;
        cached.upstream_account_attempt_refresh_scheduled = true;
        guard.topics.insert(topic_key.clone(), cached);
        guard.active_subscribers.insert(topic_key.clone(), 1);
    }

    assert!(
        hub.begin_upstream_account_attempt_topic_refresh(&topic, stale_generation)
            .await,
        "the old worker must have acquired its lease before the owner disconnects"
    );
    assert!(
        hub.finish_upstream_account_attempt_topic_refresh_without_owner(&topic, stale_generation,)
            .await,
        "the worker should still see its original owner before the reconnect"
    );

    hub.release_topic_subscribers(vec![topic_key.clone()], Vec::new(), false)
        .await;
    {
        let mut guard = hub.state.lock().await;
        guard.active_subscribers.insert(topic_key.clone(), 1);
    }
    let refreshed = hub
        .refresh_topic_if_active(state.clone(), topic.clone(), true)
        .await
        .expect("the reconnected owner refresh should build")
        .expect("the reconnected owner remains active");
    let cursor_after_reconnect = refreshed.cursor;
    let payload_after_reconnect = refreshed.snapshot_frame.payload_value();

    assert!(
        hub.refresh_upstream_account_attempt_topic_if_active(
            state,
            topic.clone(),
            true,
            stale_generation,
        )
        .await
        .expect("a stale worker exits without a build error")
        .is_none(),
        "a worker generation invalidated after begin must not overwrite the reconnected frame"
    );
    assert!(
        !hub.finish_upstream_account_attempt_topic_refresh(&topic, stale_generation)
            .await,
        "a stale worker must not affect the reconnected scheduler state"
    );
    let guard = hub.state.lock().await;
    let cached = guard
        .topics
        .get(&topic_key)
        .expect("cached account attempt topic");
    assert_eq!(cached.cursor, cursor_after_reconnect);
    assert_eq!(
        cached.snapshot_frame.payload_value(),
        payload_after_reconnect
    );
}

#[tokio::test]
async fn pool_attempt_broadcast_schedules_only_matching_active_account_topic() {
    let state = crate::tests::test_state_with_openai_base(
        Url::parse("http://127.0.0.1:9").expect("valid test URL"),
    )
    .await;
    let hub = state.subscription_hub.clone();
    let matching_topic = SubscriptionTopic::UpstreamAccountAttemptsWindow {
        account_id: 42,
        page: 1,
        page_size: 50,
        attempt_type: None,
        model: None,
        sticky_key: None,
    };
    let other_topic = SubscriptionTopic::UpstreamAccountAttemptsWindow {
        account_id: 43,
        page: 1,
        page_size: 50,
        attempt_type: None,
        model: None,
        sticky_key: None,
    };
    let matching_key = matching_topic.cache_key().expect("matching topic key");
    let other_key = other_topic.cache_key().expect("other topic key");
    {
        let mut guard = hub.state.lock().await;
        for (topic, topic_key) in [(&matching_topic, &matching_key), (&other_topic, &other_key)] {
            guard.topics.insert(
                topic_key.clone(),
                seeded_cached_topic(topic.clone(), &[], Utc::now()),
            );
            guard.active_subscribers.insert(topic_key.clone(), 1);
        }
    }
    let attempt = insert_upstream_account_attempt_fixture(
        state.as_ref(),
        42,
        "account-attempt-match-only",
        None,
        None,
    )
    .await;

    hub.handle_internal_broadcast(
        state,
        BroadcastPayload::PoolAttempts {
            invoke_id: attempt.invoke_id.clone(),
            attempts: vec![attempt],
        },
    )
    .await;

    let guard = hub.state.lock().await;
    assert!(
        guard
            .topics
            .get(&matching_key)
            .is_some_and(|cached| cached.upstream_account_attempt_refresh_scheduled)
    );
    assert!(
        guard
            .topics
            .get(&other_key)
            .is_some_and(|cached| !cached.upstream_account_attempt_refresh_scheduled)
    );
}

#[tokio::test]
async fn unavailable_pool_attempt_snapshot_recovers_only_active_account_topics() {
    let state = crate::tests::test_state_with_openai_base(
        Url::parse("http://127.0.0.1:9").expect("valid test URL"),
    )
    .await;
    let hub = state.subscription_hub.clone();
    let first_topic = SubscriptionTopic::UpstreamAccountAttemptsWindow {
        account_id: 42,
        page: 1,
        page_size: 50,
        attempt_type: None,
        model: None,
        sticky_key: None,
    };
    let second_topic = SubscriptionTopic::UpstreamAccountAttemptsWindow {
        account_id: 43,
        page: 2,
        page_size: 20,
        attempt_type: Some("responses".to_string()),
        model: Some("gpt-5".to_string()),
        sticky_key: Some("sticky-43".to_string()),
    };
    let inactive_topic = SubscriptionTopic::UpstreamAccountAttemptsWindow {
        account_id: 44,
        page: 1,
        page_size: 50,
        attempt_type: None,
        model: None,
        sticky_key: None,
    };
    let history_topic = SubscriptionTopic::InvocationHistoryWindow {
        scope: ConversationSubscriptionScope::PromptCacheKey("unaffected-history".to_string()),
    };
    let first_key = first_topic.cache_key().expect("first account topic key");
    let second_key = second_topic.cache_key().expect("second account topic key");
    let inactive_key = inactive_topic
        .cache_key()
        .expect("inactive account topic key");
    let history_key = history_topic.cache_key().expect("history topic key");
    let (first_payload, inactive_payload, history_payload, stale_generation) = {
        let mut guard = hub.state.lock().await;
        for (topic, topic_key, cursor) in [
            (&first_topic, &first_key, 41),
            (&second_topic, &second_key, 42),
            (&inactive_topic, &inactive_key, 43),
            (&history_topic, &history_key, 44),
        ] {
            guard.topics.insert(
                topic_key.clone(),
                seeded_cached_topic(topic.clone(), &[cursor], Utc::now()),
            );
        }
        for topic_key in [&first_key, &second_key, &history_key] {
            guard.active_subscribers.insert(topic_key.clone(), 1);
        }
        (
            guard
                .topics
                .get(&first_key)
                .expect("first cached topic")
                .snapshot_frame
                .payload_value()
                .clone(),
            guard
                .topics
                .get(&inactive_key)
                .expect("inactive cached topic")
                .snapshot_frame
                .payload_value()
                .clone(),
            guard
                .topics
                .get(&history_key)
                .expect("history cached topic")
                .snapshot_frame
                .payload_value()
                .clone(),
            guard
                .topics
                .get(&first_key)
                .expect("first cached topic")
                .runtime_topic_recovery_generation,
        )
    };

    hub.handle_internal_broadcast(
        state,
        BroadcastPayload::PoolAttemptsSnapshotUnavailable {
            invoke_id: "snapshot-query-failed".to_string(),
        },
    )
    .await;

    let guard = hub.state.lock().await;
    for topic_key in [&first_key, &second_key] {
        let cached = guard.topics.get(topic_key).expect("active account topic");
        assert!(cached.dirty);
        assert!(cached.upstream_account_attempt_refresh_scheduled);
        assert!(!cached.upstream_account_attempt_refresh_in_flight);
        assert!(!cached.upstream_account_attempt_refresh_pending);
        assert_eq!(cached.continuity_reset_cursor, Some(cached.cursor));
        assert_ne!(cached.runtime_topic_recovery_generation, stale_generation);
    }
    let first = guard.topics.get(&first_key).expect("first account topic");
    assert_eq!(first.cursor, 41);
    assert_eq!(first.snapshot_frame.payload_value(), first_payload);

    let inactive = guard
        .topics
        .get(&inactive_key)
        .expect("inactive account topic");
    assert!(!inactive.dirty);
    assert!(!inactive.upstream_account_attempt_refresh_scheduled);
    assert_eq!(inactive.cursor, 43);
    assert_eq!(inactive.snapshot_frame.payload_value(), inactive_payload);

    let history = guard.topics.get(&history_key).expect("history topic");
    assert!(!history.dirty);
    assert_eq!(history.cursor, 44);
    assert_eq!(history.snapshot_frame.payload_value(), history_payload);
}

#[tokio::test]
async fn unavailable_pool_attempt_snapshot_from_producer_recovers_active_account_topic() {
    let state = crate::tests::test_state_with_openai_base(
        Url::parse("http://127.0.0.1:9").expect("valid test URL"),
    )
    .await;
    let hub = state.subscription_hub.clone();
    let topic = SubscriptionTopic::UpstreamAccountAttemptsWindow {
        account_id: 42,
        page: 1,
        page_size: 50,
        attempt_type: None,
        model: None,
        sticky_key: None,
    };
    let topic_key = topic.cache_key().expect("account attempt topic key");
    {
        let mut guard = hub.state.lock().await;
        guard.topics.insert(
            topic_key.clone(),
            seeded_cached_topic(topic.clone(), &[7], Utc::now()),
        );
    }
    let _lease = hub
        .register_topic_subscribers(std::slice::from_ref(&topic))
        .await
        .expect("register active account attempt topic");
    spawn_subscription_broadcast_listener(state.clone());
    state.pool.close().await;

    let err = crate::proxy::broadcast_pool_upstream_attempts_snapshot(
        state.as_ref(),
        "snapshot-query-failed",
    )
    .await
    .expect_err("closed SQLite pool must fail the attempt snapshot query");
    assert!(
        err.to_string()
            .contains("failed to load pool attempt snapshot")
    );

    tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            let recovered = hub
                .state
                .lock()
                .await
                .topics
                .get(&topic_key)
                .is_some_and(|cached| {
                    cached.dirty
                        && cached.upstream_account_attempt_refresh_scheduled
                        && cached.upstream_account_attempt_refresh_generation > 0
                });
            if recovered {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the internal listener should schedule active account-topic recovery");

    state.shutdown.cancel();
}

#[tokio::test]
async fn lagged_internal_broadcast_receiver_recovers_active_account_attempt_topics() {
    let state = crate::tests::test_state_with_openai_base(
        Url::parse("http://127.0.0.1:9").expect("valid test URL"),
    )
    .await;
    let hub = state.subscription_hub.clone();
    let first_topic = SubscriptionTopic::UpstreamAccountAttemptsWindow {
        account_id: 42,
        page: 1,
        page_size: 50,
        attempt_type: None,
        model: None,
        sticky_key: None,
    };
    let second_topic = SubscriptionTopic::UpstreamAccountAttemptsWindow {
        account_id: 43,
        page: 2,
        page_size: 20,
        attempt_type: Some("responses".to_string()),
        model: Some("gpt-5".to_string()),
        sticky_key: Some("sticky-43".to_string()),
    };
    let inactive_topic = SubscriptionTopic::UpstreamAccountAttemptsWindow {
        account_id: 44,
        page: 1,
        page_size: 50,
        attempt_type: None,
        model: None,
        sticky_key: None,
    };
    let first_key = first_topic.cache_key().expect("first account topic key");
    let second_key = second_topic.cache_key().expect("second account topic key");
    let inactive_key = inactive_topic
        .cache_key()
        .expect("inactive account topic key");
    let (first_last_good_payload, stale_refresh_generation) = {
        let mut guard = hub.state.lock().await;
        for (topic, topic_key, cursor) in [
            (&first_topic, &first_key, 41),
            (&second_topic, &second_key, 42),
            (&inactive_topic, &inactive_key, 43),
        ] {
            guard.topics.insert(
                topic_key.clone(),
                seeded_cached_topic(topic.clone(), &[cursor], Utc::now()),
            );
        }
        guard.active_subscribers.insert(first_key.clone(), 1);
        guard.active_subscribers.insert(second_key.clone(), 1);
        let cached = guard
            .topics
            .get(&first_key)
            .expect("first cached account topic");
        (
            cached.snapshot_frame.payload_value().clone(),
            cached.runtime_topic_recovery_generation,
        )
    };

    let mut receiver = state.broadcaster.subscribe();
    for sequence in 0..32 {
        state
            .broadcaster
            .send(BroadcastPayload::Version {
                version: format!("broadcast-lag-{sequence}"),
            })
            .expect("test listener should receive each broadcast");
    }
    let skipped = match receiver.recv().await {
        Err(broadcast::error::RecvError::Lagged(skipped)) => skipped,
        outcome => panic!("expected a lagged internal broadcast receiver, got {outcome:?}"),
    };
    assert!(skipped > 0);

    hub.mark_internal_broadcast_gap_and_recover(state, skipped)
        .await;

    let guard = hub.state.lock().await;
    for topic_key in [&first_key, &second_key] {
        let cached = guard.topics.get(topic_key).expect("active account topic");
        assert!(
            cached.dirty,
            "broadcast lag must retain the last-good frame"
        );
        assert!(
            cached.upstream_account_attempt_refresh_scheduled,
            "every active account topic must enter the dedicated recovery"
        );
        assert!(!cached.upstream_account_attempt_refresh_in_flight);
        assert!(!cached.upstream_account_attempt_refresh_pending);
        assert_eq!(cached.continuity_reset_cursor, Some(cached.cursor));
    }
    assert_ne!(
        guard
            .topics
            .get(&first_key)
            .expect("first cached account topic")
            .runtime_topic_recovery_generation,
        stale_refresh_generation,
        "a lagged receiver must invalidate an older build before it can commit"
    );
    assert_eq!(
        guard
            .topics
            .get(&first_key)
            .expect("first cached account topic")
            .snapshot_frame
            .payload_value(),
        first_last_good_payload,
        "the recovery must not clear the last-good payload before the rebuild succeeds"
    );
    let inactive = guard
        .topics
        .get(&inactive_key)
        .expect("inactive account topic");
    assert!(!inactive.dirty);
    assert!(!inactive.upstream_account_attempt_refresh_scheduled);
}

#[tokio::test]
async fn lagged_broadcast_gap_rejects_stale_account_attempt_build_commit() {
    let state = crate::tests::test_state_with_openai_base(
        Url::parse("http://127.0.0.1:9").expect("valid test URL"),
    )
    .await;
    let hub = state.subscription_hub.clone();
    let topic = SubscriptionTopic::UpstreamAccountAttemptsWindow {
        account_id: 42,
        page: 1,
        page_size: 50,
        attempt_type: None,
        model: None,
        sticky_key: None,
    };
    let topic_key = topic.cache_key().expect("account attempt topic key");
    let last_good_payload = {
        let mut guard = hub.state.lock().await;
        guard.topics.insert(
            topic_key.clone(),
            seeded_cached_topic(topic.clone(), &[7], Utc::now()),
        );
        guard.active_subscribers.insert(topic_key.clone(), 1);
        guard
            .topics
            .get(&topic_key)
            .expect("cached account attempt topic")
            .snapshot_frame
            .payload_value()
            .clone()
    };

    // Exhaust the test pool so the old refresh captures its generation, then blocks before
    // it can build or commit a replacement frame.
    let mut held_connections = Vec::new();
    for _ in 0..4 {
        held_connections.push(
            state
                .pool
                .acquire()
                .await
                .expect("hold every test SQLite connection"),
        );
    }
    let old_refresh = tokio::spawn({
        let hub = hub.clone();
        let state = state.clone();
        let topic = topic.clone();
        async move { hub.refresh_topic_if_active(state, topic, true).await }
    });
    tokio::time::sleep(Duration::from_millis(10)).await;

    hub.mark_internal_broadcast_gap_and_recover(state.clone(), 1)
        .await;
    drop(held_connections);

    let old_result = tokio::time::timeout(Duration::from_secs(1), old_refresh)
        .await
        .expect("stale refresh must finish after SQLite is released")
        .expect("stale refresh task must not panic")
        .expect("stale refresh must not fail its database read");
    assert!(
        old_result.is_none(),
        "a build captured before the gap must be rejected at commit"
    );

    let guard = hub.state.lock().await;
    let cached = guard
        .topics
        .get(&topic_key)
        .expect("cached account attempt topic");
    assert!(cached.dirty);
    assert!(cached.upstream_account_attempt_refresh_scheduled);
    assert_eq!(cached.cursor, 7);
    assert_eq!(cached.snapshot_frame.payload_value(), last_good_payload);
}

#[tokio::test]
async fn dirty_account_attempt_broadcast_starts_the_dedicated_recovery() {
    let state = crate::tests::test_state_with_openai_base(
        Url::parse("http://127.0.0.1:9").expect("valid test URL"),
    )
    .await;
    let hub = state.subscription_hub.clone();
    let topic = SubscriptionTopic::UpstreamAccountAttemptsWindow {
        account_id: 42,
        page: 1,
        page_size: 50,
        attempt_type: None,
        model: None,
        sticky_key: None,
    };
    let topic_key = topic.cache_key().expect("account attempt topic key");
    {
        let mut guard = hub.state.lock().await;
        let mut cached = seeded_cached_topic(topic.clone(), &[7], Utc::now());
        cached.dirty = true;
        guard.topics.insert(topic_key.clone(), cached);
    }
    let lease = hub
        .register_topic_subscribers(std::slice::from_ref(&topic))
        .await
        .expect("register active account attempt topic");
    let attempt = insert_upstream_account_attempt_fixture(
        state.as_ref(),
        42,
        "account-attempt-dirty-recovery",
        None,
        None,
    )
    .await;

    hub.handle_internal_broadcast(
        state,
        BroadcastPayload::PoolAttempts {
            invoke_id: attempt.invoke_id.clone(),
            attempts: vec![attempt],
        },
    )
    .await;
    let guard = hub.state.lock().await;
    let cached = guard.topics.get(&topic_key).expect("dirty account topic");
    assert!(cached.dirty);
    assert!(
        cached.upstream_account_attempt_refresh_scheduled,
        "a matching event must enter the dedicated fixed-window recovery instead of being ignored"
    );
    assert!(guard.runtime_topic_recovery_queue.is_empty());
    drop(guard);
    drop(lease);
}

#[tokio::test]
async fn upstream_account_attempt_refresh_failure_retains_last_good_frame() {
    let state = crate::tests::test_state_with_openai_base(
        Url::parse("http://127.0.0.1:9").expect("valid test URL"),
    )
    .await;
    let hub = state.subscription_hub.clone();
    let topic = SubscriptionTopic::UpstreamAccountAttemptsWindow {
        account_id: 42,
        page: 1,
        page_size: 50,
        attempt_type: None,
        model: None,
        sticky_key: None,
    };
    let topic_key = topic.cache_key().expect("account topic key");
    {
        let mut guard = hub.state.lock().await;
        guard.topics.insert(
            topic_key.clone(),
            seeded_cached_topic(topic.clone(), &[7], Utc::now()),
        );
        guard.active_subscribers.insert(topic_key.clone(), 1);
    }
    let attempt = insert_upstream_account_attempt_fixture(
        state.as_ref(),
        42,
        "account-attempt-last-good",
        None,
        None,
    )
    .await;
    let (cursor_before, payload_before) = {
        let guard = hub.state.lock().await;
        let cached = guard.topics.get(&topic_key).expect("cached account topic");
        (cached.cursor, cached.snapshot_frame.payload_value())
    };
    state.pool.close().await;

    hub.handle_internal_broadcast(
        state,
        BroadcastPayload::PoolAttempts {
            invoke_id: attempt.invoke_id.clone(),
            attempts: vec![attempt],
        },
    )
    .await;

    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if hub
                .state
                .lock()
                .await
                .topics
                .get(&topic_key)
                .is_some_and(|cached| cached.dirty)
            {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("failed refresh should mark the account topic dirty");

    let guard = hub.state.lock().await;
    let cached = guard.topics.get(&topic_key).expect("cached account topic");
    assert_eq!(cached.cursor, cursor_before);
    assert_eq!(cached.snapshot_frame.payload_value(), payload_before);
}

#[tokio::test]
async fn conversation_overview_includes_runtime_records_in_chart_samples() {
    let state =
        crate::tests::test_state_with_openai_base(Url::parse("http://127.0.0.1:9").unwrap()).await;
    let prompt_cache_key = "runtime-overview-pck";
    let occurred_at = crate::proxy::shanghai_now_string();
    let runtime_record = crate::proxy::build_admitted_proxy_capture_runtime_snapshot(
        "runtime-overview-invoke",
        &occurred_at,
        ProxyCaptureTarget::Responses,
        None,
        None,
        Some(prompt_cache_key),
    );
    state
        .proxy_runtime_invocations
        .upsert(crate::proxy::api_invocation_from_runtime_record(
            &runtime_record,
        ));

    let topic = SubscriptionTopic::InvocationHistoryOverview {
        scope: ConversationSubscriptionScope::PromptCacheKey(prompt_cache_key.to_string()),
    };
    let payload = topic
        .build_payload(state)
        .await
        .expect("conversation overview payload should build");

    assert_eq!(
        payload
            .pointer("/summary/totalCount")
            .and_then(Value::as_i64),
        Some(1)
    );
    assert_eq!(payload.get("chartTotal").and_then(Value::as_i64), Some(1));
    assert_eq!(
        payload
            .get("records")
            .and_then(Value::as_array)
            .and_then(|records| records.first())
            .and_then(|record| record.get("invokeId"))
            .and_then(Value::as_str),
        Some("runtime-overview-invoke")
    );
}

#[tokio::test]
async fn conversation_overview_keeps_non_divisor_page_limits_contiguous() {
    let mut state = crate::tests::test_state_with_openai_base(
        Url::parse("http://127.0.0.1:9").expect("valid test URL"),
    )
    .await;
    Arc::get_mut(&mut state)
        .expect("overview test state should not have external owners")
        .config
        .list_limit_max = 37;
    let prompt_cache_key = "overview-page-limit-pck";
    for index in 0..75 {
        sqlx::query(
            r#"
                INSERT INTO codex_invocations (
                    invoke_id, occurred_at, source, status, payload, raw_response
                )
                VALUES (?1, ?2, 'proxy', 'success', ?3, '{}')
                "#,
        )
        .bind(format!("overview-page-limit-{index:03}"))
        .bind(format!("2026-03-02 12:{:02}:{:02}", index / 60, index % 60))
        .bind(format!(r#"{{"promptCacheKey":"{prompt_cache_key}"}}"#))
        .execute(&state.pool)
        .await
        .expect("insert overview pagination seed row");
    }

    let topic = SubscriptionTopic::InvocationHistoryOverview {
        scope: ConversationSubscriptionScope::PromptCacheKey(prompt_cache_key.to_string()),
    };
    let payload = topic
        .build_payload(state)
        .await
        .expect("conversation overview payload should build");
    let records = payload
        .get("records")
        .and_then(Value::as_array)
        .expect("overview records should serialize");
    let mut invoke_ids = records
        .iter()
        .filter_map(|record| record.get("invokeId").and_then(Value::as_str))
        .collect::<Vec<_>>();
    invoke_ids.sort_unstable();
    invoke_ids.dedup();

    assert_eq!(payload.get("chartTotal").and_then(Value::as_i64), Some(75));
    assert_eq!(records.len(), 75);
    assert_eq!(invoke_ids.len(), 75);
}

#[test]
fn dashboard_network_topics_use_projection_cadence_not_subscription_push_tasks() {
    let topics = [
        SubscriptionTopic::DashboardNetworkTimeseriesWindow {
            range: "today".to_string(),
            time_zone: SUBSCRIPTION_DEFAULT_TIME_ZONE.to_string(),
            upstream_account_id: None,
        },
        SubscriptionTopic::DashboardNetworkRecentCurrent,
    ];

    for topic in topics {
        assert!(topic.uses_dashboard_network_live_snapshot());
        assert!(
            !topic.uses_server_push_cadence(RuntimeProjectionMode::Auto),
            "{} must be driven by the shared network projection cadence",
            topic.name()
        );
    }
    assert!(
        SubscriptionTopic::DashboardNetworkRecentCurrent
            .uses_server_push_cadence(RuntimeProjectionMode::Legacy)
    );
}

#[tokio::test]
async fn dashboard_network_recent_legacy_push_cadence_emits_live_payload() {
    let state = crate::tests::test_state_with_openai_base_and_runtime_projection_mode(
        Url::parse("http://127.0.0.1:9").unwrap(),
        RuntimeProjectionMode::Legacy,
    )
    .await;
    let topic = SubscriptionTopic::DashboardNetworkRecentCurrent;
    let topic_key = topic.cache_key().expect("legacy recent topic key");
    let descriptor = topic.descriptor();
    let mut receiver = state.subscription_hub.subscribe();
    assert!(!topic.uses_server_push_cadence(RuntimeProjectionMode::Auto));
    assert!(topic.uses_server_push_cadence(RuntimeProjectionMode::Legacy));

    let _response = topic_sse_stream(
        State(state.clone()),
        Query(SubscriptionStreamQuery {
            topics: Some(
                serde_json::to_string(std::slice::from_ref(&descriptor))
                    .expect("serialize legacy recent topic"),
            ),
            resume: None,
            attempt: Some(1),
            reason: Some("legacy-network-cadence-test".to_string()),
            delta: None,
        }),
    )
    .await
    .expect("open legacy recent network SSE stream");
    {
        let guard = state.subscription_hub.state.lock().await;
        assert_eq!(
            guard.server_push_subscribers.get(&topic_key).copied(),
            Some(1),
            "legacy SSE entrypoint must retain the recent cadence owner",
        );
        assert!(guard.server_push_tasks.contains(&topic_key));
    }

    tokio::time::sleep(DASHBOARD_NETWORK_RECENT_TOPIC_PUSH_INTERVAL * 2).await;
    state.dashboard_network_speed_cache.record_request_bytes(
        "legacy-recent-network-cadence",
        &crate::proxy::shanghai_now_string(),
        None,
        Some("api.openai.com"),
        128,
        Utc::now(),
    );

    let dispatch = tokio::time::timeout(Duration::from_secs(2), receiver.recv())
        .await
        .expect("legacy recent network push should be emitted")
        .expect("legacy recent network dispatch");

    assert_eq!(dispatch.frame.descriptor, descriptor);
    assert_eq!(
        dispatch.frame.schema_epoch,
        "dashboard.network-recent.current/v1"
    );
    let dispatch_payload = dispatch.frame.payload_value();
    assert_eq!(
        dispatch_payload
            .get("windowSeconds")
            .and_then(Value::as_i64),
        Some(300)
    );
    assert_eq!(
        dispatch_payload
            .get("sampleSeconds")
            .and_then(Value::as_i64),
        Some(1)
    );
    assert_eq!(
        dispatch_payload
            .get("points")
            .and_then(Value::as_array)
            .map(Vec::len),
        Some(300)
    );
}
