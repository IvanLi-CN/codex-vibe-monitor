#[tokio::test]
async fn prompt_cache_conversations_include_recent_upstream_account_summaries() {
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
        account_id: Option<i64>,
        account_name: Option<&str>,
        total_tokens: i64,
        cost: f64,
    ) {
        let mut payload = json!({ "promptCacheKey": key });
        if let Some(account_id) = account_id {
            payload["upstreamAccountId"] = json!(account_id);
        }
        if let Some(account_name) = account_name {
            payload["upstreamAccountName"] = json!(account_name);
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
        .expect("insert invocation row");
    }

    insert_row(
        &state.pool,
        "pck-upstream-beta-history",
        now - ChronoDuration::hours(48),
        "pck-upstream",
        None,
        Some("Beta"),
        40,
        0.4,
    )
    .await;
    insert_row(
        &state.pool,
        "pck-upstream-alpha",
        now - ChronoDuration::hours(6),
        "pck-upstream",
        Some(1),
        Some("Alpha"),
        10,
        0.1,
    )
    .await;
    insert_row(
        &state.pool,
        "pck-upstream-id-only",
        now - ChronoDuration::hours(3),
        "pck-upstream",
        Some(7),
        None,
        20,
        0.2,
    )
    .await;
    insert_row(
        &state.pool,
        "pck-upstream-beta-recent",
        now - ChronoDuration::hours(2),
        "pck-upstream",
        Some(2),
        Some("Beta"),
        15,
        0.15,
    )
    .await;
    insert_row(
        &state.pool,
        "pck-upstream-gamma",
        now - ChronoDuration::hours(1),
        "pck-upstream",
        Some(9),
        Some("Gamma"),
        30,
        0.3,
    )
    .await;
    insert_row(
        &state.pool,
        "pck-upstream-unknown",
        now - ChronoDuration::minutes(90),
        "pck-upstream",
        None,
        None,
        25,
        0.25,
    )
    .await;

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
        }),
    )
    .await
    .expect("prompt cache conversation stats should succeed");

    let conversation = response
        .conversations
        .iter()
        .find(|item| item.prompt_cache_key == "pck-upstream")
        .expect("pck-upstream should be included");

    assert_eq!(conversation.upstream_accounts.len(), 3);

    let first = &conversation.upstream_accounts[0];
    assert_eq!(first.upstream_account_id, Some(9));
    assert_eq!(first.upstream_account_name.as_deref(), Some("Gamma"));
    assert_eq!(first.request_count, 1);
    assert_eq!(first.total_tokens, 30);

    let second = &conversation.upstream_accounts[1];
    assert_eq!(second.upstream_account_id, None);
    assert_eq!(second.upstream_account_name, None);
    assert_eq!(second.request_count, 1);
    assert_eq!(second.total_tokens, 25);
    assert!((second.total_cost - 0.25).abs() < 1e-9);

    let third = &conversation.upstream_accounts[2];
    assert_eq!(third.upstream_account_id, Some(2));
    assert_eq!(third.upstream_account_name.as_deref(), Some("Beta"));
    assert_eq!(third.request_count, 2);
    assert_eq!(third.total_tokens, 55);
    assert!((third.total_cost - 0.55).abs() < 1e-9);

    assert!(
        conversation
            .upstream_accounts
            .iter()
            .all(|account| account.upstream_account_id != Some(7))
    );

    assert!(
        conversation
            .upstream_accounts
            .iter()
            .any(|account| account.upstream_account_id.is_none()
                && account.upstream_account_name.is_none())
    );
    assert!(
        conversation
            .upstream_accounts
            .iter()
            .all(|account| account.upstream_account_id != Some(1))
    );
}

