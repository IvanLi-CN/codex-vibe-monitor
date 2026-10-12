use super::*;
use serde_json::json;

#[test]
fn all_time_account_overflow_markers_stay_bounded() {
    let mut state = SubscriptionHubState::default();
    for account_id in 0..=(SUMMARY_TERMINAL_OVERLAY_MAX_ACCOUNT_OVERFLOW_MARKERS as i64) {
        record_all_time_account_overflow_marker(&mut state, account_id, account_id as u64 + 1);
    }
    assert_eq!(
        state
            .summary_terminal_overlay_all_time_overflowed_through_account
            .len(),
        SUMMARY_TERMINAL_OVERLAY_MAX_ACCOUNT_OVERFLOW_MARKERS
    );
    assert_eq!(
        state.summary_terminal_overlay_all_time_overflowed_through_unknown_account,
        Some((SUMMARY_TERMINAL_OVERLAY_MAX_ACCOUNT_OVERFLOW_MARKERS as u64) + 1)
    );
}

#[tokio::test]
async fn summary_snapshot_registration_preserves_last_good_response() {
    let hub = SubscriptionHub::new();
    let key = SummarySnapshotKey::from_query(&SummaryQuery {
        window: Some("1d".to_string()),
        limit: None,
        time_zone: Some("Asia/Shanghai".to_string()),
        upstream_account_id: None,
    });
    let mut response = StatsTotals::default().into_response();
    response.total_count = 7;

    hub.store_summary_snapshot(key.clone(), response).await;
    hub.ensure_summary_snapshot_key(key.clone()).await;

    assert_eq!(
        hub.summary_snapshot(&key)
            .await
            .expect("registered key should retain its last-good response")
            .total_count,
        7
    );
}

#[test]
fn stale_summary_snapshot_does_not_serve_last_good_indefinitely() {
    let entry = SummarySnapshotEntry {
        response: Some(StatsTotals::default().into_response()),
        refreshed_at: Some(Instant::now() - SUMMARY_SNAPSHOT_MAX_STALE - Duration::from_secs(1)),
    };

    assert!(entry.fresh_response().is_none());
}

#[tokio::test]
async fn summary_snapshot_admission_rejects_keys_after_the_bounded_capacity() {
    let hub = SubscriptionHub::new();
    for index in 0..SUMMARY_SNAPSHOT_MAX_KEYS {
        assert!(
            hub.ensure_summary_snapshot_key(SummarySnapshotKey::from_query(&SummaryQuery {
                window: Some("1d".to_string()),
                limit: Some(index as i64 + 1),
                time_zone: Some("Asia/Shanghai".to_string()),
                upstream_account_id: Some(index as i64 + 1),
            }))
            .await
        );
    }

    assert!(
        !hub.ensure_summary_snapshot_key(SummarySnapshotKey::from_query(&SummaryQuery {
            window: Some("1d".to_string()),
            limit: Some(99),
            time_zone: Some("Asia/Shanghai".to_string()),
            upstream_account_id: Some(99),
        }))
        .await
    );
}

#[test]
fn summary_snapshot_key_canonicalizes_equivalent_query_forms() {
    let one_day = SummarySnapshotKey::from_query(&SummaryQuery {
        window: Some("1d".to_string()),
        limit: Some(99),
        time_zone: Some("Asia/Shanghai".to_string()),
        upstream_account_id: None,
    });
    let twenty_four_hours = SummarySnapshotKey::from_query(&SummaryQuery {
        window: Some("24h".to_string()),
        limit: None,
        time_zone: Some("Asia/Shanghai".to_string()),
        upstream_account_id: None,
    });

    assert_eq!(one_day, twenty_four_hours);
}

#[test]
fn runtime_mutation_sequence_gap_discards_partial_batch() {
    let mut last_sequence = 0;
    let first = [SequencedRuntimeMutation {
        sequence: 1,
        mutation: RuntimeMutation::AttemptChanged {
            invoke_id: "first".to_string(),
        },
    }];
    assert!(!runtime_mutation_batch_has_sequence_gap(
        &mut last_sequence,
        &first
    ));

    let gap = [SequencedRuntimeMutation {
        sequence: 3,
        mutation: RuntimeMutation::AttemptChanged {
            invoke_id: "after-gap".to_string(),
        },
    }];
    assert!(runtime_mutation_batch_has_sequence_gap(
        &mut last_sequence,
        &gap
    ));
    assert_eq!(last_sequence, 3);

    let recovered = [SequencedRuntimeMutation {
        sequence: 4,
        mutation: RuntimeMutation::AttemptChanged {
            invoke_id: "after-recovery".to_string(),
        },
    }];
    assert!(
        !runtime_mutation_batch_has_sequence_gap(&mut last_sequence, &recovered),
        "the first batch after recovery must advance from the discarded gap batch"
    );
}

#[tokio::test]
async fn runtime_dependency_index_selects_only_the_matching_active_history_topic() {
    let hub = Arc::new(SubscriptionHub::new());
    let active_topic = SubscriptionTopic::InvocationHistoryWindow {
        scope: ConversationSubscriptionScope::PromptCacheKey("selected-key".to_string()),
    };
    let inactive_topic = SubscriptionTopic::InvocationHistoryWindow {
        scope: ConversationSubscriptionScope::PromptCacheKey("retained-key".to_string()),
    };
    let active_key = active_topic.cache_key().expect("active topic key");
    let inactive_key = inactive_topic.cache_key().expect("inactive topic key");
    {
        let mut guard = hub.state.lock().await;
        guard.topics.insert(
            active_key.clone(),
            seeded_cached_topic(active_topic.clone(), &[], Utc::now()),
        );
        guard.topics.insert(
            inactive_key.clone(),
            seeded_cached_topic(inactive_topic, &[], Utc::now()),
        );
    }
    let lease = hub
        .register_topic_subscribers(std::slice::from_ref(&active_topic))
        .await
        .expect("register active history topic");
    let mutations = [SequencedRuntimeMutation {
        sequence: 1,
        mutation: RuntimeMutation::Invocation(RuntimeInvocationMutation {
            identity: RuntimeInvocationIdentity::new("invoke-1", "2026-08-09 12:00:00"),
            kind: RuntimeMutationKind::RuntimeUpsert,
            row_id: None,
            is_terminal: false,
            prompt_cache_key: Some("selected-key".to_string()),
            sticky_key: None,
            upstream_account_id: None,
        }),
    }];

    let guard = hub.state.lock().await;
    let work = SubscriptionHub::collect_runtime_topic_work(&guard, &mutations);
    let indexed = SubscriptionHub::active_topic_keys_for_dependency(
        &guard,
        &RuntimeTopicDependency::HistoryPromptCacheKey("selected-key".to_string()),
    );
    assert_eq!(indexed, vec![active_key.clone()]);
    assert_eq!(work.len(), 1);
    assert_eq!(
        work[0].topic.cache_key().expect("work topic key"),
        active_key
    );
    assert_ne!(
        work[0].topic.cache_key().expect("work topic key"),
        inactive_key,
        "retained inactive topic must not become router work"
    );
    drop(guard);
    drop(lease);
}

#[tokio::test]
async fn releasing_last_owner_removes_dependency_index_and_marks_last_good_dirty() {
    let hub = Arc::new(SubscriptionHub::new());
    let topic = SubscriptionTopic::InvocationHistoryWindow {
        scope: ConversationSubscriptionScope::PromptCacheKey("disconnect-key".to_string()),
    };
    let topic_key = topic.cache_key().expect("history topic key");
    {
        let mut guard = hub.state.lock().await;
        guard.topics.insert(
            topic_key.clone(),
            seeded_cached_topic(topic.clone(), &[], Utc::now()),
        );
    }
    let mut lease = hub
        .register_topic_subscribers(std::slice::from_ref(&topic))
        .await
        .expect("register history owner");
    let topic_keys = std::mem::take(&mut lease.topic_keys);
    let topic_names = std::mem::take(&mut lease.topic_names);
    hub.release_topic_subscribers(topic_keys, topic_names, lease.owns_dashboard_live)
        .await;
    drop(lease);

    let guard = hub.state.lock().await;
    assert!(!guard.active_topics.contains_key(&topic_key));
    assert!(
        SubscriptionHub::active_topic_keys_for_dependency(
            &guard,
            &RuntimeTopicDependency::HistoryPromptCacheKey("disconnect-key".to_string()),
        )
        .is_empty()
    );
    assert!(
        guard
            .topics
            .get(&topic_key)
            .is_some_and(|cached| cached.dirty),
        "the retained frame must rebuild before a future owner reconnects"
    );
}

