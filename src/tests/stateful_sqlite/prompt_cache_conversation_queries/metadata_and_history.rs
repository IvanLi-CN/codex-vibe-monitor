use super::test_support::{
    fetch_prompt_cache_conversations, materialize_prompt_cache_hourly_rollups,
};
use super::*;
use serde_json::json;

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

    materialize_prompt_cache_hourly_rollups(&state.pool).await;

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
                 '$.serviceTier', ?20 \
             ) \
         WHERE invoke_id = ?21",
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
    .bind("preview-06")
    .execute(&state.pool)
    .await
    .expect("augment preview-06 extras");
    insert_row(
        &state.pool,
        "preview-secondary-source",
        now - ChronoDuration::hours(1),
        SOURCE_XY,
        "pck-preview",
        "success",
        999,
        9.99,
        "Secondary Source",
        Some(404),
        Some("Secondary Source"),
        "/v1/responses",
        "gpt-5.4",
    )
    .await;

    materialize_prompt_cache_hourly_rollups(&state.pool).await;

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
            "preview-secondary-source",
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
    assert_eq!(
        latest.proxy_display_name.as_deref(),
        Some("Secondary Source")
    );
    assert_eq!(latest.upstream_account_id, Some(404));
    assert_eq!(
        latest.upstream_account_name.as_deref(),
        Some("Secondary Source")
    );
    assert_eq!(latest.endpoint.as_deref(), Some("/v1/responses"));
    assert_eq!(latest.failure_class.as_deref(), Some("none"));
    assert_eq!(latest.route_mode.as_deref(), Some("pool"));
    assert_eq!(latest.source.as_deref(), Some(SOURCE_XY));

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

    let Json(expanded_response) = fetch_prompt_cache_conversations(
        State(state.clone()),
        Query(PromptCacheConversationsQuery {
            limit: Some(20),
            activity_hours: None,
            activity_minutes: None,
            page_size: None,
            cursor: None,
            snapshot_at: None,
            detail: None,
            recent_invocation_limit: Some(6),

            blocked_binding_upstream_account_id: None,

            blocked_binding_constraint_source: None,
        }),
    )
    .await
    .expect("non-paginated recent invocation limit should be honored");
    let expanded_conversation = expanded_response
        .conversations
        .iter()
        .find(|item| item.prompt_cache_key == "pck-preview")
        .expect("expanded pck-preview should be included");
    assert_eq!(expanded_conversation.recent_invocations.len(), 6);
    assert_eq!(
        expanded_conversation.recent_invocations[5].invoke_id,
        "preview-02"
    );

    let Json(expanded_compact_response) = fetch_prompt_cache_conversations(
        State(state.clone()),
        Query(PromptCacheConversationsQuery {
            limit: Some(20),
            activity_hours: None,
            activity_minutes: None,
            page_size: None,
            cursor: None,
            snapshot_at: None,
            detail: Some("compact".to_string()),
            recent_invocation_limit: Some(6),

            blocked_binding_upstream_account_id: None,

            blocked_binding_constraint_source: None,
        }),
    )
    .await
    .expect("compact non-paginated recent invocation limit should be honored");
    let expanded_compact_conversation = expanded_compact_response
        .conversations
        .iter()
        .find(|item| item.prompt_cache_key == "pck-preview")
        .expect("expanded compact pck-preview should be included");
    assert_eq!(expanded_compact_conversation.recent_invocations.len(), 6);
    assert_eq!(
        expanded_compact_conversation.recent_invocations[5].invoke_id,
        "preview-02"
    );

    sqlx::query(
        r#"
        INSERT INTO pool_upstream_accounts (
            id, kind, provider, display_name, status, enabled, plan_type, created_at, updated_at
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8)
        "#,
    )
    .bind(303_i64)
    .bind("oauth_codex")
    .bind("codex")
    .bind("Pool Gamma")
    .bind("active")
    .bind(1_i64)
    .bind("free")
    .bind(format_utc_iso(now))
    .execute(&state.pool)
    .await
    .expect("insert upstream account with stale row plan");
    sqlx::query(
        r#"
        INSERT INTO pool_upstream_account_limit_samples (
            account_id, captured_at, limit_id, limit_name, plan_type
        )
        VALUES (?1, ?2, ?3, ?4, ?5)
        "#,
    )
    .bind(303_i64)
    .bind(format_utc_iso(now + ChronoDuration::minutes(1)))
    .bind("primary")
    .bind("Primary")
    .bind("team")
    .execute(&state.pool)
    .await
    .expect("insert newer sample-backed plan type");

    let effective_plan_rows = query_prompt_cache_conversation_recent_invocations(
        &state.pool,
        InvocationSourceScope::All,
        &["pck-preview".to_string()],
        5,
        None,
    )
    .await
    .expect("recent invocation previews should resolve effective plan type");
    let sample_backed_plan = effective_plan_rows
        .iter()
        .find(|item| item.invoke_id == "preview-06")
        .expect("preview-06 should be included in direct rows");
    assert_eq!(
        sample_backed_plan.upstream_account_plan_type.as_deref(),
        Some("team")
    );

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
            .all(|item| item.invoke_id != "preview-secondary-source")
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
async fn prompt_cache_conversations_include_manual_binding_summaries() {
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
        .bind(24_i64)
        .bind(0.24_f64)
        .bind(json!({ "promptCacheKey": key }).to_string())
        .bind("{}")
        .bind(format_utc_iso_millis(occurred_at))
        .execute(pool)
        .await
        .expect("insert invocation row");
    }

    for (offset_minutes, key) in [
        (4_i64, "pck-binding-group"),
        (3_i64, "pck-binding-account"),
        (2_i64, "pck-binding-none"),
        (1_i64, "pck-binding-empty-group"),
    ] {
        insert_row(
            &state.pool,
            &format!("invoke-{key}"),
            now - ChronoDuration::minutes(offset_minutes),
            key,
        )
        .await;
    }

    sqlx::query(
        r#"
        INSERT INTO pool_upstream_accounts (
            id, kind, provider, display_name, group_name, status, enabled, plan_type, created_at, updated_at
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?9)
        "#,
    )
    .bind(512_i64)
    .bind("oauth_codex")
    .bind("codex")
    .bind("Codex Pro - Tokyo")
    .bind("Tokyo")
    .bind("active")
    .bind(1_i64)
    .bind("team")
    .bind(format_utc_iso(now))
    .execute(&state.pool)
    .await
    .expect("insert bound upstream account");

    sqlx::query(
        r#"
        INSERT INTO prompt_cache_conversation_bindings (
            prompt_cache_key,
            binding_kind,
            group_name,
            upstream_account_id,
            created_at,
            updated_at
        )
        VALUES (?1, ?2, ?3, ?4, datetime('now'), datetime('now'))
        "#,
    )
    .bind("pck-binding-group")
    .bind(PROMPT_CACHE_BINDING_KIND_GROUP)
    .bind("CIII")
    .bind(None::<i64>)
    .execute(&state.pool)
    .await
    .expect("insert group manual binding");

    sqlx::query(
        r#"
        INSERT INTO prompt_cache_conversation_bindings (
            prompt_cache_key,
            binding_kind,
            group_name,
            upstream_account_id,
            created_at,
            updated_at
        )
        VALUES (?1, ?2, ?3, ?4, datetime('now'), datetime('now'))
        "#,
    )
    .bind("pck-binding-account")
    .bind(PROMPT_CACHE_BINDING_KIND_UPSTREAM_ACCOUNT)
    .bind(None::<String>)
    .bind(Some(512_i64))
    .execute(&state.pool)
    .await
    .expect("insert account manual binding");

    sqlx::query(
        r#"
        INSERT INTO prompt_cache_conversation_bindings (
            prompt_cache_key,
            binding_kind,
            group_name,
            upstream_account_id,
            created_at,
            updated_at
        )
        VALUES (?1, ?2, ?3, ?4, datetime('now'), datetime('now'))
        "#,
    )
    .bind("pck-binding-empty-group")
    .bind(PROMPT_CACHE_BINDING_KIND_GROUP)
    .bind("   ")
    .bind(None::<i64>)
    .execute(&state.pool)
    .await
    .expect("insert empty group manual binding");

    materialize_prompt_cache_hourly_rollups(&state.pool).await;

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

    let group_conversation = response
        .conversations
        .iter()
        .find(|item| item.prompt_cache_key == "pck-binding-group")
        .expect("group conversation should be included");
    let group_binding = group_conversation
        .manual_binding
        .as_ref()
        .expect("group manual binding should be present");
    assert_eq!(group_binding.binding_kind, "group");
    assert_eq!(group_binding.group_name.as_deref(), Some("CIII"));
    assert_eq!(group_binding.upstream_account_id, None);
    assert_eq!(group_binding.upstream_account_name, None);

    let account_conversation = response
        .conversations
        .iter()
        .find(|item| item.prompt_cache_key == "pck-binding-account")
        .expect("account conversation should be included");
    let account_binding = account_conversation
        .manual_binding
        .as_ref()
        .expect("account manual binding should be present");
    assert_eq!(account_binding.binding_kind, "upstreamAccount");
    assert_eq!(account_binding.group_name, None);
    assert_eq!(account_binding.upstream_account_id, Some(512));
    assert_eq!(
        account_binding.upstream_account_name.as_deref(),
        Some("Codex Pro - Tokyo")
    );

    let none_conversation = response
        .conversations
        .iter()
        .find(|item| item.prompt_cache_key == "pck-binding-none")
        .expect("unbound conversation should be included");
    assert!(none_conversation.manual_binding.is_none());

    let empty_group_conversation = response
        .conversations
        .iter()
        .find(|item| item.prompt_cache_key == "pck-binding-empty-group")
        .expect("empty-group conversation should be included");
    assert!(empty_group_conversation.manual_binding.is_none());
}

