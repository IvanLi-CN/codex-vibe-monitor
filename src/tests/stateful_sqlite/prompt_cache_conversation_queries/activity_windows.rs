use super::test_support::{
    fetch_prompt_cache_conversations, materialize_prompt_cache_hourly_rollups,
};
use super::*;
use serde_json::json;

async fn insert_count_mode_row(
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
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
        "#,
    )
    .bind(invoke_id)
    .bind(format_naive(
        occurred_at.with_timezone(&Shanghai).naive_local(),
    ))
    .bind(SOURCE_PROXY)
    .bind("success")
    .bind(10)
    .bind(0.01)
    .bind(json!({ "promptCacheKey": key }).to_string())
    .bind("{}")
    .execute(pool)
    .await
    .expect("insert invocation row");
}

#[tokio::test]
async fn prompt_cache_conversations_count_mode_reports_inactive_recent_history_filter() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let now = Utc::now();

    for index in 0..19 {
        insert_count_mode_row(
            &state.pool,
            &format!("count-active-{index}"),
            now - ChronoDuration::hours(23) + ChronoDuration::minutes(index as i64),
            &format!("count-active-{index}"),
        )
        .await;
    }
    insert_count_mode_row(
        &state.pool,
        "count-inactive",
        now - ChronoDuration::hours(72),
        "count-inactive",
    )
    .await;

    materialize_prompt_cache_hourly_rollups(&state.pool).await;

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
    .expect("prompt cache conversations should succeed");

    assert_eq!(
        response.implicit_filter.kind,
        Some(PromptCacheConversationImplicitFilterKind::InactiveOutside24h)
    );
    assert_eq!(response.implicit_filter.filtered_count, 1);
    assert_eq!(response.conversations.len(), 19);
}

#[tokio::test]
async fn prompt_cache_conversations_count_mode_reports_all_skipped_newer_inactive_rows() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let now = Utc::now();

    for index in 0..25 {
        insert_count_mode_row(
            &state.pool,
            &format!("count-inactive-{index}"),
            now - ChronoDuration::hours(25) + ChronoDuration::minutes(index as i64),
            &format!("count-inactive-{index}"),
        )
        .await;
    }

    for index in 0..20 {
        insert_count_mode_row(
            &state.pool,
            &format!("count-active-{index}-history"),
            now - ChronoDuration::days(4) + ChronoDuration::minutes(index as i64),
            &format!("count-active-{index}"),
        )
        .await;
        insert_count_mode_row(
            &state.pool,
            &format!("count-active-{index}-recent"),
            now - ChronoDuration::hours(12) + ChronoDuration::minutes(index as i64),
            &format!("count-active-{index}"),
        )
        .await;
    }

    sync_hourly_rollups_from_live_tables(&state.pool)
        .await
        .expect("materialize prompt cache rollups before legacy activity-minutes read");

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
    .expect("prompt cache conversations should succeed");

    assert_eq!(response.conversations.len(), 20);
    assert_eq!(
        response.implicit_filter.kind,
        Some(PromptCacheConversationImplicitFilterKind::InactiveOutside24h)
    );
    assert_eq!(response.implicit_filter.filtered_count, 25);
}

#[tokio::test]
async fn prompt_cache_conversations_count_mode_clamps_sparse_inactive_hidden_rows_to_top_n_window()
{
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let now = Utc::now();

    for index in 0..25 {
        insert_count_mode_row(
            &state.pool,
            &format!("sparse-inactive-{index}"),
            now - ChronoDuration::hours(25) + ChronoDuration::minutes(index as i64),
            &format!("sparse-inactive-{index}"),
        )
        .await;
    }

    insert_count_mode_row(
        &state.pool,
        "sparse-active-history",
        now - ChronoDuration::days(4),
        "sparse-active",
    )
    .await;
    insert_count_mode_row(
        &state.pool,
        "sparse-active-recent",
        now - ChronoDuration::hours(6),
        "sparse-active",
    )
    .await;

    materialize_prompt_cache_hourly_rollups(&state.pool).await;

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
    .expect("prompt cache conversations should succeed");

    assert_eq!(response.conversations.len(), 1);
    assert_eq!(
        response.implicit_filter.kind,
        Some(PromptCacheConversationImplicitFilterKind::InactiveOutside24h)
    );
    assert_eq!(response.implicit_filter.filtered_count, 20);
}

