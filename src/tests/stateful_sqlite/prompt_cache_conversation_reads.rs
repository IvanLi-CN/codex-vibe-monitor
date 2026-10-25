use super::*;
#[test]
fn normalize_prompt_cache_conversation_limit_accepts_whitelist_values_only() {
    assert_eq!(normalize_prompt_cache_conversation_limit(None), 50);
    assert_eq!(normalize_prompt_cache_conversation_limit(Some(20)), 20);
    assert_eq!(normalize_prompt_cache_conversation_limit(Some(50)), 50);
    assert_eq!(normalize_prompt_cache_conversation_limit(Some(100)), 100);
    assert_eq!(normalize_prompt_cache_conversation_limit(Some(10)), 50);
    assert_eq!(normalize_prompt_cache_conversation_limit(Some(200)), 50);
}

#[test]
fn normalize_prompt_cache_conversation_activity_hours_accepts_whitelist_values_only() {
    assert_eq!(
        normalize_prompt_cache_conversation_activity_hours(None),
        None
    );
    assert_eq!(
        normalize_prompt_cache_conversation_activity_hours(Some(1)),
        Some(1)
    );
    assert_eq!(
        normalize_prompt_cache_conversation_activity_hours(Some(3)),
        Some(3)
    );
    assert_eq!(
        normalize_prompt_cache_conversation_activity_hours(Some(6)),
        Some(6)
    );
    assert_eq!(
        normalize_prompt_cache_conversation_activity_hours(Some(12)),
        Some(12)
    );
    assert_eq!(
        normalize_prompt_cache_conversation_activity_hours(Some(24)),
        Some(24)
    );
    assert_eq!(
        normalize_prompt_cache_conversation_activity_hours(Some(2)),
        None
    );
    assert_eq!(
        normalize_prompt_cache_conversation_activity_hours(Some(48)),
        None
    );
}

#[test]
fn normalize_prompt_cache_conversation_activity_minutes_accepts_precise_five_minutes_only() {
    assert_eq!(
        normalize_prompt_cache_conversation_activity_minutes(None),
        None
    );
    assert_eq!(
        normalize_prompt_cache_conversation_activity_minutes(Some(5)),
        Some(5)
    );
    assert_eq!(
        normalize_prompt_cache_conversation_activity_minutes(Some(1)),
        None
    );
    assert_eq!(
        normalize_prompt_cache_conversation_activity_minutes(Some(10)),
        None
    );
}

#[test]
fn resolve_prompt_cache_conversation_selection_rejects_mutually_exclusive_params() {
    let err = resolve_prompt_cache_conversation_selection(PromptCacheConversationsQuery {
        limit: Some(20),
        activity_hours: Some(3),
        activity_minutes: None,
        page_size: None,
        cursor: None,
        snapshot_at: None,
        detail: None,
        recent_invocation_limit: None,

        blocked_binding_upstream_account_id: None,

        blocked_binding_constraint_source: None,
    })
    .expect_err("selection should reject mutually exclusive params");

    match err {
        ApiError::BadRequest(inner) => {
            let message = inner.to_string();
            assert!(message.contains("mutually exclusive"));
        }
        other => panic!("expected bad request, got {other:?}"),
    }
}

#[test]
fn resolve_prompt_cache_conversation_selection_rejects_activity_hours_and_minutes_combo() {
    let err = resolve_prompt_cache_conversation_selection(PromptCacheConversationsQuery {
        limit: None,
        activity_hours: Some(3),
        activity_minutes: Some(5),
        page_size: None,
        cursor: None,
        snapshot_at: None,
        detail: None,
        recent_invocation_limit: None,

        blocked_binding_upstream_account_id: None,

        blocked_binding_constraint_source: None,
    })
    .expect_err("selection should reject mixed hour and minute windows");

    match err {
        ApiError::BadRequest(inner) => {
            let message = inner.to_string();
            assert!(message.contains("mutually exclusive"));
        }
        other => panic!("expected bad request, got {other:?}"),
    }
}

