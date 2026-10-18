use super::*;

#[tokio::test]
async fn parallel_work_concurrent_baselines_complete_with_one_connection() {
    assert_parallel_work_baselines_with_one_connection(RuntimeProjectionMode::Auto).await;
}

#[tokio::test]
async fn parallel_work_legacy_baselines_retain_json_with_one_connection() {
    assert_parallel_work_baselines_with_one_connection(RuntimeProjectionMode::Legacy).await;
}

async fn assert_parallel_work_baselines_with_one_connection(mode: RuntimeProjectionMode) {
    let mut state = crate::tests::test_state_with_openai_base_and_runtime_projection_mode(
        Url::parse("http://127.0.0.1:9").expect("valid test URL"),
        mode,
    )
    .await;
    // The constructor returns the sole AppState owner. Reconnect to its existing
    // shared in-memory database before dropping the ordinary four-connection pool.
    let original_pool = state.pool.clone();
    let constrained_pool = sqlx::sqlite::SqlitePoolOptions::new()
        .min_connections(1)
        .max_connections(1)
        .connect_with((*original_pool.connect_options()).clone())
        .await
        .expect("connect constrained pool to the seeded schema");
    Arc::get_mut(&mut state)
        .expect("test state has no other AppState owners before building")
        .pool = constrained_pool;
    drop(original_pool);

    sqlx::query(
        "INSERT INTO codex_invocations (invoke_id, occurred_at, source, status, payload, raw_response) VALUES (?1, ?2, ?3, 'success', ?4, '{}')",
    )
    .bind("one-connection-baseline")
    .bind(format_naive(
        (Utc::now() - ChronoDuration::minutes(2))
            .with_timezone(&Shanghai)
            .naive_local(),
    ))
    .bind(SOURCE_PROXY)
    .bind(r#"{"promptCacheKey":"one-connection-key"}"#)
    .execute(&state.pool)
    .await
    .expect("seed a persisted parallel-work conversation");
    let topic = SubscriptionTopic::ParallelWorkCurrent {
        range: "1d".to_string(),
        time_zone: "UTC".to_string(),
        bucket: Some("1m".to_string()),
        upstream_account_id: None,
    };
    let hub = state.subscription_hub.clone();
    let topic_key = topic.cache_key().expect("constrained-pool topic key");
    let lease = hub
        .register_topic_subscribers(std::slice::from_ref(&topic))
        .await
        .expect("register constrained-pool topic owner");
    hub.schedule_parallel_work_topic_projection(
        state.clone(),
        topic.clone(),
        &[SequencedRuntimeMutation {
            sequence: 1,
            mutation: RuntimeMutation::Invocation(RuntimeInvocationMutation {
                identity: RuntimeInvocationIdentity::new(
                    "one-connection-runtime",
                    format_utc_iso(Utc::now()),
                ),
                kind: RuntimeMutationKind::RuntimeUpsert,
                row_id: None,
                is_terminal: false,
                prompt_cache_key: Some("one-connection-runtime-key".to_string()),
                sticky_key: None,
                upstream_account_id: None,
            }),
        }],
    )
    .await
    .expect("buffer a real runtime mutation before the cold baseline");
    let builders =
        (0..6).map(|_| hub.build_parallel_work_consistent_baseline(state.clone(), &topic));
    // Preserve SQLx's original 30-second acquire deadline and all existing tests.
    // One connection makes the old nested-acquire path fail without a timing hook.
    let results = futures_util::future::join_all(builders).await;
    for result in results {
        let (payload, _) = result.expect("concurrent constrained-pool baseline completes");
        let response = match payload {
            BuiltSubscriptionTopicPayload::Dashboard(
                DashboardTopicMaterializer::ParallelWork { base },
            ) => {
                assert_eq!(mode, RuntimeProjectionMode::Auto);
                serde_json::to_value(
                    &base
                        .lock()
                        .expect("parallel-work materializer state lock")
                        .response,
                )
                .expect("serialize the typed baseline")
            }
            BuiltSubscriptionTopicPayload::Json(response) => {
                assert_eq!(mode, RuntimeProjectionMode::Legacy);
                response
            }
            _ => panic!("parallel-work baseline must preserve its mode-specific payload"),
        };
        let conversations = response["current"]["conversations"]
            .as_array()
            .expect("baseline conversation array");
        let conversation = conversations
            .iter()
            .find(|conversation| conversation["conversationId"] == "one-connection-key")
            .expect("every baseline retains the persisted conversation");
        assert_eq!(conversation["requestCount"], 1);
        assert!(!conversations.iter().any(|conversation| {
            conversation["conversationId"] == "one-connection-runtime-key"
        }));
        assert!(
            response["current"]["points"]
                .as_array()
                .expect("baseline points array")
                .iter()
                .any(|point| point["parallelCount"] == 1)
        );
    }
    hub.prepare_connection(state.clone(), vec![topic.descriptor()], Vec::new())
        .await
        .expect("commit the constrained-pool baseline through the real caller");
    let guard = hub.state.lock().await;
    let cached = &guard.topics[&topic_key];
    assert_eq!(
        cached.dashboard_materializer.is_some(),
        mode == RuntimeProjectionMode::Auto
    );
    let response: Value = serde_json::from_slice(cached.snapshot_frame.payload_bytes.as_ref())
        .expect("decode the committed subscription snapshot");
    let conversations = response["current"]["conversations"]
        .as_array()
        .expect("committed conversation array");
    assert!(conversations.iter().any(|conversation| {
        conversation["conversationId"] == "one-connection-key" && conversation["requestCount"] == 1
    }));
    assert_eq!(
        conversations
            .iter()
            .any(|conversation| { conversation["conversationId"] == "one-connection-runtime-key" }),
        mode == RuntimeProjectionMode::Auto,
        "only the Auto materializer replays buffered runtime mutations"
    );
    drop(guard);
    drop(lease);
}

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
fn parallel_work_projection_promotes_persisted_current_minute_at_utc_boundary() {
    let minute_start = Utc
        .timestamp_opt(1_700_000_040, 0)
        .single()
        .expect("construct minute start");
    let rebase = |seconds| {
        rolling_dashboard_window_requires_rebase(
            Some(minute_start),
            Some(minute_start + ChronoDuration::seconds(seconds)),
        )
    };
    assert!(!rebase(59));
    assert!(rebase(60));
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
