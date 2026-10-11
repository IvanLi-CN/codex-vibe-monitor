use super::*;
use serde_json::json;
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

async fn working_conversations_snapshot_builder_uses_one_transaction_snapshot() {
    let (state, temp_dir, _) = crate::tests::file_backed_test_state_from_current_schema(
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
    sqlx::query(
            "INSERT OR REPLACE INTO prompt_cache_conversation_stats_refresh_queue (prompt_cache_key) VALUES (?1)",
        )
        .bind("runtime-overlay-pending-key")
        .execute(&state.pool)
        .await
        .expect("queue prompt-cache statistics refresh before SSE baseline");

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
    crate::tests::complete_prompt_cache_conversation_materialization_for_test(&state.pool).await;

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
    // Bounded hydration reads current-hour P1 rows plus materialized prior-hour history.
    // Keep both fixture rows in the live hour so the contract does not depend on an
    // hour-rollup boundary while CI happens to run near the top of an hour.
    let occurred_at = format_naive(now.with_timezone(&Shanghai).naive_local());
    for (invoke_id, occurred_at, total_tokens) in [
        ("reentered-older", occurred_at.as_str(), 11_i64),
        ("reentered-newer", occurred_at.as_str(), 31_i64),
    ] {
        sqlx::query(
                r#"
                INSERT INTO codex_invocations (
                    invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response
                ) VALUES (?1, ?2, 'proxy', 'success', ?3, 0.25, ?4, '{}')
                "#,
            )
            .bind(invoke_id)
            .bind(occurred_at)
            .bind(total_tokens)
            .bind(
                json!({
                    "promptCacheKey": "reentered-key",
                    "upstreamAccountId": 42,
                    "upstreamAccountName": "Hydrated Account",
                })
                .to_string(),
            )
            .execute(&state.pool)
            .await
            .expect("persist reentered working conversation history");
    }
    let mut runtime_terminal = dashboard_runtime_topology_live_record(&occurred_at);
    runtime_terminal.id = 0;
    runtime_terminal.invoke_id = "reentered-runtime".to_string();
    runtime_terminal.status = Some("success".to_string());
    runtime_terminal.live_phase = None;
    runtime_terminal.prompt_cache_key = Some("reentered-key".to_string());
    runtime_terminal.total_tokens = Some(7);
    runtime_terminal.cost = Some(0.1);
    let runtime_terminal_identity = runtime_prompt_cache_overlay_identity(&runtime_terminal);
    state.proxy_runtime_invocations.upsert(runtime_terminal);
    // The hydration snapshot is captured after these writes. If CI crosses an hour
    // boundary in between, account details read the materialized historical bucket
    // rather than the prior-hour P1 rows, just as production does after rollup.
    sqlx::query(
        r#"
            INSERT INTO prompt_cache_upstream_account_hourly (
                bucket_start_epoch,
                source,
                prompt_cache_key,
                upstream_account_key,
                upstream_account_id,
                upstream_account_name,
                request_count,
                success_count,
                failure_count,
                total_tokens,
                total_cost,
                first_seen_at,
                last_seen_at,
                updated_at
            )
            VALUES (?1, 'proxy', 'reentered-key', 'id:42|name:Hydrated Account', 42,
                    'Hydrated Account', 2, 2, 0, 42, 0.5, ?2, ?2, datetime('now'))
            "#,
    )
    .bind(now.timestamp().div_euclid(3_600) * 3_600)
    .bind(&occurred_at)
    .execute(&state.pool)
    .await
    .expect("materialize reentered working conversation account history");
    sqlx::query(
            r#"
            INSERT INTO prompt_cache_conversation_bindings (
                prompt_cache_key, binding_kind, group_name, upstream_account_id, created_at, updated_at
            ) VALUES ('reentered-key', 'group', 'Bounded Group', NULL, datetime('now'), datetime('now'))
            "#,
        )
        .execute(&state.pool)
        .await
        .expect("persist reentered key manual binding");
    crate::tests::complete_prompt_cache_conversation_materialization_for_test(&state.pool).await;

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
    assert_eq!(conversation.request_count, 3);
    assert_eq!(conversation.total_tokens, 49);
    assert_eq!(conversation.last24h_requests.len(), 3);
    assert_eq!(conversation.recent_invocations.len(), 3);
    assert_eq!(
        conversation
            .manual_binding
            .as_ref()
            .and_then(|binding| binding.group_name.as_deref()),
        Some("Bounded Group"),
        "key hydration must restore manual binding metadata",
    );
    assert_eq!(
        conversation.upstream_accounts[0].upstream_account_id,
        Some(42),
        "key hydration must retain account metadata rather than synthesizing a delta-only row",
    );
    assert_eq!(cached.prompt_cache_full_hydration_count, 1);
    assert_eq!(cached.prompt_cache_bounded_key_hydration_count, 1);
    assert!(cached.prompt_cache_pending_key_hydrations.is_empty());
    assert!(!cached.prompt_cache_reconcile_required);
    assert!(
        cached
            .prompt_cache_applied_terminal_ids
            .contains(&runtime_terminal_identity)
    );
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
    crate::tests::complete_prompt_cache_conversation_materialization_for_test(&state.pool).await;

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
        materializer.response.conversations[0].prompt_cache_key, "candidate-newer",
        "candidate refill must hydrate only the selected active page in sort order",
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
async fn working_conversations_bounded_hydration_reads_while_statistics_materialize() {
    let state = crate::tests::test_state_with_openai_base(
        Url::parse("http://127.0.0.1:9").expect("valid test URL"),
    )
    .await;
    let occurred_at = format_naive(Utc::now().with_timezone(&Shanghai).naive_local());
    let mut record = dashboard_runtime_topology_live_record(&occurred_at);
    record.id = 0;
    record.invoke_id = "queued-working-hydration".to_string();
    record.status = Some("success".to_string());
    record.live_phase = None;
    record.prompt_cache_key = Some("queued-working-hydration".to_string());
    state.proxy_runtime_invocations.upsert(record);
    sqlx::query(
            "INSERT OR REPLACE INTO prompt_cache_conversation_stats_refresh_queue (prompt_cache_key) VALUES (?1)",
        )
        .bind("queued-working-hydration")
        .execute(&state.pool)
        .await
        .expect("queue prompt cache statistics refresh");

    let range_end = Utc::now();
    let range_start_bound = db_occurred_at_lower_bound(
        range_end
            - ChronoDuration::minutes(SUBSCRIPTION_DEFAULT_WORKING_CONVERSATIONS_ACTIVITY_MINUTES),
    );
    let source_scope = resolve_default_source_scope(&state.pool)
        .await
        .expect("resolve default source scope");
    let conversation = hydrate_working_prompt_cache_conversation_for_key(
        state.as_ref(),
        source_scope,
        "queued-working-hydration",
        range_end,
        &range_start_bound,
        16,
        None,
    )
    .await
    .expect("bounded hydration should keep serving while statistics are queued")
    .expect("queued runtime-only working conversation should remain visible");

    assert_eq!(conversation.prompt_cache_key, "queued-working-hydration");
    assert_eq!(conversation.request_count, 1);
    assert_eq!(conversation.success_count, None);
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
    crate::tests::complete_prompt_cache_conversation_materialization_for_test(&state.pool).await;

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

#[tokio::test]
async fn working_conversations_bounded_hydration_keeps_runtime_overlay_after_snapshot_boundary() {
    let state = crate::tests::test_state_with_openai_base(
        Url::parse("http://127.0.0.1:9").expect("valid test URL"),
    )
    .await;
    let range_end = Utc::now();
    let occurred_at = format_naive(range_end.with_timezone(&Shanghai).naive_local());
    let mut record = dashboard_runtime_topology_live_record(&occurred_at);
    record.id = 0;
    record.invoke_id = "post-snapshot-runtime-overlay-hydration".to_string();
    record.status = Some("success".to_string());
    record.live_phase = None;
    record.prompt_cache_key = Some("post-snapshot-runtime-overlay-hydration-key".to_string());
    record.upstream_account_id = Some(79);
    record.upstream_account_name = Some("Post Snapshot Hydration Account".to_string());
    record.total_tokens = Some(43);
    record.cost = Some(0.6);
    state.proxy_runtime_invocations.upsert(record.clone());

    // Simulate P1 committing after the bounded hydrate's durable visibility boundary. The
    // durable row must not remove the runtime terminal until a matching snapshot can see it.
    let created_after_snapshot = format_utc_iso_precise(range_end + ChronoDuration::minutes(5));
    sqlx::query(
        r#"
            INSERT INTO codex_invocations (
                invoke_id, occurred_at, source, status, total_tokens, cost, payload,
                raw_response, created_at
            ) VALUES (?1, ?2, 'proxy', 'success', 43, 0.6, ?3, '{}', ?4)
            "#,
    )
    .bind(&record.invoke_id)
    .bind(&record.occurred_at)
    .bind(
        json!({
            "promptCacheKey": "post-snapshot-runtime-overlay-hydration-key",
            "upstreamAccountId": 79,
            "upstreamAccountName": "Post Snapshot Hydration Account",
        })
        .to_string(),
    )
    .bind(created_after_snapshot)
    .execute(&state.pool)
    .await
    .expect("persist terminal beyond bounded hydration snapshot");
    crate::tests::complete_prompt_cache_conversation_materialization_for_test(&state.pool).await;

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
        "post-snapshot-runtime-overlay-hydration-key",
        range_end,
        &range_start_bound,
        16,
        None,
    )
    .await
    .expect("hydrate post-snapshot persisted runtime overlay")
    .expect("runtime overlay key is selected");

    assert_eq!(conversation.request_count, 1);
    assert_eq!(conversation.total_tokens, 43);
    assert!((conversation.total_cost - 0.6).abs() < f64::EPSILON);
    assert_eq!(conversation.last24h_requests.len(), 1);
    assert_eq!(conversation.recent_invocations.len(), 1);
    assert_eq!(conversation.upstream_accounts.len(), 1);
    assert_eq!(conversation.upstream_accounts[0].request_count, 1);
}

#[tokio::test]
async fn working_conversations_bounded_hydration_prefers_p1_lifecycle_over_stale_working_set() {
    let state = crate::tests::test_state_with_openai_base(
        Url::parse("http://127.0.0.1:9").expect("valid test URL"),
    )
    .await;
    let range_end = Utc::now();
    let old_occurred_at = format_naive(
        (range_end - ChronoDuration::minutes(2))
            .with_timezone(&Shanghai)
            .naive_local(),
    );
    let newest_occurred_at = format_naive(
        (range_end - ChronoDuration::minutes(1))
            .with_timezone(&Shanghai)
            .naive_local(),
    );
    let prompt_cache_key = "p1-before-p2-working-hydration";
    for (invoke_id, occurred_at, total_tokens) in [
        ("p1-before-p2-old", old_occurred_at.as_str(), 11_i64),
        ("p1-before-p2-new", newest_occurred_at.as_str(), 31_i64),
    ] {
        sqlx::query(
                r#"
                INSERT INTO codex_invocations (
                    invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response
                ) VALUES (?1, ?2, 'proxy', 'success', ?3, 0.25, ?4, '{}')
                "#,
            )
            .bind(invoke_id)
            .bind(occurred_at)
            .bind(total_tokens)
            .bind(json!({ "promptCacheKey": prompt_cache_key }).to_string())
            .execute(&state.pool)
            .await
            .expect("persist terminal before P2 working-set refresh");
    }
    sqlx::query(
        r#"
            INSERT INTO prompt_cache_working_set_live (
                prompt_cache_key, source_scope_all, source_scope_proxy_only,
                created_at, last_activity_at, last_terminal_at, sort_anchor_at,
                request_count, total_tokens, total_cost, updated_at
            ) VALUES (?1, 1, 1, ?2, ?2, ?2, ?2, 1, 11, 0.25, datetime('now'))
            ON CONFLICT(prompt_cache_key) DO UPDATE SET
                source_scope_all = excluded.source_scope_all,
                source_scope_proxy_only = excluded.source_scope_proxy_only,
                created_at = excluded.created_at,
                last_activity_at = excluded.last_activity_at,
                last_terminal_at = excluded.last_terminal_at,
                last_in_flight_at = NULL,
                sort_anchor_at = excluded.sort_anchor_at,
                request_count = excluded.request_count,
                total_tokens = excluded.total_tokens,
                total_cost = excluded.total_cost,
                updated_at = excluded.updated_at
            "#,
    )
    .bind(prompt_cache_key)
    .bind(&old_occurred_at)
    .execute(&state.pool)
    .await
    .expect("seed stale P2 working-set row");
    crate::tests::complete_prompt_cache_conversation_materialization_for_test(&state.pool).await;

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
        prompt_cache_key,
        range_end,
        &range_start_bound,
        16,
        None,
    )
    .await
    .expect("hydrate a key with P1 ahead of P2")
    .expect("P1 lifecycle remains selected");

    assert_eq!(conversation.request_count, 2);
    assert_eq!(conversation.total_tokens, 42);
    assert_eq!(conversation.recent_invocations.len(), 2);
    assert_eq!(
        conversation.last_terminal_at.as_deref(),
        Some(newest_occurred_at.as_str())
    );
    assert_eq!(conversation.last_activity_at, newest_occurred_at);
}

#[tokio::test]
async fn working_conversations_bounded_hydration_keeps_transient_runtime_records_after_filtering_persisted_terminal()
 {
    let state = crate::tests::test_state_with_openai_base(
        Url::parse("http://127.0.0.1:9").expect("valid test URL"),
    )
    .await;
    let range_end = Utc::now();
    let occurred_at = format_naive(range_end.with_timezone(&Shanghai).naive_local());
    let prompt_cache_key = "bounded-hydration-overlay-filter-key";
    let mut persisted_terminal = dashboard_runtime_topology_live_record(&occurred_at);
    persisted_terminal.id = 0;
    persisted_terminal.invoke_id = "bounded-hydration-persisted-terminal".to_string();
    persisted_terminal.status = Some("success".to_string());
    persisted_terminal.live_phase = None;
    persisted_terminal.prompt_cache_key = Some(prompt_cache_key.to_string());
    persisted_terminal.upstream_account_id = Some(79);
    persisted_terminal.upstream_account_name = Some("Bounded Filter Account".to_string());
    persisted_terminal.total_tokens = Some(41);
    persisted_terminal.cost = Some(0.5);
    state
        .proxy_runtime_invocations
        .upsert(persisted_terminal.clone());
    sqlx::query(
        r#"
            INSERT INTO codex_invocations (
                invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response
            ) VALUES (?1, ?2, 'proxy', 'success', 41, 0.5, ?3, '{}')
            "#,
    )
    .bind(&persisted_terminal.invoke_id)
    .bind(&persisted_terminal.occurred_at)
    .bind(
        json!({
            "promptCacheKey": prompt_cache_key,
            "upstreamAccountId": 79,
            "upstreamAccountName": "Bounded Filter Account",
        })
        .to_string(),
    )
    .execute(&state.pool)
    .await
    .expect("persist terminal before bounded hydration acknowledgement");
    crate::tests::complete_prompt_cache_conversation_materialization_for_test(&state.pool).await;

    let mut runtime_terminal = persisted_terminal.clone();
    runtime_terminal.invoke_id = "bounded-hydration-runtime-terminal".to_string();
    runtime_terminal.total_tokens = Some(37);
    runtime_terminal.cost = Some(0.75);
    state
        .proxy_runtime_invocations
        .upsert(runtime_terminal.clone());

    let mut in_flight = persisted_terminal.clone();
    in_flight.invoke_id = "bounded-hydration-runtime-in-flight".to_string();
    in_flight.status = Some("running".to_string());
    in_flight.live_phase = Some("requesting".to_string());
    in_flight.total_tokens = None;
    in_flight.cost = None;
    state.proxy_runtime_invocations.upsert(in_flight.clone());

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
        prompt_cache_key,
        range_end,
        &range_start_bound,
        16,
        None,
    )
    .await
    .expect("hydrate mixed runtime overlay working conversation")
    .expect("mixed runtime overlay key is selected");

    assert_eq!(conversation.request_count, 3);
    assert_eq!(conversation.total_tokens, 78);
    assert!((conversation.total_cost - 1.25).abs() < f64::EPSILON);
    assert_eq!(conversation.recent_invocations.len(), 3);
    assert!(
        conversation
            .recent_invocations
            .iter()
            .any(|preview| preview.invoke_id == runtime_terminal.invoke_id)
    );
    assert!(
        conversation
            .recent_invocations
            .iter()
            .any(|preview| preview.invoke_id == in_flight.invoke_id)
    );
    assert!(conversation.last_in_flight_at.is_some());
}