#[tokio::test]
async fn prompt_cache_conversations_groups_recent_keys_and_uses_history_totals() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let now = Utc::now();

    async fn insert_row(
        pool: &Pool<Sqlite>,
        invoke_id: &str,
        occurred_at: DateTime<Utc>,
        key: Option<&str>,
        status: &str,
        total_tokens: i64,
        cost: f64,
    ) {
        let payload = match key {
            Some(key) => json!({ "promptCacheKey": key }).to_string(),
            None => "{}".to_string(),
        };
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
        .bind(total_tokens)
        .bind(cost)
        .bind(payload)
        .bind("{}")
        .execute(pool)
        .await
        .expect("insert invocation row");
    }

    // key-a: active in 24h + older history
    insert_row(
        &state.pool,
        "pck-a-history",
        now - ChronoDuration::hours(48),
        Some("pck-a"),
        "success",
        100,
        1.0,
    )
    .await;
    insert_row(
        &state.pool,
        "pck-a-24h-1",
        now - ChronoDuration::hours(2),
        Some("pck-a"),
        "success",
        20,
        0.2,
    )
    .await;
    insert_row(
        &state.pool,
        "pck-a-24h-2",
        now - ChronoDuration::hours(1),
        Some("pck-a"),
        "failed",
        30,
        0.3,
    )
    .await;

    // key-b: newer created_at so it should rank before key-a.
    insert_row(
        &state.pool,
        "pck-b-24h-1",
        now - ChronoDuration::hours(10),
        Some("pck-b"),
        "success",
        10,
        0.1,
    )
    .await;

    // key-c: not active in last 24h; should be excluded.
    insert_row(
        &state.pool,
        "pck-c-history",
        now - ChronoDuration::hours(72),
        Some("pck-c"),
        "success",
        8,
        0.08,
    )
    .await;

    // missing key in last 24h; should be ignored.
    insert_row(
        &state.pool,
        "pck-missing-24h",
        now - ChronoDuration::minutes(40),
        None,
        "success",
        999,
        9.99,
    )
    .await;

    sync_hourly_rollups_from_live_tables(&state.pool)
        .await
        .expect("materialize prompt cache rollups before count-mode prompt-cache read");
    complete_prompt_cache_conversation_materialization_for_test(&state.pool).await;

    let Json(response) = fetch_prompt_cache_conversations(
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
    .expect("prompt cache conversation stats should succeed");

    assert_eq!(
        response.selection_mode,
        PromptCacheConversationSelectionMode::Count
    );
    assert_eq!(response.selected_limit, Some(20));
    assert_eq!(response.selected_activity_hours, None);
    assert_eq!(
        response.implicit_filter.kind,
        Some(PromptCacheConversationImplicitFilterKind::InactiveOutside24h)
    );
    assert_eq!(response.implicit_filter.filtered_count, 1);
    assert_eq!(response.conversations.len(), 2);
    assert_eq!(response.conversations[0].prompt_cache_key, "pck-b");
    assert_eq!(response.conversations[1].prompt_cache_key, "pck-a");

    let key_a = response
        .conversations
        .iter()
        .find(|item| item.prompt_cache_key == "pck-a")
        .expect("pck-a should be included");
    assert_eq!(key_a.request_count, 3);
    assert_eq!(key_a.total_tokens, 150);
    assert!((key_a.total_cost - 1.5).abs() < 1e-9);
    assert_eq!(key_a.last24h_requests.len(), 2);
    assert_eq!(key_a.last24h_requests[0].request_tokens, 20);
    assert_eq!(key_a.last24h_requests[0].cumulative_tokens, 20);
    assert!(key_a.last24h_requests[0].is_success);
    assert_eq!(key_a.last24h_requests[0].outcome, "success");
    assert_eq!(key_a.last24h_requests[1].request_tokens, 30);
    assert_eq!(key_a.last24h_requests[1].cumulative_tokens, 50);
    assert!(!key_a.last24h_requests[1].is_success);
    assert_eq!(key_a.last24h_requests[1].outcome, "failure");
}

