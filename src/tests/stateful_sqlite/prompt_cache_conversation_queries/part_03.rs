async fn insert_prompt_cache_row_03_01(
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

async fn insert_prompt_cache_row_03_02(
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

async fn insert_prompt_cache_row_03_03(
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

async fn insert_prompt_cache_row_03_04(
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

async fn insert_prompt_cache_row_03_05(
    pool: &Pool<Sqlite>,
    invoke_id: &str,
    occurred_at: DateTime<Utc>,
    key: &str,
    total_tokens: i64,
    cost: f64,
) {
    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response, created_at
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
        "#,
    )
    .bind(invoke_id)
    .bind(format_naive(
        occurred_at.with_timezone(&Shanghai).naive_local(),
    ))
    .bind(SOURCE_PROXY)
    .bind("success")
    .bind(total_tokens)
    .bind(cost)
    .bind(
        json!({
            "promptCacheKey": key,
            "routeMode": "pool",
            "model": "gpt-5.4",
        })
        .to_string(),
    )
    .bind("{}")
    .bind(format_utc_iso_millis(occurred_at))
    .execute(pool)
    .await
    .expect("insert paginated snapshot row");
}

async fn seed_compact_working_pages(pool: &Pool<Sqlite>, now: DateTime<Utc>) {
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
        .execute(pool)
        .await
        .expect("insert paginated working row");
    }
}

async fn fetch_compact_working_page(
    state: Arc<AppState>,
    page_size: usize,
    cursor: Option<String>,
    snapshot_at: Option<String>,
) -> PromptCacheConversationsResponse {
    fetch_prompt_cache_test_response(
        state,
        PromptCacheConversationsQuery {
            activity_minutes: Some(5),
            page_size: Some(page_size as i64),
            cursor,
            snapshot_at,
            detail: Some("compact".to_string()),
            ..prompt_cache_query()
        },
    )
    .await
}

fn assert_first_compact_working_page(
    first_page: &PromptCacheConversationsResponse,
) -> (i64, String, String) {
    let total_matched = first_page.total_matched.expect("first page total_matched");
    assert_eq!(first_page.conversations.len(), 20);
    assert!(total_matched > 50);
    assert!(first_page.has_more);
    assert_eq!(first_page.implicit_filter.kind, None);
    assert_eq!(first_page.implicit_filter.filtered_count, 0);
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
    (
        total_matched,
        first_page
            .snapshot_at
            .clone()
            .expect("first page snapshot_at"),
        first_page
            .next_cursor
            .clone()
            .expect("first page next_cursor"),
    )
}

#[tokio::test]
pub(crate) async fn prompt_cache_conversations_activity_minutes_paginated_compact_can_scroll_past_fifty_without_duplicates()
 {
    let state = prompt_cache_test_state().await;
    let now = Utc::now();
    seed_compact_working_pages(&state.pool, now).await;

    let first_page = fetch_compact_working_page(state.clone(), 20, None, None).await;
    let (total_matched, first_snapshot, first_next_cursor) =
        assert_first_compact_working_page(&first_page);
    let second_page = fetch_compact_working_page(
        state.clone(),
        20,
        Some(first_next_cursor),
        Some(first_snapshot.clone()),
    )
    .await;

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
    let second_page_from_row_cursor = fetch_compact_working_page(
        state.clone(),
        20,
        Some(last_visible_row_cursor),
        Some(first_snapshot.clone()),
    )
    .await;

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

    let third_page = fetch_compact_working_page(
        state,
        20,
        second_page.next_cursor.clone(),
        Some(first_snapshot),
    )
    .await;

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
pub(crate) async fn prompt_cache_conversations_activity_minutes_paginated_preserves_sort_anchor_order()
 {
    let state = prompt_cache_test_state().await;
    let now = Utc::now();

    insert_prompt_cache_row_03_01(
        &state.pool,
        "working-reactivated-history",
        now - ChronoDuration::hours(2),
        "working-reactivated",
        "success",
    )
    .await;
    insert_prompt_cache_row_03_01(
        &state.pool,
        "working-reactivated-current",
        now - ChronoDuration::seconds(5),
        "working-reactivated",
        "success",
    )
    .await;
    insert_prompt_cache_row_03_01(
        &state.pool,
        "working-fresh",
        now - ChronoDuration::seconds(20),
        "working-fresh",
        "success",
    )
    .await;
    insert_prompt_cache_row_03_01(
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
            activity_minutes: Some(5),

            ..prompt_cache_query()
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

    let first_page = fetch_compact_working_page(state.clone(), 1, None, None).await;

    assert_eq!(first_page.conversations.len(), 1);
    assert_eq!(
        first_page.conversations[0].prompt_cache_key,
        expected_order[0]
    );

    let second_page = fetch_compact_working_page(
        state,
        1,
        first_page.next_cursor.clone(),
        first_page.snapshot_at.clone(),
    )
    .await;

    assert_eq!(second_page.conversations.len(), 1);
    assert_eq!(
        second_page.conversations[0].prompt_cache_key,
        expected_order[1]
    );
}

#[tokio::test]
pub(crate) async fn prompt_cache_conversations_activity_minutes_paginated_cursor_uses_working_sort_key_after_lifecycle_totals()
 {
    let state = prompt_cache_test_state().await;
    let now = Utc::now();
    let tied_current_at = now - ChronoDuration::seconds(5);

    insert_prompt_cache_row_03_02(
        &state.pool,
        "working-z-boundary-history",
        now - ChronoDuration::hours(2),
        "working-z-boundary",
        "success",
    )
    .await;
    insert_prompt_cache_row_03_02(
        &state.pool,
        "working-z-boundary-current",
        tied_current_at,
        "working-z-boundary",
        "success",
    )
    .await;
    insert_prompt_cache_row_03_02(
        &state.pool,
        "working-m-between-current",
        tied_current_at,
        "working-m-between",
        "success",
    )
    .await;
    insert_prompt_cache_row_03_02(
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

    let first_page = fetch_compact_working_page(state.clone(), 1, None, None).await;

    assert_eq!(first_page.conversations.len(), 1);
    assert_eq!(
        first_page.conversations[0].prompt_cache_key,
        "working-z-boundary"
    );
    assert_eq!(
        first_page.conversations[0].request_count, 2,
        "card totals should still use lifecycle data"
    );

    let second_page = fetch_compact_working_page(
        state.clone(),
        1,
        first_page.next_cursor.clone(),
        first_page.snapshot_at.clone(),
    )
    .await;

    assert_eq!(second_page.conversations.len(), 1);
    assert_eq!(
        second_page.conversations[0].prompt_cache_key,
        "working-m-between"
    );

    let third_page = fetch_compact_working_page(
        state,
        1,
        second_page.next_cursor.clone(),
        second_page.snapshot_at.clone(),
    )
    .await;

    assert_eq!(third_page.conversations.len(), 1);
    assert_eq!(
        third_page.conversations[0].prompt_cache_key,
        "working-a-last"
    );
}

#[tokio::test]
pub(crate) async fn prompt_cache_conversations_activity_minutes_exclude_stale_terminal_rows_from_totals()
 {
    let state = prompt_cache_test_state().await;
    let now = Utc::now();

    insert_prompt_cache_row_03_03(
        &state.pool,
        "working-in-flight",
        now - ChronoDuration::minutes(20),
        "working-in-flight",
        "pending",
    )
    .await;
    insert_prompt_cache_row_03_03(
        &state.pool,
        "working-recent-terminal",
        now - ChronoDuration::seconds(45),
        "working-recent-terminal",
        "success",
    )
    .await;
    insert_prompt_cache_row_03_03(
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
            activity_minutes: Some(5),

            ..prompt_cache_query()
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
            activity_minutes: Some(5),
            page_size: Some(20),
            detail: Some("compact".to_string()),

            ..prompt_cache_query()
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
pub(crate) async fn prompt_cache_conversations_paginated_cursors_support_prompt_cache_keys_with_pipes()
 {
    let state = prompt_cache_test_state().await;
    let now = Utc::now();

    insert_prompt_cache_row_03_04(
        &state.pool,
        "working-pipe-head",
        now - ChronoDuration::seconds(5),
        "pipe|head",
    )
    .await;
    insert_prompt_cache_row_03_04(
        &state.pool,
        "working-pipe-tail",
        now - ChronoDuration::seconds(10),
        "pipe|tail",
    )
    .await;

    let Json(first_page) = fetch_prompt_cache_conversations(
        State(state.clone()),
        Query(PromptCacheConversationsQuery {
            activity_minutes: Some(5),
            page_size: Some(1),
            detail: Some("compact".to_string()),

            ..prompt_cache_query()
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
            activity_minutes: Some(5),
            page_size: Some(1),
            cursor: Some(next_cursor),
            snapshot_at: Some(snapshot_at.clone()),
            detail: Some("compact".to_string()),

            ..prompt_cache_query()
        }),
    )
    .await
    .expect("second pipe-key page should succeed");

    assert_eq!(second_page.conversations[0].prompt_cache_key, "pipe|tail");

    let Json(second_page_from_row_cursor) = fetch_prompt_cache_conversations(
        State(state),
        Query(PromptCacheConversationsQuery {
            activity_minutes: Some(5),
            page_size: Some(1),
            cursor: Some(row_cursor),
            snapshot_at: Some(snapshot_at),
            detail: Some("compact".to_string()),

            ..prompt_cache_query()
        }),
    )
    .await
    .expect("second pipe-key row-cursor page should succeed");

    assert_eq!(
        second_page_from_row_cursor.conversations[0].prompt_cache_key,
        "pipe|tail"
    );
}

#[test]
pub(crate) fn prompt_cache_conversations_omitted_snapshot_preserves_current_precision() {
    let precise_now = Utc
        .timestamp_opt(1_744_298_800, 456_000_000)
        .single()
        .expect("valid precise utc instant");
    let snapshot_at = resolve_prompt_cache_conversation_snapshot_at_with_default(None, precise_now)
        .expect("omitted snapshotAt should resolve");
    assert_eq!(snapshot_at, precise_now);
    assert_eq!(
        db_occurred_at_upper_bound(snapshot_at),
        db_occurred_at_lower_bound(precise_now + ChronoDuration::seconds(1))
    );
}

#[tokio::test]
pub(crate) async fn prompt_cache_conversations_paginated_invalid_snapshot_at_returns_bad_request() {
    let state = prompt_cache_test_state().await;

    let err = fetch_prompt_cache_conversations(
        State(state),
        Query(PromptCacheConversationsQuery {
            activity_minutes: Some(5),
            page_size: Some(20),
            snapshot_at: Some("not-a-timestamp".to_string()),
            detail: Some("compact".to_string()),

            ..prompt_cache_query()
        }),
    )
    .await
    .expect_err("invalid snapshotAt should be rejected");

    match err {
        ApiError::BadRequest(inner) => {
            assert!(inner.to_string().contains("invalid snapshotAt"));
        }
        other => panic!("expected bad request, got {other:?}"),
    }
}

#[tokio::test]
pub(crate) async fn prompt_cache_conversations_paginated_invalid_cursor_returns_bad_request() {
    let state = prompt_cache_test_state().await;

    let err = fetch_prompt_cache_conversations(
        State(state),
        Query(PromptCacheConversationsQuery {
            activity_minutes: Some(5),
            page_size: Some(20),
            cursor: Some("not-base64".to_string()),
            snapshot_at: Some(Utc::now().to_rfc3339()),
            detail: Some("compact".to_string()),

            ..prompt_cache_query()
        }),
    )
    .await
    .expect_err("invalid cursor should be rejected");

    match err {
        ApiError::BadRequest(inner) => {
            let message = inner.to_string();
            assert!(
                message.contains("invalid cursor"),
                "unexpected error message: {message}"
            );
        }
        other => panic!("expected bad request, got {other:?}"),
    }
}

#[tokio::test]
pub(crate) async fn prompt_cache_conversations_activity_minutes_paginated_respect_requested_snapshot_totals()
 {
    let state = prompt_cache_test_state().await;
    let snapshot_at = Utc::now() - ChronoDuration::seconds(20);

    insert_prompt_cache_row_03_05(
        &state.pool,
        "working-snapshot-head-pre",
        snapshot_at - ChronoDuration::seconds(5),
        "working-snapshot-head",
        20,
        0.20,
    )
    .await;
    insert_prompt_cache_row_03_05(
        &state.pool,
        "working-snapshot-target-pre",
        snapshot_at - ChronoDuration::seconds(15),
        "working-snapshot-target",
        10,
        0.10,
    )
    .await;
    insert_prompt_cache_row_03_05(
        &state.pool,
        "working-snapshot-target-post",
        snapshot_at + ChronoDuration::seconds(5),
        "working-snapshot-target",
        999,
        9.99,
    )
    .await;

    let snapshot_at_rfc3339 = snapshot_at.to_rfc3339();
    let Json(first_page) = fetch_prompt_cache_conversations(
        State(state.clone()),
        Query(PromptCacheConversationsQuery {
            activity_minutes: Some(5),
            page_size: Some(1),
            snapshot_at: Some(snapshot_at_rfc3339.clone()),
            detail: Some("compact".to_string()),

            ..prompt_cache_query()
        }),
    )
    .await
    .expect("first snapshot page should succeed");

    assert_eq!(first_page.conversations.len(), 1);
    assert_eq!(
        first_page.conversations[0].prompt_cache_key,
        "working-snapshot-target"
    );

    let Json(second_page) = fetch_prompt_cache_conversations(
        State(state),
        Query(PromptCacheConversationsQuery {
            activity_minutes: Some(5),
            page_size: Some(1),
            cursor: first_page.next_cursor.clone(),
            snapshot_at: Some(snapshot_at_rfc3339),
            detail: Some("compact".to_string()),

            ..prompt_cache_query()
        }),
    )
    .await
    .expect("second snapshot page should succeed");

    assert_eq!(second_page.conversations.len(), 1);
    let target = &second_page.conversations[0];
    assert_eq!(target.prompt_cache_key, "working-snapshot-head");
    assert_eq!(target.request_count, 1);
    assert_eq!(target.total_tokens, 20);
    assert!((target.total_cost - 0.20).abs() < 1e-9);
}

use super::*;
