use super::*;
use serde_json::json;

#[tokio::test]
async fn build_account_sticky_keys_response_keeps_attached_keys_without_recent_activity() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let account_id = insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;

    sqlx::query(
        r#"
        INSERT INTO pool_sticky_routes (sticky_key, account_id, created_at, updated_at, last_seen_at)
        VALUES (?1, ?2, ?3, ?4, ?5)
        "#,
    )
    .bind("sticky-stale")
    .bind(account_id)
    .bind("2026-03-10 00:00:00")
    .bind("2026-03-10 00:00:00")
    .bind("2026-03-10 00:00:00")
    .execute(&state.pool)
    .await
    .expect("insert sticky route");

    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
        "#,
    )
    .bind("sticky-stale-invoke")
    .bind("2026-03-10 00:00:00")
    .bind(SOURCE_PROXY)
    .bind("success")
    .bind(123_i64)
    .bind(0.42_f64)
    .bind(
        json!({
            "stickyKey": "sticky-stale",
            "upstreamAccountId": account_id,
        })
        .to_string(),
    )
    .bind("{}")
    .execute(&state.pool)
    .await
    .expect("insert stale sticky invocation");
    sync_hourly_rollups_from_live_tables(&state.pool)
        .await
        .expect("sync hourly rollups before sticky aggregate response");

    let response = build_account_sticky_keys_response(
        &state.pool,
        account_id,
        AccountStickyKeySelection::Count(20),
    )
    .await
    .expect("build sticky response");
    let json = serde_json::to_value(&response).expect("serialize sticky response");
    assert_eq!(json["selectionMode"].as_str(), Some("count"));
    assert_eq!(json["selectedLimit"].as_i64(), Some(20));
    let conversations = json["conversations"]
        .as_array()
        .expect("sticky conversations array");
    assert_eq!(conversations.len(), 1);
    let conversation = &conversations[0];
    assert_eq!(conversation["stickyKey"].as_str(), Some("sticky-stale"));
    assert_eq!(conversation["requestCount"].as_i64(), Some(1));
    assert_eq!(conversation["totalTokens"].as_i64(), Some(123));
    assert_eq!(
        conversation["last24hRequests"].as_array().map(Vec::len),
        Some(0)
    );
}

#[tokio::test]
async fn upstream_account_sticky_keys_reads_inline_rollups_without_read_time_catch_up() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let account_id = insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;
    let sticky_key = "sticky-inline-rollup";
    let recent_time = format_naive(
        (Utc::now() - ChronoDuration::minutes(15))
            .with_timezone(&Shanghai)
            .naive_local(),
    );

    upsert_sticky_route(&state.pool, sticky_key, account_id, &recent_time)
        .await
        .expect("seed sticky route");

    let mut record = test_proxy_capture_record("sticky-inline-rollup-invoke", &recent_time);
    record.model = Some("gpt-5.4".to_string());
    record.usage.input_tokens = Some(51);
    record.usage.output_tokens = Some(26);
    record.usage.cache_input_tokens = Some(0);
    record.usage.reasoning_tokens = Some(0);
    record.usage.total_tokens = Some(77);
    record.cost = Some(0.18);
    record.payload = Some(
        json!({
            "stickyKey": sticky_key,
            "upstreamAccountId": account_id,
            "upstreamAccountName": "Primary",
            "routeMode": "sticky",
            "endpoint": "/v1/responses",
            "model": "gpt-5.4",
        })
        .to_string(),
    );
    persist_proxy_capture_record(&state.pool, std::time::Instant::now(), record)
        .await
        .expect("persist sticky proxy capture")
        .expect("sticky invocation should persist");

    let response = build_account_sticky_keys_response(
        &state.pool,
        account_id,
        AccountStickyKeySelection::Count(20),
    )
    .await
    .expect("sticky-key endpoint should stay fresh without read-time catch-up");
    let json = serde_json::to_value(&response).expect("serialize sticky response");
    let conversations = json["conversations"]
        .as_array()
        .expect("sticky conversations array");
    assert_eq!(conversations.len(), 1);
    let conversation = &conversations[0];
    assert_eq!(conversation["stickyKey"].as_str(), Some(sticky_key));
    assert_eq!(conversation["requestCount"].as_i64(), Some(1));
    assert_eq!(conversation["totalTokens"].as_i64(), Some(77));
    assert_eq!(conversation["totalCost"].as_f64(), Some(0.18));
    assert_eq!(
        conversation["recentInvocations"].as_array().map(Vec::len),
        Some(1)
    );
    assert_eq!(
        conversation["recentInvocations"][0]["invokeId"].as_str(),
        Some("sticky-inline-rollup-invoke")
    );
}