#[tokio::test]
async fn prompt_cache_dependency_index_omits_retained_inactive_windows() {
    let hub = Arc::new(SubscriptionHub::new());
    let active_topic = SubscriptionTopic::PromptCacheWindow {
        selection: PromptCacheConversationSelection::Count(20),
        detail_level: PromptCacheConversationDetailLevel::Full,
        recent_invocation_limit: Some(16),
    };
    let inactive_topic = SubscriptionTopic::PromptCacheWindow {
        selection: PromptCacheConversationSelection::Count(10),
        detail_level: PromptCacheConversationDetailLevel::Compact,
        recent_invocation_limit: Some(5),
    };
    let active_key = active_topic
        .cache_key()
        .expect("active prompt cache topic key");
    let inactive_key = inactive_topic
        .cache_key()
        .expect("inactive prompt cache topic key");
    {
        let mut guard = hub.state.lock().await;
        guard.topics.insert(
            active_key.clone(),
            seeded_cached_topic(active_topic.clone(), &[], Utc::now()),
        );
        guard.topics.insert(
            inactive_key.clone(),
            seeded_cached_topic(inactive_topic, &[], Utc::now()),
        );
    }
    let lease = hub
        .register_topic_subscribers(std::slice::from_ref(&active_topic))
        .await
        .expect("register active prompt cache topic");

    let guard = hub.state.lock().await;
    let projection_keys = SubscriptionHub::active_topic_keys_for_dependency(
        &guard,
        &RuntimeTopicDependency::PromptCacheProjection,
    );
    let binding_keys = SubscriptionHub::active_topic_keys_for_dependency(
        &guard,
        &RuntimeTopicDependency::PromptCacheWindow,
    );
    assert_eq!(projection_keys, vec![active_key.clone()]);
    assert_eq!(binding_keys, vec![active_key]);
    assert!(
        !projection_keys.contains(&inactive_key),
        "retained inactive prompt cache windows must not receive runtime deltas"
    );
    drop(guard);
    drop(lease);
}

#[tokio::test]
async fn inactive_prompt_cache_owner_skips_runtime_projection_materialization() {
    let state = crate::tests::test_state_with_openai_base(
        Url::parse("http://127.0.0.1:9").expect("valid test URL"),
    )
    .await;
    let hub = state.subscription_hub.clone();
    let topic = SubscriptionTopic::PromptCacheWindow {
        selection: PromptCacheConversationSelection::Count(20),
        detail_level: PromptCacheConversationDetailLevel::Full,
        recent_invocation_limit: Some(16),
    };
    let topic_key = topic.cache_key().expect("prompt cache topic key");
    {
        let mut guard = hub.state.lock().await;
        guard.topics.insert(
            topic_key.clone(),
            seeded_cached_topic(topic.clone(), &[7], Utc::now()),
        );
    }
    let mut record = dashboard_runtime_topology_live_record("2026-08-08 10:00:00");
    record.invoke_id = "inactive-prompt-cache".to_string();
    record.prompt_cache_key = Some("cache-key".to_string());
    state.proxy_runtime_invocations.upsert(record.clone());
    let mutations = [SequencedRuntimeMutation {
        sequence: 1,
        mutation: RuntimeMutation::invocation(&record, RuntimeMutationKind::RuntimeUpsert),
    }];

    hub.schedule_prompt_cache_topic_projection(state.clone(), &mutations)
        .await;

    let guard = hub.state.lock().await;
    let cached = guard
        .topics
        .get(&topic_key)
        .expect("retained inactive prompt cache topic");
    assert_eq!(cached.cursor, 7);
    assert!(cached.prompt_cache_pending_records.is_empty());
    assert!(!cached.prompt_cache_refresh_scheduled);
    assert!(
        !guard
            .prompt_cache_prebaseline_records
            .contains_key(&topic_key),
        "an inactive owner must not receive deferred prompt cache work"
    );
}

#[tokio::test]
async fn active_prompt_cache_projection_uses_typed_preview_without_full_record_clone() {
    let state = crate::tests::test_state_with_openai_base(
        Url::parse("http://127.0.0.1:9").expect("valid test URL"),
    )
    .await;
    let hub = state.subscription_hub.clone();
    let topic = SubscriptionTopic::PromptCacheWindow {
        selection: PromptCacheConversationSelection::Count(20),
        detail_level: PromptCacheConversationDetailLevel::Full,
        recent_invocation_limit: Some(16),
    };
    let topic_key = topic.cache_key().expect("prompt cache topic key");
    hub.state.lock().await.topics.insert(
        topic_key.clone(),
        seeded_cached_topic(topic.clone(), &[7], Utc::now()),
    );
    let lease = hub
        .register_topic_subscribers(std::slice::from_ref(&topic))
        .await
        .expect("register prompt cache owner");
    let mut record = dashboard_runtime_topology_live_record("2026-08-08 10:00:00");
    record.invoke_id = "compact-prompt-cache-projection".to_string();
    record.prompt_cache_key = Some("cache-key".to_string());
    record.request_raw_path = Some("runtime-only-raw-path".repeat(256));
    state.proxy_runtime_invocations.upsert(record.clone());
    let mutations = [SequencedRuntimeMutation {
        sequence: 1,
        mutation: RuntimeMutation::invocation(&record, RuntimeMutationKind::RuntimeUpsert),
    }];

    state
        .proxy_runtime_invocations
        .reset_full_record_clone_count();
    hub.schedule_prompt_cache_topic_projection(state.clone(), &mutations)
        .await;

    assert_eq!(
        state.proxy_runtime_invocations.full_record_clone_count(),
        0,
        "active prompt cache projection must not clone ApiInvocation"
    );
    let guard = hub.state.lock().await;
    let delta = guard
        .topics
        .get(&topic_key)
        .and_then(|cached| cached.prompt_cache_pending_records.values().next())
        .expect("typed prompt cache delta");
    let preview = delta.preview.as_ref().expect("typed prompt cache preview");
    assert_eq!(preview.invoke_id, "compact-prompt-cache-projection");
    assert_eq!(preview.prompt_cache_key.as_deref(), Some("cache-key"));
    drop(guard);
    drop(lease);
}

