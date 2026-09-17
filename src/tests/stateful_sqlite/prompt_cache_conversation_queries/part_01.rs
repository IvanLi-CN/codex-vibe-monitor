async fn assert_manual_binding_summaries(state: &Arc<AppState>) {
    let Json(response) = fetch_prompt_cache_conversations(
        State(state.clone()),
        Query(PromptCacheConversationsQuery {
            limit: Some(20),

            ..prompt_cache_query()
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

async fn seed_manual_conversation_bindings(state: &Arc<AppState>) {
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
}

async fn seed_manual_binding_conversations(state: &Arc<AppState>, now: DateTime<Utc>) {
    for (offset_minutes, key) in [
        (4_i64, "pck-binding-group"),
        (3_i64, "pck-binding-account"),
        (2_i64, "pck-binding-none"),
        (1_i64, "pck-binding-empty-group"),
    ] {
        insert_prompt_cache_row_01_02(
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
}

async fn assert_preview_plan_and_proxy_scope(state: &Arc<AppState>, now: DateTime<Utc>) {
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

fn assert_enriched_preview(conversation: &PromptCacheConversationResponse) {
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
}

async fn assert_preview_limit_overrides(state: &Arc<AppState>) {
    let Json(expanded_response) = fetch_prompt_cache_conversations(
        State(state.clone()),
        Query(PromptCacheConversationsQuery {
            limit: Some(20),
            recent_invocation_limit: Some(6),

            ..prompt_cache_query()
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
            detail: Some("compact".to_string()),
            recent_invocation_limit: Some(6),

            ..prompt_cache_query()
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
}

async fn assert_default_preview_response(state: &Arc<AppState>) {
    let Json(response) = fetch_prompt_cache_conversations(
        State(state.clone()),
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

    assert_enriched_preview(conversation);
}

async fn seed_secondary_preview(state: &Arc<AppState>, now: DateTime<Utc>) {
    insert_prompt_cache_row_01_01(
        &state.pool,
        PreviewInvocationSeed {
            invoke_id: "preview-secondary-source",
            occurred_at: now - ChronoDuration::hours(1),
            source: SOURCE_XY,
            key: "pck-preview",
            status: "success",
            total_tokens: 999,
            cost: 9.99,
            proxy_display_name: "Secondary Source",
            account_id: Some(404),
            account_name: Some("Secondary Source"),
            endpoint: "/v1/responses",
            model: "gpt-5.4",
        },
    )
    .await;

    materialize_prompt_cache_hourly_rollups(&state.pool).await;
}

async fn seed_enriched_preview(state: &Arc<AppState>, now: DateTime<Utc>) {
    insert_prompt_cache_row_01_01(
        &state.pool,
        PreviewInvocationSeed {
            invoke_id: "preview-06",
            occurred_at: now - ChronoDuration::hours(2),
            source: SOURCE_PROXY,
            key: "pck-preview",
            status: "success",
            total_tokens: 200,
            cost: 0.20,
            proxy_display_name: "Proxy Gamma",
            account_id: Some(303),
            account_name: Some("Pool Gamma"),
            endpoint: "/v1/responses",
            model: "gpt-5.4",
        },
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
}

async fn seed_preview_failure_variants(state: &Arc<AppState>, now: DateTime<Utc>) {
    sqlx::query(
    "UPDATE codex_invocations SET payload = json_set(payload, '$.reasoningEffort', 7) WHERE invoke_id = ?1",
)
.bind("preview-04")
.execute(&state.pool)
.await
.expect("mark preview-04 reasoning effort as non-text");
    insert_prompt_cache_row_01_01(
        &state.pool,
        PreviewInvocationSeed {
            invoke_id: "preview-05",
            occurred_at: now - ChronoDuration::hours(3),
            source: SOURCE_PROXY,
            key: "pck-preview",
            status: "success",
            total_tokens: 180,
            cost: 0.18,
            proxy_display_name: "Proxy Gamma",
            account_id: Some(303),
            account_name: Some("Pool Gamma"),
            endpoint: "/v1/responses",
            model: "gpt-5.4",
        },
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
}

async fn seed_base_previews(state: &Arc<AppState>, now: DateTime<Utc>) {
    insert_prompt_cache_row_01_01(
        &state.pool,
        PreviewInvocationSeed {
            invoke_id: "preview-01",
            occurred_at: now - ChronoDuration::hours(7),
            source: SOURCE_PROXY,
            key: "pck-preview",
            status: "success",
            total_tokens: 100,
            cost: 0.10,
            proxy_display_name: "Proxy Alpha",
            account_id: Some(101),
            account_name: Some("Pool Alpha"),
            endpoint: "/v1/responses",
            model: "gpt-5.4",
        },
    )
    .await;
    insert_prompt_cache_row_01_01(
        &state.pool,
        PreviewInvocationSeed {
            invoke_id: "preview-02",
            occurred_at: now - ChronoDuration::hours(6),
            source: SOURCE_PROXY,
            key: "pck-preview",
            status: "success",
            total_tokens: 120,
            cost: 0.12,
            proxy_display_name: "Proxy Alpha",
            account_id: Some(101),
            account_name: Some("Pool Alpha"),
            endpoint: "/v1/responses",
            model: "gpt-5.4",
        },
    )
    .await;
    insert_prompt_cache_row_01_01(
        &state.pool,
        PreviewInvocationSeed {
            invoke_id: "preview-03",
            occurred_at: now - ChronoDuration::hours(5),
            source: SOURCE_PROXY,
            key: "pck-preview",
            status: "http_502",
            total_tokens: 140,
            cost: 0.14,
            proxy_display_name: "Proxy Beta",
            account_id: None,
            account_name: None,
            endpoint: "/v1/chat/completions",
            model: "gpt-5.4-mini",
        },
    )
    .await;
    insert_prompt_cache_row_01_01(
        &state.pool,
        PreviewInvocationSeed {
            invoke_id: "preview-04",
            occurred_at: now - ChronoDuration::hours(4),
            source: SOURCE_PROXY,
            key: "pck-preview",
            status: "success",
            total_tokens: 160,
            cost: 0.16,
            proxy_display_name: "Proxy Beta",
            account_id: Some(202),
            account_name: None,
            endpoint: "/v1/responses",
            model: "gpt-5.4-mini",
        },
    )
    .await;
}

struct PreviewInvocationSeed<'a> {
    invoke_id: &'a str,
    occurred_at: DateTime<Utc>,
    source: &'a str,
    key: &'a str,
    status: &'a str,
    total_tokens: i64,
    cost: f64,
    proxy_display_name: &'a str,
    account_id: Option<i64>,
    account_name: Option<&'a str>,
    endpoint: &'a str,
    model: &'a str,
}

async fn insert_prompt_cache_row_01_01(pool: &Pool<Sqlite>, seed: PreviewInvocationSeed<'_>) {
    let mut payload = json!({
        "promptCacheKey": seed.key,
        "proxyDisplayName": seed.proxy_display_name,
        "endpoint": seed.endpoint,
        "model": seed.model,
        "routeMode": "pool",
    });
    if let Some(account_id) = seed.account_id {
        payload["upstreamAccountId"] = json!(account_id);
    }
    if let Some(account_name) = seed.account_name {
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
    .bind(seed.invoke_id)
    .bind(format_naive(
        seed.occurred_at.with_timezone(&Shanghai).naive_local(),
    ))
    .bind(seed.source)
    .bind(seed.status)
    .bind(seed.model)
    .bind(seed.total_tokens)
    .bind(seed.cost)
    .bind(payload.to_string())
    .bind("{}")
    .execute(pool)
    .await
    .expect("insert invocation row");
}

async fn insert_prompt_cache_row_01_02(
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

pub(crate) async fn materialize_prompt_cache_hourly_rollups(pool: &Pool<Sqlite>) {
    sync_hourly_rollups_from_live_tables(pool)
        .await
        .expect("materialize prompt-cache hourly rollups for read-only prompt-cache tests");
}

struct UpstreamSummarySeed<'a> {
    invoke_id: &'a str,
    occurred_at: DateTime<Utc>,
    account_id: Option<i64>,
    account_name: Option<&'a str>,
    total_tokens: i64,
    cost: f64,
}

async fn insert_upstream_summary_seed(pool: &Pool<Sqlite>, seed: UpstreamSummarySeed<'_>) {
    let mut payload = json!({ "promptCacheKey": "pck-upstream" });
    if let Some(account_id) = seed.account_id {
        payload["upstreamAccountId"] = json!(account_id);
    }
    if let Some(account_name) = seed.account_name {
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
    .bind(seed.invoke_id)
    .bind(format_naive(
        seed.occurred_at.with_timezone(&Shanghai).naive_local(),
    ))
    .bind(SOURCE_PROXY)
    .bind("success")
    .bind(seed.total_tokens)
    .bind(seed.cost)
    .bind(payload.to_string())
    .bind("{}")
    .bind(format_utc_iso_millis(seed.occurred_at))
    .execute(pool)
    .await
    .expect("insert upstream summary invocation");
}

fn assert_recent_upstream_summaries(response: &PromptCacheConversationsResponse) {
    let accounts = &prompt_cache_conversation(response, "pck-upstream").upstream_accounts;
    assert_eq!(accounts.len(), 3);
    let first = &accounts[0];
    assert_eq!(first.upstream_account_id, Some(9));
    assert_eq!(first.upstream_account_name.as_deref(), Some("Gamma"));
    assert_eq!((first.request_count, first.total_tokens), (1, 30));

    let second = &accounts[1];
    assert_eq!(second.upstream_account_id, None);
    assert_eq!(second.upstream_account_name, None);
    assert_eq!((second.request_count, second.total_tokens), (1, 25));
    assert!((second.total_cost - 0.25).abs() < 1e-9);

    let third = &accounts[2];
    assert_eq!(third.upstream_account_id, Some(2));
    assert_eq!(third.upstream_account_name.as_deref(), Some("Beta"));
    assert_eq!((third.request_count, third.total_tokens), (2, 55));
    assert!((third.total_cost - 0.55).abs() < 1e-9);
    assert!(
        accounts
            .iter()
            .all(|account| !matches!(account.upstream_account_id, Some(1 | 7)))
    );
    assert!(accounts.iter().any(|account| {
        account.upstream_account_id.is_none() && account.upstream_account_name.is_none()
    }));
}

#[tokio::test]
pub(crate) async fn prompt_cache_conversations_include_recent_upstream_account_summaries() {
    let state = prompt_cache_test_state().await;
    let now = Utc::now();
    for seed in [
        UpstreamSummarySeed {
            invoke_id: "pck-upstream-beta-history",
            occurred_at: now - ChronoDuration::hours(48),
            account_id: None,
            account_name: Some("Beta"),
            total_tokens: 40,
            cost: 0.4,
        },
        UpstreamSummarySeed {
            invoke_id: "pck-upstream-alpha",
            occurred_at: now - ChronoDuration::hours(6),
            account_id: Some(1),
            account_name: Some("Alpha"),
            total_tokens: 10,
            cost: 0.1,
        },
        UpstreamSummarySeed {
            invoke_id: "pck-upstream-id-only",
            occurred_at: now - ChronoDuration::hours(3),
            account_id: Some(7),
            account_name: None,
            total_tokens: 20,
            cost: 0.2,
        },
        UpstreamSummarySeed {
            invoke_id: "pck-upstream-beta-recent",
            occurred_at: now - ChronoDuration::hours(2),
            account_id: Some(2),
            account_name: Some("Beta"),
            total_tokens: 15,
            cost: 0.15,
        },
        UpstreamSummarySeed {
            invoke_id: "pck-upstream-gamma",
            occurred_at: now - ChronoDuration::hours(1),
            account_id: Some(9),
            account_name: Some("Gamma"),
            total_tokens: 30,
            cost: 0.3,
        },
        UpstreamSummarySeed {
            invoke_id: "pck-upstream-unknown",
            occurred_at: now - ChronoDuration::minutes(90),
            account_id: None,
            account_name: None,
            total_tokens: 25,
            cost: 0.25,
        },
    ] {
        insert_upstream_summary_seed(&state.pool, seed).await;
    }

    materialize_prompt_cache_hourly_rollups(&state.pool).await;
    let response = fetch_prompt_cache_test_response(
        state,
        PromptCacheConversationsQuery {
            limit: Some(20),
            ..prompt_cache_query()
        },
    )
    .await;
    assert_recent_upstream_summaries(&response);
}

#[tokio::test]
pub(crate) async fn prompt_cache_conversations_include_recent_invocation_previews_with_limit_and_proxy_scope()
 {
    let state = prompt_cache_test_state().await;
    let now = Utc::now();

    seed_base_previews(&state, now).await;
    seed_preview_failure_variants(&state, now).await;
    seed_enriched_preview(&state, now).await;
    seed_secondary_preview(&state, now).await;
    assert_default_preview_response(&state).await;
    assert_preview_limit_overrides(&state).await;
    assert_preview_plan_and_proxy_scope(&state, now).await;
}

#[tokio::test]
pub(crate) async fn prompt_cache_conversations_include_manual_binding_summaries() {
    let state = prompt_cache_test_state().await;
    let now = Utc::now();

    seed_manual_binding_conversations(&state, now).await;
    seed_manual_conversation_bindings(&state).await;
    assert_manual_binding_summaries(&state).await;
}

use super::*;