#[tokio::test]
async fn prompt_cache_conversations_include_recent_invocation_previews_with_limit_and_proxy_scope()
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
        source: &str,
        key: &str,
        status: &str,
        total_tokens: i64,
        cost: f64,
        proxy_display_name: &str,
        account_id: Option<i64>,
        account_name: Option<&str>,
        endpoint: &str,
        model: &str,
    ) {
        let mut payload = json!({
            "promptCacheKey": key,
            "proxyDisplayName": proxy_display_name,
            "endpoint": endpoint,
            "model": model,
            "routeMode": "pool",
        });
        if let Some(account_id) = account_id {
            payload["upstreamAccountId"] = json!(account_id);
        }
        if let Some(account_name) = account_name {
            payload["upstreamAccountName"] = json!(account_name);
        }

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
        .bind(status)
        .bind(model)
        .bind(total_tokens)
        .bind(cost)
        .bind(payload.to_string())
        .bind("{}")
        .execute(pool)
        .await
        .expect("insert invocation row");
    }

    insert_row(
        &state.pool,
        "preview-01",
        now - ChronoDuration::hours(7),
        SOURCE_PROXY,
        "pck-preview",
        "success",
        100,
        0.10,
        "Proxy Alpha",
        Some(101),
        Some("Pool Alpha"),
        "/v1/responses",
        "gpt-5.4",
    )
    .await;
    insert_row(
        &state.pool,
        "preview-02",
        now - ChronoDuration::hours(6),
        SOURCE_PROXY,
        "pck-preview",
        "success",
        120,
        0.12,
        "Proxy Alpha",
        Some(101),
        Some("Pool Alpha"),
        "/v1/responses",
        "gpt-5.4",
    )
    .await;
    insert_row(
        &state.pool,
        "preview-03",
        now - ChronoDuration::hours(5),
        SOURCE_PROXY,
        "pck-preview",
        "http_502",
        140,
        0.14,
        "Proxy Beta",
        None,
        None,
        "/v1/chat/completions",
        "gpt-5.4-mini",
    )
    .await;
    insert_row(
        &state.pool,
        "preview-04",
        now - ChronoDuration::hours(4),
        SOURCE_PROXY,
        "pck-preview",
        "success",
        160,
        0.16,
        "Proxy Beta",
        Some(202),
        None,
        "/v1/responses",
        "gpt-5.4-mini",
    )
    .await;
    sqlx::query(
        "UPDATE codex_invocations SET payload = json_set(payload, '$.reasoningEffort', 7) WHERE invoke_id = ?1",
    )
    .bind("preview-04")
    .execute(&state.pool)
    .await
    .expect("mark preview-04 reasoning effort as non-text");
    insert_row(
        &state.pool,
        "preview-05",
        now - ChronoDuration::hours(3),
        SOURCE_PROXY,
        "pck-preview",
        "success",
        180,
        0.18,
        "Proxy Gamma",
        Some(303),
        Some("Pool Gamma"),
        "/v1/responses",
        "gpt-5.4",
    )
    .await;
    sqlx::query(
        "UPDATE codex_invocations SET failure_kind = ?1, failure_class = ?2, error_message = ?3 WHERE invoke_id = ?4",
    )
    .bind("upstream_response_failed")
    .bind("none")
    .bind("[upstream_response_failed] legacy upstream failure")
    .bind("preview-05")
    .execute(&state.pool)
    .await
    .expect("mark preview-05 as legacy failure");
    insert_row(
        &state.pool,
        "preview-06",
        now - ChronoDuration::hours(2),
        SOURCE_PROXY,
        "pck-preview",
        "success",
        200,
        0.20,
        "Proxy Gamma",
        Some(303),
        Some("Pool Gamma"),
        "/v1/responses",
        "gpt-5.4",
    )
    .await;
    sqlx::query(
        "UPDATE codex_invocations \
         SET input_tokens = ?1, \
             output_tokens = ?2, \
             cache_input_tokens = ?3, \
             reasoning_tokens = ?4, \
             error_message = ?5, \
             failure_kind = ?6, \
             failure_class = ?7, \
             is_actionable = ?8, \
             t_req_read_ms = ?9, \
             t_req_parse_ms = ?10, \
             t_upstream_connect_ms = ?11, \
             t_upstream_ttfb_ms = ?12, \
             t_upstream_stream_ms = ?13, \
             t_resp_parse_ms = ?14, \
             t_persist_ms = ?15, \
             t_total_ms = ?16, \
             payload = json_set( \
                 payload, \
                 '$.reasoningEffort', ?17, \
                 '$.responseContentEncoding', ?18, \
                 '$.requestedServiceTier', ?19, \
                 '$.serviceTier', ?20, \
                 '$.downstreamStatusCode', ?21, \
                 '$.downstreamErrorMessage', ?22 \
             ) \
         WHERE invoke_id = ?23",
    )
    .bind(120_i64)
    .bind(80_i64)
    .bind(40_i64)
    .bind(12_i64)
    .bind("[upstream_response_failed] preview extra error")
    .bind("upstream_response_failed")
    .bind("service_failure")
    .bind(1_i64)
    .bind(10.0_f64)
    .bind(11.0_f64)
    .bind(12.0_f64)
    .bind(13.0_f64)
    .bind(14.0_f64)
    .bind(15.0_f64)
    .bind(16.0_f64)
    .bind(91.0_f64)
    .bind("high")
    .bind("br")
    .bind("flex")
    .bind("scale")
    .bind(502_i64)
    .bind("pool upstream responded with 502: failed to contact oauth codex upstream")
    .bind("preview-06")
    .execute(&state.pool)
    .await
    .expect("augment preview-06 extras");
    insert_row(
        &state.pool,
        "preview-crs-hidden",
        now - ChronoDuration::hours(1),
        SOURCE_CRS,
        "pck-preview",
        "success",
        999,
        9.99,
        "CRS Hidden",
        Some(404),
        Some("CRS Hidden"),
        "/v1/responses",
        "gpt-5.4",
    )
    .await;

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
        }),
    )
    .await
    .expect("prompt cache conversations should succeed");

    let conversation = response
        .conversations
        .iter()
        .find(|item| item.prompt_cache_key == "pck-preview")
        .expect("pck-preview should be included");

    assert_eq!(conversation.request_count, 7);
    assert_eq!(conversation.total_tokens, 1899);
    assert!((conversation.total_cost - 10.89).abs() < 1e-9);
    assert_eq!(conversation.recent_invocations.len(), 5);
    assert_eq!(
        conversation
            .recent_invocations
            .iter()
            .map(|item| item.invoke_id.as_str())
            .collect::<Vec<_>>(),
        vec![
            "preview-crs-hidden",
            "preview-06",
            "preview-05",
            "preview-04",
            "preview-03",
        ]
    );

    let latest = &conversation.recent_invocations[0];
    assert_eq!(latest.status, "success");
    assert_eq!(latest.model.as_deref(), Some("gpt-5.4"));
    assert_eq!(latest.total_tokens, 999);
    assert_eq!(latest.proxy_display_name.as_deref(), Some("CRS Hidden"));
    assert_eq!(latest.upstream_account_id, Some(404));
    assert_eq!(latest.upstream_account_name.as_deref(), Some("CRS Hidden"));
    assert_eq!(latest.endpoint.as_deref(), Some("/v1/responses"));
    assert_eq!(latest.failure_class.as_deref(), Some("none"));
    assert_eq!(latest.route_mode.as_deref(), Some("pool"));
    assert_eq!(latest.source.as_deref(), Some(SOURCE_CRS));

    let preview_with_downstream = conversation
        .recent_invocations
        .iter()
        .find(|item| item.invoke_id == "preview-06")
        .expect("preview-06 should be included");
    assert_eq!(preview_with_downstream.downstream_status_code, Some(502));
    assert_eq!(
        preview_with_downstream.downstream_error_message.as_deref(),
        Some("pool upstream responded with 502: failed to contact oauth codex upstream")
    );

    let id_only = conversation
        .recent_invocations
        .iter()
        .find(|item| item.invoke_id == "preview-04")
        .expect("id-only preview should be included");
    assert_eq!(id_only.upstream_account_id, Some(202));
    assert_eq!(id_only.upstream_account_name, None);
    assert_eq!(id_only.reasoning_effort, None);

    let failed_preview = conversation
        .recent_invocations
        .iter()
        .find(|item| item.invoke_id == "preview-03")
        .expect("failed preview should be included");
    assert_eq!(failed_preview.status, "failed");
    assert_eq!(
        failed_preview.failure_class.as_deref(),
        Some("service_failure")
    );
    assert_eq!(failed_preview.route_mode.as_deref(), Some("pool"));

    let legacy_failed_preview = conversation
        .recent_invocations
        .iter()
        .find(|item| item.invoke_id == "preview-05")
        .expect("legacy failed preview should be included");
    assert_eq!(legacy_failed_preview.status, "failed");
    assert_eq!(
        legacy_failed_preview.failure_class.as_deref(),
        Some("service_failure")
    );
    assert_eq!(legacy_failed_preview.route_mode.as_deref(), Some("pool"));

    let enriched_preview = conversation
        .recent_invocations
        .iter()
        .find(|item| item.invoke_id == "preview-06")
        .expect("preview-06 should be included");
    assert_eq!(enriched_preview.source.as_deref(), Some(SOURCE_PROXY));
    assert_eq!(enriched_preview.input_tokens, Some(120));
    assert_eq!(enriched_preview.output_tokens, Some(80));
    assert_eq!(enriched_preview.cache_input_tokens, Some(40));
    assert_eq!(enriched_preview.reasoning_tokens, Some(12));
    assert_eq!(enriched_preview.reasoning_effort.as_deref(), Some("high"));
    assert_eq!(
        enriched_preview.error_message.as_deref(),
        Some("[upstream_response_failed] preview extra error")
    );
    assert_eq!(
        enriched_preview.failure_kind.as_deref(),
        Some("upstream_response_failed")
    );
    assert_eq!(enriched_preview.is_actionable, Some(true));
    assert_eq!(
        enriched_preview.response_content_encoding.as_deref(),
        Some("br")
    );
    assert_eq!(
        enriched_preview.requested_service_tier.as_deref(),
        Some("flex")
    );
    assert_eq!(enriched_preview.service_tier.as_deref(), Some("scale"));
    assert_eq!(enriched_preview.t_req_read_ms, Some(10.0));
    assert_eq!(enriched_preview.t_req_parse_ms, Some(11.0));
    assert_eq!(enriched_preview.t_upstream_connect_ms, Some(12.0));
    assert_eq!(enriched_preview.t_upstream_ttfb_ms, Some(13.0));
    assert_eq!(enriched_preview.t_upstream_stream_ms, Some(14.0));
    assert_eq!(enriched_preview.t_resp_parse_ms, Some(15.0));
    assert_eq!(enriched_preview.t_persist_ms, Some(16.0));
    assert_eq!(enriched_preview.t_total_ms, Some(91.0));

    let proxy_only_rows = query_prompt_cache_conversation_recent_invocations(
        &state.pool,
        InvocationSourceScope::ProxyOnly,
        &["pck-preview".to_string()],
        5,
        None,
    )
    .await
    .expect("proxy-only recent invocation previews should succeed");

    assert_eq!(
        proxy_only_rows
            .iter()
            .map(|item| item.invoke_id.as_str())
            .collect::<Vec<_>>(),
        vec![
            "preview-06",
            "preview-05",
            "preview-04",
            "preview-03",
            "preview-02",
        ]
    );
    assert!(
        proxy_only_rows
            .iter()
            .all(|item| item.invoke_id != "preview-crs-hidden")
    );
    let proxy_enriched_row = proxy_only_rows
        .iter()
        .find(|item| item.invoke_id == "preview-06")
        .expect("preview-06 row should be present");
    assert_eq!(proxy_enriched_row.source.as_deref(), Some(SOURCE_PROXY));
    assert_eq!(proxy_enriched_row.input_tokens, Some(120));
    assert_eq!(
        proxy_enriched_row.requested_service_tier.as_deref(),
        Some("flex")
    );
    assert_eq!(proxy_enriched_row.t_total_ms, Some(91.0));
}