#[test]
fn resolve_sticky_key_selection_rejects_mutually_exclusive_params() {
    let err = crate::upstream_accounts::resolve_sticky_key_selection(&AccountStickyKeysQuery {
        limit: Some(20),
        activity_hours: Some(3),
    })
    .expect_err("selection should reject mutually exclusive params");

    assert_eq!(err.0, StatusCode::BAD_REQUEST);
    assert!(err.1.contains("provide either limit or activityHours"));
}

#[test]
fn resolve_sticky_key_selection_rejects_unsupported_activity_hours() {
    let err = crate::upstream_accounts::resolve_sticky_key_selection(&AccountStickyKeysQuery {
        limit: None,
        activity_hours: Some(2),
    })
    .expect_err("selection should reject unsupported activityHours");

    assert_eq!(err.0, StatusCode::BAD_REQUEST);
    assert!(
        err.1
            .contains("activityHours must be one of 1, 3, 6, 12, or 24")
    );
}

#[tokio::test]
async fn build_account_sticky_keys_response_activity_window_filters_recent_keys_only() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let account_id = insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;
    let recent_time = format_naive(
        (Utc::now() - ChronoDuration::hours(2))
            .with_timezone(&Shanghai)
            .naive_local(),
    );
    let stale_time = format_naive(
        (Utc::now() - ChronoDuration::hours(5))
            .with_timezone(&Shanghai)
            .naive_local(),
    );

    for (sticky_key, occurred_at) in [
        ("sticky-window-recent", recent_time.as_str()),
        ("sticky-window-stale", stale_time.as_str()),
    ] {
        sqlx::query(
            r#"
            INSERT INTO pool_sticky_routes (sticky_key, account_id, created_at, updated_at, last_seen_at)
            VALUES (?1, ?2, ?3, ?4, ?5)
            "#,
        )
        .bind(sticky_key)
        .bind(account_id)
        .bind(occurred_at)
        .bind(occurred_at)
        .bind(occurred_at)
        .execute(&state.pool)
        .await
        .expect("insert sticky route");
        sqlx::query(
            r#"
            INSERT INTO codex_invocations (
                invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
            "#,
        )
        .bind(format!("{sticky_key}-invoke"))
        .bind(occurred_at)
        .bind(SOURCE_PROXY)
        .bind("success")
        .bind(64_i64)
        .bind(0.08_f64)
        .bind(
            json!({
                "stickyKey": sticky_key,
                "upstreamAccountId": account_id,
            })
            .to_string(),
        )
        .bind("{}")
        .execute(&state.pool)
        .await
        .expect("insert sticky invocation");
    }

    sync_hourly_rollups_from_live_tables(&state.pool)
        .await
        .expect("sync hourly rollups before sticky activity-window response");

    let response = build_account_sticky_keys_response(
        &state.pool,
        account_id,
        AccountStickyKeySelection::ActivityWindow(3),
    )
    .await
    .expect("build sticky response");
    let json = serde_json::to_value(&response).expect("serialize sticky response");
    assert_eq!(json["selectionMode"].as_str(), Some("activityWindow"));
    assert_eq!(json["selectedLimit"], Value::Null);
    assert_eq!(json["selectedActivityHours"].as_i64(), Some(3));
    assert_eq!(
        json["implicitFilter"]["kind"].as_str(),
        Some("inactiveOutside24h")
    );
    assert_eq!(json["implicitFilter"]["filteredCount"].as_i64(), Some(1));
    let conversations = json["conversations"]
        .as_array()
        .expect("sticky conversations array");
    assert_eq!(conversations.len(), 1);
    assert_eq!(
        conversations[0]["stickyKey"].as_str(),
        Some("sticky-window-recent")
    );
}