#[tokio::test]
async fn prompt_cache_recent_invocations_keep_per_key_limits_for_snapshot_and_proxy_scope() {
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

    let _alpha_older = insert_row(
        &state.pool,
        "recent-alpha-older",
        snapshot_second - ChronoDuration::seconds(30),
        SOURCE_PROXY,
        "recent-alpha",
    )
    .await;
    let alpha_boundary_id = insert_row(
        &state.pool,
        "recent-alpha-boundary",
        snapshot_second,
        SOURCE_PROXY,
        "recent-alpha",
    )
    .await;
    let _alpha_late_same_second = insert_row(
        &state.pool,
        "recent-alpha-late-same-second",
        snapshot_second,
        SOURCE_PROXY,
        "recent-alpha",
    )
    .await;
    let _alpha_post_snapshot = insert_row(
        &state.pool,
        "recent-alpha-post-snapshot",
        snapshot_second + ChronoDuration::seconds(2),
        SOURCE_PROXY,
        "recent-alpha",
    )
    .await;
    let _beta_proxy_old = insert_row(
        &state.pool,
        "recent-beta-proxy-old",
        snapshot_second - ChronoDuration::seconds(15),
        SOURCE_PROXY,
        "recent-beta",
    )
    .await;
    let _beta_proxy_new = insert_row(
        &state.pool,
        "recent-beta-proxy-new",
        snapshot_second - ChronoDuration::seconds(3),
        SOURCE_PROXY,
        "recent-beta",
    )
    .await;
    let _beta_xy = insert_row(
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

    let all_rows = query_prompt_cache_conversation_recent_invocations(
        &state.pool,
        InvocationSourceScope::All,
        &["recent-beta".to_string(), "recent-alpha".to_string()],
        2,
        Some(&snapshot),
    )
    .await
    .expect("all-scope recent preview query should succeed");

    assert_eq!(
        all_rows
            .iter()
            .map(|row| (row.prompt_cache_key.as_str(), row.invoke_id.as_str()))
            .collect::<Vec<_>>(),
        vec![
            ("recent-alpha", "recent-alpha-boundary"),
            ("recent-alpha", "recent-alpha-older"),
            ("recent-beta", "recent-beta-xy"),
            ("recent-beta", "recent-beta-proxy-new"),
        ]
    );

    let proxy_only_rows = query_prompt_cache_conversation_recent_invocations(
        &state.pool,
        InvocationSourceScope::ProxyOnly,
        &["recent-beta".to_string(), "recent-alpha".to_string()],
        2,
        Some(&snapshot),
    )
    .await
    .expect("proxy-only recent preview query should succeed");

    assert_eq!(
        proxy_only_rows
            .iter()
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
        .find(|item| item.prompt_cache_key == "pck-preview-empty")
        .expect("conversation should survive through hourly rollups");

    assert_eq!(conversation.request_count, 2);
    assert_eq!(conversation.total_tokens, 300);
    assert!((conversation.total_cost - 0.30).abs() < 1e-9);
    assert!(conversation.recent_invocations.is_empty());
}

#[tokio::test]
async fn prompt_cache_conversations_live_response_includes_encrypted_owner_metadata() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    enable_encrypted_session_owner_routing_for_test(&state).await;
    let group_name = "prompt-cache-live-owner-group";
    let owner_account_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Prompt Cache Live Owner",
        "sk-prompt-cache-live-owner",
        Some(group_name),
        None,
        None,
    )
    .await;
    let occurred_at = Utc::now() - ChronoDuration::minutes(10);

    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response, created_at
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
        "#,
    )
    .bind("prompt-cache-live-owner")
    .bind(format_naive(
        occurred_at.with_timezone(&Shanghai).naive_local(),
    ))
    .bind(SOURCE_PROXY)
    .bind("success")
    .bind(42)
    .bind(0.42)
    .bind(json!({ "promptCacheKey": "prompt-cache-live-owner" }).to_string())
    .bind("{}")
    .bind(format_utc_iso_millis(occurred_at))
    .execute(&state.pool)
    .await
    .expect("insert prompt cache invocation row");

    upsert_prompt_cache_encrypted_session_owner(
        &state.pool,
        "prompt-cache-live-owner",
        owner_account_id,
    )
    .await
    .expect("persist live owner");

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
        .find(|item| item.prompt_cache_key == "prompt-cache-live-owner")
        .expect("live owner conversation should exist");
    assert!(conversation.has_encrypted_session_owner);
    assert_eq!(
        conversation.encrypted_owner_account_id,
        Some(owner_account_id)
    );
    assert_eq!(
        conversation.encrypted_owner_account_name.as_deref(),
        Some("Prompt Cache Live Owner")
    );
    assert_eq!(
        conversation.encrypted_owner_group_name.as_deref(),
        Some(group_name)
    );
}