#[tokio::test]
async fn prompt_cache_conversations_preserve_upstream_account_history_after_raw_rows_are_removed() {
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
        account_id: Option<i64>,
        account_name: Option<&str>,
        total_tokens: i64,
        cost: f64,
    ) {
        let mut payload = json!({ "promptCacheKey": key });
        if let Some(account_id) = account_id {
            payload["upstreamAccountId"] = json!(account_id);
        }
        if let Some(account_name) = account_name {
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
        .execute(pool)
        .await
        .expect("insert invocation row");
    }

    insert_row(
        &state.pool,
        "pck-upstream-history-beta",
        now - ChronoDuration::hours(48),
        "pck-upstream-history",
        None,
        Some("Beta"),
        40,
        0.4,
    )
    .await;
    insert_row(
        &state.pool,
        "pck-upstream-recent-beta",
        now - ChronoDuration::hours(2),
        "pck-upstream-history",
        Some(2),
        Some("Beta"),
        15,
        0.15,
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
            activity_hours: None,
            activity_minutes: None,
            page_size: None,
            cursor: None,
            snapshot_at: None,
            detail: None,
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
async fn prompt_cache_conversations_keep_totals_when_recent_preview_is_empty() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
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
            activity_hours: None,
            activity_minutes: None,
            page_size: None,
            cursor: None,
            snapshot_at: None,
            detail: None,
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
async fn prompt_cache_conversations_count_mode_reports_inactive_recent_history_filter() {
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
        .bind(10)
        .bind(0.01)
        .bind(json!({ "promptCacheKey": key }).to_string())
        .bind("{}")
        .execute(pool)
        .await
        .expect("insert invocation row");
    }

    for index in 0..19 {
        insert_row(
            &state.pool,
            &format!("count-active-{index}"),
            now - ChronoDuration::hours(23) + ChronoDuration::minutes(index as i64),
            &format!("count-active-{index}"),
        )
        .await;
    }
    insert_row(
        &state.pool,
        "count-inactive",
        now - ChronoDuration::hours(72),
        "count-inactive",
    )
    .await;

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
        .bind(10)
        .bind(0.01)
        .bind(json!({ "promptCacheKey": key }).to_string())
        .bind("{}")
        .execute(pool)
        .await
        .expect("insert invocation row");
    }

    for index in 0..25 {
        insert_row(
            &state.pool,
            &format!("count-inactive-{index}"),
            now - ChronoDuration::hours(25) + ChronoDuration::minutes(index as i64),
            &format!("count-inactive-{index}"),
        )
        .await;
    }

    for index in 0..20 {
        insert_row(
            &state.pool,
            &format!("count-active-{index}-history"),
            now - ChronoDuration::days(4) + ChronoDuration::minutes(index as i64),
            &format!("count-active-{index}"),
        )
        .await;
        insert_row(
            &state.pool,
            &format!("count-active-{index}-recent"),
            now - ChronoDuration::hours(12) + ChronoDuration::minutes(index as i64),
            &format!("count-active-{index}"),
        )
        .await;
    }

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
        .bind(10)
        .bind(0.01)
        .bind(json!({ "promptCacheKey": key }).to_string())
        .bind("{}")
        .execute(pool)
        .await
        .expect("insert invocation row");
    }

    for index in 0..25 {
        insert_row(
            &state.pool,
            &format!("sparse-inactive-{index}"),
            now - ChronoDuration::hours(25) + ChronoDuration::minutes(index as i64),
            &format!("sparse-inactive-{index}"),
        )
        .await;
    }

    insert_row(
        &state.pool,
        "sparse-active-history",
        now - ChronoDuration::days(4),
        "sparse-active",
    )
    .await;
    insert_row(
        &state.pool,
        "sparse-active-recent",
        now - ChronoDuration::hours(6),
        "sparse-active",
    )
    .await;

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
    assert_eq!(running.recent_invocations[0].status, "running");
    assert_eq!(running.recent_invocations[1].status, "success");
}

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
        .bind(format_naive(occurred_at.with_timezone(&Shanghai).naive_local()))
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
        }),
    )
    .await
    .expect("second paginated working page should succeed");

    assert_eq!(second_page.conversations.len(), 20);
    assert_eq!(second_page.total_matched, Some(total_matched));
    assert!(second_page.has_more);
    assert_eq!(second_page.snapshot_at.as_deref(), Some(first_snapshot.as_str()));

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
        }),
    )
    .await
    .expect("third paginated working page should succeed");

    assert_eq!(third_page.conversations.len(), (total_matched - 40) as usize);
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
        }),
    )
    .await
    .expect("first pipe-key page should succeed");

    assert_eq!(
        first_page.conversations[0].prompt_cache_key,
        "pipe|head"
    );
    let snapshot_at = first_page
        .snapshot_at
        .clone()
        .expect("pipe-key snapshotAt");
    let next_cursor = first_page
        .next_cursor
        .clone()
        .expect("pipe-key nextCursor");
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
        }),
    )
    .await
    .expect("second pipe-key page should succeed");

    assert_eq!(
        second_page.conversations[0].prompt_cache_key,
        "pipe|tail"
    );

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
        db_occurred_at_lower_bound(
            precise_now
                + ChronoDuration::seconds(1)
        )
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
async fn prompt_cache_conversations_activity_minutes_paginated_respect_requested_snapshot_totals()
{
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let current_hour_start = Utc
        .timestamp_opt(align_bucket_epoch(Utc::now().timestamp(), 3_600, 0), 0)
        .single()
        .expect("current hour start should be valid");
    let snapshot_at = current_hour_start + ChronoDuration::minutes(20);

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
        }),
    )
    .await
    .expect("first snapshot page should succeed");

    assert_eq!(first_page.conversations.len(), 1);
    assert_eq!(
        first_page.conversations[0].prompt_cache_key,
        "working-snapshot-head"
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
        }),
    )
    .await
    .expect("second snapshot page should succeed");

    assert_eq!(second_page.conversations.len(), 1);
    let target = &second_page.conversations[0];
    assert_eq!(target.prompt_cache_key, "working-snapshot-target");
    assert_eq!(target.request_count, 1);
    assert_eq!(target.total_tokens, 10);
    assert!((target.total_cost - 0.10).abs() < 1e-9);
}