#[tokio::test]
async fn prompt_cache_conversations_activity_window_caps_results_to_fifty() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let now = Utc::now();

    for index in 0..55 {
        sqlx::query(
            r#"
            INSERT INTO codex_invocations (
                invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
            "#,
        )
        .bind(format!("window-{index}"))
        .bind(format_naive(
            (now - ChronoDuration::minutes(index as i64))
                .with_timezone(&Shanghai)
                .naive_local(),
        ))
        .bind(SOURCE_PROXY)
        .bind("success")
        .bind(10)
        .bind(0.01)
        .bind(json!({ "promptCacheKey": format!("window-key-{index}") }).to_string())
        .bind("{}")
        .execute(&state.pool)
        .await
        .expect("insert invocation row");
    }

    materialize_prompt_cache_hourly_rollups(&state.pool).await;

    let Json(response) = fetch_prompt_cache_conversations(
        State(state),
        Query(PromptCacheConversationsQuery {
            limit: None,
            activity_hours: Some(3),
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
    .expect("activity-window prompt cache conversations should succeed");

    assert_eq!(
        response.selection_mode,
        PromptCacheConversationSelectionMode::ActivityWindow
    );
    assert_eq!(response.selected_limit, None);
    assert_eq!(response.selected_activity_hours, Some(3));
    assert_eq!(response.conversations.len(), 50);
    assert_eq!(
        response.implicit_filter.kind,
        Some(PromptCacheConversationImplicitFilterKind::CappedTo50)
    );
    assert_eq!(response.implicit_filter.filtered_count, 5);
}

#[tokio::test]
async fn prompt_cache_conversations_activity_minutes_legacy_path_still_caps_results_to_fifty() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let now = Utc::now();

    for index in 0..55 {
        sqlx::query(
            r#"
            INSERT INTO codex_invocations (
                invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
            "#,
        )
        .bind(format!("working-legacy-{index}"))
        .bind(format_naive(
            (now - ChronoDuration::seconds(index as i64 * 2))
                .with_timezone(&Shanghai)
                .naive_local(),
        ))
        .bind(SOURCE_PROXY)
        .bind("success")
        .bind(10)
        .bind(0.01)
        .bind(json!({ "promptCacheKey": format!("working-legacy-key-{index}") }).to_string())
        .bind("{}")
        .execute(&state.pool)
        .await
        .expect("insert working legacy row");
    }

    materialize_prompt_cache_hourly_rollups(&state.pool).await;

    let Json(response) = fetch_prompt_cache_conversations(
        State(state),
        Query(PromptCacheConversationsQuery {
            limit: None,
            activity_hours: None,
            activity_minutes: Some(5),
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
    .expect("legacy activity-minutes prompt cache conversations should succeed");

    assert_eq!(response.conversations.len(), 50);
    assert_eq!(
        response.implicit_filter.kind,
        Some(PromptCacheConversationImplicitFilterKind::CappedTo50)
    );
    assert_eq!(response.implicit_filter.filtered_count, 5);
    assert_eq!(response.total_matched, None);
    assert!(!response.has_more);
    assert_eq!(response.snapshot_at, None);
}

#[tokio::test]
async fn prompt_cache_conversations_activity_minutes_include_running_only_rows_and_report_selected_minutes()
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
        total_tokens: i64,
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
        .bind(total_tokens)
        .bind(0.01_f64)
        .bind(json!({ "promptCacheKey": key, "routeMode": "pool" }).to_string())
        .bind("{}")
        .execute(pool)
        .await
        .expect("insert prompt cache invocation row");
    }

    insert_row(
        &state.pool,
        "pck-terminal-early",
        now - ChronoDuration::minutes(4),
        "pck-terminal-early",
        "success",
        100,
    )
    .await;
    insert_row(
        &state.pool,
        "pck-running-old-terminal",
        now - ChronoDuration::minutes(12),
        "pck-running",
        "success",
        120,
    )
    .await;
    insert_row(
        &state.pool,
        "pck-running-live",
        now - ChronoDuration::minutes(1),
        "pck-running",
        "running",
        140,
    )
    .await;
    insert_row(
        &state.pool,
        "pck-terminal-late",
        now - ChronoDuration::minutes(2),
        "pck-terminal-late",
        "http_502",
        160,
    )
    .await;

    sync_hourly_rollups_from_live_tables(&state.pool)
        .await
        .expect("materialize prompt cache rollups before working-conversations read");

    let Json(response) = fetch_prompt_cache_conversations(
        State(state),
        Query(PromptCacheConversationsQuery {
            limit: None,
            activity_hours: None,
            activity_minutes: Some(5),
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
    .expect("5-minute prompt cache conversations should succeed");

    assert_eq!(
        response.selection_mode,
        PromptCacheConversationSelectionMode::ActivityWindow
    );
    assert_eq!(response.selected_limit, None);
    assert_eq!(response.selected_activity_hours, None);
    assert_eq!(response.selected_activity_minutes, Some(5));
    assert_eq!(
        response
            .conversations
            .iter()
            .map(|item| item.prompt_cache_key.as_str())
            .collect::<Vec<_>>(),
        vec!["pck-running", "pck-terminal-late", "pck-terminal-early"]
    );

    let running = response
        .conversations
        .iter()
        .find(|item| item.prompt_cache_key == "pck-running")
        .expect("pck-running should remain visible");
    assert_eq!(
        running.request_count, 2,
        "working card totals must include the full prompt-cache lifecycle"
    );
    assert_eq!(running.total_tokens, 260);
    assert!(
        (running.total_cost - 0.02).abs() < f64::EPSILON,
        "working card cost should include stale lifecycle history"
    );
    assert_eq!(running.recent_invocations[0].status, "running");
    assert_eq!(running.recent_invocations[1].status, "success");
}

#[tokio::test]
async fn prompt_cache_conversations_activity_minutes_exclude_stale_terminal_rows_from_totals() {
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
        .bind(42_i64)
        .bind(0.01_f64)
        .bind(
            json!({
                "promptCacheKey": key,
                "routeMode": "pool",
                "model": "gpt-5.4",
            })
            .to_string(),
        )
        .bind("{}")
        .execute(pool)
        .await
        .expect("insert working row");
    }

    insert_row(
        &state.pool,
        "working-in-flight",
        now - ChronoDuration::minutes(20),
        "working-in-flight",
        "pending",
    )
    .await;
    insert_row(
        &state.pool,
        "working-recent-terminal",
        now - ChronoDuration::seconds(45),
        "working-recent-terminal",
        "success",
    )
    .await;
    insert_row(
        &state.pool,
        "working-stale-terminal",
        now - ChronoDuration::minutes(11),
        "working-stale-terminal",
        "success",
    )
    .await;

    let Json(non_paginated) = fetch_prompt_cache_conversations(
        State(state.clone()),
        Query(PromptCacheConversationsQuery {
            limit: None,
            activity_hours: None,
            activity_minutes: Some(5),
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
    .expect("non-paginated working conversations should succeed");

    assert_eq!(non_paginated.conversations.len(), 2);
    assert_eq!(
        non_paginated
            .conversations
            .iter()
            .map(|conversation| conversation.prompt_cache_key.as_str())
            .collect::<Vec<_>>(),
        vec!["working-recent-terminal", "working-in-flight"]
    );

    let Json(first_page) = fetch_prompt_cache_conversations(
        State(state),
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
    .expect("paginated working conversations should succeed");

    assert_eq!(
        first_page.total_matched,
        Some(2),
        "snapshot totals must exclude stale terminal rows",
    );
    assert_eq!(first_page.conversations.len(), 2);
    assert!(!first_page.has_more);
    assert!(first_page.next_cursor.is_none());
}

#[tokio::test]
async fn prompt_cache_conversations_chart_window_caps_history_to_recent_24_hours() {
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
        total_tokens: i64,
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
        .bind("success")
        .bind(total_tokens)
        .bind(0.01)
        .bind(json!({ "promptCacheKey": key }).to_string())
        .bind("{}")
        .execute(pool)
        .await
        .expect("insert invocation row");
    }

    insert_row(
        &state.pool,
        "chart-cap-history",
        now - ChronoDuration::hours(50),
        "chart-cap",
        90,
    )
    .await;
    insert_row(
        &state.pool,
        "chart-cap-recent-a",
        now - ChronoDuration::hours(2),
        "chart-cap",
        30,
    )
    .await;
    insert_row(
        &state.pool,
        "chart-cap-recent-b",
        now - ChronoDuration::minutes(20),
        "chart-cap",
        45,
    )
    .await;

    materialize_prompt_cache_hourly_rollups(&state.pool).await;

    let Json(response) = fetch_prompt_cache_conversations(
        State(state),
        Query(PromptCacheConversationsQuery {
            limit: None,
            activity_hours: Some(1),
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
    .expect("activity-window prompt cache conversations should succeed");

    assert_eq!(response.conversations.len(), 1);
    let conversation = &response.conversations[0];
    assert_eq!(conversation.last24h_requests.len(), 2);
    assert_eq!(conversation.last24h_requests[0].request_tokens, 30);
    assert_eq!(conversation.last24h_requests[0].cumulative_tokens, 30);
    assert_eq!(conversation.last24h_requests[1].request_tokens, 45);
    assert_eq!(conversation.last24h_requests[1].cumulative_tokens, 75);
}

#[tokio::test]
async fn prompt_cache_conversation_activity_keeps_http_200_errors_out_of_success_points() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let now = Utc::now();

    async fn insert_row(
        pool: &Pool<Sqlite>,
        invoke_id: &str,
        occurred_at: DateTime<Utc>,
        status: &str,
        error_message: Option<&str>,
        failure_kind: Option<&str>,
        failure_class: Option<&str>,
        key: &str,
        total_tokens: i64,
    ) {
        sqlx::query(
            r#"
            INSERT INTO codex_invocations (
                invoke_id,
                occurred_at,
                source,
                status,
                error_message,
                failure_kind,
                failure_class,
                total_tokens,
                cost,
                payload,
                raw_response
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
            "#,
        )
        .bind(invoke_id)
        .bind(format_naive(
            occurred_at.with_timezone(&Shanghai).naive_local(),
        ))
        .bind(SOURCE_PROXY)
        .bind(status)
        .bind(error_message)
        .bind(failure_kind)
        .bind(failure_class)
        .bind(total_tokens)
        .bind(0.01)
        .bind(json!({ "promptCacheKey": key }).to_string())
        .bind("{}")
        .execute(pool)
        .await
        .expect("insert invocation row");
    }

    insert_row(
        &state.pool,
        "http-200-success-like",
        now - ChronoDuration::minutes(10),
        "http_200",
        None,
        None,
        None,
        "http-200-activity",
        10,
    )
    .await;
    insert_row(
        &state.pool,
        "http-200-failure-like",
        now - ChronoDuration::minutes(5),
        "http_200",
        Some("upstream parse failed"),
        None,
        None,
        "http-200-activity",
        12,
    )
    .await;
    insert_row(
        &state.pool,
        "http-200-structured-failure",
        now - ChronoDuration::minutes(1),
        "http_200",
        None,
        Some("upstream_response_failed"),
        Some("service_failure"),
        "http-200-activity",
        14,
    )
    .await;

    materialize_prompt_cache_hourly_rollups(&state.pool).await;

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
    .expect("prompt cache conversations should succeed");

    let conversation = response
        .conversations
        .iter()
        .find(|item| item.prompt_cache_key == "http-200-activity")
        .expect("http-200-activity should be included");

    assert_eq!(conversation.last24h_requests.len(), 3);
    assert!(conversation.last24h_requests[0].is_success);
    assert!(!conversation.last24h_requests[1].is_success);
    assert!(!conversation.last24h_requests[2].is_success);
}

#[tokio::test]
async fn prompt_cache_conversation_timestamps_serialize_as_utc_iso() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let occurred_at = Utc::now() - ChronoDuration::minutes(15);

    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
        "#,
    )
    .bind("prompt-cache-utc-iso")
    .bind(format_naive(
        occurred_at.with_timezone(&Shanghai).naive_local(),
    ))
    .bind(SOURCE_PROXY)
    .bind("success")
    .bind(42)
    .bind(0.42)
    .bind(json!({ "promptCacheKey": "prompt-cache-utc-iso" }).to_string())
    .bind("{}")
    .execute(&state.pool)
    .await
    .expect("insert prompt cache invocation row");

    materialize_prompt_cache_hourly_rollups(&state.pool).await;

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
    .expect("prompt cache conversations should succeed");

    let payload = serde_json::to_value(&response).expect("serialize prompt cache response");
    let conversation = payload["conversations"][0]
        .as_object()
        .expect("conversation should be serialized as object");
    let created_at = conversation["createdAt"]
        .as_str()
        .expect("createdAt should serialize as string");
    let last_activity_at = conversation["lastActivityAt"]
        .as_str()
        .expect("lastActivityAt should serialize as string");

    assert_eq!(
        DateTime::parse_from_rfc3339(created_at)
            .unwrap()
            .offset()
            .utc_minus_local(),
        0
    );
    assert_eq!(
        DateTime::parse_from_rfc3339(last_activity_at)
            .unwrap()
            .offset()
            .utc_minus_local(),
        0
    );
    assert!(created_at.ends_with('Z'));
    assert!(last_activity_at.ends_with('Z'));
}
