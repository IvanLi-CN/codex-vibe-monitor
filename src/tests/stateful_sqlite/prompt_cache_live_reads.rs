use super::*;
use serde_json::json;

#[tokio::test]
async fn prompt_cache_subscription_baseline_serves_current_while_refresh_queue_is_pending() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response
        ) VALUES ('pending-subscription-read', ?3, ?1, 'success', 12, 0.25, ?2, '{}')
        "#,
    )
    .bind(SOURCE_PROXY)
    .bind(json!({"promptCacheKey": "pending-subscription-key"}).to_string())
    .bind(db_occurred_at_lower_bound(Utc::now()))
    .execute(&state.pool)
    .await
    .expect("insert pending subscription invocation");

    let descriptor = SubscriptionTopicDescriptor {
        topic: "dashboard.working-conversations.current".to_string(),
        params: BTreeMap::from([
            ("pageSize".to_string(), "20".to_string()),
            ("recentInvocationLimit".to_string(), "16".to_string()),
        ]),
    };
    let prepared = state
        .subscription_hub
        .prepare_connection(state.clone(), vec![descriptor], Vec::new())
        .await
        .expect("prepare current subscription snapshot with pending statistics");
    assert_eq!(prepared.outcomes.len(), 1);
    assert_eq!(
        prepared.outcomes[0].disposition,
        TopicInitDisposition::SnapshotNoResume
    );
    assert_eq!(prepared.initial.len(), 1);
    let payload = prepared.initial[0].frame.payload_value();
    assert_eq!(
        payload["conversations"]
            .as_array()
            .expect("current conversations")
            .len(),
        1
    );
    assert_eq!(
        payload["conversations"][0]["promptCacheKey"],
        "pending-subscription-key"
    );
    assert!(payload["conversations"][0]["successCount"].is_null());
}

#[tokio::test]
async fn prompt_cache_conversation_reads_serve_current_but_gate_history_while_refresh_queue_is_pending()
 {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response
        ) VALUES ('pending-read-1', ?3, ?1, 'success', 12, 0.25, ?2, '{}')
        "#,
    )
    .bind(SOURCE_PROXY)
    .bind(json!({"promptCacheKey": "pending-read-key"}).to_string())
    .bind(db_occurred_at_lower_bound(Utc::now()))
    .execute(&state.pool)
    .await
    .expect("insert pending prompt-cache invocation");

    for (limit, activity_hours, activity_minutes, page_size) in [
        (Some(20), None, None, None),
        (None, Some(3), None, None),
        (None, None, Some(5), Some(20)),
    ] {
        let Json(response) = crate::fetch_prompt_cache_conversations(
            State(state.clone()),
            Query(PromptCacheConversationsQuery {
                limit,
                activity_hours,
                activity_minutes,
                page_size,
                cursor: None,
                snapshot_at: None,
                detail: None,
                recent_invocation_limit: None,
                blocked_binding_upstream_account_id: None,
                blocked_binding_constraint_source: None,
            }),
        )
        .await
        .expect("current read should remain available before rollups and statistics settle");
        assert_eq!(response.conversations.len(), 1);
        let conversation = &response.conversations[0];
        assert_eq!(conversation.prompt_cache_key, "pending-read-key");
        assert_eq!(conversation.request_count, 1);
        assert_eq!(conversation.total_tokens, 12);
        assert_eq!(conversation.success_count, None);
    }

    let error = crate::fetch_prompt_cache_conversations(
        State(state.clone()),
        Query(PromptCacheConversationsQuery {
            limit: None,
            activity_hours: None,
            activity_minutes: Some(5),
            page_size: Some(20),
            cursor: None,
            snapshot_at: Some(format_utc_iso_precise(Utc::now())),
            detail: None,
            recent_invocation_limit: None,
            blocked_binding_upstream_account_id: None,
            blocked_binding_constraint_source: None,
        }),
    )
    .await
    .expect_err("explicit historical snapshot should fail closed while coverage is incomplete");
    assert!(matches!(error, ApiError::Unavailable(_)));

    state.pool.close().await;
    let error = crate::fetch_prompt_cache_conversations(
        State(state.clone()),
        Query(PromptCacheConversationsQuery {
            limit: Some(20),
            activity_hours: None,
            activity_minutes: None,
            page_size: None,
            cursor: None,
            snapshot_at: None,
            detail: None,
            recent_invocation_limit: None,
            blocked_binding_upstream_account_id: None,
            blocked_binding_constraint_source: None,
        }),
    )
    .await
    .expect_err("warm legacy cache must not conceal a closed database pool");
    assert!(matches!(error, ApiError::Internal(_)));
    assert!(
        build_prompt_cache_conversations_response(
            state.as_ref(),
            PromptCacheConversationSelection::Count(20)
        )
        .await
        .is_err()
    );
}