#[tokio::test]
async fn prompt_cache_conversations_activity_minutes_paginated_snapshot_excludes_same_second_post_snapshot_writes(
) {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let current_hour_start = Utc
        .timestamp_opt(align_bucket_epoch(Utc::now().timestamp(), 3_600, 0), 0)
        .single()
        .expect("current hour start should be valid");
    let snapshot_second = current_hour_start + ChronoDuration::minutes(20);
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
        }),
    )
    .await
    .expect("first same-second snapshot page should succeed");

    assert_eq!(first_page.conversations.len(), 1);
    assert_eq!(first_page.total_matched, Some(2));
    let expected_snapshot_at = format_utc_iso_precise(requested_snapshot_at);
    assert_eq!(
        first_page.conversations[0].prompt_cache_key,
        "working-same-second-head"
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
        }),
    )
    .await
    .expect("second same-second snapshot page should succeed");

    assert_eq!(second_page.total_matched, Some(2));
    assert_eq!(second_page.conversations.len(), 1);
    assert_eq!(
        second_page.conversations[0].prompt_cache_key,
        "working-same-second-tail"
    );
    assert!(
        second_page
            .conversations
            .iter()
            .all(|conversation| conversation.prompt_cache_key != "working-same-second-post")
    );
    assert!(
        second_page
            .conversations
            .iter()
            .all(|conversation| {
                conversation.prompt_cache_key != "working-same-second-preexisting-post"
            })
    );
}

#[tokio::test]
async fn prompt_cache_conversations_activity_minutes_paginated_whole_second_snapshot_excludes_post_snapshot_writes(
) {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let current_hour_start = Utc
        .timestamp_opt(align_bucket_epoch(Utc::now().timestamp(), 3_600, 0), 0)
        .single()
        .expect("current hour start should be valid");
    let snapshot_second = current_hour_start + ChronoDuration::minutes(24);

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
        }),
    )
    .await
    .expect("first whole-second snapshot page should succeed");

    assert_eq!(first_page.conversations.len(), 1);
    assert_eq!(first_page.total_matched, Some(2));
    assert_eq!(
        first_page.conversations[0].prompt_cache_key,
        "working-whole-second-head"
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
        }),
    )
    .await
    .expect("second whole-second snapshot page should succeed");

    assert_eq!(second_page.total_matched, Some(2));
    assert_eq!(second_page.conversations.len(), 1);
    assert_eq!(
        second_page.conversations[0].prompt_cache_key,
        "working-whole-second-tail"
    );
    assert!(
        second_page
            .conversations
            .iter()
            .all(|conversation| conversation.prompt_cache_key != "working-whole-second-post")
    );
    assert!(
        second_page
            .conversations
            .iter()
            .all(|conversation| {
                conversation.prompt_cache_key != "working-whole-second-preexisting-post"
            })
    );
}

#[tokio::test]
async fn prompt_cache_conversations_activity_minutes_paginated_snapshot_excludes_late_persisted_pre_snapshot_occurrence(
) {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let current_hour_start = Utc
        .timestamp_opt(align_bucket_epoch(Utc::now().timestamp(), 3_600, 0), 0)
        .single()
        .expect("current hour start should be valid");
    let snapshot_second = current_hour_start + ChronoDuration::minutes(28);
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
        }),
    )
    .await
    .expect("second late-persist snapshot page should succeed");

    assert_eq!(second_page.total_matched, Some(2));
    assert_eq!(second_page.conversations.len(), 1);
    assert_eq!(
        second_page.conversations[0].prompt_cache_key,
        "working-late-persist-tail"
    );
    assert!(
        second_page
            .conversations
            .iter()
            .all(|conversation| conversation.prompt_cache_key != "working-late-persist-post")
    );
}

