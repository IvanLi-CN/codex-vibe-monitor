use super::test_support::fetch_prompt_cache_conversations;
use super::*;
use serde_json::json;

#[tokio::test]
async fn prompt_cache_conversations_activity_minutes_paginated_compact_can_scroll_past_fifty_without_duplicates()
 {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let now = Utc::now();

    for index in 0..55 {
        let occurred_at = now - ChronoDuration::seconds(index as i64 * 2);
        sqlx::query(
            r#"
            INSERT INTO codex_invocations (
                invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
            "#,
        )
        .bind(format!("working-page-{index}"))
        .bind(format_naive(
            occurred_at.with_timezone(&Shanghai).naive_local(),
        ))
        .bind(SOURCE_PROXY)
        .bind(if index % 7 == 0 { "running" } else { "success" })
        .bind(10 + index as i64)
        .bind(0.01_f64)
        .bind(
            json!({
                "promptCacheKey": format!("working-page-key-{index:02}"),
                "routeMode": "pool",
                "model": "gpt-5.4",
            })
            .to_string(),
        )
        .bind("{}")
        .execute(&state.pool)
        .await
        .expect("insert paginated working row");
    }

    let Json(first_page) = fetch_prompt_cache_conversations(
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
    .expect("first paginated working page should succeed");

    let total_matched = first_page.total_matched.expect("first page total_matched");
    assert_eq!(first_page.conversations.len(), 20);
    assert!(total_matched > 50);
    assert!(first_page.has_more);
    assert_eq!(first_page.implicit_filter.kind, None);
    assert_eq!(first_page.implicit_filter.filtered_count, 0);
    let first_snapshot = first_page
        .snapshot_at
        .clone()
        .expect("first page snapshot_at");
    let first_next_cursor = first_page
        .next_cursor
        .clone()
        .expect("first page next_cursor");
    assert!(
        first_page
            .conversations
            .iter()
            .all(|conversation| conversation.upstream_accounts.is_empty())
    );
    assert!(
        first_page
            .conversations
            .iter()
            .all(|conversation| conversation.last24h_requests.is_empty())
    );
    assert!(
        first_page
            .conversations
            .iter()
            .all(|conversation| conversation.recent_invocations.len() <= 2)
    );
    assert!(
        first_page
            .conversations
            .iter()
            .all(|conversation| conversation.cursor.is_some())
    );

    let Json(second_page) = fetch_prompt_cache_conversations(
        State(state.clone()),
        Query(PromptCacheConversationsQuery {
            limit: None,
            activity_hours: None,
            activity_minutes: Some(5),
            page_size: Some(20),
            cursor: Some(first_next_cursor),
            snapshot_at: Some(first_snapshot.clone()),
            detail: Some("compact".to_string()),
            recent_invocation_limit: None,

            blocked_binding_upstream_account_id: None,

            blocked_binding_constraint_source: None,
        }),
    )
    .await
    .expect("second paginated working page should succeed");

    assert_eq!(second_page.conversations.len(), 20);
    assert_eq!(second_page.total_matched, Some(total_matched));
    assert!(second_page.has_more);
    assert_eq!(
        second_page.snapshot_at.as_deref(),
        Some(first_snapshot.as_str())
    );

    let last_visible_row_cursor = first_page
        .conversations
        .last()
        .and_then(|conversation| conversation.cursor.clone())
        .expect("last visible row cursor");
    let Json(second_page_from_row_cursor) = fetch_prompt_cache_conversations(
        State(state.clone()),
        Query(PromptCacheConversationsQuery {
            limit: None,
            activity_hours: None,
            activity_minutes: Some(5),
            page_size: Some(20),
            cursor: Some(last_visible_row_cursor),
            snapshot_at: Some(first_snapshot.clone()),
            detail: Some("compact".to_string()),
            recent_invocation_limit: None,

            blocked_binding_upstream_account_id: None,

            blocked_binding_constraint_source: None,
        }),
    )
    .await
    .expect("second page from row cursor should succeed");

    assert_eq!(
        second_page_from_row_cursor
            .conversations
            .iter()
            .map(|conversation| conversation.prompt_cache_key.as_str())
            .collect::<Vec<_>>(),
        second_page
            .conversations
            .iter()
            .map(|conversation| conversation.prompt_cache_key.as_str())
            .collect::<Vec<_>>()
    );
    assert_eq!(
        second_page_from_row_cursor.next_cursor,
        second_page.next_cursor
    );

    let Json(third_page) = fetch_prompt_cache_conversations(
        State(state),
        Query(PromptCacheConversationsQuery {
            limit: None,
            activity_hours: None,
            activity_minutes: Some(5),
            page_size: Some(20),
            cursor: second_page.next_cursor.clone(),
            snapshot_at: Some(first_snapshot),
            detail: Some("compact".to_string()),
            recent_invocation_limit: None,

            blocked_binding_upstream_account_id: None,

            blocked_binding_constraint_source: None,
        }),
    )
    .await
    .expect("third paginated working page should succeed");

    assert_eq!(
        third_page.conversations.len(),
        (total_matched - 40) as usize
    );
    assert_eq!(third_page.total_matched, Some(total_matched));
    assert!(!third_page.has_more);
    assert_eq!(third_page.next_cursor, None);

    let all_keys = first_page
        .conversations
        .iter()
        .chain(second_page.conversations.iter())
        .chain(third_page.conversations.iter())
        .map(|conversation| conversation.prompt_cache_key.clone())
        .collect::<Vec<_>>();
    let unique_keys = all_keys.iter().cloned().collect::<HashSet<_>>();

    assert_eq!(all_keys.len(), total_matched as usize);
    assert_eq!(unique_keys.len(), total_matched as usize);
}

#[tokio::test]
async fn prompt_cache_conversations_activity_minutes_paginated_preserves_sort_anchor_order() {
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
        .expect("insert paginated working row");
    }

    insert_row(
        &state.pool,
        "working-reactivated-history",
        now - ChronoDuration::hours(2),
        "working-reactivated",
        "success",
    )
    .await;
    insert_row(
        &state.pool,
        "working-reactivated-current",
        now - ChronoDuration::seconds(5),
        "working-reactivated",
        "success",
    )
    .await;
    insert_row(
        &state.pool,
        "working-fresh",
        now - ChronoDuration::seconds(20),
        "working-fresh",
        "success",
    )
    .await;
    insert_row(
        &state.pool,
        "working-older",
        now - ChronoDuration::seconds(40),
        "working-older",
        "success",
    )
    .await;

    sync_hourly_rollups_from_live_tables(&state.pool)
        .await
        .expect("materialize prompt cache rollups before paginated working read");

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

    let expected_order = non_paginated
        .conversations
        .iter()
        .map(|conversation| conversation.prompt_cache_key.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        expected_order,
        vec!["working-reactivated", "working-fresh", "working-older"]
    );

    let Json(first_page) = fetch_prompt_cache_conversations(
        State(state.clone()),
        Query(PromptCacheConversationsQuery {
            limit: None,
            activity_hours: None,
            activity_minutes: Some(5),
            page_size: Some(1),
            cursor: None,
            snapshot_at: None,
            detail: Some("compact".to_string()),
            recent_invocation_limit: None,

            blocked_binding_upstream_account_id: None,

            blocked_binding_constraint_source: None,
        }),
    )
    .await
    .expect("first paginated working page should succeed");

    assert_eq!(first_page.conversations.len(), 1);
    assert_eq!(
        first_page.conversations[0].prompt_cache_key,
        expected_order[0]
    );

    let Json(second_page) = fetch_prompt_cache_conversations(
        State(state.clone()),
        Query(PromptCacheConversationsQuery {
            limit: None,
            activity_hours: None,
            activity_minutes: Some(5),
            page_size: Some(1),
            cursor: first_page.next_cursor.clone(),
            snapshot_at: first_page.snapshot_at.clone(),
            detail: Some("compact".to_string()),
            recent_invocation_limit: None,

            blocked_binding_upstream_account_id: None,

            blocked_binding_constraint_source: None,
        }),
    )
    .await
    .expect("second paginated working page should succeed");

    assert_eq!(second_page.conversations.len(), 1);
    assert_eq!(
        second_page.conversations[0].prompt_cache_key,
        expected_order[1]
    );
}

#[tokio::test]
async fn prompt_cache_conversations_activity_minutes_paginated_cursor_uses_working_sort_key_after_lifecycle_totals()
 {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let now = Utc::now();
    let tied_current_at = now - ChronoDuration::seconds(5);

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
        .expect("insert tied working row");
    }

    insert_row(
        &state.pool,
        "working-z-boundary-history",
        now - ChronoDuration::hours(2),
        "working-z-boundary",
        "success",
    )
    .await;
    insert_row(
        &state.pool,
        "working-z-boundary-current",
        tied_current_at,
        "working-z-boundary",
        "success",
    )
    .await;
    insert_row(
        &state.pool,
        "working-m-between-current",
        tied_current_at,
        "working-m-between",
        "success",
    )
    .await;
    insert_row(
        &state.pool,
        "working-a-last-current",
        tied_current_at,
        "working-a-last",
        "success",
    )
    .await;

    sync_hourly_rollups_from_live_tables(&state.pool)
        .await
        .expect("materialize prompt cache rollups before tied paginated read");

    let Json(first_page) = fetch_prompt_cache_conversations(
        State(state.clone()),
        Query(PromptCacheConversationsQuery {
            limit: None,
            activity_hours: None,
            activity_minutes: Some(5),
            page_size: Some(1),
            cursor: None,
            snapshot_at: None,
            detail: Some("compact".to_string()),
            recent_invocation_limit: None,

            blocked_binding_upstream_account_id: None,

            blocked_binding_constraint_source: None,
        }),
    )
    .await
    .expect("first tied page should succeed");

    assert_eq!(first_page.conversations.len(), 1);
    assert_eq!(
        first_page.conversations[0].prompt_cache_key,
        "working-z-boundary"
    );
    assert_eq!(
        first_page.conversations[0].request_count, 2,
        "card totals should still use lifecycle data"
    );

    let Json(second_page) = fetch_prompt_cache_conversations(
        State(state.clone()),
        Query(PromptCacheConversationsQuery {
            limit: None,
            activity_hours: None,
            activity_minutes: Some(5),
            page_size: Some(1),
            cursor: first_page.next_cursor.clone(),
            snapshot_at: first_page.snapshot_at.clone(),
            detail: Some("compact".to_string()),
            recent_invocation_limit: None,

            blocked_binding_upstream_account_id: None,

            blocked_binding_constraint_source: None,
        }),
    )
    .await
    .expect("second tied page should succeed");

    assert_eq!(second_page.conversations.len(), 1);
    assert_eq!(
        second_page.conversations[0].prompt_cache_key,
        "working-m-between"
    );

    let Json(third_page) = fetch_prompt_cache_conversations(
        State(state),
        Query(PromptCacheConversationsQuery {
            limit: None,
            activity_hours: None,
            activity_minutes: Some(5),
            page_size: Some(1),
            cursor: second_page.next_cursor.clone(),
            snapshot_at: second_page.snapshot_at.clone(),
            detail: Some("compact".to_string()),
            recent_invocation_limit: None,

            blocked_binding_upstream_account_id: None,

            blocked_binding_constraint_source: None,
        }),
    )
    .await
    .expect("third tied page should succeed");

    assert_eq!(third_page.conversations.len(), 1);
    assert_eq!(
        third_page.conversations[0].prompt_cache_key,
        "working-a-last"
    );
}

