async fn insert_prompt_cache_row_02_01(
    pool: &Pool<Sqlite>,
    invoke_id: &str,
    occurred_at: DateTime<Utc>,
    source: &str,
    key: &str,
) -> i64 {
    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id, occurred_at, source, status, model, total_tokens, cost, payload, raw_response
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
        "#,
    )
    .bind(invoke_id)
    .bind(format_naive(
        occurred_at.with_timezone(&Shanghai).naive_local(),
    ))
    .bind(source)
    .bind("success")
    .bind("gpt-5.4")
    .bind(10_i64)
    .bind(0.01_f64)
    .bind(
        json!({
            "promptCacheKey": key,
            "routeMode": "pool",
            "endpoint": "/v1/responses",
            "model": "gpt-5.4",
        })
        .to_string(),
    )
    .bind("{}")
    .execute(pool)
    .await
    .expect("insert recent invocation row")
    .last_insert_rowid()
}

struct PromptCacheAccountRow<'a> {
    invoke_id: &'a str,
    occurred_at: DateTime<Utc>,
    key: &'a str,
    account_id: Option<i64>,
    account_name: Option<&'a str>,
    total_tokens: i64,
    cost: f64,
}

async fn insert_prompt_cache_row_02_02(pool: &Pool<Sqlite>, row: PromptCacheAccountRow<'_>) {
    let mut payload = json!({ "promptCacheKey": row.key });
    if let Some(account_id) = row.account_id {
        payload["upstreamAccountId"] = json!(account_id);
    }
    if let Some(account_name) = row.account_name {
        payload["upstreamAccountName"] = json!(account_name);
    }
    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
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
    .execute(pool)
    .await
    .expect("insert invocation row");
}