#[tokio::test]
async fn prompt_cache_conversations_activity_minutes_paginated_snapshot_preserves_previous_hour_lifetime_totals()
{
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let current_hour_start = Utc
        .timestamp_opt(align_bucket_epoch(Utc::now().timestamp(), 3_600, 0), 0)
        .single()
        .expect("current hour start should be valid");
    let snapshot_at = current_hour_start + ChronoDuration::minutes(3);

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
        }),
    )
    .await
    .expect("first lifetime snapshot page should succeed");

    assert_eq!(first_page.conversations.len(), 1);
    assert_eq!(
        first_page.conversations[0].prompt_cache_key,
        "working-window-head"
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
        }),
    )
    .await
    .expect("second lifetime snapshot page should succeed");

    assert_eq!(second_page.conversations.len(), 1);
    let target = &second_page.conversations[0];
    assert_eq!(target.prompt_cache_key, "working-window-target");
    assert_eq!(target.request_count, 2);
    assert_eq!(target.total_tokens, 1_009);
    assert!((target.total_cost - 10.09).abs() < 1e-9);
}

#[tokio::test]
async fn prompt_cache_conversations_activity_minutes_paginated_snapshot_keeps_hydrated_details_consistent()
{
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let current_hour_start = Utc
        .timestamp_opt(align_bucket_epoch(Utc::now().timestamp(), 3_600, 0), 0)
        .single()
        .expect("current hour start should be valid");
    let snapshot_at = current_hour_start + ChronoDuration::minutes(20);

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
        }),
    )
    .await
    .expect("first full snapshot page should succeed");

    assert_eq!(first_page.conversations.len(), 1);
    assert_eq!(
        first_page.conversations[0].prompt_cache_key,
        "working-snapshot-full-head"
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
        }),
    )
    .await
    .expect("second full snapshot page should succeed");

    assert_eq!(second_page.conversations.len(), 1);
    let target = &second_page.conversations[0];
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
        Duration::from_secs(2),
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

    tokio::time::timeout(Duration::from_secs(1), async {
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
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("drop cleanup should remove in-flight marker");
}

#[test]
fn decode_response_payload_for_usage_decompresses_gzip_stream() {
    let raw = [
        "event: response.completed",
        "data: {\"type\":\"response.completed\",\"response\":{\"usage\":{\"input_tokens\":123,\"output_tokens\":45,\"total_tokens\":168,\"input_tokens_details\":{\"cached_tokens\":7},\"output_tokens_details\":{\"reasoning_tokens\":4}}}}",
    ]
    .join("\n");
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder
        .write_all(raw.as_bytes())
        .expect("write gzip payload");
    let compressed = encoder.finish().expect("finish gzip payload");

    let (decoded, decode_error) = decode_response_payload_for_usage(&compressed, Some("gzip"));
    assert!(decode_error.is_none());

    let parsed =
        parse_target_response_payload(ProxyCaptureTarget::Responses, decoded.as_ref(), true, None);
    assert_eq!(parsed.usage.input_tokens, Some(123));
    assert_eq!(parsed.usage.output_tokens, Some(45));
    assert_eq!(parsed.usage.total_tokens, Some(168));
    assert_eq!(parsed.usage.cache_input_tokens, Some(7));
    assert_eq!(parsed.usage.reasoning_tokens, Some(4));
}

fn encode_brotli_payload(bytes: &[u8]) -> Vec<u8> {
    let mut output = Vec::new();
    {
        let mut writer = CompressorWriter::new(&mut output, 4096, 5, 22);
        writer.write_all(bytes).expect("write brotli payload");
    }
    output
}

#[test]
fn decode_response_payload_for_usage_decompresses_brotli_stream() {
    let raw = br#"{"usage":{"input_tokens":9,"output_tokens":4,"total_tokens":13}}"#;
    let compressed = encode_brotli_payload(raw);

    let (decoded, decode_error) = decode_response_payload_for_usage(&compressed, Some("br"));
    assert!(decode_error.is_none());
    assert_eq!(decoded.as_ref(), raw);
}

#[test]
fn decode_response_payload_for_usage_decompresses_deflate_streams() {
    let raw = br#"{"usage":{"input_tokens":11,"output_tokens":5,"total_tokens":16}}"#;

    let mut zlib_encoder = ZlibEncoder::new(Vec::new(), Compression::default());
    zlib_encoder.write_all(raw).expect("write zlib payload");
    let zlib_compressed = zlib_encoder.finish().expect("finish zlib payload");

    let (decoded_zlib, decode_error_zlib) =
        decode_response_payload_for_usage(&zlib_compressed, Some("deflate"));
    assert!(decode_error_zlib.is_none());
    assert_eq!(decoded_zlib.as_ref(), raw);

    let mut raw_encoder = DeflateEncoder::new(Vec::new(), Compression::default());
    raw_encoder
        .write_all(raw)
        .expect("write raw deflate payload");
    let raw_compressed = raw_encoder.finish().expect("finish raw deflate payload");

    let (decoded_raw, decode_error_raw) =
        decode_response_payload_for_usage(&raw_compressed, Some("deflate"));
    assert!(decode_error_raw.is_none());
    assert_eq!(decoded_raw.as_ref(), raw);
}

#[test]
fn decode_response_payload_for_usage_decompresses_stacked_content_encodings() {
    let raw = br#"{"usage":{"input_tokens":21,"output_tokens":8,"total_tokens":29}}"#;
    let mut gzip_encoder = GzEncoder::new(Vec::new(), Compression::default());
    gzip_encoder
        .write_all(raw)
        .expect("write stacked gzip payload");
    let gzip_compressed = gzip_encoder.finish().expect("finish stacked gzip payload");
    let stacked = encode_brotli_payload(&gzip_compressed);

    let (decoded, decode_error) = decode_response_payload_for_usage(&stacked, Some("gzip, br"));
    assert!(decode_error.is_none());
    assert_eq!(decoded.as_ref(), raw);
}

#[tokio::test]
async fn backfill_proxy_prompt_cache_keys_updates_payload_and_is_idempotent() {
    let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
        .await
        .expect("connect in-memory sqlite");
    ensure_schema(&pool)
        .await
        .expect("schema should initialize");

    let temp_dir = make_temp_test_dir("proxy-prompt-cache-key-backfill");
    let request_path = temp_dir.join("request.json");
    write_backfill_request_payload(&request_path, Some("pck-backfill-1"));

    insert_proxy_prompt_cache_backfill_row(
        &pool,
        "proxy-pck-backfill-1",
        &request_path,
        "{\"endpoint\":\"/v1/responses\",\"requesterIp\":\"198.51.100.77\",\"codexSessionId\":\"legacy-session-1\"}",
    )
    .await;
    insert_proxy_prompt_cache_backfill_row(
        &pool,
        "proxy-pck-backfill-ready",
        &request_path,
        "{\"endpoint\":\"/v1/responses\",\"promptCacheKey\":\"already-present\"}",
    )
    .await;

    let summary_first = backfill_proxy_prompt_cache_keys(&pool, None)
        .await
        .expect("first prompt cache key backfill should succeed");
    assert_eq!(summary_first.scanned, 1);
    assert_eq!(summary_first.updated, 1);
    assert_eq!(summary_first.skipped_missing_file, 0);
    assert_eq!(summary_first.skipped_invalid_json, 0);
    assert_eq!(summary_first.skipped_missing_key, 0);

    let payload: String =
        sqlx::query_scalar("SELECT payload FROM codex_invocations WHERE invoke_id = ?1")
            .bind("proxy-pck-backfill-1")
            .fetch_one(&pool)
            .await
            .expect("query backfilled payload");
    let payload_json: Value = serde_json::from_str(&payload).expect("decode payload JSON");
    assert_eq!(payload_json["promptCacheKey"], "pck-backfill-1");
    assert!(
        payload_json.get("codexSessionId").is_none(),
        "legacy codexSessionId key should be removed during backfill"
    );

    let summary_second = backfill_proxy_prompt_cache_keys(&pool, None)
        .await
        .expect("second prompt cache key backfill should succeed");
    assert_eq!(summary_second.scanned, 0);
    assert_eq!(summary_second.updated, 0);

    let _ = fs::remove_dir_all(&temp_dir);
}

#[tokio::test]
async fn sync_hourly_rollups_rebuilds_after_prompt_cache_key_backfill_updates_existing_rows() {
    let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
        .await
        .expect("connect in-memory sqlite");
    ensure_schema(&pool)
        .await
        .expect("schema should initialize");

    let temp_dir = make_temp_test_dir("hourly-rollup-rebuild-after-prompt-cache-backfill");
    let request_path = temp_dir.join("request.json");
    write_backfill_request_payload(&request_path, Some("pck-rollup-rebuild"));

    insert_proxy_prompt_cache_backfill_row(
        &pool,
        "proxy-pck-rollup-rebuild",
        &request_path,
        r#"{"endpoint":"/v1/responses","requesterIp":"198.51.100.77"}"#,
    )
    .await;

    bootstrap_hourly_rollups(&pool)
        .await
        .expect("initial hourly rollup bootstrap should succeed");

    let initial_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM prompt_cache_rollup_hourly WHERE prompt_cache_key = ?1",
    )
    .bind("pck-rollup-rebuild")
    .fetch_one(&pool)
    .await
    .expect("query initial prompt cache rollup count");
    assert_eq!(initial_count, 0);

    let summary = backfill_proxy_prompt_cache_keys(&pool, None)
        .await
        .expect("prompt cache key backfill should succeed");
    assert_eq!(summary.updated, 1);

    sync_hourly_rollups_from_live_tables(&pool)
        .await
        .expect("hourly rollup sync should rebuild invocation-backed rollups");

    let request_count: i64 = sqlx::query_scalar(
        "SELECT request_count FROM prompt_cache_rollup_hourly WHERE prompt_cache_key = ?1",
    )
    .bind("pck-rollup-rebuild")
    .fetch_one(&pool)
    .await
    .expect("query rebuilt prompt cache rollup row");
    assert_eq!(request_count, 1);

    let _ = fs::remove_dir_all(&temp_dir);
}

