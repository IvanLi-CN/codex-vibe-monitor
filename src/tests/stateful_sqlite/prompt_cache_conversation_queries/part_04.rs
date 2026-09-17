async fn insert_prompt_cache_row_04_01(
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

async fn insert_prompt_cache_row_04_02(
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

async fn insert_prompt_cache_row_04_03(
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

async fn insert_prompt_cache_row_04_04(
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

async fn insert_prompt_cache_row_04_05(
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

async fn insert_prompt_cache_row_04_06(
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

struct PromptCacheSnapshotAccountRow<'a> {
    invoke_id: &'a str,
    occurred_at: DateTime<Utc>,
    key: &'a str,
    upstream_account_id: Option<i64>,
    upstream_account_name: Option<&'a str>,
    total_tokens: i64,
    cost: f64,
}

async fn seed_late_backfilled_rollup_case(
    pool: &Pool<Sqlite>,
    snapshot_hour_start: DateTime<Utc>,
    snapshot_at: DateTime<Utc>,
) {
    let archived_rollup_at = snapshot_hour_start - ChronoDuration::hours(2);
    let old_hour_at = snapshot_hour_start - ChronoDuration::minutes(15);
    let current_working_at = snapshot_at - ChronoDuration::minutes(1);

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
    .execute(pool)
    .await
    .expect("insert archived-only prompt-cache rollup row");

    insert_prompt_cache_row_04_06(
        pool,
        "working-late-rollup-created-after-snapshot",
        old_hour_at + ChronoDuration::seconds(2),
        snapshot_at + ChronoDuration::seconds(10),
        500,
        5.00,
    )
    .await;
    insert_prompt_cache_row_04_06(
        pool,
        "working-late-rollup-old",
        old_hour_at,
        old_hour_at,
        30,
        0.30,
    )
    .await;
    insert_prompt_cache_row_04_06(
        pool,
        "working-late-rollup-current",
        current_working_at,
        current_working_at,
        70,
        0.70,
    )
    .await;
    materialize_prompt_cache_hourly_rollups(pool).await;

    insert_prompt_cache_row_04_06(
        pool,
        "working-late-rollup-backfilled",
        old_hour_at + ChronoDuration::seconds(5),
        snapshot_at + ChronoDuration::seconds(5),
        900,
        9.00,
    )
    .await;
    materialize_prompt_cache_hourly_rollups(pool).await;

    insert_prompt_cache_row_04_06(
        pool,
        "working-late-rollup-unmaterialized-backfill",
        archived_rollup_at + ChronoDuration::seconds(10),
        snapshot_at + ChronoDuration::seconds(15),
        400,
        4.00,
    )
    .await;
}

fn assert_hydrated_snapshot_target(target: &PromptCacheConversationResponse) {
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

async fn insert_prompt_cache_row_04_07(
    pool: &Pool<Sqlite>,
    row: PromptCacheSnapshotAccountRow<'_>,
) {
    let mut payload = json!({
        "promptCacheKey": row.key,
        "routeMode": "pool",
        "model": "gpt-5.4",
    });
    if let Some(upstream_account_id) = row.upstream_account_id {
        payload["upstreamAccountId"] = json!(upstream_account_id);
    }
    if let Some(upstream_account_name) = row.upstream_account_name {
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
    .bind(row.invoke_id)
    .bind(format_naive(
        row.occurred_at.with_timezone(&Shanghai).naive_local(),
    ))
    .bind(SOURCE_PROXY)
    .bind("success")
    .bind(row.total_tokens)
    .bind(row.cost)
    .bind(payload.to_string())
    .bind("{}")
    .bind(format_utc_iso_millis(row.occurred_at))
    .execute(pool)
    .await
    .expect("insert paginated snapshot hydration row");
}

#[tokio::test]
pub(crate) async fn prompt_cache_conversations_activity_minutes_paginated_snapshot_excludes_same_second_post_snapshot_writes()
 {
    let state = prompt_cache_test_state().await;
    let snapshot_second = Utc::now() - ChronoDuration::seconds(20);
    let requested_snapshot_at = snapshot_second + ChronoDuration::milliseconds(123);

    insert_prompt_cache_row_04_01(
        &state.pool,
        "working-same-second-head",
        snapshot_second - ChronoDuration::seconds(5),
        snapshot_second - ChronoDuration::seconds(5),
        "working-same-second-head",
        20,
    )
    .await;
    insert_prompt_cache_row_04_01(
        &state.pool,
        "working-same-second-tail",
        snapshot_second - ChronoDuration::seconds(15),
        snapshot_second - ChronoDuration::seconds(15),
        "working-same-second-tail",
        10,
    )
    .await;
    insert_prompt_cache_row_04_01(
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
            activity_minutes: Some(5),
            page_size: Some(1),
            snapshot_at: Some(requested_snapshot_at.to_rfc3339()),
            detail: Some("compact".to_string()),

            ..prompt_cache_query()
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

    insert_prompt_cache_row_04_01(
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
            activity_minutes: Some(5),
            page_size: Some(1),
            cursor: first_page.next_cursor.clone(),
            snapshot_at: first_page.snapshot_at.clone(),
            detail: Some("compact".to_string()),

            ..prompt_cache_query()
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
pub(crate) async fn prompt_cache_conversations_activity_minutes_paginated_whole_second_snapshot_excludes_post_snapshot_writes()
 {
    let state = prompt_cache_test_state().await;
    let snapshot_second = Utc::now() - ChronoDuration::seconds(20);

    insert_prompt_cache_row_04_02(
        &state.pool,
        "working-whole-second-head",
        snapshot_second,
        snapshot_second,
        "working-whole-second-head",
        20,
    )
    .await;
    insert_prompt_cache_row_04_02(
        &state.pool,
        "working-whole-second-tail",
        snapshot_second - ChronoDuration::seconds(15),
        snapshot_second - ChronoDuration::seconds(15),
        "working-whole-second-tail",
        10,
    )
    .await;
    insert_prompt_cache_row_04_02(
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
            activity_minutes: Some(5),
            page_size: Some(1),
            snapshot_at: Some(format_utc_iso(snapshot_second)),
            detail: Some("compact".to_string()),

            ..prompt_cache_query()
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

    insert_prompt_cache_row_04_02(
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
            activity_minutes: Some(5),
            page_size: Some(1),
            cursor: first_page.next_cursor.clone(),
            snapshot_at: first_page.snapshot_at.clone(),
            detail: Some("compact".to_string()),

            ..prompt_cache_query()
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
pub(crate) async fn prompt_cache_conversations_activity_minutes_paginated_snapshot_excludes_late_persisted_pre_snapshot_occurrence()
 {
    let state = prompt_cache_test_state().await;
    let snapshot_second = Utc::now() - ChronoDuration::seconds(20);
    let requested_snapshot_at = snapshot_second + ChronoDuration::milliseconds(123);

    insert_prompt_cache_row_04_03(
        &state.pool,
        "working-late-persist-head",
        snapshot_second - ChronoDuration::seconds(5),
        snapshot_second - ChronoDuration::seconds(5),
        "working-late-persist-head",
        20,
    )
    .await;
    insert_prompt_cache_row_04_03(
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
            activity_minutes: Some(5),
            page_size: Some(1),
            snapshot_at: Some(requested_snapshot_at.to_rfc3339()),
            detail: Some("compact".to_string()),

            ..prompt_cache_query()
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

    insert_prompt_cache_row_04_03(
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
            activity_minutes: Some(5),
            page_size: Some(1),
            cursor: first_page.next_cursor.clone(),
            snapshot_at: first_page.snapshot_at.clone(),
            detail: Some("compact".to_string()),

            ..prompt_cache_query()
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
pub(crate) async fn prompt_cache_conversations_activity_minutes_paginated_snapshot_preserves_previous_hour_lifetime_totals()
 {
    let state = prompt_cache_test_state().await;
    let snapshot_at = Utc::now() - ChronoDuration::seconds(20);

    insert_prompt_cache_row_04_04(
        &state.pool,
        "working-window-head",
        snapshot_at - ChronoDuration::seconds(10),
        "working-window-head",
        20,
        0.20,
    )
    .await;
    insert_prompt_cache_row_04_04(
        &state.pool,
        "working-window-target-stale",
        snapshot_at - ChronoDuration::minutes(33),
        "working-window-target",
        999,
        9.99,
    )
    .await;
    insert_prompt_cache_row_04_04(
        &state.pool,
        "working-window-target-pre",
        snapshot_at - ChronoDuration::minutes(4),
        "working-window-target",
        10,
        0.10,
    )
    .await;
    insert_prompt_cache_row_04_04(
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
            activity_minutes: Some(5),
            page_size: Some(1),
            snapshot_at: Some(snapshot_at_rfc3339.clone()),
            detail: Some("compact".to_string()),

            ..prompt_cache_query()
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
            activity_minutes: Some(5),
            page_size: Some(1),
            cursor: first_page.next_cursor.clone(),
            snapshot_at: Some(snapshot_at_rfc3339),
            detail: Some("compact".to_string()),

            ..prompt_cache_query()
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
pub(crate) async fn prompt_cache_conversations_activity_minutes_paginated_snapshot_includes_unmaterialized_previous_hour_lifecycle_tail()
 {
    let state = prompt_cache_test_state().await;
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

    insert_prompt_cache_row_04_05(
        &state.pool,
        "working-lagging-rollup-old-tail",
        old_tail_at,
        30,
        0.30,
    )
    .await;
    insert_prompt_cache_row_04_05(
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
            activity_minutes: Some(5),
            page_size: Some(20),
            snapshot_at: Some(snapshot_at.to_rfc3339()),
            detail: Some("compact".to_string()),

            ..prompt_cache_query()
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
pub(crate) async fn prompt_cache_conversations_activity_minutes_paginated_snapshot_excludes_late_backfilled_rollup_lifecycle_totals()
 {
    let state = prompt_cache_test_state().await;
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
    seed_late_backfilled_rollup_case(&state.pool, snapshot_hour_start, snapshot_at).await;

    let Json(response) = fetch_prompt_cache_conversations(
        State(state),
        Query(PromptCacheConversationsQuery {
            activity_minutes: Some(5),
            page_size: Some(20),
            snapshot_at: Some(snapshot_at.to_rfc3339()),
            detail: Some("compact".to_string()),

            ..prompt_cache_query()
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
pub(crate) async fn prompt_cache_conversations_activity_minutes_paginated_snapshot_keeps_hydrated_details_consistent()
 {
    let state = prompt_cache_test_state().await;
    let snapshot_at = Utc::now() - ChronoDuration::seconds(20);

    insert_prompt_cache_row_04_07(
        &state.pool,
        PromptCacheSnapshotAccountRow {
            invoke_id: "working-snapshot-full-head-pre",
            occurred_at: snapshot_at - ChronoDuration::seconds(5),
            key: "working-snapshot-full-head",
            upstream_account_id: Some(11),
            upstream_account_name: Some("Head"),
            total_tokens: 20,
            cost: 0.20,
        },
    )
    .await;
    insert_prompt_cache_row_04_07(
        &state.pool,
        PromptCacheSnapshotAccountRow {
            invoke_id: "working-snapshot-full-target-pre",
            occurred_at: snapshot_at - ChronoDuration::seconds(15),
            key: "working-snapshot-full-target",
            upstream_account_id: Some(1),
            upstream_account_name: Some("Alpha"),
            total_tokens: 10,
            cost: 0.10,
        },
    )
    .await;
    insert_prompt_cache_row_04_07(
        &state.pool,
        PromptCacheSnapshotAccountRow {
            invoke_id: "working-snapshot-full-target-post",
            occurred_at: snapshot_at + ChronoDuration::seconds(5),
            key: "working-snapshot-full-target",
            upstream_account_id: Some(2),
            upstream_account_name: Some("Beta"),
            total_tokens: 999,
            cost: 9.99,
        },
    )
    .await;

    materialize_prompt_cache_hourly_rollups(&state.pool).await;

    let snapshot_at_rfc3339 = snapshot_at.to_rfc3339();
    let Json(first_page) = fetch_prompt_cache_conversations(
        State(state.clone()),
        Query(PromptCacheConversationsQuery {
            activity_minutes: Some(5),
            page_size: Some(1),
            snapshot_at: Some(snapshot_at_rfc3339.clone()),
            detail: Some("full".to_string()),

            ..prompt_cache_query()
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
            activity_minutes: Some(5),
            page_size: Some(1),
            cursor: first_page.next_cursor.clone(),
            snapshot_at: Some(snapshot_at_rfc3339),
            detail: Some("full".to_string()),

            ..prompt_cache_query()
        }),
    )
    .await
    .expect("second full snapshot page should succeed");

    assert_eq!(second_page.conversations.len(), 1);
    assert_eq!(
        second_page.conversations[0].prompt_cache_key,
        "working-snapshot-full-head"
    );
    assert_hydrated_snapshot_target(&first_page.conversations[0]);
}

use super::*;