#[tokio::test]
async fn prompt_cache_window_topic_materializes_live_statistics_delta() {
    let state = crate::tests::test_state_with_openai_base(
        Url::parse("http://127.0.0.1:9").expect("valid test URL"),
    )
    .await;
    let hub = state.subscription_hub.clone();
    let topic = SubscriptionTopic::PromptCacheWindow {
        selection: PromptCacheConversationSelection::ActivityWindowMinutes(5),
        detail_level: PromptCacheConversationDetailLevel::Full,
        recent_invocation_limit: Some(16),
    };
    let now = Utc::now();
    let baseline_occurred_at = format_naive(
        (now - ChronoDuration::minutes(2))
            .with_timezone(&Shanghai)
            .naive_local(),
    );
    sqlx::query(
        r#"
            INSERT INTO codex_invocations (
                invoke_id, occurred_at, source, status, input_tokens, output_tokens,
                cache_input_tokens, reported_cache_write_tokens, reasoning_tokens, total_tokens,
                cost, cost_input, cost_cache_write, cost_cache_read, cost_output, cost_reasoning,
                payload, raw_response
            ) VALUES (
                'subscription-baseline', ?1, 'proxy', 'success', 4, 5, 6, 7, 8, 20,
                0.20, 0.01, 0.02, 0.03, 0.04, 0.05, ?2, '{}'
            )
            "#,
    )
    .bind(&baseline_occurred_at)
    .bind(json!({ "promptCacheKey": "subscription-live-key" }).to_string())
    .execute(&state.pool)
    .await
    .expect("persist subscription baseline invocation");
    crate::tests::complete_prompt_cache_conversation_materialization_for_test(&state.pool).await;

    let preexisting_occurred_at = format_naive(
        (now - ChronoDuration::seconds(90))
            .with_timezone(&Shanghai)
            .naive_local(),
    );
    let mut preexisting = dashboard_runtime_topology_live_record(&preexisting_occurred_at);
    preexisting.id = 0;
    preexisting.invoke_id = "subscription-preexisting-terminal".to_string();
    preexisting.status = Some("success".to_string());
    preexisting.live_phase = None;
    preexisting.prompt_cache_key = Some("subscription-live-key".to_string());
    preexisting.input_tokens = Some(1);
    preexisting.output_tokens = Some(2);
    preexisting.cache_input_tokens = Some(3);
    preexisting.reported_cache_write_tokens = Some(4);
    preexisting.reasoning_tokens = Some(5);
    preexisting.total_tokens = Some(30);
    preexisting.cost = Some(0.30);
    preexisting.cost_input = Some(0.06);
    preexisting.cost_cache_write = Some(0.07);
    preexisting.cost_cache_read = Some(0.08);
    preexisting.cost_output = Some(0.09);
    preexisting.cost_reasoning = Some(0.10);
    state.proxy_runtime_invocations.upsert(preexisting);

    let lease = hub
        .register_topic_subscribers(std::slice::from_ref(&topic))
        .await
        .expect("register prompt cache topic owner");
    let preexisting_mutations = [SequencedRuntimeMutation {
        sequence: 1,
        mutation: RuntimeMutation::invocation(
            &state
                .proxy_runtime_invocations
                .snapshot()
                .into_iter()
                .find(|record| record.invoke_id == "subscription-preexisting-terminal")
                .expect("preexisting runtime record"),
            RuntimeMutationKind::RuntimeUpsert,
        ),
    }];
    hub.schedule_prompt_cache_topic_projection(state.clone(), &preexisting_mutations)
        .await;
    hub.refresh_topic(state.clone(), topic.clone(), false)
        .await
        .expect("build prompt cache subscription baseline");
    {
        let topic_key = topic.cache_key().expect("prompt cache topic key");
        let guard = hub.state.lock().await;
        let cached = guard
            .topics
            .get(&topic_key)
            .expect("cached prompt cache topic");
        let payload = cached.snapshot_frame.payload_value();
        assert_eq!(payload["conversations"].as_array().map(Vec::len), Some(1));
        assert_eq!(
            payload["conversations"][0]["requestCount"], 2,
            "the production topic baseline must include persisted and preexisting runtime fixtures",
        );
        assert_eq!(payload["conversations"][0]["successCount"], 2);
        assert_eq!(payload["conversations"][0]["inputTokens"], 5);
    }

    let live_occurred_at = format_naive(
        (now - ChronoDuration::minutes(1))
            .with_timezone(&Shanghai)
            .naive_local(),
    );
    let mut live = dashboard_runtime_topology_live_record(&live_occurred_at);
    live.id = 0;
    live.invoke_id = "subscription-live-terminal".to_string();
    live.status = Some("success".to_string());
    live.live_phase = None;
    live.prompt_cache_key = Some("subscription-live-key".to_string());
    live.input_tokens = Some(10);
    live.output_tokens = Some(11);
    live.cache_input_tokens = Some(12);
    live.reported_cache_write_tokens = Some(13);
    live.reasoning_tokens = Some(14);
    live.total_tokens = Some(50);
    live.cost = Some(0.50);
    live.cost_input = Some(0.11);
    live.cost_cache_write = Some(0.12);
    live.cost_cache_read = Some(0.13);
    live.cost_output = Some(0.14);
    live.cost_reasoning = Some(0.15);
    state.proxy_runtime_invocations.upsert(live.clone());
    let mutations = [SequencedRuntimeMutation {
        sequence: 1,
        mutation: RuntimeMutation::invocation(&live, RuntimeMutationKind::RuntimeUpsert),
    }];
    let live_identity = format!("{}\0{}", live.invoke_id, live.occurred_at);
    hub.schedule_prompt_cache_topic_projection(state.clone(), &mutations)
        .await;
    {
        let guard = hub.state.lock().await;
        assert!(
            guard
                .topics
                .get(&topic.cache_key().expect("prompt cache topic key"))
                .expect("cached prompt cache topic")
                .prompt_cache_pending_records
                .contains_key(&live_identity)
        );
    }
    let topic_key = topic.cache_key().expect("prompt cache topic key");
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let materialized = {
                let guard = hub.state.lock().await;
                let cached = guard
                    .topics
                    .get(&topic_key)
                    .expect("cached prompt cache topic");
                let payload = cached.snapshot_frame.payload_value();
                payload["conversations"]
                    .as_array()
                    .and_then(|conversations| conversations.first())
                    .and_then(|conversation| conversation.get("requestCount"))
                    .and_then(Value::as_i64)
                    == Some(3)
                    && cached.prompt_cache_pending_records.is_empty()
                    && !cached.prompt_cache_refresh_scheduled
            };
            if materialized {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("production prompt cache materialization debounce completes");

    let guard = hub.state.lock().await;
    let cached = guard
        .topics
        .get(&topic_key)
        .expect("cached prompt cache topic");
    let payload = cached.snapshot_frame.payload_value();
    let conversation = payload["conversations"]
        .as_array()
        .and_then(|conversations| conversations.first())
        .expect("subscription conversation payload");
    assert_eq!(conversation["promptCacheKey"], "subscription-live-key");
    assert_eq!(conversation["requestCount"], 3);
    assert_eq!(conversation["totalTokens"], 100);
    assert_eq!(conversation["successCount"], 3);
    assert_eq!(conversation["failureCount"], 0);
    assert_eq!(conversation["inputTokens"], 15);
    assert_eq!(conversation["outputTokens"], 18);
    assert_eq!(conversation["cacheInputTokens"], 21);
    assert_eq!(conversation["reportedCacheWriteTokens"], 24);
    assert_eq!(conversation["reasoningTokens"], 27);
    assert!((conversation["costInput"].as_f64().unwrap() - 0.18).abs() < 1e-9);
    assert_ne!(
        conversation["firstInvocationAt"],
        conversation["lastInvocationAt"]
    );
    drop(guard);
    drop(lease);
}

#[tokio::test]
async fn prompt_cache_response_deduplicates_persisted_terminal_runtime_overlay() {
    let state = crate::tests::test_state_with_openai_base(
        Url::parse("http://127.0.0.1:9").expect("valid test URL"),
    )
    .await;
    let occurred_at = format_naive(
        (Utc::now() - ChronoDuration::minutes(2))
            .with_timezone(&Shanghai)
            .naive_local(),
    );
    sqlx::query(
        r#"
            INSERT INTO codex_invocations (
                invoke_id, occurred_at, source, status, input_tokens, output_tokens,
                cache_input_tokens, reported_cache_write_tokens, reasoning_tokens, total_tokens,
                cost, cost_input, cost_cache_write, cost_cache_read, cost_output, cost_reasoning,
                payload, raw_response
            ) VALUES (
                'response-deduplication', ?1, 'proxy', 'success', 4, 5, 6, 7, 8, 20,
                0.20, 0.01, 0.02, 0.03, 0.04, 0.05, ?2, '{}'
            )
            "#,
    )
    .bind(&occurred_at)
    .bind(json!({ "promptCacheKey": "response-deduplication-key" }).to_string())
    .execute(&state.pool)
    .await
    .expect("persist response deduplication fixture");
    sync_hourly_rollups_from_live_tables(&state.pool)
        .await
        .expect("materialize response deduplication rollup");
    crate::tests::complete_prompt_cache_conversation_materialization_for_test(&state.pool).await;

    let mut runtime = dashboard_runtime_topology_live_record(&occurred_at);
    runtime.id = 0;
    runtime.invoke_id = "response-deduplication".to_string();
    runtime.status = Some("success".to_string());
    runtime.live_phase = None;
    runtime.prompt_cache_key = Some("response-deduplication-key".to_string());
    runtime.input_tokens = Some(4);
    runtime.output_tokens = Some(5);
    runtime.cache_input_tokens = Some(6);
    runtime.reported_cache_write_tokens = Some(7);
    runtime.reasoning_tokens = Some(8);
    runtime.total_tokens = Some(20);
    runtime.cost = Some(0.20);
    runtime.cost_input = Some(0.01);
    runtime.cost_cache_write = Some(0.02);
    runtime.cost_cache_read = Some(0.03);
    runtime.cost_output = Some(0.04);
    runtime.cost_reasoning = Some(0.05);
    state.proxy_runtime_invocations.upsert(runtime);

    let mut transient_runtime = dashboard_runtime_topology_live_record(&occurred_at);
    transient_runtime.id = 0;
    transient_runtime.invoke_id = "response-transient-runtime".to_string();
    transient_runtime.status = Some("success".to_string());
    transient_runtime.live_phase = None;
    transient_runtime.prompt_cache_key = Some("response-deduplication-key".to_string());
    transient_runtime.input_tokens = Some(1);
    transient_runtime.output_tokens = Some(2);
    transient_runtime.total_tokens = Some(10);
    transient_runtime.cost = Some(0.10);
    state.proxy_runtime_invocations.upsert(transient_runtime);

    let response = build_prompt_cache_conversations_response_with_recent_limit(
        &state,
        PromptCacheConversationSelection::Count(20),
        Some(16),
    )
    .await
    .expect("prompt cache response should deduplicate persisted runtime overlay");
    let conversation = response
        .conversations
        .iter()
        .find(|conversation| conversation.prompt_cache_key == "response-deduplication-key")
        .expect("deduplication conversation");
    assert_eq!(conversation.request_count, 2);
    assert_eq!(conversation.success_count, Some(2));
    assert_eq!(conversation.input_tokens, Some(5));
    assert_eq!(conversation.total_tokens, 30);
}

#[tokio::test]
async fn runtime_gap_preserves_prompt_cache_last_good_and_defers_reconcile() {
    let state = crate::tests::test_state_with_openai_base(
        Url::parse("http://127.0.0.1:9").expect("valid test URL"),
    )
    .await;
    let hub = state.subscription_hub.clone();
    let topic = SubscriptionTopic::PromptCacheWindow {
        selection: PromptCacheConversationSelection::Count(20),
        detail_level: PromptCacheConversationDetailLevel::Full,
        recent_invocation_limit: Some(16),
    };
    let topic_key = topic.cache_key().expect("prompt cache topic key");
    let last_good = seeded_cached_topic(topic.clone(), &[7], Utc::now());
    let last_good_frame = last_good.snapshot_frame.clone();
    {
        let mut guard = hub.state.lock().await;
        guard.topics.insert(topic_key.clone(), last_good);
    }
    let lease = hub
        .register_topic_subscribers(std::slice::from_ref(&topic))
        .await
        .expect("register prompt cache owner");

    hub.mark_runtime_mutation_gap_and_recover(state.clone(), 4, "cursor_gap")
        .await;

    let guard = hub.state.lock().await;
    let cached = guard
        .topics
        .get(&topic_key)
        .expect("active prompt cache topic");
    assert!(cached.dirty);
    assert_eq!(cached.continuity_reset_cursor, Some(7));
    assert!(cached.prompt_cache_reconcile_required);
    assert_eq!(cached.prompt_cache_full_hydration_count, 0);
    assert!(Arc::ptr_eq(&cached.snapshot_frame, &last_good_frame));
    assert!(guard.runtime_topic_recovery_queue.is_empty());
    assert!(!guard.runtime_topic_recovery_running);
    drop(guard);
    drop(lease);
}

#[tokio::test]
async fn active_dirty_prompt_cache_last_good_degrades_aggregate_health() {
    let state = crate::tests::test_state_with_openai_base(
        Url::parse("http://127.0.0.1:9").expect("valid test URL"),
    )
    .await;
    let hub = state.subscription_hub.clone();
    let topic = SubscriptionTopic::PromptCacheWindow {
        selection: PromptCacheConversationSelection::Count(20),
        detail_level: PromptCacheConversationDetailLevel::Full,
        recent_invocation_limit: Some(16),
    };
    let topic_key = topic.cache_key().expect("prompt cache topic key");
    let mut cached = seeded_cached_topic(topic.clone(), &[7], Utc::now());
    cached.dirty = true;
    cached.prompt_cache_reconcile_required = true;
    hub.state
        .lock()
        .await
        .topics
        .insert(topic_key.clone(), cached);
    let lease = hub
        .register_topic_subscribers(std::slice::from_ref(&topic))
        .await
        .expect("register prompt cache owner");

    let stale = crate::load_runtime_pressure_health(state.as_ref()).await;
    assert_eq!(stale.state, "degraded");
    assert_eq!(
        stale.prompt_cache_projection.recovery_state,
        "failed_or_stale"
    );
    assert_eq!(stale.prompt_cache_projection.failed_or_stale_topic_count, 1);

    hub.state
        .lock()
        .await
        .topics
        .get_mut(&topic_key)
        .expect("active prompt cache topic")
        .prompt_cache_pressure_deferred = true;
    let deferred = crate::load_runtime_pressure_health(state.as_ref()).await;
    assert_eq!(deferred.state, "deferred");
    assert_eq!(
        deferred.prompt_cache_projection.recovery_state,
        "pressure_deferred"
    );
    assert_eq!(
        deferred
            .prompt_cache_projection
            .pressure_deferred_topic_count,
        1
    );
    drop(lease);
}

#[tokio::test]
async fn prompt_cache_live_db_reads_degrade_aggregate_health() {
    let state = crate::tests::test_state_with_openai_base(
        Url::parse("http://127.0.0.1:9").expect("valid test URL"),
    )
    .await;
    let hub = state.subscription_hub.clone();
    let topic = SubscriptionTopic::DashboardWorkingConversationsCurrent {
        page_size: 20,
        recent_invocation_limit: 16,
        blocked_binding_upstream_account_id: None,
        blocked_binding_constraint_source: None,
    };
    let topic_key = topic.cache_key().expect("working conversation topic key");
    let mut cached = seeded_cached_topic(topic.clone(), &[7], Utc::now());
    cached.prompt_cache_full_hydration_count = 2;
    cached.prompt_cache_bounded_key_hydration_count = 1;
    hub.state.lock().await.topics.insert(topic_key, cached);
    let lease = hub
        .register_topic_subscribers(std::slice::from_ref(&topic))
        .await
        .expect("register working conversation owner");
    let health = crate::load_runtime_pressure_health(state.as_ref()).await;
    assert_eq!(health.prompt_cache_projection.live_path_db_read_count, 1);
    assert_eq!(health.prompt_cache_projection.recovery_state, "hot_db_read");
    assert_eq!(health.state, "degraded");
    drop(lease);
}

#[tokio::test]
async fn prompt_cache_bounded_cold_recovery_defers_aggregate_health() {
    let state = crate::tests::test_state_with_openai_base(
        Url::parse("http://127.0.0.1:9").expect("valid test URL"),
    )
    .await;
    let hub = state.subscription_hub.clone();
    let topic = SubscriptionTopic::DashboardWorkingConversationsCurrent {
        page_size: 20,
        recent_invocation_limit: 16,
        blocked_binding_upstream_account_id: None,
        blocked_binding_constraint_source: None,
    };
    let topic_key = topic.cache_key().expect("working conversation topic key");
    let mut cached = seeded_cached_topic(topic.clone(), &[7], Utc::now());
    cached.prompt_cache_full_hydration_count = 1;
    cached.prompt_cache_bounded_key_hydration_count = 1;
    cached.prompt_cache_response_source = "database_bounded_key_hydrate";
    hub.state.lock().await.topics.insert(topic_key, cached);
    let lease = hub
        .register_topic_subscribers(std::slice::from_ref(&topic))
        .await
        .expect("register working conversation owner");
    let health = crate::load_runtime_pressure_health(state.as_ref()).await;
    assert_eq!(health.prompt_cache_projection.live_path_db_read_count, 0);
    assert_eq!(
        health
            .prompt_cache_projection
            .bounded_cold_recovery_topic_count,
        1
    );
    assert_eq!(
        health.prompt_cache_projection.recovery_state,
        "bounded_cold_recovery"
    );
    assert_eq!(health.state, "deferred");
    drop(lease);
}
#[tokio::test]
async fn repeated_runtime_gaps_dedupe_bounded_recovery_jobs() {
    let state = crate::tests::test_state_with_openai_base(
        Url::parse("http://127.0.0.1:9").expect("valid test URL"),
    )
    .await;
    let hub = state.subscription_hub.clone();
    let topic = SubscriptionTopic::InvocationHistoryWindow {
        scope: ConversationSubscriptionScope::PromptCacheKey("selected-key".to_string()),
    };
    let topic_key = topic.cache_key().expect("history topic key");
    {
        let mut guard = hub.state.lock().await;
        guard.topics.insert(
            topic_key.clone(),
            seeded_cached_topic(topic.clone(), &[7], Utc::now()),
        );
        // Keep the worker parked so this test can observe the queued representation.
        guard.runtime_topic_recovery_running = true;
    }
    let lease = hub
        .register_topic_subscribers(std::slice::from_ref(&topic))
        .await
        .expect("register history owner");
    hub.mark_runtime_mutation_gap_and_recover(state.clone(), 2, "cursor_gap")
        .await;
    hub.mark_runtime_mutation_gap_and_recover(state.clone(), 3, "receiver_lagged")
        .await;
    let mut guard = hub.state.lock().await;
    assert_eq!(guard.runtime_topic_recovery_queue.len(), 1);
    assert_eq!(
        guard.runtime_topic_recovery_queue.front(),
        Some(&(topic_key.clone(), 1)),
        "the queued job retains its original generation and is re-enqueued once with the\n             newer generation after the worker observes the mismatch"
    );
    assert!(guard.runtime_topic_recovery_queued.contains(&topic_key));
    assert_eq!(
        guard
            .topics
            .get(&topic_key)
            .expect("active history topic")
            .runtime_topic_recovery_generation,
        2
    );
    guard.runtime_topic_recovery_running = false;
    drop(guard);
    drop(lease);
}
#[tokio::test]
async fn inactive_owner_skips_cold_prompt_cache_hydration() {
    let state = crate::tests::test_state_with_openai_base(
        Url::parse("http://127.0.0.1:9").expect("valid test URL"),
    )
    .await;
    let hub = state.subscription_hub.clone();
    let topic = SubscriptionTopic::PromptCacheWindow {
        selection: PromptCacheConversationSelection::Count(20),
        detail_level: PromptCacheConversationDetailLevel::Full,
        recent_invocation_limit: Some(16),
    };
    state.pool.close().await;
    assert!(
        hub.refresh_topic_if_active(state, topic, true)
            .await
            .expect("inactive guard must return before acquiring the closed database pool")
            .is_none()
    );
}
#[tokio::test]
async fn active_owner_without_cached_topic_commits_guarded_refresh() {
    let state = crate::tests::test_state_with_openai_base(
        Url::parse("http://127.0.0.1:9").expect("valid test URL"),
    )
    .await;
    let hub = state.subscription_hub.clone();
    let topic = SubscriptionTopic::InvocationHistoryWindow {
        scope: ConversationSubscriptionScope::PromptCacheKey("selected-key".to_string()),
    };
    let topic_key = topic.cache_key().expect("history topic key");
    let lease = hub
        .register_topic_subscribers(std::slice::from_ref(&topic))
        .await
        .expect("register history owner");
    let cached = hub
        .refresh_topic_if_active(state, topic, true)
        .await
        .expect("active owner can cold build")
        .expect("guarded refresh commits when no generation changed");
    assert_eq!(
        cached.topic.cache_key().expect("cached topic key"),
        topic_key
    );
    drop(lease);
}
#[tokio::test]
async fn unrelated_topic_disconnect_does_not_block_active_refresh() {
    let state = crate::tests::test_state_with_openai_base(
        Url::parse("http://127.0.0.1:9").expect("valid test URL"),
    )
    .await;
    let hub = state.subscription_hub.clone();
    let active_topic = SubscriptionTopic::InvocationHistoryWindow {
        scope: ConversationSubscriptionScope::PromptCacheKey("active-key".to_string()),
    };
    let released_topic = SubscriptionTopic::InvocationHistoryWindow {
        scope: ConversationSubscriptionScope::PromptCacheKey("released-key".to_string()),
    };
    let active_key = active_topic.cache_key().expect("active topic key");
    let released_key = released_topic.cache_key().expect("released topic key");
    {
        let mut guard = hub.state.lock().await;
        guard.topics.insert(
            active_key.clone(),
            seeded_cached_topic(active_topic.clone(), &[7], Utc::now()),
        );
        guard.topics.insert(
            released_key.clone(),
            seeded_cached_topic(released_topic.clone(), &[7], Utc::now()),
        );
    }
    let active_lease = hub
        .register_topic_subscribers(std::slice::from_ref(&active_topic))
        .await
        .expect("register active topic");
    let released_lease = hub
        .register_topic_subscribers(std::slice::from_ref(&released_topic))
        .await
        .expect("register released topic");
    hub.release_topic_subscribers(
        vec![released_key],
        vec![released_topic.name().to_string()],
        false,
    )
    .await;
    assert!(
        hub.refresh_topic_if_active(state, active_topic, true)
            .await
            .expect("active refresh")
            .is_some()
    );
    drop(released_lease);
    drop(active_lease);
}
#[tokio::test]
async fn dirty_last_good_reconnect_skips_synchronous_cold_hydration() {
    let state = crate::tests::test_state_with_openai_base(
        Url::parse("http://127.0.0.1:9").expect("valid test URL"),
    )
    .await;
    let hub = state.subscription_hub.clone();
    let topic = SubscriptionTopic::InvocationHistoryWindow {
        scope: ConversationSubscriptionScope::PromptCacheKey("selected-key".to_string()),
    };
    let topic_key = topic.cache_key().expect("history topic key");
    let mut last_good = seeded_cached_topic(topic.clone(), &[7], Utc::now());
    last_good.dirty = true;
    {
        let mut guard = hub.state.lock().await;
        guard.topics.insert(topic_key.clone(), last_good);
        // Keep the bounded recovery worker parked so this test can prove that preparing the
        // connection itself never acquires SQLite for a retained last-good frame.
        guard.runtime_topic_recovery_running = true;
    }
    let lease = hub
        .register_topic_subscribers(std::slice::from_ref(&topic))
        .await
        .expect("register history owner");
    state.pool.close().await;
    let prepared = hub
        .prepare_connection(state, vec![topic.descriptor()], Vec::new())
        .await
        .expect("reconnect must serve last-good without a database read");
    assert_eq!(prepared.initial.len(), 1);
    let guard = hub.state.lock().await;
    assert!(
        guard
            .topics
            .get(&topic_key)
            .expect("retained history topic")
            .dirty
    );
    assert_eq!(guard.runtime_topic_recovery_queue.len(), 1);
    drop(guard);
    drop(lease);
}
#[tokio::test]
async fn runtime_recovery_retry_cooldown_defers_dirty_topic_requeue() {
    let hub = Arc::new(SubscriptionHub::new());
    let topic = SubscriptionTopic::InvocationHistoryWindow {
        scope: ConversationSubscriptionScope::PromptCacheKey("selected-key".to_string()),
    };
    let topic_key = topic.cache_key().expect("history topic key");
    {
        let mut guard = hub.state.lock().await;
        let mut cached = seeded_cached_topic(topic.clone(), &[7], Utc::now());
        cached.dirty = true;
        guard.topics.insert(topic_key.clone(), cached);
    }
    let lease = hub
        .register_topic_subscribers(std::slice::from_ref(&topic))
        .await
        .expect("register history owner");
    assert_eq!(
        hub.defer_runtime_topic_recovery_retry(&topic).await,
        RUNTIME_TOPIC_RECOVERY_RETRY_BACKOFF
    );
    let mut guard = hub.state.lock().await;
    assert!(
        guard
            .topics
            .get(&topic_key)
            .and_then(|cached| cached.runtime_topic_recovery_retry_at)
            .is_some_and(|retry_at| retry_at > Instant::now())
    );
    assert!(!SubscriptionHub::enqueue_runtime_topic_recovery_locked(
        &mut guard
    ));
    assert!(guard.runtime_topic_recovery_queue.is_empty());
    assert!(SubscriptionHub::next_runtime_topic_recovery_retry_delay_locked(&guard).is_some());
    guard
        .topics
        .get_mut(&topic_key)
        .expect("dirty history topic")
        .runtime_topic_recovery_retry_at = Some(Instant::now());
    assert!(SubscriptionHub::enqueue_runtime_topic_recovery_locked(
        &mut guard
    ));
    assert_eq!(guard.runtime_topic_recovery_queue.len(), 1);
    drop(guard);
    drop(lease);
}

#[tokio::test]
async fn newer_runtime_gap_clears_topic_recovery_cooldown() {
    let state = crate::tests::test_state_with_openai_base(
        Url::parse("http://127.0.0.1:9").expect("valid test URL"),
    )
    .await;
    let hub = state.subscription_hub.clone();
    let topic = SubscriptionTopic::InvocationHistoryWindow {
        scope: ConversationSubscriptionScope::PromptCacheKey("selected-key".to_string()),
    };
    let topic_key = topic.cache_key().expect("history topic key");
    {
        let mut guard = hub.state.lock().await;
        let mut cached = seeded_cached_topic(topic.clone(), &[7], Utc::now());
        cached.dirty = true;
        cached.runtime_topic_recovery_retry_at =
            Some(Instant::now() + RUNTIME_TOPIC_RECOVERY_RETRY_BACKOFF);
        guard.topics.insert(topic_key.clone(), cached);
        guard.runtime_topic_recovery_running = true;
    }
    let lease = hub
        .register_topic_subscribers(std::slice::from_ref(&topic))
        .await
        .expect("register history owner");

    hub.mark_runtime_mutation_gap_and_recover(state, 1, "cursor_gap")
        .await;

    let guard = hub.state.lock().await;
    assert!(
        guard
            .topics
            .get(&topic_key)
            .expect("dirty history topic")
            .runtime_topic_recovery_retry_at
            .is_none()
    );
    assert_eq!(guard.runtime_topic_recovery_queue.len(), 1);
    drop(guard);
    drop(lease);
}

#[tokio::test]
async fn dirty_topics_do_not_bypass_bounded_gap_recovery() {
    let hub = Arc::new(SubscriptionHub::new());
    let topic = SubscriptionTopic::InvocationHistoryWindow {
        scope: ConversationSubscriptionScope::PromptCacheKey("selected-key".to_string()),
    };
    let topic_key = topic.cache_key().expect("history topic key");
    {
        let mut guard = hub.state.lock().await;
        let mut cached = seeded_cached_topic(topic.clone(), &[7], Utc::now());
        cached.dirty = true;
        guard.topics.insert(topic_key, cached);
    }
    let lease = hub
        .register_topic_subscribers(std::slice::from_ref(&topic))
        .await
        .expect("register history owner");
    let mutations = [SequencedRuntimeMutation {
        sequence: 1,
        mutation: RuntimeMutation::Invocation(RuntimeInvocationMutation {
            identity: RuntimeInvocationIdentity::new("invoke-1", "2026-08-09 12:00:00"),
            kind: RuntimeMutationKind::RuntimeUpsert,
            row_id: None,
            is_terminal: false,
            prompt_cache_key: Some("selected-key".to_string()),
            sticky_key: None,
            upstream_account_id: None,
        }),
    }];

    let guard = hub.state.lock().await;
    assert!(SubscriptionHub::collect_runtime_topic_work(&guard, &mutations).is_empty());
    drop(guard);
    drop(lease);
}

#[tokio::test]
async fn dirty_prompt_cache_topic_retains_last_good_until_bounded_reconcile() {
    let state = crate::tests::test_state_with_openai_base(
        Url::parse("http://127.0.0.1:9").expect("valid test URL"),
    )
    .await;
    let hub = state.subscription_hub.clone();
    let topic = SubscriptionTopic::PromptCacheWindow {
        selection: PromptCacheConversationSelection::Count(20),
        detail_level: PromptCacheConversationDetailLevel::Full,
        recent_invocation_limit: Some(16),
    };
    let topic_key = topic.cache_key().expect("prompt cache topic key");
    let last_good = seeded_cached_topic(topic.clone(), &[7], Utc::now());
    let last_good_frame = last_good.snapshot_frame.clone();
    {
        let mut guard = hub.state.lock().await;
        guard.topics.insert(topic_key.clone(), last_good);
    }
    let lease = hub
        .register_topic_subscribers(std::slice::from_ref(&topic))
        .await
        .expect("register prompt cache owner");
    hub.mark_runtime_mutation_gap_and_recover(state.clone(), 4, "cursor_gap")
        .await;

    let mut record = dashboard_runtime_topology_live_record("2026-08-08 10:00:00");
    record.invoke_id = "post-gap-prompt-cache".to_string();
    record.prompt_cache_key = Some("cache-key".to_string());
    state.proxy_runtime_invocations.upsert(record.clone());
    let mutations = [SequencedRuntimeMutation {
        sequence: 5,
        mutation: RuntimeMutation::invocation(&record, RuntimeMutationKind::RuntimeUpsert),
    }];
    hub.schedule_prompt_cache_topic_projection(state.clone(), &mutations)
        .await;

    let guard = hub.state.lock().await;
    let cached = guard
        .topics
        .get(&topic_key)
        .expect("active prompt cache topic");
    assert!(cached.dirty);
    assert!(cached.prompt_cache_reconcile_scheduled);
    assert!(!cached.prompt_cache_refresh_scheduled);
    assert!(cached.prompt_cache_pending_records.is_empty());
    assert_eq!(cached.cursor, 7);
    assert!(Arc::ptr_eq(&cached.snapshot_frame, &last_good_frame));
    drop(guard);
    drop(lease);
}

#[tokio::test]
async fn dirty_prompt_cache_reconcile_respects_baseline_cadence() {
    let hub = Arc::new(SubscriptionHub::new());
    let topic = SubscriptionTopic::PromptCacheWindow {
        selection: PromptCacheConversationSelection::Count(20),
        detail_level: PromptCacheConversationDetailLevel::Full,
        recent_invocation_limit: Some(16),
    };
    let topic_key = topic.cache_key().expect("prompt cache topic key");
    {
        let mut guard = hub.state.lock().await;
        let mut cached = seeded_cached_topic(topic.clone(), &[7], Utc::now());
        cached.dirty = true;
        cached.prompt_cache_reconcile_required = true;
        cached.prompt_cache_baseline_at = Some(Instant::now());
        guard.topics.insert(topic_key, cached);
    }
    let lease = hub
        .register_topic_subscribers(std::slice::from_ref(&topic))
        .await
        .expect("register prompt cache owner");

    assert!(
        hub.prompt_cache_topic_reconcile_delay(&topic)
            .await
            .is_some_and(|delay| delay > Duration::from_secs(59))
    );
    drop(lease);
}

#[tokio::test]
async fn prompt_cache_pressure_defer_wakes_on_eligibility_change() {
    let gate = DbPressureGate::new(1, Duration::from_millis(10));
    let permit = gate
        .try_begin_background("prompt_cache_topic_reconcile")
        .expect("occupy only background slot");
    let observed_eligibility = gate.eligibility_generation();
    let reason = gate
        .try_begin_background("prompt_cache_topic_reconcile")
        .expect_err("second background task is deferred");

    drop(permit);

    tokio::time::timeout(
        Duration::from_secs(1),
        wait_for_prompt_cache_reconcile_eligibility(&gate, observed_eligibility, reason),
    )
    .await
    .expect("eligible background slot wakes deferred prompt cache recovery");
}

#[tokio::test]
async fn missing_runtime_identity_schedules_active_prompt_cache_reconcile() {
    let state = crate::tests::test_state_with_openai_base(
        Url::parse("http://127.0.0.1:9").expect("valid test URL"),
    )
    .await;
    let hub = state.subscription_hub.clone();
    let topic = SubscriptionTopic::PromptCacheWindow {
        selection: PromptCacheConversationSelection::Count(20),
        detail_level: PromptCacheConversationDetailLevel::Full,
        recent_invocation_limit: Some(16),
    };
    let topic_key = topic.cache_key().expect("prompt cache topic key");
    {
        let mut guard = hub.state.lock().await;
        guard.topics.insert(
            topic_key.clone(),
            seeded_cached_topic(topic.clone(), &[7], Utc::now()),
        );
    }
    let lease = hub
        .register_topic_subscribers(std::slice::from_ref(&topic))
        .await
        .expect("register prompt cache owner");
    let mutations = [SequencedRuntimeMutation {
        sequence: 1,
        mutation: RuntimeMutation::Invocation(RuntimeInvocationMutation {
            identity: RuntimeInvocationIdentity::new("recovered", "2026-08-09 12:00:00"),
            kind: RuntimeMutationKind::Recovery,
            row_id: Some(1),
            is_terminal: true,
            prompt_cache_key: Some("cache-key".to_string()),
            sticky_key: None,
            upstream_account_id: None,
        }),
    }];

    hub.schedule_prompt_cache_topic_projection(state.clone(), &mutations)
        .await;

    let guard = hub.state.lock().await;
    let cached = guard
        .topics
        .get(&topic_key)
        .expect("active prompt cache topic");
    assert!(cached.dirty);
    assert!(cached.prompt_cache_reconcile_required);
    assert!(cached.prompt_cache_reconcile_scheduled);
    assert!(cached.prompt_cache_pending_records.is_empty());
    drop(guard);
    drop(lease);
}

#[tokio::test]
async fn prompt_cache_materialization_failure_marks_and_schedules_reconcile() {
    let state = crate::tests::test_state_with_openai_base(
        Url::parse("http://127.0.0.1:9").expect("valid test URL"),
    )
    .await;
    let hub = state.subscription_hub.clone();
    let topic = SubscriptionTopic::PromptCacheWindow {
        selection: PromptCacheConversationSelection::Count(20),
        detail_level: PromptCacheConversationDetailLevel::Full,
        recent_invocation_limit: Some(16),
    };
    let topic_key = topic.cache_key().expect("prompt cache topic key");
    {
        let mut guard = hub.state.lock().await;
        guard.topics.insert(
            topic_key.clone(),
            seeded_cached_topic(topic.clone(), &[7], Utc::now()),
        );
    }
    let lease = hub
        .register_topic_subscribers(std::slice::from_ref(&topic))
        .await
        .expect("register prompt cache owner");

    assert!(
        hub.mark_prompt_cache_topic_dirty_and_schedule_reconcile(&topic)
            .await
    );

    let guard = hub.state.lock().await;
    let cached = guard
        .topics
        .get(&topic_key)
        .expect("active prompt cache topic");
    assert!(cached.dirty);
    assert!(cached.prompt_cache_reconcile_required);
    assert!(cached.prompt_cache_reconcile_scheduled);
    drop(guard);
    drop(lease);
}

#[tokio::test]
async fn summary_delta_journal_conflicting_ack_records_gap() {
    let state = crate::tests::test_state_with_openai_base(
        Url::parse("http://127.0.0.1:9").expect("valid test URL"),
    )
    .await;
    let occurred_at = format_naive(Utc::now().with_timezone(&Shanghai).naive_local());
    let mut first = dashboard_runtime_topology_live_record(&occurred_at);
    first.id = 0;
    first.invoke_id = "summary-delta-journal-first".to_string();
    first.status = Some("success".to_string());
    first.live_phase = None;
    let mut first = apply_dashboard_activity_terminal_record(state.as_ref(), &first)
        .await
        .terminal_delta
        .expect("accept first terminal delta");
    first.persisted_row_id = Some(1);
    let mut skipped = first.clone();
    skipped.invoke_id = "summary-delta-journal-skipped".to_string();
    skipped.terminal_sequence = first.terminal_sequence.saturating_add(2);

    let mut journal = SummaryDeltaJournal::default();
    assert!(journal.append(first, 1));
    assert!(
        !journal.append(skipped, 3),
        "a non-contiguous durable ACK must not be treated as an exact delta"
    );
    assert!(
        journal
            .gap_proofs
            .iter()
            .any(|proof| proof.occurred_at.is_empty() && proof.upstream_account_id.is_none()),
        "a missing cursor without durable metadata must remain globally fail-closed"
    );
    assert!(
        journal
            .gap_proofs
            .iter()
            .any(|proof| proof.cursor == SummaryDeltaCursor(3)),
        "the later conflicting ACK retains its bounded recovery cursor"
    );
    assert_eq!(journal.overflowed_through_sequence, Some(3));
}
#[test]
fn summary_delta_source_cursor_gap_does_not_advance_terminal_cursor() {
    let mut journal = SummaryDeltaJournal::default();

    journal.note_unknown_source_cursor_gap(99);

    assert_eq!(journal.cursor, SummaryDeltaCursor(0));
    let proof = journal
        .gap_proofs
        .front()
        .expect("source compaction retains a broad proof");
    assert_eq!(proof.cursor, SummaryDeltaCursor(99));
    assert_eq!(proof.terminal_sequence, None);
    assert!(proof.source_gap);
}
#[test]
fn clearing_source_compaction_gap_preserves_scoped_source_proof() {
    let mut journal = SummaryDeltaJournal::default();
    journal.note_unknown_source_cursor_gap(10);
    journal.retain_gap_proof(DeltaGapProof {
        cursor: SummaryDeltaCursor(11),
        terminal_sequence: None,
        source_gap: true,
        compaction_gap: false,
        upstream_account_id: Some(42),
        occurred_at: "2026-01-01T00:00:00Z".to_string(),
        row_id: Some(7),
        invoke_id: Some("scoped-source-gap".to_string()),
    });

    journal.clear_source_compaction_gap();

    assert!(!journal.source_compaction_gap);
    assert!(!journal.gap_proof_budget_exhausted);
    assert_eq!(journal.gap_proofs.len(), 1);
    let proof = journal
        .gap_proofs
        .front()
        .expect("scoped source proof survives compaction cleanup");
    assert!(!proof.compaction_gap);
    assert_eq!(proof.row_id, Some(7));
    assert_eq!(proof.upstream_account_id, Some(42));
}
#[tokio::test]
async fn summary_projection_ack_after_absorbing_swap_is_idempotent() {
    let state = crate::tests::test_state_with_openai_base(
        Url::parse("http://127.0.0.1:9").expect("valid test URL"),
    )
    .await;
    let occurred_at = format_naive(Utc::now().with_timezone(&Shanghai).naive_local());
    let mut record = dashboard_runtime_topology_live_record(&occurred_at);
    record.id = 9_100_001;
    record.invoke_id = "summary-live-tail-absorbed".to_string();
    record.status = Some("success".to_string());
    record.live_phase = None;
    let mut delta = apply_dashboard_activity_terminal_record(state.as_ref(), &record)
        .await
        .terminal_delta
        .expect("materialize terminal delta");
    delta.persisted_row_id = Some(record.id);

    let mut journal = SummaryDeltaJournal::default();
    assert!(journal.append(delta.clone(), delta.terminal_sequence));
    journal.absorb_committed_delta(&delta);
    assert!(journal.entries.is_empty());
    assert!(journal.replayed_entries.is_empty());
    assert!(journal.gap_proofs.is_empty());
    assert_eq!(journal.cursor, SummaryDeltaCursor(delta.terminal_sequence));

    // A replay of the exact durable identity must be idempotent while the swap is in flight.
    assert!(journal.append_replayed(delta.clone()));
    assert!(journal.append_replayed(delta));
    assert_eq!(journal.replayed_entries.len(), 1);
    assert!(journal.gap_proofs.is_empty());
}
#[tokio::test]
async fn summary_projection_replayed_identity_after_restart_is_idempotent() {
    let state = crate::tests::test_state_with_openai_base(
        Url::parse("http://127.0.0.1:9").expect("valid test URL"),
    )
    .await;
    let occurred_at = format_naive(Utc::now().with_timezone(&Shanghai).naive_local());
    let mut record = dashboard_runtime_topology_live_record(&occurred_at);
    record.id = 9_100_002;
    record.invoke_id = "summary-live-tail-replayed".to_string();
    record.status = Some("success".to_string());
    record.live_phase = None;
    let mut delta = apply_dashboard_activity_terminal_record(state.as_ref(), &record)
        .await
        .terminal_delta
        .expect("materialize replay delta");
    delta.persisted_row_id = Some(record.id);

    let mut journal = SummaryDeltaJournal::default();
    assert!(journal.append_replayed(delta.clone()));
    assert!(journal.append_replayed(delta.clone()));
    assert_eq!(journal.replayed_entries.len(), 1);
    assert!(!SummaryDeltaJournal::contains_same_identity(
        &delta,
        &DashboardActivityTerminalDelta {
            persisted_row_id: None,
            ..delta.clone()
        }
    ));
    journal.absorb_committed_delta(&delta);
    assert!(journal.replayed_entries.is_empty());
    assert!(journal.gap_proofs.is_empty());
}
#[tokio::test]
async fn summary_projection_replayed_identity_then_ack_is_not_duplicated() {
    let state = crate::tests::test_state_with_openai_base(
        Url::parse("http://127.0.0.1:9").expect("valid test URL"),
    )
    .await;
    let occurred_at = format_naive(Utc::now().with_timezone(&Shanghai).naive_local());
    let mut record = dashboard_runtime_topology_live_record(&occurred_at);
    record.id = 9_100_004;
    record.invoke_id = "summary-live-tail-replayed-ack".to_string();
    record.status = Some("success".to_string());
    record.live_phase = None;
    let mut replay = apply_dashboard_activity_terminal_record(state.as_ref(), &record)
        .await
        .terminal_delta
        .expect("materialize replay delta");
    replay.persisted_row_id = Some(record.id);
    replay.terminal_sequence = 0;
    let mut ack = replay.clone();
    ack.terminal_sequence = 1;

    let mut journal = SummaryDeltaJournal::default();
    assert!(journal.append_replayed(replay));
    assert!(journal.append(ack, 1));
    assert_eq!(journal.replayed_entries.len(), 0);
    assert_eq!(journal.entries.len(), 1);
    assert!(journal.gap_proofs.is_empty());
}
#[tokio::test]
async fn summary_projection_conflicting_row_identity_remains_fail_closed() {
    let state = crate::tests::test_state_with_openai_base(
        Url::parse("http://127.0.0.1:9").expect("valid test URL"),
    )
    .await;
    let occurred_at = format_naive(Utc::now().with_timezone(&Shanghai).naive_local());
    let mut first = dashboard_runtime_topology_live_record(&occurred_at);
    first.id = 9_100_003;
    first.invoke_id = "summary-live-tail-conflict-a".to_string();
    first.status = Some("success".to_string());
    first.live_phase = None;
    let mut first = apply_dashboard_activity_terminal_record(state.as_ref(), &first)
        .await
        .terminal_delta
        .expect("materialize first terminal delta");
    first.persisted_row_id = Some(9_100_003);

    let mut conflicting = first.clone();
    conflicting.invoke_id = "summary-live-tail-conflict-b".to_string();
    let mut journal = SummaryDeltaJournal::default();
    assert!(journal.append_replayed(first));
    assert!(!journal.append_replayed(conflicting));
    assert_eq!(journal.gap_proofs.len(), 1);
    assert_eq!(
        journal.gap_proofs.front().and_then(|proof| proof.row_id),
        Some(9_100_003)
    );
}
#[tokio::test]
async fn summary_delta_journal_rollback_removes_speculative_entry() {
    let state = crate::tests::test_state_with_openai_base(
        Url::parse("http://127.0.0.1:9").expect("valid test URL"),
    )
    .await;
    let occurred_at = format_naive(Utc::now().with_timezone(&Shanghai).naive_local());
    let mut record = dashboard_runtime_topology_live_record(&occurred_at);
    record.id = 1;
    record.invoke_id = "summary-delta-rolled-back".to_string();
    record.status = Some("success".to_string());
    record.live_phase = None;
    let mut speculative = apply_dashboard_activity_terminal_record(state.as_ref(), &record)
        .await
        .terminal_delta
        .expect("accept speculative terminal delta");
    speculative.terminal_sequence = 1;
    let mut journal = SummaryDeltaJournal::default();
    assert!(journal.register_pending(speculative.clone()));
    assert!(journal.entries.is_empty());
    journal.rollback_pending(Some(speculative.terminal_sequence));
    assert!(journal.pending.is_empty());
    assert!(journal.entries.is_empty());
    assert_eq!(journal.cursor, SummaryDeltaCursor(1));
    assert!(journal.gap_proofs.is_empty());
    let mut committed = speculative;
    committed.invoke_id = "summary-delta-after-rollback".to_string();
    committed.terminal_sequence = 2;
    committed.persisted_row_id = Some(2);
    assert!(journal.register_pending(committed.clone()));
    assert!(journal.acknowledge_pending(committed));
    assert_eq!(journal.entries.len(), 1);
    assert_eq!(
        journal.entries.front().map(|entry| entry.cursor),
        Some(SummaryDeltaCursor(2))
    );
    assert!(journal.gap_proofs.is_empty());
}
#[tokio::test]
async fn summary_delta_journal_capacity_overflow_retains_local_proof() {
    let state = crate::tests::test_state_with_openai_base(
        Url::parse("http://127.0.0.1:9").expect("valid test URL"),
    )
    .await;
    let occurred_at = format_naive(Utc::now().with_timezone(&Shanghai).naive_local());
    let mut template = dashboard_runtime_topology_live_record(&occurred_at);
    template.id = 1;
    template.invoke_id = "summary-delta-capacity-template".to_string();
    template.status = Some("success".to_string());
    template.live_phase = None;
    template.upstream_account_id = Some(42);
    let template = apply_dashboard_activity_terminal_record(state.as_ref(), &template)
        .await
        .terminal_delta
        .expect("accept terminal delta template");
    let mut journal = SummaryDeltaJournal::default();
    for sequence in 1..=SUMMARY_TERMINAL_OVERLAY_MAX_DELTAS as u64 {
        let mut delta = template.clone();
        delta.invoke_id = format!("summary-delta-capacity-{sequence}");
        delta.terminal_sequence = sequence;
        delta.persisted_row_id = Some(sequence as i64);
        assert!(
            journal.append(delta, sequence),
            "the bounded journal must admit its exact configured capacity"
        );
    }
    let mut overflow = template;
    overflow.invoke_id = "summary-delta-capacity-overflow".to_string();
    overflow.terminal_sequence = SUMMARY_TERMINAL_OVERLAY_MAX_DELTAS as u64 + 1;
    overflow.persisted_row_id = Some(SUMMARY_TERMINAL_OVERLAY_MAX_DELTAS as i64 + 1);
    assert!(
        !journal.append(overflow, SUMMARY_TERMINAL_OVERLAY_MAX_DELTAS as u64 + 1),
        "the first entry beyond capacity must not become an exact delta"
    );
    let proof = journal
        .gap_proofs
        .back()
        .expect("known overflow retains a bounded gap proof");
    assert_eq!(proof.upstream_account_id, Some(42));
    assert!(!proof.occurred_at.is_empty());
}
#[tokio::test]
async fn summary_delta_journal_proof_budget_retains_broad_fail_closed_guard() {
    let state = crate::tests::test_state_with_openai_base(
        Url::parse("http://127.0.0.1:9").expect("valid test URL"),
    )
    .await;
    let occurred_at = format_naive(Utc::now().with_timezone(&Shanghai).naive_local());
    let mut template = dashboard_runtime_topology_live_record(&occurred_at);
    template.id = 1;
    template.status = Some("success".to_string());
    template.live_phase = None;
    let template = apply_dashboard_activity_terminal_record(state.as_ref(), &template)
        .await
        .terminal_delta
        .expect("accept terminal delta template");

    let mut journal = SummaryDeltaJournal::default();
    for sequence in 1..=SUMMARY_DELTA_JOURNAL_MAX_GAP_PROOFS as u64 + 1 {
        let mut gap = template.clone();
        gap.invoke_id = format!("summary-delta-proof-budget-{sequence}");
        gap.terminal_sequence = sequence;
        gap.upstream_account_id = Some(sequence as i64);
        journal.note_gap(&gap);
    }

    assert!(journal.gap_proof_budget_exhausted);
    assert_eq!(journal.gap_proofs.len(), 1);
    let proof = journal
        .gap_proofs
        .front()
        .expect("budget exhaustion retains a broad proof");
    assert!(proof.occurred_at.is_empty());
    assert_eq!(proof.upstream_account_id, None);
    assert!(
        !proof.source_gap,
        "terminal-only overflow must remain terminal"
    );

    journal.note_unknown_source_cursor_gap(9_999);
    assert!(
        journal
            .gap_proofs
            .front()
            .is_some_and(|proof| proof.source_gap),
        "a later source gap must tighten an already broad terminal proof"
    );
}