#[tokio::test]
async fn backfill_proxy_prompt_cache_keys_tracks_skip_counters() {
    let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
        .await
        .expect("connect in-memory sqlite");
    ensure_schema(&pool)
        .await
        .expect("schema should initialize");

    let temp_dir = make_temp_test_dir("proxy-prompt-cache-key-backfill-skips");
    let ok_request_path = temp_dir.join("request-ok.json");
    let missing_key_request_path = temp_dir.join("request-missing-key.json");
    let invalid_json_request_path = temp_dir.join("request-invalid-json.json");
    let missing_file_request_path = temp_dir.join("request-missing.json");

    write_backfill_request_payload(&ok_request_path, Some("pck-backfill-ok"));
    write_backfill_request_payload(&missing_key_request_path, None);
    fs::write(&invalid_json_request_path, b"not-json").expect("write invalid request payload");

    let base_payload = "{\"endpoint\":\"/v1/responses\",\"requesterIp\":\"198.51.100.77\"}";
    insert_proxy_prompt_cache_backfill_row(
        &pool,
        "proxy-pck-backfill-ok",
        &ok_request_path,
        base_payload,
    )
    .await;
    insert_proxy_prompt_cache_backfill_row(
        &pool,
        "proxy-pck-backfill-missing-file",
        &missing_file_request_path,
        base_payload,
    )
    .await;
    insert_proxy_prompt_cache_backfill_row(
        &pool,
        "proxy-pck-backfill-invalid-json",
        &invalid_json_request_path,
        base_payload,
    )
    .await;
    insert_proxy_prompt_cache_backfill_row(
        &pool,
        "proxy-pck-backfill-missing-key",
        &missing_key_request_path,
        base_payload,
    )
    .await;

    let summary = backfill_proxy_prompt_cache_keys(&pool, None)
        .await
        .expect("prompt cache key backfill should succeed");
    assert_eq!(summary.scanned, 4);
    assert_eq!(summary.updated, 1);
    assert_eq!(summary.skipped_missing_file, 1);
    assert_eq!(summary.skipped_invalid_json, 1);
    assert_eq!(summary.skipped_missing_key, 1);

    let payload: String =
        sqlx::query_scalar("SELECT payload FROM codex_invocations WHERE invoke_id = ?1")
            .bind("proxy-pck-backfill-ok")
            .fetch_one(&pool)
            .await
            .expect("query backfilled payload");
    let payload_json: Value = serde_json::from_str(&payload).expect("decode payload JSON");
    assert_eq!(payload_json["promptCacheKey"], "pck-backfill-ok");

    let _ = fs::remove_dir_all(&temp_dir);
}

