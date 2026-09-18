struct PromptCacheNullableCostRow<'a> {
    invoke_id: &'a str,
    occurred_at: DateTime<Utc>,
    key: &'a str,
    upstream_account_id: Option<i64>,
    upstream_account_name: Option<&'a str>,
    total_tokens: i64,
    cost: Option<f64>,
}

async fn insert_prompt_cache_row_05_01(pool: &Pool<Sqlite>, row: PromptCacheNullableCostRow<'_>) {
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
    .expect("insert null-cost full snapshot row");
}

async fn insert_prompt_cache_row_05_02(
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

async fn insert_prompt_cache_row_05_03(
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

async fn insert_prompt_cache_row_05_04(
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

struct PromptCacheFailureRow<'a> {
    invoke_id: &'a str,
    occurred_at: DateTime<Utc>,
    status: &'a str,
    error_message: Option<&'a str>,
    failure_kind: Option<&'a str>,
    failure_class: Option<&'a str>,
    key: &'a str,
    total_tokens: i64,
}

async fn insert_prompt_cache_row_05_05(pool: &Pool<Sqlite>, row: PromptCacheFailureRow<'_>) {
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
    .bind(row.invoke_id)
    .bind(format_naive(
        row.occurred_at.with_timezone(&Shanghai).naive_local(),
    ))
    .bind(SOURCE_PROXY)
    .bind(row.status)
    .bind(row.error_message)
    .bind(row.failure_kind)
    .bind(row.failure_class)
    .bind(row.total_tokens)
    .bind(0.01)
    .bind(json!({ "promptCacheKey": row.key }).to_string())
    .bind("{}")
    .execute(pool)
    .await
    .expect("insert invocation row");
}

async fn insert_terminal_prompt_cache_row(
    pool: &Pool<Sqlite>,
    invoke_id: &str,
    occurred_at: DateTime<Utc>,
    created_at: DateTime<Utc>,
    key: &str,
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
    .bind(10)
    .bind(0.01_f64)
    .bind(json!({ "promptCacheKey": key, "routeMode": "pool" }).to_string())
    .bind("{}")
    .bind(format_utc_iso_precise(created_at))
    .execute(pool)
    .await
    .expect("insert terminal prompt cache invocation row");
}

fn interrupted_prompt_cache_runtime_row(
    invoke_id: &str,
    occurred_at: &str,
    key: &str,
) -> ApiInvocation {
    let record = build_running_proxy_capture_record(RunningProxyCaptureRecordRequest(
        invoke_id,
        occurred_at,
        ProxyCaptureTarget::Responses,
        &RequestCaptureInfo {
            model: Some("gpt-5.5".to_string()),
            prompt_cache_key: Some(key.to_string()),
            ..RequestCaptureInfo::default()
        },
        Some("198.51.100.42"),
        None,
        Some(key),
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
    ));
    let mut invocation = api_invocation_from_runtime_record(&record);
    invocation.status = Some("interrupted".to_string());
    invocation
}

async fn seed_stale_terminal_runtime_rows(state: &AppState, snapshot_at: DateTime<Utc>) {
    insert_terminal_prompt_cache_row(
        &state.pool,
        "working-runtime-window-recent",
        snapshot_at - ChronoDuration::minutes(1),
        snapshot_at - ChronoDuration::seconds(1),
        "working-runtime-window-recent",
    )
    .await;

    let boundary_at = format_naive(
        (snapshot_at - ChronoDuration::minutes(5))
            .with_timezone(&Shanghai)
            .naive_local(),
    );
    state
        .proxy_runtime_invocations
        .upsert_terminal(interrupted_prompt_cache_runtime_row(
            "working-runtime-window-boundary-terminal-invoke",
            &boundary_at,
            "working-runtime-window-boundary-terminal",
        ));

    let stale_at = format_naive(
        (snapshot_at - ChronoDuration::minutes(15))
            .with_timezone(&Shanghai)
            .naive_local(),
    );
    state
        .proxy_runtime_invocations
        .upsert_terminal(interrupted_prompt_cache_runtime_row(
            "working-runtime-window-stale-terminal-invoke",
            &stale_at,
            "working-runtime-window-stale-terminal",
        ));
}

async fn seed_memory_overlay_rows(state: &AppState, now: DateTime<Utc>) {
    for index in 0..2 {
        insert_prompt_cache_row_05_02(
            &state.pool,
            &format!("memory-overlay-terminal-{}", index + 1),
            now - ChronoDuration::minutes(index + 1),
            &format!("memory-overlay-terminal-{}", index + 1),
            "success",
        )
        .await;
    }
    insert_prompt_cache_row_05_02(
        &state.pool,
        "memory-overlay-running-3-old-terminal",
        now - ChronoDuration::minutes(20),
        "memory-overlay-running-3",
        "success",
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
        let running_record = build_running_proxy_capture_record(RunningProxyCaptureRecordRequest(
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
        ));
        state
            .proxy_runtime_invocations
            .upsert(api_invocation_from_runtime_record(&running_record));
    }
}

fn assert_null_cost_full_detail(
    first_page: &PromptCacheConversationsResponse,
    second_page: &PromptCacheConversationsResponse,
) {
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
    assert_eq!(
        second_page.conversations[0].prompt_cache_key,
        "working-null-cost-full-head"
    );
}

#[tokio::test]
pub(crate) async fn prompt_cache_conversations_activity_minutes_paginated_snapshot_full_detail_null_cost_stays_real_zero()
 {
    let state = prompt_cache_test_state().await;
    let snapshot_at = Utc::now() - ChronoDuration::seconds(20);

    insert_prompt_cache_row_05_01(
        &state.pool,
        PromptCacheNullableCostRow {
            invoke_id: "working-null-cost-full-head-pre",
            occurred_at: snapshot_at - ChronoDuration::seconds(5),
            key: "working-null-cost-full-head",
            upstream_account_id: Some(11),
            upstream_account_name: Some("Head"),
            total_tokens: 20,
            cost: Some(0.20),
        },
    )
    .await;
    insert_prompt_cache_row_05_01(
        &state.pool,
        PromptCacheNullableCostRow {
            invoke_id: "working-null-cost-full-target-pre",
            occurred_at: snapshot_at - ChronoDuration::seconds(15),
            key: "working-null-cost-full-target",
            upstream_account_id: Some(1),
            upstream_account_name: Some("Alpha"),
            total_tokens: 10,
            cost: None,
        },
    )
    .await;
    insert_prompt_cache_row_05_01(
        &state.pool,
        PromptCacheNullableCostRow {
            invoke_id: "working-null-cost-full-target-post",
            occurred_at: snapshot_at + ChronoDuration::seconds(5),
            key: "working-null-cost-full-target",
            upstream_account_id: Some(2),
            upstream_account_name: Some("Beta"),
            total_tokens: 999,
            cost: Some(9.99),
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
    .expect("first null-cost full snapshot page should succeed");

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
    .expect("second null-cost full snapshot page should succeed");

    assert_null_cost_full_detail(&first_page, &second_page);
}

#[tokio::test]
pub(crate) async fn prompt_cache_conversations_activity_minutes_paginated_keeps_running_and_pending_working_rows()
 {
    let state = prompt_cache_test_state().await;
    let now = Utc::now();

    insert_prompt_cache_row_05_02(
        &state.pool,
        "working-recent-terminal",
        now - ChronoDuration::minutes(4),
        "working-recent-terminal",
        "success",
    )
    .await;
    insert_prompt_cache_row_05_02(
        &state.pool,
        "working-running-terminal-old",
        now - ChronoDuration::minutes(12),
        "working-running",
        "success",
    )
    .await;
    insert_prompt_cache_row_05_02(
        &state.pool,
        "working-running-live",
        now - ChronoDuration::minutes(1),
        "working-running",
        "running",
    )
    .await;
    insert_prompt_cache_row_05_02(
        &state.pool,
        "working-pending-live",
        now - ChronoDuration::minutes(2),
        "working-pending",
        "pending",
    )
    .await;
    insert_prompt_cache_row_05_02(
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
            activity_minutes: Some(5),
            page_size: Some(10),
            detail: Some("compact".to_string()),

            ..prompt_cache_query()
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
pub(crate) async fn prompt_cache_conversations_activity_minutes_paginated_overlays_memory_running_rows()
 {
    let state = prompt_cache_test_state().await;
    let now = Utc::now();

    seed_memory_overlay_rows(state.as_ref(), now).await;

    let Json(response) = fetch_prompt_cache_conversations(
        State(state),
        Query(PromptCacheConversationsQuery {
            activity_minutes: Some(5),
            page_size: Some(20),
            detail: Some("compact".to_string()),

            ..prompt_cache_query()
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

#[tokio::test]
pub(crate) async fn prompt_cache_conversations_activity_minutes_paginated_ignores_stale_terminal_runtime_rows()
 {
    let state = prompt_cache_test_state().await;
    let snapshot_at = Utc
        .timestamp_opt(Utc::now().timestamp(), 500_000_000)
        .single()
        .expect("valid snapshot timestamp");

    seed_stale_terminal_runtime_rows(state.as_ref(), snapshot_at).await;

    let Json(response) = fetch_prompt_cache_conversations(
        State(state),
        Query(PromptCacheConversationsQuery {
            activity_minutes: Some(5),
            page_size: Some(20),
            snapshot_at: Some(format_utc_iso_precise(snapshot_at)),
            detail: Some("compact".to_string()),

            ..prompt_cache_query()
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
pub(crate) async fn prompt_cache_conversations_activity_minutes_paginated_sorts_by_newer_in_flight_anchor()
 {
    let state = prompt_cache_test_state().await;
    let now = Utc::now();

    insert_prompt_cache_row_05_03(
        &state.pool,
        "working-mixed-terminal",
        now - ChronoDuration::minutes(4),
        "working-mixed",
        "success",
    )
    .await;
    insert_prompt_cache_row_05_03(
        &state.pool,
        "working-mixed-running",
        now - ChronoDuration::minutes(1),
        "working-mixed",
        "running",
    )
    .await;
    insert_prompt_cache_row_05_03(
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
            activity_minutes: Some(5),
            page_size: Some(10),
            detail: Some("compact".to_string()),

            ..prompt_cache_query()
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
pub(crate) async fn prompt_cache_conversations_chart_window_caps_history_to_recent_24_hours() {
    let state = prompt_cache_test_state().await;
    let now = Utc::now();

    insert_prompt_cache_row_05_04(
        &state.pool,
        "chart-cap-history",
        now - ChronoDuration::hours(50),
        "chart-cap",
        90,
    )
    .await;
    insert_prompt_cache_row_05_04(
        &state.pool,
        "chart-cap-recent-a",
        now - ChronoDuration::hours(2),
        "chart-cap",
        30,
    )
    .await;
    insert_prompt_cache_row_05_04(
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
            activity_hours: Some(1),

            ..prompt_cache_query()
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
pub(crate) async fn prompt_cache_conversation_activity_keeps_http_200_errors_out_of_success_points()
{
    let state = prompt_cache_test_state().await;
    let now = Utc::now();

    insert_prompt_cache_row_05_05(
        &state.pool,
        PromptCacheFailureRow {
            invoke_id: "http-200-success-like",
            occurred_at: now - ChronoDuration::minutes(10),
            status: "http_200",
            error_message: None,
            failure_kind: None,
            failure_class: None,
            key: "http-200-activity",
            total_tokens: 10,
        },
    )
    .await;
    insert_prompt_cache_row_05_05(
        &state.pool,
        PromptCacheFailureRow {
            invoke_id: "http-200-failure-like",
            occurred_at: now - ChronoDuration::minutes(5),
            status: "http_200",
            error_message: Some("upstream parse failed"),
            failure_kind: None,
            failure_class: None,
            key: "http-200-activity",
            total_tokens: 12,
        },
    )
    .await;
    insert_prompt_cache_row_05_05(
        &state.pool,
        PromptCacheFailureRow {
            invoke_id: "http-200-structured-failure",
            occurred_at: now - ChronoDuration::minutes(1),
            status: "http_200",
            error_message: None,
            failure_kind: Some("upstream_response_failed"),
            failure_class: Some("service_failure"),
            key: "http-200-activity",
            total_tokens: 14,
        },
    )
    .await;

    materialize_prompt_cache_hourly_rollups(&state.pool).await;

    let Json(response) = fetch_prompt_cache_conversations(
        State(state),
        Query(PromptCacheConversationsQuery {
            limit: Some(20),

            ..prompt_cache_query()
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

use super::*;
