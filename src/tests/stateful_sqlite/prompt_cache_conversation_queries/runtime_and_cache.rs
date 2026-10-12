use super::test_support::{
    fetch_prompt_cache_conversations, materialize_prompt_cache_hourly_rollups,
};
use super::*;
use serde_json::json;

#[tokio::test]
async fn prompt_cache_conversations_activity_minutes_paginated_keeps_running_and_pending_working_rows()
 {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let now = Utc::now();

    async fn insert_row(
        pool: &Pool<Sqlite>,
        invoke_id: &str,
        occurred_at: DateTime<Utc>,
        key: &str,
        status: &str,
    ) {
        sqlx::query(
            r#"
            INSERT INTO codex_invocations (
                invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
            "#,
        )
        .bind(invoke_id)
        .bind(format_naive(
            occurred_at.with_timezone(&Shanghai).naive_local(),
        ))
        .bind(SOURCE_PROXY)
        .bind(status)
        .bind(10)
        .bind(0.01_f64)
        .bind(json!({ "promptCacheKey": key, "routeMode": "pool" }).to_string())
        .bind("{}")
        .execute(pool)
        .await
        .expect("insert paginated working-semantics row");
    }

    insert_row(
        &state.pool,
        "working-recent-terminal",
        now - ChronoDuration::minutes(4),
        "working-recent-terminal",
        "success",
    )
    .await;
    insert_row(
        &state.pool,
        "working-running-terminal-old",
        now - ChronoDuration::minutes(12),
        "working-running",
        "success",
    )
    .await;
    insert_row(
        &state.pool,
        "working-running-live",
        now - ChronoDuration::minutes(1),
        "working-running",
        "running",
    )
    .await;
    insert_row(
        &state.pool,
        "working-pending-live",
        now - ChronoDuration::minutes(2),
        "working-pending",
        "pending",
    )
    .await;
    insert_row(
        &state.pool,
        "working-stale-terminal",
        now - ChronoDuration::minutes(7),
        "working-stale-terminal",
        "success",
    )
    .await;

    let Json(response) = fetch_prompt_cache_conversations(
        State(state),
        Query(PromptCacheConversationsQuery {
            limit: None,
            activity_hours: None,
            activity_minutes: Some(5),
            page_size: Some(10),
            cursor: None,
            snapshot_at: None,
            detail: Some("compact".to_string()),
            recent_invocation_limit: None,

            blocked_binding_upstream_account_id: None,

            blocked_binding_constraint_source: None,
        }),
    )
    .await
    .expect("paginated working semantics response should succeed");

    let prompt_cache_keys = response
        .conversations
        .iter()
        .map(|conversation| conversation.prompt_cache_key.as_str())
        .collect::<HashSet<_>>();
    let running = response
        .conversations
        .iter()
        .find(|conversation| conversation.prompt_cache_key == "working-running")
        .expect("running working row should remain visible");
    let pending = response
        .conversations
        .iter()
        .find(|conversation| conversation.prompt_cache_key == "working-pending")
        .expect("pending working row should remain visible");

    assert_eq!(response.total_matched, Some(3));
    assert!(prompt_cache_keys.contains("working-recent-terminal"));
    assert!(prompt_cache_keys.contains("working-running"));
    assert!(prompt_cache_keys.contains("working-pending"));
    assert!(!prompt_cache_keys.contains("working-stale-terminal"));
    assert!(running.last_terminal_at.is_none());
    assert!(running.last_in_flight_at.is_some());
    assert!(pending.last_terminal_at.is_none());
    assert!(pending.last_in_flight_at.is_some());
}

#[tokio::test]
async fn prompt_cache_conversations_activity_minutes_paginated_ignores_stale_terminal_runtime_rows()
{
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let snapshot_at = Utc
        .timestamp_opt(Utc::now().timestamp(), 500_000_000)
        .single()
        .expect("valid snapshot timestamp");

    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response, created_at
        )
        VALUES (?1, ?2, ?3, 'success', ?4, ?5, ?6, ?7, ?8)
        "#,
    )
    .bind("working-runtime-window-recent")
    .bind(format_naive(
        (snapshot_at - ChronoDuration::minutes(1))
            .with_timezone(&Shanghai)
            .naive_local(),
    ))
    .bind(SOURCE_PROXY)
    .bind(10)
    .bind(0.01_f64)
    .bind(json!({ "promptCacheKey": "working-runtime-window-recent", "routeMode": "pool" }).to_string())
    .bind("{}")
    .bind(format_utc_iso_precise(
        snapshot_at - ChronoDuration::seconds(1),
    ))
    .execute(&state.pool)
    .await
    .expect("insert recent working prompt-cache row");

    let boundary_terminal_occurred_at = format_naive(
        (snapshot_at - ChronoDuration::minutes(5))
            .with_timezone(&Shanghai)
            .naive_local(),
    );
    let boundary_terminal = build_running_proxy_capture_record(
        "working-runtime-window-boundary-terminal-invoke",
        &boundary_terminal_occurred_at,
        ProxyCaptureTarget::Responses,
        &RequestCaptureInfo {
            model: Some("gpt-5.5".to_string()),
            prompt_cache_key: Some("working-runtime-window-boundary-terminal".to_string()),
            ..RequestCaptureInfo::default()
        },
        Some("198.51.100.42"),
        None,
        Some("working-runtime-window-boundary-terminal"),
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
    let mut boundary_terminal = api_invocation_from_runtime_record(&boundary_terminal);
    boundary_terminal.status = Some("interrupted".to_string());
    state
        .proxy_runtime_invocations
        .upsert_terminal(boundary_terminal);

    let stale_terminal_occurred_at = format_naive(
        (snapshot_at - ChronoDuration::minutes(15))
            .with_timezone(&Shanghai)
            .naive_local(),
    );
    let stale_terminal = build_running_proxy_capture_record(
        "working-runtime-window-stale-terminal-invoke",
        &stale_terminal_occurred_at,
        ProxyCaptureTarget::Responses,
        &RequestCaptureInfo {
            model: Some("gpt-5.5".to_string()),
            prompt_cache_key: Some("working-runtime-window-stale-terminal".to_string()),
            ..RequestCaptureInfo::default()
        },
        Some("198.51.100.42"),
        None,
        Some("working-runtime-window-stale-terminal"),
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
    let mut stale_terminal = api_invocation_from_runtime_record(&stale_terminal);
    stale_terminal.status = Some("interrupted".to_string());
    state
        .proxy_runtime_invocations
        .upsert_terminal(stale_terminal);

    let Json(response) = fetch_prompt_cache_conversations(
        State(state),
        Query(PromptCacheConversationsQuery {
            limit: None,
            activity_hours: None,
            activity_minutes: Some(5),
            page_size: Some(20),
            cursor: None,
            snapshot_at: Some(format_utc_iso_precise(snapshot_at)),
            detail: Some("compact".to_string()),
            recent_invocation_limit: None,

            blocked_binding_upstream_account_id: None,

            blocked_binding_constraint_source: None,
        }),
    )
    .await
    .expect("paginated working response should ignore stale terminal runtime rows");

    let prompt_cache_keys = response
        .conversations
        .iter()
        .map(|conversation| conversation.prompt_cache_key.as_str())
        .collect::<HashSet<_>>();

    assert_eq!(response.total_matched, Some(2));
    assert!(prompt_cache_keys.contains("working-runtime-window-recent"));
    assert!(prompt_cache_keys.contains("working-runtime-window-boundary-terminal"));
    assert!(!prompt_cache_keys.contains("working-runtime-window-stale-terminal"));
}

#[tokio::test]
async fn prompt_cache_conversations_activity_minutes_paginated_sorts_by_newer_in_flight_anchor() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let now = Utc::now();

    async fn insert_row(
        pool: &Pool<Sqlite>,
        invoke_id: &str,
        occurred_at: DateTime<Utc>,
        key: &str,
        status: &str,
    ) {
        sqlx::query(
            r#"
            INSERT INTO codex_invocations (
                invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
            "#,
        )
        .bind(invoke_id)
        .bind(format_naive(
            occurred_at.with_timezone(&Shanghai).naive_local(),
        ))
        .bind(SOURCE_PROXY)
        .bind(status)
        .bind(10)
        .bind(0.01_f64)
        .bind(json!({ "promptCacheKey": key, "routeMode": "pool" }).to_string())
        .bind("{}")
        .execute(pool)
        .await
        .expect("insert mixed sort-anchor row");
    }

    insert_row(
        &state.pool,
        "working-mixed-terminal",
        now - ChronoDuration::minutes(4),
        "working-mixed",
        "success",
    )
    .await;
    insert_row(
        &state.pool,
        "working-mixed-running",
        now - ChronoDuration::minutes(1),
        "working-mixed",
        "running",
    )
    .await;
    insert_row(
        &state.pool,
        "working-terminal-only",
        now - ChronoDuration::minutes(2),
        "working-terminal-only",
        "success",
    )
    .await;

    let Json(response) = fetch_prompt_cache_conversations(
        State(state),
        Query(PromptCacheConversationsQuery {
            limit: None,
            activity_hours: None,
            activity_minutes: Some(5),
            page_size: Some(10),
            cursor: None,
            snapshot_at: None,
            detail: Some("compact".to_string()),
            recent_invocation_limit: None,

            blocked_binding_upstream_account_id: None,

            blocked_binding_constraint_source: None,
        }),
    )
    .await
    .expect("paginated working response should succeed");

    assert_eq!(
        response
            .conversations
            .iter()
            .map(|conversation| conversation.prompt_cache_key.as_str())
            .collect::<Vec<_>>(),
        vec!["working-mixed", "working-terminal-only"]
    );
}

#[tokio::test]
async fn prompt_cache_conversations_cache_reuses_recent_result_within_ttl() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let now = Utc::now();
    let occurred_a = format_naive(
        (now - ChronoDuration::minutes(80))
            .with_timezone(&Shanghai)
            .naive_local(),
    );
    let occurred_b = format_naive(
        (now - ChronoDuration::minutes(30))
            .with_timezone(&Shanghai)
            .naive_local(),
    );

    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
        "#,
    )
    .bind("pck-cache-1")
    .bind(&occurred_a)
    .bind(SOURCE_PROXY)
    .bind("success")
    .bind(10)
    .bind(0.01)
    .bind(r#"{"promptCacheKey":"pck-cache"}"#)
    .bind("{}")
    .execute(&state.pool)
    .await
    .expect("insert first cache row");

    materialize_prompt_cache_hourly_rollups(&state.pool).await;

    let Json(first) = fetch_prompt_cache_conversations(
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
    .expect("first fetch should succeed");
    let first_count = first
        .conversations
        .iter()
        .find(|item| item.prompt_cache_key == "pck-cache")
        .map(|item| item.request_count)
        .expect("pck-cache should be present");
    assert_eq!(first_count, 1);

    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
        "#,
    )
    .bind("pck-cache-2")
    .bind(&occurred_b)
    .bind(SOURCE_PROXY)
    .bind("success")
    .bind(15)
    .bind(0.015)
    .bind(r#"{"promptCacheKey":"pck-cache"}"#)
    .bind("{}")
    .execute(&state.pool)
    .await
    .expect("insert second cache row");

    let Json(second) = fetch_prompt_cache_conversations(
        State(state),
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
    .expect("second fetch should use cached result");
    let second_count = second
        .conversations
        .iter()
        .find(|item| item.prompt_cache_key == "pck-cache")
        .map(|item| item.request_count)
        .expect("pck-cache should still be present");
    assert_eq!(second_count, 1);
}

#[tokio::test]
async fn prompt_cache_conversations_cache_invalidation_exposes_new_proxy_capture_immediately() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let now = Utc::now();
    let occurred_a = format_naive(
        (now - ChronoDuration::minutes(80))
            .with_timezone(&Shanghai)
            .naive_local(),
    );
    let occurred_b = format_naive(
        (now - ChronoDuration::minutes(30))
            .with_timezone(&Shanghai)
            .naive_local(),
    );

    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
        "#,
    )
    .bind("pck-cache-live-1")
    .bind(&occurred_a)
    .bind(SOURCE_PROXY)
    .bind("success")
    .bind(10)
    .bind(0.01)
    .bind(r#"{"promptCacheKey":"pck-broadcast-1"}"#)
    .bind("{}")
    .execute(&state.pool)
    .await
    .expect("insert initial prompt cache row");

    materialize_prompt_cache_hourly_rollups(&state.pool).await;

    let Json(first) = fetch_prompt_cache_conversations(
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
    .expect("first fetch should populate prompt cache stats");
    let first_count = first
        .conversations
        .iter()
        .find(|item| item.prompt_cache_key == "pck-broadcast-1")
        .map(|item| item.request_count)
        .expect("pck-broadcast-1 should be present");
    assert_eq!(first_count, 1);

    persist_and_broadcast_proxy_capture(
        state.as_ref(),
        Instant::now(),
        test_proxy_capture_record("pck-cache-live-2", &occurred_b),
    )
    .await
    .expect("persist+broadcast should invalidate prompt cache conversation cache");
    state
        .sqlite_batch_writer
        .flush_buffered_for_test(&state.pool)
        .await;

    let Json(second) = fetch_prompt_cache_conversations(
        State(state),
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
    .expect("second fetch should see the freshly persisted proxy capture");
    let second_count = second
        .conversations
        .iter()
        .find(|item| item.prompt_cache_key == "pck-broadcast-1")
        .map(|item| item.request_count)
        .expect("pck-broadcast-1 should remain present");
    assert_eq!(second_count, 2);
}

#[tokio::test]
async fn prompt_cache_conversations_cache_ignores_proxy_captures_without_prompt_cache_key() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let now = Utc::now();
    let occurred_a = format_naive(
        (now - ChronoDuration::minutes(80))
            .with_timezone(&Shanghai)
            .naive_local(),
    );
    let occurred_b = format_naive(
        (now - ChronoDuration::minutes(30))
            .with_timezone(&Shanghai)
            .naive_local(),
    );

    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
        "#,
    )
    .bind("pck-cache-unrelated-1")
    .bind(&occurred_a)
    .bind(SOURCE_PROXY)
    .bind("success")
    .bind(10)
    .bind(0.01)
    .bind(r#"{"promptCacheKey":"pck-unrelated"}"#)
    .bind("{}")
    .execute(&state.pool)
    .await
    .expect("insert prompt cache seed row");

    sync_hourly_rollups_from_live_tables(&state.pool)
        .await
        .expect("materialize prompt cache rollups before cached read");

    let Json(first) = fetch_prompt_cache_conversations(
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
    .expect("first fetch should populate prompt cache stats");
    let first_count = first
        .conversations
        .iter()
        .find(|item| item.prompt_cache_key == "pck-unrelated")
        .map(|item| item.request_count)
        .expect("pck-unrelated should be present");
    assert_eq!(first_count, 1);

    let mut unrelated_record = test_proxy_capture_record("pck-cache-unrelated-2", &occurred_b);
    unrelated_record.payload =
        Some("{\"endpoint\":\"/v1/responses\",\"statusCode\":200}".to_string());
    persist_and_broadcast_proxy_capture(state.as_ref(), Instant::now(), unrelated_record)
        .await
        .expect("persist+broadcast should keep prompt cache cache warm for unrelated traffic");

    let Json(second) = fetch_prompt_cache_conversations(
        State(state),
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
    .expect("second fetch should still use cached result");
    let second_count = second
        .conversations
        .iter()
        .find(|item| item.prompt_cache_key == "pck-unrelated")
        .map(|item| item.request_count)
        .expect("pck-unrelated should remain present");
    assert_eq!(second_count, 1);
}

#[tokio::test]
async fn prompt_cache_conversations_cache_returns_under_sustained_invalidations() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let now = Utc::now();

    for index in 0..256 {
        let occurred = format_naive(
            (now - ChronoDuration::minutes(120 - index as i64))
                .with_timezone(&Shanghai)
                .naive_local(),
        );
        sqlx::query(
            r#"
            INSERT INTO codex_invocations (
                invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
            "#,
        )
        .bind(format!("pck-cache-sustained-{index}"))
        .bind(&occurred)
        .bind(SOURCE_PROXY)
        .bind("success")
        .bind(10 + index as i64)
        .bind(0.01)
        .bind(format!(
            r#"{{"promptCacheKey":"pck-sustained-{index:03}"}}"#
        ))
        .bind("{}")
        .execute(&state.pool)
        .await
        .expect("insert sustained-invalidations seed row");
    }

    materialize_prompt_cache_hourly_rollups(&state.pool).await;

    let stop = Arc::new(AtomicBool::new(false));
    let invalidator_stop = stop.clone();
    let cache = state.prompt_cache_conversation_cache.clone();
    let invalidator = tokio::spawn(async move {
        while !invalidator_stop.load(Ordering::Relaxed) {
            invalidate_prompt_cache_conversations_cache(&cache).await;
            tokio::task::yield_now().await;
        }
    });

    let result = tokio::time::timeout(
        Duration::from_secs(5),
        fetch_prompt_cache_conversations(
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
        ),
    )
    .await;

    stop.store(true, Ordering::Relaxed);
    invalidator
        .await
        .expect("invalidator task should exit cleanly");

    let Json(response) = result
        .expect("prompt cache fetch should not hang under sustained invalidations")
        .expect("prompt cache fetch should succeed");
    assert!(
        !response.conversations.is_empty(),
        "sustained invalidations should still return a usable snapshot",
    );
}

#[tokio::test]
async fn prompt_cache_conversations_concurrent_requests_same_limit_do_not_stall() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let now = Utc::now();
    let occurred = format_naive(
        (now - ChronoDuration::minutes(20))
            .with_timezone(&Shanghai)
            .naive_local(),
    );

    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
        "#,
    )
    .bind("pck-concurrent-1")
    .bind(&occurred)
    .bind(SOURCE_PROXY)
    .bind("success")
    .bind(18)
    .bind(0.018)
    .bind(r#"{"promptCacheKey":"pck-concurrent"}"#)
    .bind("{}")
    .execute(&state.pool)
    .await
    .expect("insert concurrent cache row");

    materialize_prompt_cache_hourly_rollups(&state.pool).await;

    let mut handles = Vec::new();
    for _ in 0..8 {
        let state_clone = state.clone();
        handles.push(tokio::spawn(async move {
            tokio::time::timeout(
                Duration::from_secs(2),
                fetch_prompt_cache_conversations(
                    State(state_clone),
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
                ),
            )
            .await
        }));
    }

    for handle in handles {
        let response = handle
            .await
            .expect("join should succeed")
            .expect("concurrent request should not timeout")
            .expect("concurrent request should succeed");
        let Json(payload) = response;
        assert!(
            payload
                .conversations
                .iter()
                .any(|item| item.prompt_cache_key == "pck-concurrent"),
            "expected pck-concurrent to be present in each response",
        );
    }
}

#[tokio::test]
async fn prompt_cache_conversation_flight_guard_cleans_in_flight_on_drop() {
    let cache = Arc::new(Mutex::new(PromptCacheConversationsCacheState::default()));
    let (signal, _receiver) = watch::channel(false);
    {
        let mut state = cache.lock().await;
        state.in_flight.insert(
            PromptCacheConversationSelection::Count(20),
            PromptCacheConversationInFlight {
                signal,
                generation: 0,
            },
        );
    }

    {
        let _guard = PromptCacheConversationFlightGuard::new(
            cache.clone(),
            PromptCacheConversationSelection::Count(20),
            0,
        );
    }

    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let has_entry = {
                let state = cache.lock().await;
                state
                    .in_flight
                    .contains_key(&PromptCacheConversationSelection::Count(20))
            };
            if !has_entry {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("drop cleanup should remove in-flight marker");
}