#[tokio::test]
async fn backfill_proxy_requested_service_tiers_updates_payload_and_is_idempotent() {
    let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
        .await
        .expect("connect in-memory sqlite");
    ensure_schema(&pool)
        .await
        .expect("schema should initialize");

    let temp_dir = make_temp_test_dir("proxy-requested-service-tier-backfill");
    let request_path = temp_dir.join("request.json");
    write_backfill_request_payload_with_requested_service_tier(
        &request_path,
        Some("priority"),
        ProxyCaptureTarget::Responses,
    );

    insert_proxy_prompt_cache_backfill_row(
        &pool,
        "proxy-requested-tier-backfill-1",
        &request_path,
        r#"{"endpoint":"/v1/responses"}"#,
    )
    .await;
    insert_proxy_prompt_cache_backfill_row(
        &pool,
        "proxy-requested-tier-backfill-ready",
        &request_path,
        r#"{"endpoint":"/v1/responses","requestedServiceTier":"priority"}"#,
    )
    .await;

    let summary_first = backfill_proxy_requested_service_tiers(&pool, None)
        .await
        .expect("first requested service tier backfill should succeed");
    assert_eq!(summary_first.scanned, 1);
    assert_eq!(summary_first.updated, 1);
    assert_eq!(summary_first.skipped_missing_file, 0);
    assert_eq!(summary_first.skipped_invalid_json, 0);
    assert_eq!(summary_first.skipped_missing_tier, 0);

    let payload: String =
        sqlx::query_scalar("SELECT payload FROM codex_invocations WHERE invoke_id = ?1")
            .bind("proxy-requested-tier-backfill-1")
            .fetch_one(&pool)
            .await
            .expect("query backfilled payload");
    let payload_json: Value = serde_json::from_str(&payload).expect("decode payload JSON");
    assert_eq!(payload_json["requestedServiceTier"], "priority");

    let summary_second = backfill_proxy_requested_service_tiers(&pool, None)
        .await
        .expect("second requested service tier backfill should be idempotent");
    assert_eq!(summary_second.scanned, 0);
    assert_eq!(summary_second.updated, 0);

    let _ = fs::remove_dir_all(&temp_dir);
}

#[tokio::test]
async fn backfill_proxy_requested_service_tiers_tracks_skip_counters() {
    let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
        .await
        .expect("connect in-memory sqlite");
    ensure_schema(&pool)
        .await
        .expect("schema should initialize");

    let temp_dir = make_temp_test_dir("proxy-requested-service-tier-backfill-skips");
    let missing_tier_request_path = temp_dir.join("request-missing-tier.json");
    let invalid_json_request_path = temp_dir.join("request-invalid-json.json");
    let missing_file_request_path = temp_dir.join("request-missing.json");

    write_backfill_request_payload_with_requested_service_tier(
        &missing_tier_request_path,
        None,
        ProxyCaptureTarget::Responses,
    );
    fs::write(&invalid_json_request_path, b"not-json").expect("write invalid request payload");

    let base_payload = r#"{"endpoint":"/v1/responses"}"#;
    insert_proxy_prompt_cache_backfill_row(
        &pool,
        "proxy-requested-tier-missing-file",
        &missing_file_request_path,
        base_payload,
    )
    .await;
    insert_proxy_prompt_cache_backfill_row(
        &pool,
        "proxy-requested-tier-invalid-json",
        &invalid_json_request_path,
        base_payload,
    )
    .await;
    insert_proxy_prompt_cache_backfill_row(
        &pool,
        "proxy-requested-tier-missing-tier",
        &missing_tier_request_path,
        base_payload,
    )
    .await;

    let summary = backfill_proxy_requested_service_tiers(&pool, None)
        .await
        .expect("requested service tier backfill should succeed");
    assert_eq!(summary.scanned, 3);
    assert_eq!(summary.updated, 0);
    assert_eq!(summary.skipped_missing_file, 1);
    assert_eq!(summary.skipped_invalid_json, 1);
    assert_eq!(summary.skipped_missing_tier, 1);

    let _ = fs::remove_dir_all(&temp_dir);
}

#[tokio::test]
async fn backfill_proxy_reasoning_efforts_updates_payload_and_is_idempotent() {
    let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
        .await
        .expect("connect in-memory sqlite");
    ensure_schema(&pool)
        .await
        .expect("schema should initialize");

    let temp_dir = make_temp_test_dir("proxy-reasoning-effort-backfill");
    let request_path = temp_dir.join("request.json");
    write_backfill_request_payload_with_reasoning(
        &request_path,
        Some("pck-reasoning"),
        Some("high"),
        ProxyCaptureTarget::Responses,
    );

    insert_proxy_prompt_cache_backfill_row(
        &pool,
        "proxy-reasoning-backfill-1",
        &request_path,
        r#"{"endpoint":"/v1/responses","requesterIp":"198.51.100.77"}"#,
    )
    .await;
    insert_proxy_prompt_cache_backfill_row(
        &pool,
        "proxy-reasoning-backfill-ready",
        &request_path,
        r#"{"endpoint":"/v1/responses","reasoningEffort":"medium"}"#,
    )
    .await;

    let summary_first = backfill_proxy_reasoning_efforts(&pool, None)
        .await
        .expect("first reasoning effort backfill should succeed");
    assert_eq!(summary_first.scanned, 1);
    assert_eq!(summary_first.updated, 1);
    assert_eq!(summary_first.skipped_missing_file, 0);
    assert_eq!(summary_first.skipped_invalid_json, 0);
    assert_eq!(summary_first.skipped_missing_effort, 0);

    let payload: String =
        sqlx::query_scalar("SELECT payload FROM codex_invocations WHERE invoke_id = ?1")
            .bind("proxy-reasoning-backfill-1")
            .fetch_one(&pool)
            .await
            .expect("query reasoning backfilled payload");
    let payload_json: Value = serde_json::from_str(&payload).expect("decode payload JSON");
    assert_eq!(payload_json["reasoningEffort"], "high");

    let summary_second = backfill_proxy_reasoning_efforts(&pool, None)
        .await
        .expect("second reasoning effort backfill should succeed");
    assert_eq!(summary_second.scanned, 0);
    assert_eq!(summary_second.updated, 0);

    let _ = fs::remove_dir_all(&temp_dir);
}

