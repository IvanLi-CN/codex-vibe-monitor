use super::*;

#[test]
fn prompt_cache_conversations_omitted_snapshot_preserves_current_precision() {
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
async fn prompt_cache_conversations_paginated_invalid_snapshot_at_returns_bad_request() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;

    let err = fetch_prompt_cache_conversations(
        State(state),
        Query(PromptCacheConversationsQuery {
            limit: None,
            activity_hours: None,
            activity_minutes: Some(5),
            page_size: Some(20),
            cursor: None,
            snapshot_at: Some("not-a-timestamp".to_string()),
            detail: Some("compact".to_string()),
            recent_invocation_limit: None,

            blocked_binding_upstream_account_id: None,

            blocked_binding_constraint_source: None,
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
async fn prompt_cache_conversations_paginated_invalid_cursor_returns_bad_request() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;

    let err = fetch_prompt_cache_conversations(
        State(state),
        Query(PromptCacheConversationsQuery {
            limit: None,
            activity_hours: None,
            activity_minutes: Some(5),
            page_size: Some(20),
            cursor: Some("not-base64".to_string()),
            snapshot_at: Some(Utc::now().to_rfc3339()),
            detail: Some("compact".to_string()),
            recent_invocation_limit: None,

            blocked_binding_upstream_account_id: None,

            blocked_binding_constraint_source: None,
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
async fn prompt_cache_conversations_activity_minutes_paginated_respect_requested_snapshot_totals() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let snapshot_at = Utc::now() - ChronoDuration::seconds(20);

    async fn insert_row(
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

    insert_row(
        &state.pool,
        "working-snapshot-head-pre",
        snapshot_at - ChronoDuration::seconds(5),
        "working-snapshot-head",
        20,
        0.20,
    )
    .await;
    insert_row(
        &state.pool,
        "working-snapshot-target-pre",
        snapshot_at - ChronoDuration::seconds(15),
        "working-snapshot-target",
        10,
        0.10,
    )
    .await;
    insert_row(
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
            limit: None,
            activity_hours: None,
            activity_minutes: Some(5),
            page_size: Some(1),
            cursor: None,
            snapshot_at: Some(snapshot_at_rfc3339.clone()),
            detail: Some("compact".to_string()),
            recent_invocation_limit: None,

            blocked_binding_upstream_account_id: None,

            blocked_binding_constraint_source: None,
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
            limit: None,
            activity_hours: None,
            activity_minutes: Some(5),
            page_size: Some(1),
            cursor: first_page.next_cursor.clone(),
            snapshot_at: Some(snapshot_at_rfc3339),
            detail: Some("compact".to_string()),
            recent_invocation_limit: None,

            blocked_binding_upstream_account_id: None,

            blocked_binding_constraint_source: None,
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

#[tokio::test]
async fn prompt_cache_conversations_activity_minutes_paginated_snapshot_excludes_same_second_post_snapshot_writes()
 {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let snapshot_second = Utc::now() - ChronoDuration::seconds(20);
    let requested_snapshot_at = snapshot_second + ChronoDuration::milliseconds(123);

    async fn insert_row(
        pool: &Pool<Sqlite>,
        invoke_id: &str,
        occurred_at: DateTime<Utc>,
        created_at: DateTime<Utc>,
        key: &str,
        total_tokens: i64,
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
        .bind(format_utc_iso_millis(created_at))
        .execute(pool)
        .await
        .expect("insert paginated same-second row");
    }

    insert_row(
        &state.pool,
        "working-same-second-head",
        snapshot_second - ChronoDuration::seconds(5),
        snapshot_second - ChronoDuration::seconds(5),
        "working-same-second-head",
        20,
    )
    .await;
    insert_row(
        &state.pool,
        "working-same-second-tail",
        snapshot_second - ChronoDuration::seconds(15),
        snapshot_second - ChronoDuration::seconds(15),
        "working-same-second-tail",
        10,
    )
    .await;
    insert_row(
        &state.pool,
        "working-same-second-preexisting-post",
        snapshot_second,
        requested_snapshot_at + ChronoDuration::milliseconds(200),
        "working-same-second-preexisting-post",
        888,
    )
    .await;

    materialize_prompt_cache_hourly_rollups(&state.pool).await;

    let Json(first_page) = fetch_prompt_cache_conversations(
        State(state.clone()),
        Query(PromptCacheConversationsQuery {
            limit: None,
            activity_hours: None,
            activity_minutes: Some(5),
            page_size: Some(1),
            cursor: None,
            snapshot_at: Some(requested_snapshot_at.to_rfc3339()),
            detail: Some("compact".to_string()),
            recent_invocation_limit: None,

            blocked_binding_upstream_account_id: None,

            blocked_binding_constraint_source: None,
        }),
    )
    .await
    .expect("first same-second snapshot page should succeed");

    assert_eq!(first_page.conversations.len(), 1);
    assert_eq!(first_page.total_matched, Some(3));
    let expected_snapshot_at = format_utc_iso_precise(requested_snapshot_at);
    assert_eq!(
        first_page.conversations[0].prompt_cache_key,
        "working-same-second-preexisting-post"
    );
    assert_eq!(
        first_page.snapshot_at.as_deref(),
        Some(expected_snapshot_at.as_str())
    );

    insert_row(
        &state.pool,
        "working-same-second-post",
        snapshot_second,
        requested_snapshot_at + ChronoDuration::milliseconds(400),
        "working-same-second-post",
        999,
    )
    .await;

    let Json(second_page) = fetch_prompt_cache_conversations(
        State(state),
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
    .expect("second same-second snapshot page should succeed");

    assert_eq!(second_page.total_matched, Some(4));
    assert_eq!(second_page.conversations.len(), 1);
    assert_eq!(
        second_page.conversations[0].prompt_cache_key,
        "working-same-second-post"
    );
}

#[tokio::test]
async fn prompt_cache_conversations_activity_minutes_paginated_whole_second_snapshot_excludes_post_snapshot_writes()
 {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let snapshot_second = Utc::now() - ChronoDuration::seconds(20);

    async fn insert_row(
        pool: &Pool<Sqlite>,
        invoke_id: &str,
        occurred_at: DateTime<Utc>,
        created_at: DateTime<Utc>,
        key: &str,
        total_tokens: i64,
    ) {
        sqlx::query(
            r#"
            INSERT INTO codex_invocations (
                invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response, created_at
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
            "#,
        )
        .bind(format!("{invoke_id}-{}", created_at.timestamp_millis()))
        .bind(format_naive(
            occurred_at.with_timezone(&Shanghai).naive_local(),
        ))
        .bind(SOURCE_PROXY)
        .bind("success")
        .bind(total_tokens)
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
        .bind(format_utc_iso_millis(created_at))
        .execute(pool)
        .await
        .expect("insert whole-second snapshot row");
    }

    insert_row(
        &state.pool,
        "working-whole-second-head",
        snapshot_second,
        snapshot_second,
        "working-whole-second-head",
        20,
    )
    .await;
    insert_row(
        &state.pool,
        "working-whole-second-tail",
        snapshot_second - ChronoDuration::seconds(15),
        snapshot_second - ChronoDuration::seconds(15),
        "working-whole-second-tail",
        10,
    )
    .await;
    insert_row(
        &state.pool,
        "working-whole-second-preexisting-post",
        snapshot_second,
        snapshot_second + ChronoDuration::milliseconds(200),
        "working-whole-second-preexisting-post",
        888,
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
            snapshot_at: Some(format_utc_iso(snapshot_second)),
            detail: Some("compact".to_string()),
            recent_invocation_limit: None,

            blocked_binding_upstream_account_id: None,

            blocked_binding_constraint_source: None,
        }),
    )
    .await
    .expect("first whole-second snapshot page should succeed");

    assert_eq!(first_page.conversations.len(), 1);
    assert_eq!(first_page.total_matched, Some(3));
    assert_eq!(
        first_page.conversations[0].prompt_cache_key,
        "working-whole-second-preexisting-post"
    );
    assert_eq!(
        first_page.snapshot_at.as_deref(),
        Some(format_utc_iso(snapshot_second).as_str())
    );

    insert_row(
        &state.pool,
        "working-whole-second-post",
        snapshot_second,
        snapshot_second + ChronoDuration::milliseconds(400),
        "working-whole-second-post",
        999,
    )
    .await;

    let Json(second_page) = fetch_prompt_cache_conversations(
        State(state),
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
    .expect("second whole-second snapshot page should succeed");

    assert_eq!(second_page.total_matched, Some(4));
    assert_eq!(second_page.conversations.len(), 1);
    assert_eq!(
        second_page.conversations[0].prompt_cache_key,
        "working-whole-second-post"
    );
}

#[tokio::test]
async fn prompt_cache_conversations_activity_minutes_paginated_snapshot_excludes_late_persisted_pre_snapshot_occurrence()
 {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let snapshot_second = Utc::now() - ChronoDuration::seconds(20);
    let requested_snapshot_at = snapshot_second + ChronoDuration::milliseconds(123);

    async fn insert_row(
        pool: &Pool<Sqlite>,
        invoke_id: &str,
        occurred_at: DateTime<Utc>,
        created_at: DateTime<Utc>,
        key: &str,
        total_tokens: i64,
    ) {
        sqlx::query(
            r#"
            INSERT INTO codex_invocations (
                invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response, created_at
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
            "#,
        )
        .bind(format!("{invoke_id}-{}", created_at.timestamp_millis()))
        .bind(format_naive(
            occurred_at.with_timezone(&Shanghai).naive_local(),
        ))
        .bind(SOURCE_PROXY)
        .bind("success")
        .bind(total_tokens)
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
        .bind(format_utc_iso_millis(created_at))
        .execute(pool)
        .await
        .expect("insert late-persisted snapshot row");
    }

    insert_row(
        &state.pool,
        "working-late-persist-head",
        snapshot_second - ChronoDuration::seconds(5),
        snapshot_second - ChronoDuration::seconds(5),
        "working-late-persist-head",
        20,
    )
    .await;
    insert_row(
        &state.pool,
        "working-late-persist-tail",
        snapshot_second - ChronoDuration::seconds(15),
        snapshot_second - ChronoDuration::seconds(15),
        "working-late-persist-tail",
        10,
    )
    .await;

    materialize_prompt_cache_hourly_rollups(&state.pool).await;

    let Json(first_page) = fetch_prompt_cache_conversations(
        State(state.clone()),
        Query(PromptCacheConversationsQuery {
            limit: None,
            activity_hours: None,
            activity_minutes: Some(5),
            page_size: Some(1),
            cursor: None,
            snapshot_at: Some(requested_snapshot_at.to_rfc3339()),
            detail: Some("compact".to_string()),
            recent_invocation_limit: None,

            blocked_binding_upstream_account_id: None,

            blocked_binding_constraint_source: None,
        }),
    )
    .await
    .expect("first late-persist snapshot page should succeed");

    assert_eq!(first_page.conversations.len(), 1);
    assert_eq!(first_page.total_matched, Some(2));
    assert_eq!(
        first_page.conversations[0].prompt_cache_key,
        "working-late-persist-head"
    );

    insert_row(
        &state.pool,
        "working-late-persist-post",
        snapshot_second - ChronoDuration::seconds(10),
        requested_snapshot_at + ChronoDuration::milliseconds(400),
        "working-late-persist-post",
        999,
    )
    .await;

    let Json(second_page) = fetch_prompt_cache_conversations(
        State(state),
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
    .expect("second late-persist snapshot page should succeed");

    assert_eq!(second_page.total_matched, Some(3));
    assert_eq!(second_page.conversations.len(), 1);
    assert_eq!(
        second_page.conversations[0].prompt_cache_key,
        "working-late-persist-post"
    );
}

#[tokio::test]
async fn prompt_cache_conversations_activity_minutes_paginated_snapshot_preserves_previous_hour_lifetime_totals()
 {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let snapshot_at = Utc::now() - ChronoDuration::seconds(20);

    async fn insert_row(
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
        .expect("insert paginated previous-hour row");
    }

    insert_row(
        &state.pool,
        "working-window-head",
        snapshot_at - ChronoDuration::seconds(10),
        "working-window-head",
        20,
        0.20,
    )
    .await;
    insert_row(
        &state.pool,
        "working-window-target-stale",
        snapshot_at - ChronoDuration::minutes(33),
        "working-window-target",
        999,
        9.99,
    )
    .await;
    insert_row(
        &state.pool,
        "working-window-target-pre",
        snapshot_at - ChronoDuration::minutes(4),
        "working-window-target",
        10,
        0.10,
    )
    .await;
    insert_row(
        &state.pool,
        "working-window-target-post",
        snapshot_at + ChronoDuration::seconds(5),
        "working-window-target",
        777,
        7.77,
    )
    .await;

    materialize_prompt_cache_hourly_rollups(&state.pool).await;

    let snapshot_at_rfc3339 = snapshot_at.to_rfc3339();
    let Json(first_page) = fetch_prompt_cache_conversations(
        State(state.clone()),
        Query(PromptCacheConversationsQuery {
            limit: None,
            activity_hours: None,
            activity_minutes: Some(5),
            page_size: Some(1),
            cursor: None,
            snapshot_at: Some(snapshot_at_rfc3339.clone()),
            detail: Some("compact".to_string()),
            recent_invocation_limit: None,

            blocked_binding_upstream_account_id: None,

            blocked_binding_constraint_source: None,
        }),
    )
    .await
    .expect("first lifetime snapshot page should succeed");

    assert_eq!(first_page.conversations.len(), 1);
    assert_eq!(
        first_page.conversations[0].prompt_cache_key,
        "working-window-target"
    );
    assert_eq!(first_page.conversations[0].request_count, 2);
    assert_eq!(first_page.conversations[0].total_tokens, 1009);
    assert!((first_page.conversations[0].total_cost - 10.09).abs() < 1e-9);

    let Json(second_page) = fetch_prompt_cache_conversations(
        State(state),
        Query(PromptCacheConversationsQuery {
            limit: None,
            activity_hours: None,
            activity_minutes: Some(5),
            page_size: Some(1),
            cursor: first_page.next_cursor.clone(),
            snapshot_at: Some(snapshot_at_rfc3339),
            detail: Some("compact".to_string()),
            recent_invocation_limit: None,

            blocked_binding_upstream_account_id: None,

            blocked_binding_constraint_source: None,
        }),
    )
    .await
    .expect("second lifetime snapshot page should succeed");

    assert_eq!(second_page.conversations.len(), 1);
    assert_eq!(
        second_page.conversations[0].prompt_cache_key,
        "working-window-head"
    );
    assert_eq!(second_page.conversations[0].request_count, 1);
    assert_eq!(second_page.conversations[0].total_tokens, 20);
    assert!((second_page.conversations[0].total_cost - 0.20).abs() < 1e-9);
}

#[tokio::test]
async fn prompt_cache_conversations_activity_minutes_paginated_snapshot_includes_unmaterialized_previous_hour_lifecycle_tail()
 {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let now = Utc::now();
    let snapshot_hour_start_epoch = align_bucket_epoch(now.timestamp(), 3_600, 0);
    let snapshot_hour_start = Utc
        .timestamp_opt(snapshot_hour_start_epoch, 0)
        .single()
        .expect("valid snapshot hour start");
    let minimum_snapshot_at = snapshot_hour_start + ChronoDuration::minutes(10);
    let snapshot_at = if now < minimum_snapshot_at {
        minimum_snapshot_at
    } else {
        now - ChronoDuration::seconds(20)
    };
    let old_tail_at = snapshot_hour_start - ChronoDuration::minutes(10);
    let current_working_at = snapshot_at - ChronoDuration::minutes(1);

    materialize_prompt_cache_hourly_rollups(&state.pool).await;

    async fn insert_row(
        pool: &Pool<Sqlite>,
        invoke_id: &str,
        occurred_at: DateTime<Utc>,
        total_tokens: i64,
        cost: f64,
    ) {
        sqlx::query(
            r#"
            INSERT INTO codex_invocations (
                invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response, created_at
            )
            VALUES (?1, ?2, ?3, 'success', ?4, ?5, ?6, ?7, ?8)
            "#,
        )
        .bind(invoke_id)
        .bind(format_naive(
            occurred_at.with_timezone(&Shanghai).naive_local(),
        ))
        .bind(SOURCE_PROXY)
        .bind(total_tokens)
        .bind(cost)
        .bind(
            json!({
                "promptCacheKey": "working-lagging-rollup",
                "routeMode": "pool",
                "model": "gpt-5.4",
            })
            .to_string(),
        )
        .bind("{}")
        .bind(format_utc_iso_millis(occurred_at))
        .execute(pool)
        .await
        .expect("insert lagging rollup lifecycle row");
    }

    insert_row(
        &state.pool,
        "working-lagging-rollup-old-tail",
        old_tail_at,
        30,
        0.30,
    )
    .await;
    insert_row(
        &state.pool,
        "working-lagging-rollup-current",
        current_working_at,
        70,
        0.70,
    )
    .await;

    let Json(response) = fetch_prompt_cache_conversations(
        State(state),
        Query(PromptCacheConversationsQuery {
            limit: None,
            activity_hours: None,
            activity_minutes: Some(5),
            page_size: Some(20),
            cursor: None,
            snapshot_at: Some(snapshot_at.to_rfc3339()),
            detail: Some("compact".to_string()),
            recent_invocation_limit: None,

            blocked_binding_upstream_account_id: None,

            blocked_binding_constraint_source: None,
        }),
    )
    .await
    .expect("snapshot working response should include unmaterialized lifecycle tail");

    let conversation = response
        .conversations
        .iter()
        .find(|conversation| conversation.prompt_cache_key == "working-lagging-rollup")
        .expect("working conversation should be visible from current-hour row");
    assert_eq!(conversation.request_count, 2);
    assert_eq!(conversation.total_tokens, 100);
    assert!((conversation.total_cost - 1.0).abs() < f64::EPSILON);
}

#[tokio::test]
async fn prompt_cache_conversations_activity_minutes_paginated_snapshot_excludes_late_backfilled_rollup_lifecycle_totals()
 {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let now = Utc::now();
    let snapshot_hour_start_epoch = align_bucket_epoch(now.timestamp(), 3_600, 0);
    let snapshot_hour_start = Utc
        .timestamp_opt(snapshot_hour_start_epoch, 0)
        .single()
        .expect("valid snapshot hour start");
    let minimum_snapshot_at = snapshot_hour_start + ChronoDuration::minutes(10);
    let snapshot_at = if now < minimum_snapshot_at {
        minimum_snapshot_at
    } else {
        now - ChronoDuration::seconds(20)
    };
    let archived_rollup_at = snapshot_hour_start - ChronoDuration::hours(2);
    let old_hour_at = snapshot_hour_start - ChronoDuration::minutes(15);
    let current_working_at = snapshot_at - ChronoDuration::minutes(1);

    async fn insert_row(
        pool: &Pool<Sqlite>,
        invoke_id: &str,
        occurred_at: DateTime<Utc>,
        created_at: DateTime<Utc>,
        total_tokens: i64,
        cost: f64,
    ) {
        sqlx::query(
            r#"
            INSERT INTO codex_invocations (
                invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response, created_at
            )
            VALUES (?1, ?2, ?3, 'success', ?4, ?5, ?6, ?7, ?8)
            "#,
        )
        .bind(invoke_id)
        .bind(format_naive(
            occurred_at.with_timezone(&Shanghai).naive_local(),
        ))
        .bind(SOURCE_PROXY)
        .bind(total_tokens)
        .bind(cost)
        .bind(
            json!({
                "promptCacheKey": "working-late-rollup",
                "routeMode": "pool",
                "model": "gpt-5.4",
            })
            .to_string(),
        )
        .bind("{}")
        .bind(format_utc_iso_millis(created_at))
        .execute(pool)
        .await
        .expect("insert late backfilled rollup lifecycle row");
    }

    sqlx::query(
        r#"
        INSERT INTO prompt_cache_rollup_hourly (
            bucket_start_epoch, source, prompt_cache_key, request_count, success_count,
            failure_count, total_tokens, total_cost, first_seen_at, last_seen_at
        )
        VALUES (?1, ?2, ?3, 1, 1, 0, ?4, ?5, ?6, ?6)
        "#,
    )
    .bind(align_bucket_epoch(archived_rollup_at.timestamp(), 3_600, 0))
    .bind(SOURCE_PROXY)
    .bind("working-late-rollup")
    .bind(40_i64)
    .bind(0.40_f64)
    .bind(format_naive(
        archived_rollup_at.with_timezone(&Shanghai).naive_local(),
    ))
    .execute(&state.pool)
    .await
    .expect("insert archived-only prompt-cache rollup row");

    insert_row(
        &state.pool,
        "working-late-rollup-created-after-snapshot",
        old_hour_at + ChronoDuration::seconds(2),
        snapshot_at + ChronoDuration::seconds(10),
        500,
        5.00,
    )
    .await;
    insert_row(
        &state.pool,
        "working-late-rollup-old",
        old_hour_at,
        old_hour_at,
        30,
        0.30,
    )
    .await;
    insert_row(
        &state.pool,
        "working-late-rollup-current",
        current_working_at,
        current_working_at,
        70,
        0.70,
    )
    .await;
    materialize_prompt_cache_hourly_rollups(&state.pool).await;

    insert_row(
        &state.pool,
        "working-late-rollup-backfilled",
        old_hour_at + ChronoDuration::seconds(5),
        snapshot_at + ChronoDuration::seconds(5),
        900,
        9.00,
    )
    .await;
    materialize_prompt_cache_hourly_rollups(&state.pool).await;

    insert_row(
        &state.pool,
        "working-late-rollup-unmaterialized-backfill",
        archived_rollup_at + ChronoDuration::seconds(10),
        snapshot_at + ChronoDuration::seconds(15),
        400,
        4.00,
    )
    .await;

    let Json(response) = fetch_prompt_cache_conversations(
        State(state),
        Query(PromptCacheConversationsQuery {
            limit: None,
            activity_hours: None,
            activity_minutes: Some(5),
            page_size: Some(20),
            cursor: None,
            snapshot_at: Some(snapshot_at.to_rfc3339()),
            detail: Some("compact".to_string()),
            recent_invocation_limit: None,

            blocked_binding_upstream_account_id: None,

            blocked_binding_constraint_source: None,
        }),
    )
    .await
    .expect("snapshot working response should exclude late rollup totals");

    let conversation = response
        .conversations
        .iter()
        .find(|conversation| conversation.prompt_cache_key == "working-late-rollup")
        .expect("working conversation should be visible from current-hour row");
    assert_eq!(conversation.request_count, 3);
    assert_eq!(conversation.total_tokens, 140);
    assert!((conversation.total_cost - 1.4).abs() < f64::EPSILON);
}

#[tokio::test]
async fn prompt_cache_conversations_activity_minutes_paginated_snapshot_keeps_hydrated_details_consistent()
 {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let snapshot_at = Utc::now() - ChronoDuration::seconds(20);

    async fn insert_row(
        pool: &Pool<Sqlite>,
        invoke_id: &str,
        occurred_at: DateTime<Utc>,
        key: &str,
        upstream_account_id: Option<i64>,
        upstream_account_name: Option<&str>,
        total_tokens: i64,
        cost: f64,
    ) {
        let mut payload = json!({
            "promptCacheKey": key,
            "routeMode": "pool",
            "model": "gpt-5.4",
        });
        if let Some(upstream_account_id) = upstream_account_id {
            payload["upstreamAccountId"] = json!(upstream_account_id);
        }
        if let Some(upstream_account_name) = upstream_account_name {
            payload["upstreamAccountName"] = json!(upstream_account_name);
        }

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
        .bind(payload.to_string())
        .bind("{}")
        .bind(format_utc_iso_millis(occurred_at))
        .execute(pool)
        .await
        .expect("insert paginated snapshot hydration row");
    }

    insert_row(
        &state.pool,
        "working-snapshot-full-head-pre",
        snapshot_at - ChronoDuration::seconds(5),
        "working-snapshot-full-head",
        Some(11),
        Some("Head"),
        20,
        0.20,
    )
    .await;
    insert_row(
        &state.pool,
        "working-snapshot-full-target-pre",
        snapshot_at - ChronoDuration::seconds(15),
        "working-snapshot-full-target",
        Some(1),
        Some("Alpha"),
        10,
        0.10,
    )
    .await;
    insert_row(
        &state.pool,
        "working-snapshot-full-target-post",
        snapshot_at + ChronoDuration::seconds(5),
        "working-snapshot-full-target",
        Some(2),
        Some("Beta"),
        999,
        9.99,
    )
    .await;

    materialize_prompt_cache_hourly_rollups(&state.pool).await;

    let snapshot_at_rfc3339 = snapshot_at.to_rfc3339();
    let Json(first_page) = fetch_prompt_cache_conversations(
        State(state.clone()),
        Query(PromptCacheConversationsQuery {
            limit: None,
            activity_hours: None,
            activity_minutes: Some(5),
            page_size: Some(1),
            cursor: None,
            snapshot_at: Some(snapshot_at_rfc3339.clone()),
            detail: Some("full".to_string()),
            recent_invocation_limit: None,

            blocked_binding_upstream_account_id: None,

            blocked_binding_constraint_source: None,
        }),
    )
    .await
    .expect("first full snapshot page should succeed");

    assert_eq!(first_page.conversations.len(), 1);
    assert_eq!(
        first_page.conversations[0].prompt_cache_key,
        "working-snapshot-full-target"
    );

    let Json(second_page) = fetch_prompt_cache_conversations(
        State(state),
        Query(PromptCacheConversationsQuery {
            limit: None,
            activity_hours: None,
            activity_minutes: Some(5),
            page_size: Some(1),
            cursor: first_page.next_cursor.clone(),
            snapshot_at: Some(snapshot_at_rfc3339),
            detail: Some("full".to_string()),
            recent_invocation_limit: None,

            blocked_binding_upstream_account_id: None,

            blocked_binding_constraint_source: None,
        }),
    )
    .await
    .expect("second full snapshot page should succeed");

    assert_eq!(second_page.conversations.len(), 1);
    assert_eq!(
        second_page.conversations[0].prompt_cache_key,
        "working-snapshot-full-head"
    );
    let target = &first_page.conversations[0];
    assert_eq!(target.prompt_cache_key, "working-snapshot-full-target");
    assert_eq!(target.request_count, 1);
    assert_eq!(target.total_tokens, 10);
    assert!((target.total_cost - 0.10).abs() < 1e-9);
    assert_eq!(target.recent_invocations.len(), 1);
    assert_eq!(
        target.recent_invocations[0].invoke_id,
        "working-snapshot-full-target-pre"
    );
    assert_eq!(target.last24h_requests.len(), 1);
    assert_eq!(target.last24h_requests[0].request_tokens, 10);
    assert_eq!(target.last24h_requests[0].cumulative_tokens, 10);
    assert_eq!(target.upstream_accounts.len(), 1);
    assert_eq!(target.upstream_accounts[0].upstream_account_id, Some(1));
    assert_eq!(
        target.upstream_accounts[0].upstream_account_name.as_deref(),
        Some("Alpha")
    );
    assert_eq!(target.upstream_accounts[0].request_count, 1);
    assert_eq!(target.upstream_accounts[0].total_tokens, 10);
    assert!((target.upstream_accounts[0].total_cost - 0.10).abs() < 1e-9);
}

#[tokio::test]
async fn prompt_cache_conversations_activity_minutes_paginated_snapshot_full_detail_null_cost_stays_real_zero()
 {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let snapshot_at = Utc::now() - ChronoDuration::seconds(20);

    async fn insert_row(
        pool: &Pool<Sqlite>,
        invoke_id: &str,
        occurred_at: DateTime<Utc>,
        key: &str,
        upstream_account_id: Option<i64>,
        upstream_account_name: Option<&str>,
        total_tokens: i64,
        cost: Option<f64>,
    ) {
        let mut payload = json!({
            "promptCacheKey": key,
            "routeMode": "pool",
            "model": "gpt-5.4",
        });
        if let Some(upstream_account_id) = upstream_account_id {
            payload["upstreamAccountId"] = json!(upstream_account_id);
        }
        if let Some(upstream_account_name) = upstream_account_name {
            payload["upstreamAccountName"] = json!(upstream_account_name);
        }

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
        .bind(payload.to_string())
        .bind("{}")
        .bind(format_utc_iso_millis(occurred_at))
        .execute(pool)
        .await
        .expect("insert null-cost full snapshot row");
    }

    insert_row(
        &state.pool,
        "working-null-cost-full-head-pre",
        snapshot_at - ChronoDuration::seconds(5),
        "working-null-cost-full-head",
        Some(11),
        Some("Head"),
        20,
        Some(0.20),
    )
    .await;
    insert_row(
        &state.pool,
        "working-null-cost-full-target-pre",
        snapshot_at - ChronoDuration::seconds(15),
        "working-null-cost-full-target",
        Some(1),
        Some("Alpha"),
        10,
        None,
    )
    .await;
    insert_row(
        &state.pool,
        "working-null-cost-full-target-post",
        snapshot_at + ChronoDuration::seconds(5),
        "working-null-cost-full-target",
        Some(2),
        Some("Beta"),
        999,
        Some(9.99),
    )
    .await;

    materialize_prompt_cache_hourly_rollups(&state.pool).await;

    let snapshot_at_rfc3339 = snapshot_at.to_rfc3339();
    let Json(first_page) = fetch_prompt_cache_conversations(
        State(state.clone()),
        Query(PromptCacheConversationsQuery {
            limit: None,
            activity_hours: None,
            activity_minutes: Some(5),
            page_size: Some(1),
            cursor: None,
            snapshot_at: Some(snapshot_at_rfc3339.clone()),
            detail: Some("full".to_string()),
            recent_invocation_limit: None,

            blocked_binding_upstream_account_id: None,

            blocked_binding_constraint_source: None,
        }),
    )
    .await
    .expect("first null-cost full snapshot page should succeed");

    let Json(second_page) = fetch_prompt_cache_conversations(
        State(state),
        Query(PromptCacheConversationsQuery {
            limit: None,
            activity_hours: None,
            activity_minutes: Some(5),
            page_size: Some(1),
            cursor: first_page.next_cursor.clone(),
            snapshot_at: Some(snapshot_at_rfc3339),
            detail: Some("full".to_string()),
            recent_invocation_limit: None,

            blocked_binding_upstream_account_id: None,

            blocked_binding_constraint_source: None,
        }),
    )
    .await
    .expect("second null-cost full snapshot page should succeed");

    assert_eq!(first_page.conversations.len(), 1);
    assert_eq!(
        first_page.conversations[0].prompt_cache_key,
        "working-null-cost-full-target"
    );
    assert_eq!(second_page.conversations.len(), 1);
    let target = &first_page.conversations[0];
    assert_eq!(target.prompt_cache_key, "working-null-cost-full-target");
    assert_eq!(target.request_count, 1);
    assert_eq!(target.total_tokens, 10);
    assert_eq!(target.total_cost, 0.0);
    assert_eq!(target.recent_invocations.len(), 1);
    assert_eq!(
        target.recent_invocations[0].invoke_id,
        "working-null-cost-full-target-pre"
    );
    assert_eq!(target.recent_invocations[0].cost, None);
    assert_eq!(target.upstream_accounts.len(), 1);
    assert_eq!(target.upstream_accounts[0].upstream_account_id, Some(1));
    assert_eq!(
        target.upstream_accounts[0].upstream_account_name.as_deref(),
        Some("Alpha")
    );
    assert_eq!(target.upstream_accounts[0].request_count, 1);
    assert_eq!(target.upstream_accounts[0].total_tokens, 10);
    assert_eq!(target.upstream_accounts[0].total_cost, 0.0);
    assert_eq!(second_page.conversations.len(), 1);
    assert_eq!(
        second_page.conversations[0].prompt_cache_key,
        "working-null-cost-full-head"
    );
}