#[tokio::test]
async fn prompt_cache_conversations_paginated_cursors_support_prompt_cache_keys_with_pipes() {
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
        .bind(10_i64)
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
        .expect("insert paginated pipe-key row");
    }

    insert_row(
        &state.pool,
        "working-pipe-head",
        now - ChronoDuration::seconds(5),
        "pipe|head",
    )
    .await;
    insert_row(
        &state.pool,
        "working-pipe-tail",
        now - ChronoDuration::seconds(10),
        "pipe|tail",
    )
    .await;

    let Json(first_page) = fetch_prompt_cache_conversations(
        State(state.clone()),
        Query(PromptCacheConversationsQuery {
            limit: None,
            activity_hours: None,
            activity_minutes: Some(5),
            page_size: Some(1),
            cursor: None,
            snapshot_at: None,
            detail: Some("compact".to_string()),
            recent_invocation_limit: None,

            blocked_binding_upstream_account_id: None,

            blocked_binding_constraint_source: None,
        }),
    )
    .await
    .expect("first pipe-key page should succeed");

    assert_eq!(first_page.conversations[0].prompt_cache_key, "pipe|head");
    let snapshot_at = first_page.snapshot_at.clone().expect("pipe-key snapshotAt");
    let next_cursor = first_page.next_cursor.clone().expect("pipe-key nextCursor");
    let row_cursor = first_page.conversations[0]
        .cursor
        .clone()
        .expect("pipe-key row cursor");

    let Json(second_page) = fetch_prompt_cache_conversations(
        State(state.clone()),
        Query(PromptCacheConversationsQuery {
            limit: None,
            activity_hours: None,
            activity_minutes: Some(5),
            page_size: Some(1),
            cursor: Some(next_cursor),
            snapshot_at: Some(snapshot_at.clone()),
            detail: Some("compact".to_string()),
            recent_invocation_limit: None,

            blocked_binding_upstream_account_id: None,

            blocked_binding_constraint_source: None,
        }),
    )
    .await
    .expect("second pipe-key page should succeed");

    assert_eq!(second_page.conversations[0].prompt_cache_key, "pipe|tail");

    let Json(second_page_from_row_cursor) = fetch_prompt_cache_conversations(
        State(state),
        Query(PromptCacheConversationsQuery {
            limit: None,
            activity_hours: None,
            activity_minutes: Some(5),
            page_size: Some(1),
            cursor: Some(row_cursor),
            snapshot_at: Some(snapshot_at),
            detail: Some("compact".to_string()),
            recent_invocation_limit: None,

            blocked_binding_upstream_account_id: None,

            blocked_binding_constraint_source: None,
        }),
    )
    .await
    .expect("second pipe-key row-cursor page should succeed");

    assert_eq!(
        second_page_from_row_cursor.conversations[0].prompt_cache_key,
        "pipe|tail"
    );
}