#[tokio::test]
async fn build_account_sticky_keys_response_activity_window_previews_respect_time_window() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let account_id = insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;
    let sticky_key = "sticky-window-preview";
    let recent_time = format_naive(
        (Utc::now() - ChronoDuration::minutes(20))
            .with_timezone(&Shanghai)
            .naive_local(),
    );
    let stale_time = format_naive(
        (Utc::now() - ChronoDuration::hours(8))
            .with_timezone(&Shanghai)
            .naive_local(),
    );

    sqlx::query(
        r#"
        INSERT INTO pool_sticky_routes (sticky_key, account_id, created_at, updated_at, last_seen_at)
        VALUES (?1, ?2, ?3, ?4, ?5)
        "#,
    )
    .bind(sticky_key)
    .bind(account_id)
    .bind(&stale_time)
    .bind(&recent_time)
    .bind(&recent_time)
    .execute(&state.pool)
    .await
    .expect("insert sticky route");

    for (invoke_id, occurred_at) in [
        ("sticky-window-preview-recent", recent_time.as_str()),
        ("sticky-window-preview-stale", stale_time.as_str()),
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
        .bind(occurred_at)
        .bind(SOURCE_PROXY)
        .bind("success")
        .bind(64_i64)
        .bind(0.08_f64)
        .bind(
            json!({
                "stickyKey": sticky_key,
                "upstreamAccountId": account_id,
                "model": "gpt-5.4",
            })
            .to_string(),
        )
        .bind("{}")
        .execute(&state.pool)
        .await
        .expect("insert sticky invocation");
    }

    sync_hourly_rollups_from_live_tables(&state.pool)
        .await
        .expect("sync hourly rollups before sticky preview response");

    let response = build_account_sticky_keys_response(
        &state.pool,
        account_id,
        AccountStickyKeySelection::ActivityWindow(3),
    )
    .await
    .expect("build sticky response");
    let json = serde_json::to_value(&response).expect("serialize sticky response");
    let previews = json["conversations"][0]["recentInvocations"]
        .as_array()
        .expect("recent invocations array");
    assert_eq!(previews.len(), 1);
    assert_eq!(
        previews[0]["invokeId"].as_str(),
        Some("sticky-window-preview-recent")
    );
}

#[tokio::test]
async fn build_account_sticky_keys_response_activity_window_caps_results_to_fifty() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let account_id = insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;
    let now = Utc::now();

    for index in 0..55 {
        let occurred_at = format_naive(
            (now - ChronoDuration::minutes(index as i64))
                .with_timezone(&Shanghai)
                .naive_local(),
        );
        let sticky_key = format!("sticky-window-cap-{index}");
        sqlx::query(
            r#"
            INSERT INTO pool_sticky_routes (sticky_key, account_id, created_at, updated_at, last_seen_at)
            VALUES (?1, ?2, ?3, ?4, ?5)
            "#,
        )
        .bind(&sticky_key)
        .bind(account_id)
        .bind(&occurred_at)
        .bind(&occurred_at)
        .bind(&occurred_at)
        .execute(&state.pool)
        .await
        .expect("insert sticky route");
        sqlx::query(
            r#"
            INSERT INTO codex_invocations (
                invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
            "#,
        )
        .bind(format!("sticky-window-cap-invoke-{index}"))
        .bind(&occurred_at)
        .bind(SOURCE_PROXY)
        .bind("success")
        .bind(64_i64)
        .bind(0.08_f64)
        .bind(
            json!({
                "stickyKey": sticky_key,
                "upstreamAccountId": account_id,
            })
            .to_string(),
        )
        .bind("{}")
        .execute(&state.pool)
        .await
        .expect("insert sticky invocation");
    }

    sync_hourly_rollups_from_live_tables(&state.pool)
        .await
        .expect("sync hourly rollups before sticky activity-window response");

    let response = build_account_sticky_keys_response(
        &state.pool,
        account_id,
        AccountStickyKeySelection::ActivityWindow(3),
    )
    .await
    .expect("build sticky response");
    let json = serde_json::to_value(&response).expect("serialize sticky response");
    let conversations = json["conversations"]
        .as_array()
        .expect("sticky conversations array");
    assert_eq!(conversations.len(), 50);
    assert_eq!(json["implicitFilter"]["kind"].as_str(), Some("cappedTo50"));
    assert_eq!(json["implicitFilter"]["filteredCount"].as_i64(), Some(5));
}