#[tokio::test]
async fn prompt_cache_last24h_requests_keep_null_status_rows_neutral() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let now = Utc::now();
    let occurred_at = format_naive(
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
    .bind("pck-neutral-success")
    .bind(occurred_at.clone())
    .bind(SOURCE_PROXY)
    .bind("success")
    .bind(20_i64)
    .bind(0.2_f64)
    .bind(json!({ "promptCacheKey": "pck-neutral" }).to_string())
    .bind("{}")
    .execute(&state.pool)
    .await
    .expect("insert success prompt cache row");

    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response,
            failure_class, error_message
        )
        VALUES (?1, ?2, ?3, NULL, ?4, ?5, ?6, ?7, ?8, ?9)
        "#,
    )
    .bind("pck-neutral-null-status")
    .bind(format_naive(
        (now - ChronoDuration::minutes(10))
            .with_timezone(&Shanghai)
            .naive_local(),
    ))
    .bind(SOURCE_PROXY)
    .bind(15_i64)
    .bind(0.15_f64)
    .bind(json!({ "promptCacheKey": "pck-neutral" }).to_string())
    .bind("{}")
    .bind("none")
    .bind("")
    .execute(&state.pool)
    .await
    .expect("insert null-status prompt cache row");

    sync_hourly_rollups_from_live_tables(&state.pool)
        .await
        .expect("materialize prompt cache rollups before neutral status read");
    complete_prompt_cache_conversation_materialization_for_test(&state.pool).await;

    let Json(response) = fetch_prompt_cache_conversations(
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
    .expect("prompt cache neutral conversation stats should succeed");

    let conversation = response
        .conversations
        .iter()
        .find(|item| item.prompt_cache_key == "pck-neutral")
        .expect("neutral prompt cache conversation should exist");
    assert_eq!(conversation.last24h_requests.len(), 2);
    assert_eq!(conversation.last24h_requests[0].outcome, "success");
    assert_eq!(conversation.last24h_requests[1].status, "unknown");
    assert!(!conversation.last24h_requests[1].is_success);
    assert_eq!(conversation.last24h_requests[1].outcome, "neutral");
}

#[tokio::test]
async fn prompt_cache_last24h_requests_treat_running_rows_with_failure_class_as_failures() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let now = Utc::now();

    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response,
            failure_class, error_message
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
        "#,
    )
    .bind("pck-running-failure")
    .bind(format_naive(
        (now - ChronoDuration::minutes(5))
            .with_timezone(&Shanghai)
            .naive_local(),
    ))
    .bind(SOURCE_PROXY)
    .bind("running")
    .bind(11_i64)
    .bind(0.11_f64)
    .bind(json!({ "promptCacheKey": "pck-running-failure" }).to_string())
    .bind("{}")
    .bind("service_failure")
    .bind("upstream stream error")
    .execute(&state.pool)
    .await
    .expect("insert running prompt cache failure row");

    sync_hourly_rollups_from_live_tables(&state.pool)
        .await
        .expect("materialize prompt cache rollups before running failure read");
    complete_prompt_cache_conversation_materialization_for_test(&state.pool).await;

    let Json(response) = fetch_prompt_cache_conversations(
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
    .expect("prompt cache running failure conversation stats should succeed");

    let conversation = response
        .conversations
        .iter()
        .find(|item| item.prompt_cache_key == "pck-running-failure")
        .expect("running failure prompt cache conversation should exist");
    assert_eq!(conversation.last24h_requests.len(), 1);
    assert_eq!(conversation.last24h_requests[0].status, "running");
    assert!(!conversation.last24h_requests[0].is_success);
    assert_eq!(conversation.last24h_requests[0].outcome, "failure");
}

#[tokio::test]
async fn prompt_cache_last24h_requests_treat_running_rows_with_error_text_as_failures() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let now = Utc::now();

    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response,
            failure_class, error_message
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
        "#,
    )
    .bind("pck-running-error-text")
    .bind(format_naive(
        (now - ChronoDuration::minutes(4))
            .with_timezone(&Shanghai)
            .naive_local(),
    ))
    .bind(SOURCE_PROXY)
    .bind("running")
    .bind(9_i64)
    .bind(0.09_f64)
    .bind(json!({ "promptCacheKey": "pck-running-error-text" }).to_string())
    .bind("{}")
    .bind("none")
    .bind("downstream closed while streaming upstream response")
    .execute(&state.pool)
    .await
    .expect("insert running prompt cache row with error text");

    sync_hourly_rollups_from_live_tables(&state.pool)
        .await
        .expect("materialize prompt cache rollups before running error-text read");
    complete_prompt_cache_conversation_materialization_for_test(&state.pool).await;

    let Json(response) = fetch_prompt_cache_conversations(
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
    .expect("prompt cache running error-text conversation stats should succeed");

    let conversation = response
        .conversations
        .iter()
        .find(|item| item.prompt_cache_key == "pck-running-error-text")
        .expect("running error-text prompt cache conversation should exist");
    assert_eq!(conversation.last24h_requests.len(), 1);
    assert_eq!(conversation.last24h_requests[0].status, "running");
    assert!(!conversation.last24h_requests[0].is_success);
    assert_eq!(conversation.last24h_requests[0].outcome, "failure");
}