#[tokio::test]
async fn prompt_cache_conversations_snapshot_excludes_future_encrypted_owner_lock() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    enable_encrypted_session_owner_routing_for_test(&state).await;
    let group_name = "prompt-cache-snapshot-owner-group";
    let owner_account_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Prompt Cache Snapshot Owner",
        "sk-prompt-cache-snapshot-owner",
        Some(group_name),
        None,
        None,
    )
    .await;
    let key = "prompt-cache-snapshot-owner";
    let first_at = Utc::now() - ChronoDuration::minutes(4);

    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response, created_at
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
        "#,
    )
    .bind("prompt-cache-snapshot-owner-initial")
    .bind(format_naive(first_at.with_timezone(&Shanghai).naive_local()))
    .bind(SOURCE_PROXY)
    .bind("success")
    .bind(11)
    .bind(0.11)
    .bind(json!({ "promptCacheKey": key }).to_string())
    .bind("{}")
    .bind(format_utc_iso_millis(first_at))
    .execute(&state.pool)
    .await
    .expect("insert initial invocation row");

    materialize_prompt_cache_hourly_rollups(&state.pool).await;

    let snapshot_at = format_utc_iso_precise(first_at + ChronoDuration::minutes(1));

    let second_at = Utc::now() - ChronoDuration::minutes(1);
    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response, created_at
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
        "#,
    )
    .bind("prompt-cache-snapshot-owner-encrypted")
    .bind(format_naive(second_at.with_timezone(&Shanghai).naive_local()))
    .bind(SOURCE_PROXY)
    .bind("success")
    .bind(17)
    .bind(0.17)
    .bind(
        json!({
            "promptCacheKey": key,
            "upstreamAccountId": owner_account_id,
            "upstreamAccountName": "Prompt Cache Snapshot Owner",
            "responseContainsEncryptedContent": true
        })
        .to_string(),
    )
    .bind("{}")
    .bind(format_utc_iso_millis(second_at))
    .execute(&state.pool)
    .await
    .expect("insert later encrypted invocation row");
    upsert_prompt_cache_encrypted_session_owner(&state.pool, key, owner_account_id)
        .await
        .expect("persist current owner after encrypted success");

    materialize_prompt_cache_hourly_rollups(&state.pool).await;

    let Json(snapshot_page) = fetch_prompt_cache_conversations(
        State(state.clone()),
        Query(PromptCacheConversationsQuery {
            limit: None,
            activity_hours: None,
            activity_minutes: Some(5),
            page_size: Some(20),
            cursor: None,
            snapshot_at: Some(snapshot_at),
            detail: Some("compact".to_string()),
            recent_invocation_limit: None,

            blocked_binding_upstream_account_id: None,

            blocked_binding_constraint_source: None,
        }),
    )
    .await
    .expect("historical snapshot page should succeed");

    let historical = snapshot_page
        .conversations
        .iter()
        .find(|item| item.prompt_cache_key == key)
        .expect("historical conversation should exist");
    assert!(!historical.has_encrypted_session_owner);
    assert_eq!(historical.encrypted_owner_account_id, None);
    assert_eq!(historical.encrypted_owner_account_name, None);
    assert_eq!(historical.encrypted_owner_group_name, None);

    let Json(current_page) = fetch_prompt_cache_conversations(
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
    .expect("current page should succeed");

    let current = current_page
        .conversations
        .iter()
        .find(|item| item.prompt_cache_key == key)
        .expect("current conversation should exist");
    assert!(current.has_encrypted_session_owner);
    assert_eq!(current.encrypted_owner_account_id, Some(owner_account_id));
    assert_eq!(
        current.encrypted_owner_account_name.as_deref(),
        Some("Prompt Cache Snapshot Owner")
    );
    assert_eq!(
        current.encrypted_owner_group_name.as_deref(),
        Some(group_name)
    );
}