#[tokio::test]
async fn prompt_cache_conversations_activity_minutes_paginated_overlays_memory_running_rows() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let now = Utc::now();

    async fn insert_terminal_row(
        pool: &Pool<Sqlite>,
        invoke_id: &str,
        occurred_at: DateTime<Utc>,
        key: &str,
    ) {
        sqlx::query(
            r#"
            INSERT INTO codex_invocations (
                invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response
            )
            VALUES (?1, ?2, ?3, 'success', ?4, ?5, ?6, ?7)
            "#,
        )
        .bind(invoke_id)
        .bind(format_naive(
            occurred_at.with_timezone(&Shanghai).naive_local(),
        ))
        .bind(SOURCE_PROXY)
        .bind(10)
        .bind(0.01_f64)
        .bind(json!({ "promptCacheKey": key, "routeMode": "pool" }).to_string())
        .bind("{}")
        .execute(pool)
        .await
        .expect("insert terminal prompt cache invocation row");
    }

    for index in 0..2 {
        insert_terminal_row(
            &state.pool,
            &format!("memory-overlay-terminal-{}", index + 1),
            now - ChronoDuration::minutes(index + 1),
            &format!("memory-overlay-terminal-{}", index + 1),
        )
        .await;
    }
    insert_terminal_row(
        &state.pool,
        "memory-overlay-running-3-old-terminal",
        now - ChronoDuration::minutes(20),
        "memory-overlay-running-3",
    )
    .await;

    for index in 0..3 {
        let runtime_started_at = if index == 2 {
            now - ChronoDuration::minutes(15)
        } else {
            now - ChronoDuration::seconds(index * 10)
        };
        let occurred_at = format_naive(runtime_started_at.with_timezone(&Shanghai).naive_local());
        let prompt_cache_key = format!("memory-overlay-running-{}", index + 1);
        let running_record = build_running_proxy_capture_record(
            &format!("memory-overlay-running-invoke-{}", index + 1),
            &occurred_at,
            ProxyCaptureTarget::Responses,
            &RequestCaptureInfo {
                model: Some("gpt-5.5".to_string()),
                prompt_cache_key: Some(prompt_cache_key.clone()),
                ..RequestCaptureInfo::default()
            },
            Some("198.51.100.42"),
            None,
            Some(&prompt_cache_key),
            true,
            Some(17),
            Some("pool-account-17"),
            None,
            None,
            Some("jp-relay-01"),
            Some(1),
            Some(1),
            None,
            None,
            1.0,
            2.0,
            3.0,
            4.0,
        );
        state
            .proxy_runtime_invocations
            .upsert(api_invocation_from_runtime_record(&running_record));
    }

    sqlx::query(
        "INSERT OR REPLACE INTO prompt_cache_conversation_stats_refresh_queue (prompt_cache_key) \
         VALUES (?1)",
    )
    .bind("memory-overlay-running-1")
    .execute(&state.pool)
    .await
    .expect("queue prompt-cache statistics refresh");

    let Json(response) = crate::fetch_prompt_cache_conversations(
        State(state.clone()),
        Query(PromptCacheConversationsQuery {
            limit: None,
            activity_hours: None,
            activity_minutes: Some(5),
            page_size: Some(20),
            cursor: None,
            snapshot_at: None,
            detail: Some("compact".to_string()),
            recent_invocation_limit: None,

            blocked_binding_upstream_account_id: None,

            blocked_binding_constraint_source: None,
        }),
    )
    .await
    .expect("paginated working response should overlay memory running rows");

    let prompt_cache_keys = response
        .conversations
        .iter()
        .map(|conversation| conversation.prompt_cache_key.as_str())
        .collect::<HashSet<_>>();

    assert_eq!(response.total_matched, Some(5));
    assert_eq!(response.conversations.len(), prompt_cache_keys.len());
    assert!(prompt_cache_keys.contains("memory-overlay-terminal-1"));
    assert!(prompt_cache_keys.contains("memory-overlay-terminal-2"));
    for index in 0..3 {
        let prompt_cache_key = format!("memory-overlay-running-{}", index + 1);
        let conversation = response
            .conversations
            .iter()
            .find(|conversation| conversation.prompt_cache_key == prompt_cache_key)
            .unwrap_or_else(|| panic!("missing {prompt_cache_key}"));
        assert!(conversation.last_in_flight_at.is_some());
        assert_eq!(
            conversation
                .recent_invocations
                .first()
                .map(|invocation| invocation.status.as_str()),
            Some("running")
        );
        if prompt_cache_key == "memory-overlay-running-1" {
            assert_eq!(conversation.request_count, 1);
            assert_eq!(conversation.total_tokens, 0);
            assert!((conversation.total_cost - 0.0).abs() < f64::EPSILON);
        }
        if prompt_cache_key == "memory-overlay-running-3" {
            assert_eq!(conversation.request_count, 2);
            assert_eq!(conversation.total_tokens, 10);
            assert!((conversation.total_cost - 0.01).abs() < f64::EPSILON);
        }
    }
}