#[tokio::test]
async fn prompt_cache_last24h_requests_treat_pending_rows_with_failure_kind_as_failures() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let now = Utc::now();

    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response,
            failure_class, error_message, failure_kind
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
        "#,
    )
    .bind("pck-pending-failure-kind")
    .bind(format_naive(
        (now - ChronoDuration::minutes(3))
            .with_timezone(&Shanghai)
            .naive_local(),
    ))
    .bind(SOURCE_PROXY)
    .bind("pending")
    .bind(7_i64)
    .bind(0.07_f64)
    .bind(
        json!({
            "promptCacheKey": "pck-pending-failure-kind",
            "downstreamErrorMessage": "pool upstream responded with 502",
        })
        .to_string(),
    )
    .bind("{}")
    .bind("none")
    .bind::<Option<&str>>(None)
    .bind(PROXY_FAILURE_UPSTREAM_RESPONSE_FAILED)
    .execute(&state.pool)
    .await
    .expect("insert pending prompt cache row with failure kind");

    sync_hourly_rollups_from_live_tables(&state.pool)
        .await
        .expect("materialize prompt cache rollups before pending failure-kind read");
    complete_prompt_cache_conversation_materialization_for_test(&state.pool).await;

    let Json(response) = fetch_prompt_cache_conversations(
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
    .expect("prompt cache pending failure-kind conversation stats should succeed");

    let conversation = response
        .conversations
        .iter()
        .find(|item| item.prompt_cache_key == "pck-pending-failure-kind")
        .expect("pending failure-kind prompt cache conversation should exist");
    assert_eq!(conversation.last24h_requests.len(), 1);
    assert_eq!(conversation.last24h_requests[0].status, "pending");
    assert!(!conversation.last24h_requests[0].is_success);
    assert_eq!(conversation.last24h_requests[0].outcome, "failure");
}

#[tokio::test]
async fn prompt_cache_last24h_requests_keep_status_only_http_failures_marked_as_failures() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let now = Utc::now();

    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response,
            failure_class, error_message
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
        "#,
    )
    .bind("pck-http-status-only-failure")
    .bind(format_naive(
        (now - ChronoDuration::minutes(2))
            .with_timezone(&Shanghai)
            .naive_local(),
    ))
    .bind(SOURCE_PROXY)
    .bind("http_500")
    .bind(5_i64)
    .bind(0.05_f64)
    .bind(json!({ "promptCacheKey": "pck-http-status-only-failure" }).to_string())
    .bind("{}")
    .bind("none")
    .bind("")
    .execute(&state.pool)
    .await
    .expect("insert http-status-only prompt cache failure row");

    sync_hourly_rollups_from_live_tables(&state.pool)
        .await
        .expect("materialize prompt cache rollups before http-status-only read");
    complete_prompt_cache_conversation_materialization_for_test(&state.pool).await;

    let Json(response) = fetch_prompt_cache_conversations(
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
    .expect("prompt cache http-status-only failure conversation stats should succeed");

    let conversation = response
        .conversations
        .iter()
        .find(|item| item.prompt_cache_key == "pck-http-status-only-failure")
        .expect("http-status-only prompt cache conversation should exist");
    assert_eq!(conversation.last24h_requests.len(), 1);
    assert_eq!(conversation.last24h_requests[0].status, "http_500");
    assert!(!conversation.last24h_requests[0].is_success);
    assert_eq!(conversation.last24h_requests[0].outcome, "failure");
}