#[tokio::test]
async fn backfill_proxy_reasoning_efforts_tracks_skip_counters_and_chat_payloads() {
    let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
        .await
        .expect("connect in-memory sqlite");
    ensure_schema(&pool)
        .await
        .expect("schema should initialize");

    let temp_dir = make_temp_test_dir("proxy-reasoning-effort-backfill-skips");
    let ok_chat_path = temp_dir.join("request-chat-ok.json");
    let missing_effort_path = temp_dir.join("request-missing-effort.json");
    let invalid_json_path = temp_dir.join("request-invalid-json.json");
    let missing_file_path = temp_dir.join("request-missing.json");

    write_backfill_request_payload_with_reasoning(
        &ok_chat_path,
        None,
        Some("medium"),
        ProxyCaptureTarget::ChatCompletions,
    );
    write_backfill_request_payload_with_reasoning(
        &missing_effort_path,
        None,
        None,
        ProxyCaptureTarget::Responses,
    );
    fs::write(&invalid_json_path, b"not-json").expect("write invalid request payload");

    let base_payload = r#"{"endpoint":"/v1/chat/completions"}"#;
    insert_proxy_prompt_cache_backfill_row(
        &pool,
        "proxy-reasoning-chat-ok",
        &ok_chat_path,
        base_payload,
    )
    .await;
    insert_proxy_prompt_cache_backfill_row(
        &pool,
        "proxy-reasoning-missing-file",
        &missing_file_path,
        base_payload,
    )
    .await;
    insert_proxy_prompt_cache_backfill_row(
        &pool,
        "proxy-reasoning-invalid-json",
        &invalid_json_path,
        base_payload,
    )
    .await;
    insert_proxy_prompt_cache_backfill_row(
        &pool,
        "proxy-reasoning-missing-effort",
        &missing_effort_path,
        r#"{"endpoint":"/v1/responses"}"#,
    )
    .await;

    let summary = backfill_proxy_reasoning_efforts(&pool, None)
        .await
        .expect("reasoning effort backfill should succeed");
    assert_eq!(summary.scanned, 4);
    assert_eq!(summary.updated, 1);
    assert_eq!(summary.skipped_missing_file, 1);
    assert_eq!(summary.skipped_invalid_json, 1);
    assert_eq!(summary.skipped_missing_effort, 1);

    let payload: String =
        sqlx::query_scalar("SELECT payload FROM codex_invocations WHERE invoke_id = ?1")
            .bind("proxy-reasoning-chat-ok")
            .fetch_one(&pool)
            .await
            .expect("query chat reasoning payload");
    let payload_json: Value = serde_json::from_str(&payload).expect("decode payload JSON");
    assert_eq!(payload_json["reasoningEffort"], "medium");

    let _ = fs::remove_dir_all(&temp_dir);
}

#[tokio::test]
async fn backfill_proxy_prompt_cache_keys_reads_from_fallback_root_for_relative_paths() {
    let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
        .await
        .expect("connect in-memory sqlite");
    ensure_schema(&pool)
        .await
        .expect("schema should initialize");

    let temp_dir = make_temp_test_dir("proxy-prompt-cache-key-backfill-fallback");
    let fallback_root = temp_dir.join("legacy-root");
    let relative_path = PathBuf::from("proxy_raw_payloads/request-fallback.json");
    let request_path = fallback_root.join(&relative_path);
    let request_dir = request_path.parent().expect("request parent dir");
    fs::create_dir_all(request_dir).expect("create fallback request dir");
    write_backfill_request_payload(&request_path, Some("pck-fallback-1"));

    insert_proxy_prompt_cache_backfill_row(
        &pool,
        "proxy-pck-backfill-fallback",
        &relative_path,
        "{\"endpoint\":\"/v1/responses\",\"requesterIp\":\"198.51.100.77\"}",
    )
    .await;

    let summary = backfill_proxy_prompt_cache_keys(&pool, Some(&fallback_root))
        .await
        .expect("prompt cache key backfill with fallback root should succeed");
    assert_eq!(summary.scanned, 1);
    assert_eq!(summary.updated, 1);
    assert_eq!(summary.skipped_missing_file, 0);
    assert_eq!(summary.skipped_invalid_json, 0);
    assert_eq!(summary.skipped_missing_key, 0);

    let payload: String =
        sqlx::query_scalar("SELECT payload FROM codex_invocations WHERE invoke_id = ?1")
            .bind("proxy-pck-backfill-fallback")
            .fetch_one(&pool)
            .await
            .expect("query fallback-backfilled payload");
    let payload_json: Value = serde_json::from_str(&payload).expect("decode payload JSON");
    assert_eq!(payload_json["promptCacheKey"], "pck-fallback-1");

    let _ = fs::remove_dir_all(&temp_dir);
}

#[tokio::test]
async fn backfill_invocation_service_tiers_updates_payload_and_is_idempotent() {
    let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
        .await
        .expect("connect in-memory sqlite");
    ensure_schema(&pool)
        .await
        .expect("schema should initialize");

    let temp_dir = make_temp_test_dir("invocation-service-tier-backfill");
    let response_path = temp_dir.join("response.bin");
    write_backfill_response_payload_with_terminal_service_tier(
        &response_path,
        Some("auto"),
        Some("default"),
    );

    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id, occurred_at, source, status, payload, raw_response
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6)
        "#,
    )
    .bind("quota-service-tier-backfill")
    .bind("2026-02-23 00:00:00")
    .bind(SOURCE_XY)
    .bind("success")
    .bind("{}")
    .bind(r#"{"service_tier":"priority"}"#)
    .execute(&pool)
    .await
    .expect("insert quota service tier row");

    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id, occurred_at, source, status, payload, raw_response, response_raw_path
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
        "#,
    )
    .bind("proxy-service-tier-backfill")
    .bind("2026-02-23 00:00:01")
    .bind(SOURCE_PROXY)
    .bind("success")
    .bind(r#"{"endpoint":"/v1/responses"}"#)
    .bind("{}")
    .bind(response_path.to_string_lossy().to_string())
    .execute(&pool)
    .await
    .expect("insert proxy service tier row");

    let summary_first = backfill_invocation_service_tiers(&pool, None)
        .await
        .expect("first service tier backfill should succeed");
    assert_eq!(summary_first.scanned, 2);
    assert_eq!(summary_first.updated, 2);
    assert_eq!(summary_first.skipped_missing_file, 0);
    assert_eq!(summary_first.skipped_missing_tier, 0);

    let quota_payload: String =
        sqlx::query_scalar("SELECT payload FROM codex_invocations WHERE invoke_id = ?1")
            .bind("quota-service-tier-backfill")
            .fetch_one(&pool)
            .await
            .expect("query quota payload");
    let quota_payload_json: Value =
        serde_json::from_str(&quota_payload).expect("decode quota payload JSON");
    assert_eq!(quota_payload_json["serviceTier"], "priority");

    let proxy_payload: String =
        sqlx::query_scalar("SELECT payload FROM codex_invocations WHERE invoke_id = ?1")
            .bind("proxy-service-tier-backfill")
            .fetch_one(&pool)
            .await
            .expect("query proxy payload");
    let proxy_payload_json: Value =
        serde_json::from_str(&proxy_payload).expect("decode proxy payload JSON");
    assert_eq!(proxy_payload_json["serviceTier"], "default");
    assert_eq!(
        proxy_payload_json["serviceTierBackfillVersion"],
        "stream-terminal-v1"
    );

    let summary_second = backfill_invocation_service_tiers(&pool, None)
        .await
        .expect("second service tier backfill should be idempotent");
    assert_eq!(summary_second.scanned, 0);
    assert_eq!(summary_second.updated, 0);

    let _ = fs::remove_dir_all(&temp_dir);
}