async fn insert_prompt_cache_row_02_03(
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

async fn insert_prompt_cache_row_02_04(
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

async fn insert_prompt_cache_row_02_05(
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

async fn insert_prompt_cache_row_02_06(
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

async fn assert_all_scope_recent_rows(
    pool: &Pool<Sqlite>,
    snapshot: &PromptCacheConversationHydrationSnapshot<'_>,
) {
    let rows = query_prompt_cache_conversation_recent_invocations(
        pool,
        InvocationSourceScope::All,
        &["recent-beta".to_string(), "recent-alpha".to_string()],
        2,
        Some(snapshot),
    )
    .await
    .expect("all-scope recent preview query should succeed");
    assert_eq!(
        rows.iter()
            .map(|row| (row.prompt_cache_key.as_str(), row.invoke_id.as_str()))
            .collect::<Vec<_>>(),
        vec![
            ("recent-alpha", "recent-alpha-boundary"),
            ("recent-alpha", "recent-alpha-older"),
            ("recent-beta", "recent-beta-xy"),
            ("recent-beta", "recent-beta-proxy-new"),
        ]
    );
}

async fn assert_proxy_scope_recent_rows(
    pool: &Pool<Sqlite>,
    snapshot: &PromptCacheConversationHydrationSnapshot<'_>,
) {
    let rows = query_prompt_cache_conversation_recent_invocations(
        pool,
        InvocationSourceScope::ProxyOnly,
        &["recent-beta".to_string(), "recent-alpha".to_string()],
        2,
        Some(snapshot),
    )
    .await
    .expect("proxy-only recent preview query should succeed");
    assert_eq!(
        rows.iter()
            .map(|row| (row.prompt_cache_key.as_str(), row.invoke_id.as_str()))
            .collect::<Vec<_>>(),
        vec![
            ("recent-alpha", "recent-alpha-boundary"),
            ("recent-alpha", "recent-alpha-older"),
            ("recent-beta", "recent-beta-proxy-new"),
            ("recent-beta", "recent-beta-proxy-old"),
        ]
    );
}

#[tokio::test]
pub(crate) async fn prompt_cache_recent_invocations_keep_per_key_limits_for_snapshot_and_proxy_scope()
 {
    let state = prompt_cache_test_state().await;
    let current_hour_start = Utc
        .timestamp_opt(align_bucket_epoch(Utc::now().timestamp(), 3_600, 0), 0)
        .single()
        .expect("current hour start should be valid");
    let snapshot_second = current_hour_start + ChronoDuration::minutes(20);
    let requested_snapshot_at = snapshot_second + ChronoDuration::milliseconds(123);

    let _alpha_older = insert_prompt_cache_row_02_01(
        &state.pool,
        "recent-alpha-older",
        snapshot_second - ChronoDuration::seconds(30),
        SOURCE_PROXY,
        "recent-alpha",
    )
    .await;
    let alpha_boundary_id = insert_prompt_cache_row_02_01(
        &state.pool,
        "recent-alpha-boundary",
        snapshot_second,
        SOURCE_PROXY,
        "recent-alpha",
    )
    .await;
    let _alpha_late_same_second = insert_prompt_cache_row_02_01(
        &state.pool,
        "recent-alpha-late-same-second",
        snapshot_second,
        SOURCE_PROXY,
        "recent-alpha",
    )
    .await;
    let _alpha_post_snapshot = insert_prompt_cache_row_02_01(
        &state.pool,
        "recent-alpha-post-snapshot",
        snapshot_second + ChronoDuration::seconds(2),
        SOURCE_PROXY,
        "recent-alpha",
    )
    .await;
    let _beta_proxy_old = insert_prompt_cache_row_02_01(
        &state.pool,
        "recent-beta-proxy-old",
        snapshot_second - ChronoDuration::seconds(15),
        SOURCE_PROXY,
        "recent-beta",
    )
    .await;
    let _beta_proxy_new = insert_prompt_cache_row_02_01(
        &state.pool,
        "recent-beta-proxy-new",
        snapshot_second - ChronoDuration::seconds(3),
        SOURCE_PROXY,
        "recent-beta",
    )
    .await;
    let _beta_xy = insert_prompt_cache_row_02_01(
        &state.pool,
        "recent-beta-xy",
        snapshot_second - ChronoDuration::seconds(1),
        SOURCE_XY,
        "recent-beta",
    )
    .await;

    let snapshot_upper_bound = db_occurred_at_upper_bound(requested_snapshot_at);
    let snapshot_hour_start_bound = format_utc_iso(current_hour_start);
    let snapshot = PromptCacheConversationHydrationSnapshot {
        snapshot_upper_bound: snapshot_upper_bound.as_str(),
        snapshot_created_at_upper_bound: None,
        snapshot_hour_start_epoch: current_hour_start.timestamp(),
        snapshot_hour_start_bound: snapshot_hour_start_bound.as_str(),
        snapshot_boundary_row_id_ceiling: Some(alpha_boundary_id),
    };

    assert_all_scope_recent_rows(&state.pool, &snapshot).await;
    assert_proxy_scope_recent_rows(&state.pool, &snapshot).await;
}

#[tokio::test]
pub(crate) async fn prompt_cache_conversations_preserve_upstream_account_history_after_raw_rows_are_removed()
 {
    let state = prompt_cache_test_state().await;
    let now = Utc::now();

    insert_prompt_cache_row_02_02(
        &state.pool,
        PromptCacheAccountRow {
            invoke_id: "pck-upstream-history-beta",
            occurred_at: now - ChronoDuration::hours(48),
            key: "pck-upstream-history",
            account_id: None,
            account_name: Some("Beta"),
            total_tokens: 40,
            cost: 0.4,
        },
    )
    .await;
    insert_prompt_cache_row_02_02(
        &state.pool,
        PromptCacheAccountRow {
            invoke_id: "pck-upstream-recent-beta",
            occurred_at: now - ChronoDuration::hours(2),
            key: "pck-upstream-history",
            account_id: Some(2),
            account_name: Some("Beta"),
            total_tokens: 15,
            cost: 0.15,
        },
    )
    .await;

    ensure_hourly_rollups_caught_up(state.as_ref())
        .await
        .expect("hourly rollups should catch up before raw rows are removed");

    sqlx::query("DELETE FROM codex_invocations WHERE invoke_id = ?1")
        .bind("pck-upstream-history-beta")
        .execute(&state.pool)
        .await
        .expect("delete archived-equivalent raw row");

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
        .find(|item| item.prompt_cache_key == "pck-upstream-history")
        .expect("conversation should survive raw-row removal through hourly rollups");

    let beta = conversation
        .upstream_accounts
        .iter()
        .find(|account| account.upstream_account_id == Some(2))
        .expect("beta account should preserve historical totals");
    assert_eq!(beta.upstream_account_name.as_deref(), Some("Beta"));
    assert_eq!(beta.request_count, 2);
    assert_eq!(beta.total_tokens, 55);
    assert!((beta.total_cost - 0.55).abs() < 1e-9);
}

#[tokio::test]
pub(crate) async fn prompt_cache_conversations_keep_totals_when_recent_preview_is_empty() {
    let state = prompt_cache_test_state().await;
    let now = Utc::now();

    for (invoke_id, minutes_ago, total_tokens, cost) in [
        ("preview-empty-1", 130, 120, 0.12),
        ("preview-empty-2", 70, 180, 0.18),
    ] {
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
            (now - ChronoDuration::minutes(minutes_ago))
                .with_timezone(&Shanghai)
                .naive_local(),
        ))
        .bind(SOURCE_PROXY)
        .bind("success")
        .bind(total_tokens)
        .bind(cost)
        .bind(json!({ "promptCacheKey": "pck-preview-empty" }).to_string())
        .bind("{}")
        .execute(&state.pool)
        .await
        .expect("insert prompt cache invocation row");
    }

    ensure_hourly_rollups_caught_up(state.as_ref())
        .await
        .expect("hourly rollups should catch up before raw rows are removed");

    sqlx::query("DELETE FROM codex_invocations WHERE invoke_id IN (?1, ?2)")
        .bind("preview-empty-1")
        .bind("preview-empty-2")
        .execute(&state.pool)
        .await
        .expect("delete raw rows after hourly rollup catch-up");

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
        .find(|item| item.prompt_cache_key == "pck-preview-empty")
        .expect("conversation should survive through hourly rollups");

    assert_eq!(conversation.request_count, 2);
    assert_eq!(conversation.total_tokens, 300);
    assert!((conversation.total_cost - 0.30).abs() < 1e-9);
    assert!(conversation.recent_invocations.is_empty());
}

#[tokio::test]
pub(crate) async fn prompt_cache_conversations_count_mode_reports_inactive_recent_history_filter() {
    let state = prompt_cache_test_state().await;
    let now = Utc::now();

    for index in 0..19 {
        insert_prompt_cache_row_02_03(
            &state.pool,
            &format!("count-active-{index}"),
            now - ChronoDuration::hours(23) + ChronoDuration::minutes(index as i64),
            &format!("count-active-{index}"),
        )
        .await;
    }
    insert_prompt_cache_row_02_03(
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

            ..prompt_cache_query()
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
pub(crate) async fn prompt_cache_conversations_count_mode_reports_all_skipped_newer_inactive_rows()
{
    let state = prompt_cache_test_state().await;
    let now = Utc::now();

    for index in 0..25 {
        insert_prompt_cache_row_02_04(
            &state.pool,
            &format!("count-inactive-{index}"),
            now - ChronoDuration::hours(25) + ChronoDuration::minutes(index as i64),
            &format!("count-inactive-{index}"),
        )
        .await;
    }

    for index in 0..20 {
        insert_prompt_cache_row_02_04(
            &state.pool,
            &format!("count-active-{index}-history"),
            now - ChronoDuration::days(4) + ChronoDuration::minutes(index as i64),
            &format!("count-active-{index}"),
        )
        .await;
        insert_prompt_cache_row_02_04(
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

            ..prompt_cache_query()
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
pub(crate) async fn prompt_cache_conversations_count_mode_clamps_sparse_inactive_hidden_rows_to_top_n_window()
 {
    let state = prompt_cache_test_state().await;
    let now = Utc::now();

    for index in 0..25 {
        insert_prompt_cache_row_02_05(
            &state.pool,
            &format!("sparse-inactive-{index}"),
            now - ChronoDuration::hours(25) + ChronoDuration::minutes(index as i64),
            &format!("sparse-inactive-{index}"),
        )
        .await;
    }

    insert_prompt_cache_row_02_05(
        &state.pool,
        "sparse-active-history",
        now - ChronoDuration::days(4),
        "sparse-active",
    )
    .await;
    insert_prompt_cache_row_02_05(
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

            ..prompt_cache_query()
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
pub(crate) async fn prompt_cache_conversations_activity_window_caps_results_to_fifty() {
    let state = prompt_cache_test_state().await;
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
            activity_hours: Some(3),

            ..prompt_cache_query()
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
pub(crate) async fn prompt_cache_conversations_activity_minutes_legacy_path_still_caps_results_to_fifty()
 {
    let state = prompt_cache_test_state().await;
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
            activity_minutes: Some(5),

            ..prompt_cache_query()
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
pub(crate) async fn prompt_cache_conversations_activity_minutes_include_running_only_rows_and_report_selected_minutes()
 {
    let state = prompt_cache_test_state().await;
    let now = Utc::now();

    insert_prompt_cache_row_02_06(
        &state.pool,
        "pck-terminal-early",
        now - ChronoDuration::minutes(4),
        "pck-terminal-early",
        "success",
        100,
    )
    .await;
    insert_prompt_cache_row_02_06(
        &state.pool,
        "pck-running-old-terminal",
        now - ChronoDuration::minutes(12),
        "pck-running",
        "success",
        120,
    )
    .await;
    insert_prompt_cache_row_02_06(
        &state.pool,
        "pck-running-live",
        now - ChronoDuration::minutes(1),
        "pck-running",
        "running",
        140,
    )
    .await;
    insert_prompt_cache_row_02_06(
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
            activity_minutes: Some(5),

            ..prompt_cache_query()
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

use super::*;