#[tokio::test]
async fn build_account_sticky_keys_response_includes_recent_invocations_sorted_and_capped() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let account_id = insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;
    let sticky_key = "sticky-preview-cap";

    sqlx::query(
        r#"
        INSERT INTO pool_sticky_routes (sticky_key, account_id, created_at, updated_at, last_seen_at)
        VALUES (?1, ?2, ?3, ?4, ?5)
        "#,
    )
    .bind(sticky_key)
    .bind(account_id)
    .bind(format_naive(
        (Utc::now() - ChronoDuration::minutes(6))
            .with_timezone(&Shanghai)
            .naive_local(),
    ))
    .bind(format_naive(
        Utc::now().with_timezone(&Shanghai).naive_local(),
    ))
    .bind(format_naive(
        Utc::now().with_timezone(&Shanghai).naive_local(),
    ))
    .execute(&state.pool)
    .await
    .expect("insert sticky preview route");

    for minutes_ago in 0..6 {
        let occurred_at = format_naive(
            (Utc::now() - ChronoDuration::minutes(minutes_ago))
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
        .bind(format!("sticky-preview-invoke-{minutes_ago}"))
        .bind(occurred_at)
        .bind(SOURCE_PROXY)
        .bind("success")
        .bind(100_i64 + minutes_ago)
        .bind(0.01_f64 + (minutes_ago as f64) * 0.001_f64)
        .bind(
            json!({
                "stickyKey": sticky_key,
                "upstreamAccountId": account_id,
                "endpoint": "/v1/responses",
                "routeMode": "sticky",
                "model": "gpt-5.4",
            })
            .to_string(),
        )
        .bind("{}")
        .execute(&state.pool)
        .await
        .expect("insert preview invocation");
    }

    sync_hourly_rollups_from_live_tables(&state.pool)
        .await
        .expect("sync hourly rollups before sticky preview response");

    let response = build_account_sticky_keys_response(
        &state.pool,
        account_id,
        AccountStickyKeySelection::Count(20),
    )
    .await
    .expect("build sticky response");
    let json = serde_json::to_value(&response).expect("serialize sticky response");
    let previews = json["conversations"][0]["recentInvocations"]
        .as_array()
        .expect("recent invocations array");
    assert_eq!(previews.len(), 5);
    assert_eq!(
        previews[0]["invokeId"].as_str(),
        Some("sticky-preview-invoke-0")
    );
    assert_eq!(
        previews[4]["invokeId"].as_str(),
        Some("sticky-preview-invoke-4")
    );
}

#[tokio::test]
async fn prompt_cache_views_ignore_sticky_only_internal_keys() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;

    for (invoke_id, payload) in [
        (
            "sticky-only",
            json!({
                "stickyKey": "sticky-only",
                "upstreamScope": "internal",
                "routeMode": "pool"
            }),
        ),
        (
            "prompt-cache",
            json!({
                "promptCacheKey": "pck-real",
                "upstreamScope": "external"
            }),
        ),
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
            (Utc::now() - ChronoDuration::minutes(5))
                .with_timezone(&Shanghai)
                .naive_local(),
        ))
        .bind(SOURCE_PROXY)
        .bind("success")
        .bind(10_i64)
        .bind(0.01_f64)
        .bind(payload.to_string())
        .bind("{}")
        .execute(&state.pool)
        .await
        .expect("insert prompt cache test invocation");
    }

    sync_hourly_rollups_from_live_tables(&state.pool)
        .await
        .expect("materialize prompt cache rollups before sticky-only prompt-cache read");
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
    .expect("prompt cache conversations should succeed");
    let keys = response
        .conversations
        .iter()
        .map(|item| item.prompt_cache_key.clone())
        .collect::<Vec<_>>();
    assert_eq!(keys, vec!["pck-real".to_string()]);

    let Json(list_response) = list_invocations(
        State(state),
        Query(ListQuery {
            limit: Some(20),
            ..Default::default()
        }),
    )
    .await
    .expect("list invocations should succeed");
    let sticky_record = list_response
        .records
        .into_iter()
        .find(|record| record.invoke_id == "sticky-only")
        .expect("sticky-only record should exist");
    assert!(sticky_record.prompt_cache_key.is_none());
}
