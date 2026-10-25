use super::*;
#[tokio::test]
async fn prompt_cache_conversation_binding_patch_is_mutually_exclusive_and_clearable() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let group_name = "prompt-cache-bindings-api-group";
    ensure_test_group_binding(&state.pool, group_name, None).await;
    let account_id = insert_test_pool_oauth_account(
        &state,
        "Prompt Cache Binding OAuth",
        "oauth-prompt-cache-binding",
    )
    .await;
    set_test_account_group_name(&state.pool, account_id, Some(group_name)).await;
    let unselectable_account_id = insert_test_pool_oauth_account(
        &state,
        "Prompt Cache Binding Unselectable OAuth",
        "oauth-prompt-cache-binding-unselectable",
    )
    .await;
    set_test_account_group_name(&state.pool, unselectable_account_id, Some(group_name)).await;
    sqlx::query("UPDATE pool_upstream_accounts SET encrypted_credentials = NULL WHERE id = ?1")
        .bind(unselectable_account_id)
        .execute(&state.pool)
        .await
        .expect("make prompt cache binding target unselectable");
    sqlx::query(
        r#"
        UPDATE pool_upstream_accounts
        SET policy_allow_cut_out = 0,
            policy_fast_mode_rewrite_mode = 'force_remove',
            policy_image_tool_rewrite_mode = 'fill_missing',
            policy_codex_imagegen_rewrite_mode = 'fill_missing',
            policy_available_models_json = '["gpt-5.1-codex-mini"]'
        WHERE id = ?1
        "#,
    )
    .bind(account_id)
    .execute(&state.pool)
    .await
    .expect("seed account policy for inherited conversation response");

    let both_payload: UpdatePromptCacheConversationBindingRequest = serde_json::from_value(json!({
        "bindingKind": "group",
        "groupName": group_name,
        "upstreamAccountId": account_id,
    }))
    .expect("deserialize mutually exclusive binding payload");
    let prompt_cache_key = "prompt-cache-binding-api+literal&part=value";
    let both_err = patch_prompt_cache_conversation_binding(
        State(state.clone()),
        AxumPath(format!("  {prompt_cache_key}  ")),
        Json(both_payload),
    )
    .await
    .expect_err("mutually exclusive binding payload should fail");
    assert!(matches!(both_err, ApiError::BadRequest(_)));

    let group_payload: UpdatePromptCacheConversationBindingRequest =
        serde_json::from_value(json!({
            "bindingKind": "group",
            "groupName": group_name,
        }))
        .expect("deserialize group binding payload");
    let Json(group_response) = patch_prompt_cache_conversation_binding(
        State(state.clone()),
        AxumPath(format!("  {prompt_cache_key}  ")),
        Json(group_payload),
    )
    .await
    .expect("group binding should save");
    assert_eq!(group_response.prompt_cache_key, prompt_cache_key);
    assert_eq!(group_response.binding_kind, "group");
    assert_eq!(group_response.group_name.as_deref(), Some(group_name));
    assert_eq!(group_response.upstream_account_id, None);

    let inherited_account_payload: UpdatePromptCacheConversationBindingRequest =
        serde_json::from_value(json!({
            "bindingKind": "upstreamAccount",
            "upstreamAccountId": account_id,
        }))
        .expect("deserialize inherited account binding payload");
    let Json(inherited_account_response) = patch_prompt_cache_conversation_binding(
        State(state.clone()),
        AxumPath(prompt_cache_key.to_string()),
        Json(inherited_account_payload),
    )
    .await
    .expect("account binding without policy override should save");
    assert_eq!(
        inherited_account_response.allow_switch_upstream,
        Some(false)
    );
    assert_eq!(
        inherited_account_response.fast_mode_rewrite_mode,
        Some(TagFastModeRewriteMode::ForceRemove)
    );
    assert_eq!(
        inherited_account_response.image_tool_rewrite_mode,
        Some(ImageToolRewriteMode::FillMissing)
    );
    assert_eq!(
        inherited_account_response.codex_imagegen_rewrite_mode,
        Some(CodexImagegenRewriteMode::FillMissing)
    );
    assert_eq!(
        inherited_account_response.available_models,
        Some(vec!["gpt-5.1-codex-mini".to_string()])
    );
    assert_eq!(
        inherited_account_response
            .policy_field_sources
            .allow_switch_upstream,
        "account"
    );
    assert_eq!(
        inherited_account_response
            .policy_field_sources
            .fast_mode_rewrite_mode,
        "account"
    );
    assert_eq!(
        inherited_account_response
            .policy_field_sources
            .image_tool_rewrite_mode,
        "account"
    );
    assert_eq!(
        inherited_account_response
            .policy_field_sources
            .codex_imagegen_rewrite_mode,
        "account"
    );
    assert_eq!(
        inherited_account_response
            .policy_field_sources
            .available_models,
        "account"
    );

    let unselectable_payload: UpdatePromptCacheConversationBindingRequest =
        serde_json::from_value(json!({
            "bindingKind": "upstreamAccount",
            "upstreamAccountId": unselectable_account_id,
        }))
        .expect("deserialize unselectable account binding payload");
    let unselectable_err = patch_prompt_cache_conversation_binding(
        State(state.clone()),
        AxumPath(prompt_cache_key.to_string()),
        Json(unselectable_payload),
    )
    .await
    .expect_err("unselectable account binding should fail");
    assert!(matches!(unselectable_err, ApiError::BadRequest(_)));

    let account_payload: UpdatePromptCacheConversationBindingRequest =
        serde_json::from_value(json!({
            "bindingKind": "upstreamAccount",
            "upstreamAccountId": account_id,
            "allowSwitchUpstream": true,
            "fastModeRewriteMode": "force_add",
            "imageToolRewriteMode": "force_remove",
            "codexImagegenRewriteMode": "force_add",
            "availableModels": ["gpt-5.1-codex-max", "gpt-5.1-codex-max", "gpt-5.1-codex-mini"],
            "forwardProxyKey": "__direct__",
        }))
        .expect("deserialize account binding payload");
    let Json(account_response) = patch_prompt_cache_conversation_binding(
        State(state.clone()),
        AxumPath(prompt_cache_key.to_string()),
        Json(account_payload),
    )
    .await
    .expect("account binding should save");
    assert_eq!(account_response.binding_kind, "upstreamAccount");
    assert_eq!(account_response.group_name, None);
    assert_eq!(account_response.upstream_account_id, Some(account_id));
    assert_eq!(account_response.allow_switch_upstream, Some(true));
    assert_eq!(
        account_response.fast_mode_rewrite_mode,
        Some(TagFastModeRewriteMode::ForceAdd)
    );
    assert_eq!(
        account_response.image_tool_rewrite_mode,
        Some(ImageToolRewriteMode::ForceRemove)
    );
    assert_eq!(
        account_response.codex_imagegen_rewrite_mode,
        Some(CodexImagegenRewriteMode::ForceAdd)
    );
    assert_eq!(
        account_response.available_models,
        Some(vec![
            "gpt-5.1-codex-max".to_string(),
            "gpt-5.1-codex-mini".to_string()
        ])
    );
    assert_eq!(
        account_response.forward_proxy_key.as_deref(),
        Some("__direct__")
    );
    let conversation_override =
        load_prompt_cache_conversation_routing_override(&state.pool, Some(prompt_cache_key))
            .await
            .expect("conversation override should load")
            .expect("conversation override should exist");
    assert_eq!(conversation_override.allow_switch_upstream, Some(true));
    assert_eq!(
        conversation_override.fast_mode_rewrite_mode,
        Some(TagFastModeRewriteMode::ForceAdd)
    );
    assert_eq!(
        conversation_override.image_tool_rewrite_mode,
        Some(ImageToolRewriteMode::ForceRemove)
    );
    assert_eq!(
        conversation_override.codex_imagegen_rewrite_mode,
        Some(CodexImagegenRewriteMode::ForceAdd)
    );
    assert_eq!(
        conversation_override.available_models,
        Some(vec![
            "gpt-5.1-codex-max".to_string(),
            "gpt-5.1-codex-mini".to_string()
        ])
    );
    assert_eq!(
        conversation_override.forward_proxy_key.as_deref(),
        Some("__direct__")
    );
    let empty_models_payload: UpdatePromptCacheConversationBindingRequest =
        serde_json::from_value(json!({
            "bindingKind": "upstreamAccount",
            "upstreamAccountId": account_id,
            "availableModels": [],
        }))
        .expect("deserialize empty available models payload");
    let Json(empty_models_response) = patch_prompt_cache_conversation_binding(
        State(state.clone()),
        AxumPath(prompt_cache_key.to_string()),
        Json(empty_models_payload),
    )
    .await
    .expect("empty available models allowlist should save and reject all models");
    assert_eq!(empty_models_response.available_models, Some(Vec::new()));
    assert_eq!(
        empty_models_response.available_models_mode,
        Some(AvailableModelsMode::Allowlist)
    );
    let sticky_account_id: i64 =
        sqlx::query_scalar("SELECT account_id FROM pool_sticky_routes WHERE sticky_key = ?1")
            .bind(prompt_cache_key)
            .fetch_one(&state.pool)
            .await
            .expect("account binding should update sticky route");
    assert_eq!(sticky_account_id, account_id);

    let clear_payload: UpdatePromptCacheConversationBindingRequest =
        serde_json::from_value(json!({
            "bindingKind": "none",
            "allowSwitchUpstream": null,
            "fastModeRewriteMode": null,
            "imageToolRewriteMode": null,
            "codexImagegenRewriteMode": null,
            "availableModels": null,
            "forwardProxyKey": null,
        }))
        .expect("deserialize clear binding payload");
    let Json(clear_response) = patch_prompt_cache_conversation_binding(
        State(state.clone()),
        AxumPath(prompt_cache_key.to_string()),
        Json(clear_payload),
    )
    .await
    .expect("clear binding should delete row");
    assert_eq!(clear_response.binding_kind, "none");
    assert_eq!(clear_response.allow_switch_upstream, Some(false));
    assert_eq!(
        clear_response.fast_mode_rewrite_mode,
        Some(TagFastModeRewriteMode::ForceRemove)
    );
    assert_eq!(
        clear_response.image_tool_rewrite_mode,
        Some(ImageToolRewriteMode::FillMissing)
    );
    assert_eq!(
        clear_response.codex_imagegen_rewrite_mode,
        Some(CodexImagegenRewriteMode::FillMissing)
    );
    assert_eq!(
        clear_response.available_models,
        Some(vec!["gpt-5.1-codex-mini".to_string()])
    );
    assert_eq!(
        clear_response.forward_proxy_key.as_deref(),
        Some("__direct__")
    );
    assert_eq!(
        clear_response.policy_field_sources.allow_switch_upstream,
        "account"
    );
    assert_eq!(
        clear_response.policy_field_sources.fast_mode_rewrite_mode,
        "account"
    );
    assert_eq!(
        clear_response.policy_field_sources.image_tool_rewrite_mode,
        "account"
    );
    assert_eq!(
        clear_response
            .policy_field_sources
            .codex_imagegen_rewrite_mode,
        "account"
    );
    assert_eq!(
        clear_response.policy_field_sources.available_models,
        "account"
    );
    let cleared_conversation_override =
        load_prompt_cache_conversation_routing_override(&state.pool, Some(prompt_cache_key))
            .await
            .expect("cleared conversation override should load");
    assert!(
        cleared_conversation_override.is_none(),
        "null policy fields should clear stored conversation overrides"
    );
    let sticky_account_id_after_clear: i64 =
        sqlx::query_scalar("SELECT account_id FROM pool_sticky_routes WHERE sticky_key = ?1")
            .bind(prompt_cache_key)
            .fetch_one(&state.pool)
            .await
            .expect("clearing a binding should leave existing sticky route intact");
    assert_eq!(sticky_account_id_after_clear, account_id);
    let Json(get_response) =
        get_prompt_cache_conversation_binding(State(state), AxumPath(prompt_cache_key.to_string()))
            .await
            .expect("get cleared binding");
    assert_eq!(get_response.binding_kind, "none");
}

#[tokio::test]
async fn prompt_cache_conversation_proxy_override_bypasses_node_shunt_group_slots() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let group_name = "prompt-cache-proxy-override-node-shunt";
    let account_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Prompt Cache Proxy Override",
        "sk-prompt-cache-proxy-override",
        Some(group_name),
        None,
        None,
    )
    .await;
    sqlx::query(
        r#"
        UPDATE pool_upstream_account_group_notes
        SET bound_proxy_keys_json = '[]',
            node_shunt_enabled = 1
        WHERE group_name = ?1
        "#,
    )
    .bind(group_name)
    .execute(&state.pool)
    .await
    .expect("enable node shunt group with no selectable slot proxies");

    let prompt_cache_key = "prompt-cache-proxy-override-node-shunt";
    let payload: UpdatePromptCacheConversationBindingRequest = serde_json::from_value(json!({
        "bindingKind": "upstreamAccount",
        "upstreamAccountId": account_id,
        "forwardProxyKey": "__direct__",
    }))
    .expect("deserialize proxy override payload");
    let Json(_) = patch_prompt_cache_conversation_binding(
        State(state.clone()),
        AxumPath(prompt_cache_key.to_string()),
        Json(payload),
    )
    .await
    .expect("proxy override binding should save");

    let conversation_override =
        load_prompt_cache_conversation_routing_override(&state.pool, Some(prompt_cache_key))
            .await
            .expect("load conversation routing override");
    let resolution =
        resolve_pool_account_for_request_with_route_requirement_and_image_intent_and_override(
            state.as_ref(),
            Some(prompt_cache_key),
            Some("gpt-5.1-codex-max"),
            &[],
            &HashSet::new(),
            None,
            None,
            conversation_override.as_ref(),
            "/v1/chat/completions",
            ImageIntent::Unknown,
        )
        .await
        .expect("resolve account with conversation proxy override");
    let PoolAccountResolution::Resolved(account) = resolution else {
        panic!("conversation proxy override should resolve through direct node");
    };
    assert_eq!(account.account_id, account_id);
    match account.forward_proxy_scope {
        ForwardProxyRouteScope::BoundProxyKeys {
            scope_key,
            bound_proxy_keys,
        } => {
            assert_eq!(scope_key, format!("conversation:{prompt_cache_key}"));
            assert_eq!(bound_proxy_keys, vec![FORWARD_PROXY_DIRECT_KEY.to_string()]);
        }
        other => panic!("expected conversation bound proxy scope, got {other:?}"),
    }
}

#[tokio::test]
async fn prompt_cache_conversation_binding_route_accepts_encoded_key() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let group_name = "prompt-cache-bindings-route-group";
    ensure_test_group_binding(&state.pool, group_name, None).await;
    insert_test_pool_api_key_account_with_options(
        &state,
        "Prompt Cache Binding Route API",
        "sk-prompt-cache-binding-route-api",
        Some(group_name),
        None,
        None,
    )
    .await;
    let prompt_cache_key = "prompt-cache route/key+literal&part=value";
    let app = build_app_router(state.clone());
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind test app listener");
    let addr = listener.local_addr().expect("read test app listener addr");
    let server = tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("serve prompt cache binding test app");
    });

    let client = reqwest::Client::new();
    let encoded_key = prompt_cache_key
        .as_bytes()
        .iter()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (*byte as char).to_string()
            }
            _ => format!("%{byte:02X}"),
        })
        .collect::<String>();
    let patch_response = client
        .patch(format!(
            "http://{addr}/api/stats/prompt-cache-conversation-bindings/{encoded_key}"
        ))
        .json(&json!({
            "bindingKind": "group",
            "groupName": group_name,
        }))
        .send()
        .await
        .expect("patch route binding over router");
    let patch_status = patch_response.status();
    let patch_body = patch_response
        .text()
        .await
        .expect("read patch response body");
    assert_eq!(
        patch_status,
        reqwest::StatusCode::OK,
        "unexpected patch response body: {patch_body}"
    );
    let patched: Value = serde_json::from_str(&patch_body).expect("decode patch response");
    assert_eq!(patched["promptCacheKey"].as_str(), Some(prompt_cache_key));
    assert_eq!(patched["bindingKind"].as_str(), Some("group"));
    assert_eq!(patched["groupName"].as_str(), Some(group_name));

    let get_response = client
        .get(format!(
            "http://{addr}/api/stats/prompt-cache-conversation-bindings/{encoded_key}"
        ))
        .send()
        .await
        .expect("get route binding over router");
    let get_status = get_response.status();
    let get_body = get_response.text().await.expect("read get response body");
    assert_eq!(
        get_status,
        reqwest::StatusCode::OK,
        "unexpected get response body: {get_body}"
    );
    let fetched: Value = serde_json::from_str(&get_body).expect("decode get response");
    assert_eq!(fetched["promptCacheKey"].as_str(), Some(prompt_cache_key));
    assert_eq!(fetched["bindingKind"].as_str(), Some("group"));
    assert_eq!(fetched["groupName"].as_str(), Some(group_name));

    server.abort();
}

#[tokio::test]
async fn prompt_cache_group_binding_timeout_response_ignores_account_overrides() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let group_name = "prompt-cache-group-timeout-sources";
    ensure_test_group_binding(&state.pool, group_name, None).await;
    let first_account_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Prompt Cache Group Timeout First",
        "sk-prompt-cache-group-timeout-first",
        Some(group_name),
        None,
        None,
    )
    .await;
    insert_test_pool_api_key_account_with_options(
        &state,
        "Prompt Cache Group Timeout Second",
        "sk-prompt-cache-group-timeout-second",
        Some(group_name),
        None,
        None,
    )
    .await;

    sqlx::query(
        r#"
        UPDATE pool_routing_settings
        SET responses_first_byte_timeout_secs = 21,
            compact_first_byte_timeout_secs = 22,
            responses_stream_timeout_secs = 23,
            compact_stream_timeout_secs = 24
        WHERE id = ?1
        "#,
    )
    .bind(1_i64)
    .execute(&state.pool)
    .await
    .expect("seed root timeout settings");
    sqlx::query(
        r#"
        UPDATE pool_upstream_account_group_notes
        SET policy_responses_first_byte_timeout_secs = 31,
            policy_responses_stream_timeout_secs = 33
        WHERE group_name = ?1
        "#,
    )
    .bind(group_name)
    .execute(&state.pool)
    .await
    .expect("seed group timeout override");
    sqlx::query(
        r#"
        UPDATE pool_upstream_accounts
        SET policy_responses_first_byte_timeout_secs = 71,
            policy_compact_first_byte_timeout_secs = 72
        WHERE id = ?1
        "#,
    )
    .bind(first_account_id)
    .execute(&state.pool)
    .await
    .expect("seed account timeout override");

    let payload: UpdatePromptCacheConversationBindingRequest = serde_json::from_value(json!({
        "bindingKind": "group",
        "groupName": group_name,
        "timeouts": {
            "compactStreamTimeoutSecs": 91
        }
    }))
    .expect("deserialize group timeout payload");
    let prompt_cache_key = "prompt-cache-group-timeout-sources-key";
    let Json(group_response) = patch_prompt_cache_conversation_binding(
        State(state.clone()),
        AxumPath(prompt_cache_key.to_string()),
        Json(payload),
    )
    .await
    .expect("save group timeout binding");

    assert_eq!(group_response.binding_kind, "group");
    assert_eq!(
        group_response.timeouts.responses_first_byte_timeout_secs,
        Some(31)
    );
    assert_eq!(
        group_response.timeouts.compact_first_byte_timeout_secs,
        Some(22)
    );
    assert_eq!(
        group_response.timeouts.responses_stream_timeout_secs,
        Some(33)
    );
    assert_eq!(
        group_response.timeouts.compact_stream_timeout_secs,
        Some(91)
    );
    assert_eq!(
        group_response
            .timeout_field_sources
            .responses_first_byte_timeout_secs,
        "group"
    );
    assert_eq!(
        group_response
            .timeout_field_sources
            .compact_first_byte_timeout_secs,
        "root"
    );
    assert_eq!(
        group_response
            .timeout_field_sources
            .responses_stream_timeout_secs,
        "group"
    );
    assert_eq!(
        group_response
            .timeout_field_sources
            .compact_stream_timeout_secs,
        "conversation"
    );

    let Json(get_response) =
        get_prompt_cache_conversation_binding(State(state), AxumPath(prompt_cache_key.to_string()))
            .await
            .expect("get group timeout binding");
    assert_eq!(
        get_response.timeouts.responses_first_byte_timeout_secs,
        group_response.timeouts.responses_first_byte_timeout_secs
    );
    assert_eq!(
        get_response.timeouts.compact_first_byte_timeout_secs,
        group_response.timeouts.compact_first_byte_timeout_secs
    );
    assert_eq!(
        get_response.timeouts.responses_stream_timeout_secs,
        group_response.timeouts.responses_stream_timeout_secs
    );
    assert_eq!(
        get_response.timeouts.compact_stream_timeout_secs,
        group_response.timeouts.compact_stream_timeout_secs
    );
    assert_eq!(
        get_response
            .timeout_field_sources
            .responses_first_byte_timeout_secs,
        group_response
            .timeout_field_sources
            .responses_first_byte_timeout_secs
    );
    assert_eq!(
        get_response
            .timeout_field_sources
            .compact_first_byte_timeout_secs,
        group_response
            .timeout_field_sources
            .compact_first_byte_timeout_secs
    );
    assert_eq!(
        get_response
            .timeout_field_sources
            .responses_stream_timeout_secs,
        group_response
            .timeout_field_sources
            .responses_stream_timeout_secs
    );
    assert_eq!(
        get_response
            .timeout_field_sources
            .compact_stream_timeout_secs,
        group_response
            .timeout_field_sources
            .compact_stream_timeout_secs
    );
}

#[tokio::test]
async fn prompt_cache_conversation_binding_reports_encrypted_owner_and_clear_keeps_owner_lock() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    enable_encrypted_session_owner_routing_for_test(&state).await;
    let group_name = "prompt-cache-owner-reporting-group";
    ensure_test_group_binding(&state.pool, group_name, None).await;
    let owner_account_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Prompt Cache Owner Reporting",
        "sk-prompt-cache-owner-reporting",
        Some(group_name),
        None,
        None,
    )
    .await;
    let prompt_cache_key = "prompt-cache-owner-reporting-key";

    let group_payload: UpdatePromptCacheConversationBindingRequest =
        serde_json::from_value(json!({
            "bindingKind": "group",
            "groupName": group_name,
        }))
        .expect("deserialize owner reporting group payload");
    let Json(group_response) = patch_prompt_cache_conversation_binding(
        State(state.clone()),
        AxumPath(prompt_cache_key.to_string()),
        Json(group_payload),
    )
    .await
    .expect("save group binding before owner lock");
    assert_eq!(group_response.binding_kind, "group");
    assert_eq!(group_response.group_name.as_deref(), Some(group_name));
    assert!(!group_response.has_encrypted_session_owner);

    upsert_prompt_cache_encrypted_session_owner(&state.pool, prompt_cache_key, owner_account_id)
        .await
        .expect("persist encrypted session owner");

    let Json(owner_response) = get_prompt_cache_conversation_binding(
        State(state.clone()),
        AxumPath(prompt_cache_key.to_string()),
    )
    .await
    .expect("load binding with encrypted owner");
    assert_eq!(owner_response.binding_kind, "group");
    assert_eq!(owner_response.group_name.as_deref(), Some(group_name));
    assert!(owner_response.has_encrypted_session_owner);
    assert_eq!(
        owner_response.encrypted_owner_account_id,
        Some(owner_account_id)
    );
    assert_eq!(
        owner_response.encrypted_owner_account_name.as_deref(),
        Some("Prompt Cache Owner Reporting")
    );
    assert_eq!(
        owner_response.encrypted_owner_group_name.as_deref(),
        Some(group_name)
    );

    let clear_payload: UpdatePromptCacheConversationBindingRequest =
        serde_json::from_value(json!({ "bindingKind": "none" }))
            .expect("deserialize owner reporting clear payload");
    let Json(clear_response) = patch_prompt_cache_conversation_binding(
        State(state.clone()),
        AxumPath(prompt_cache_key.to_string()),
        Json(clear_payload),
    )
    .await
    .expect("clear manual binding while owner remains");
    assert_eq!(clear_response.binding_kind, "none");
    assert!(clear_response.has_encrypted_session_owner);
    assert_eq!(
        clear_response.encrypted_owner_account_id,
        Some(owner_account_id)
    );
    assert_eq!(
        clear_response.encrypted_owner_account_name.as_deref(),
        Some("Prompt Cache Owner Reporting")
    );
    assert_eq!(
        clear_response.encrypted_owner_group_name.as_deref(),
        Some(group_name)
    );
}

#[tokio::test]
async fn prompt_cache_conversation_binding_hides_encrypted_owner_when_routing_disabled() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let group_name = "prompt-cache-owner-hidden-group";
    ensure_test_group_binding(&state.pool, group_name, None).await;
    let owner_account_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Prompt Cache Hidden Owner",
        "sk-prompt-cache-hidden-owner",
        Some(group_name),
        None,
        None,
    )
    .await;
    let prompt_cache_key = "prompt-cache-owner-hidden-key";

    upsert_prompt_cache_encrypted_session_owner(&state.pool, prompt_cache_key, owner_account_id)
        .await
        .expect("persist encrypted session owner");

    {
        let mut settings = state.proxy_model_settings.write().await;
        settings.encrypted_session_owner_routing_enabled = false;
    }

    let Json(owner_response) = get_prompt_cache_conversation_binding(
        State(state.clone()),
        AxumPath(prompt_cache_key.to_string()),
    )
    .await
    .expect("load binding with encrypted owner routing disabled");
    assert_eq!(owner_response.binding_kind, "none");
    assert!(!owner_response.has_encrypted_session_owner);
    assert_eq!(owner_response.encrypted_owner_account_id, None);
    assert_eq!(owner_response.encrypted_owner_account_name, None);
    assert_eq!(owner_response.encrypted_owner_group_name, None);

    let group_payload: UpdatePromptCacheConversationBindingRequest =
        serde_json::from_value(json!({
            "bindingKind": "group",
            "groupName": group_name,
        }))
        .expect("deserialize owner hidden group payload");
    let Json(group_response) = patch_prompt_cache_conversation_binding(
        State(state.clone()),
        AxumPath(prompt_cache_key.to_string()),
        Json(group_payload),
    )
    .await
    .expect("save group binding while encrypted owner routing is disabled");
    assert_eq!(group_response.binding_kind, "group");
    assert_eq!(group_response.group_name.as_deref(), Some(group_name));
    assert!(!group_response.has_encrypted_session_owner);
    assert_eq!(group_response.encrypted_owner_account_id, None);
    assert_eq!(group_response.encrypted_owner_account_name, None);
    assert_eq!(group_response.encrypted_owner_group_name, None);
}

#[tokio::test]
async fn prompt_cache_group_binding_promotes_to_account_after_encrypted_owner_lock() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    enable_encrypted_session_owner_routing_for_test(&state).await;
    let group_name = "prompt-cache-group-promote-owner";
    ensure_test_group_binding(&state.pool, group_name, None).await;
    let account_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Prompt Cache Promote Owner",
        "sk-prompt-cache-promote-owner",
        Some(group_name),
        None,
        None,
    )
    .await;
    let prompt_cache_key = "prompt-cache-group-promote-owner-key";

    let group_payload: UpdatePromptCacheConversationBindingRequest =
        serde_json::from_value(json!({
            "bindingKind": "group",
            "groupName": group_name,
        }))
        .expect("deserialize promote group payload");
    let Json(group_response) = patch_prompt_cache_conversation_binding(
        State(state.clone()),
        AxumPath(prompt_cache_key.to_string()),
        Json(group_payload),
    )
    .await
    .expect("save promotable group binding");
    assert_eq!(group_response.binding_kind, "group");

    upsert_prompt_cache_encrypted_session_owner(&state.pool, prompt_cache_key, account_id)
        .await
        .expect("persist owner before promotion");
    promote_prompt_cache_group_binding_to_upstream_account(
        &state.pool,
        prompt_cache_key,
        account_id,
    )
    .await
    .expect("promote group binding to account");

    let Json(promoted_response) = get_prompt_cache_conversation_binding(
        State(state.clone()),
        AxumPath(prompt_cache_key.to_string()),
    )
    .await
    .expect("load promoted prompt cache binding");
    assert_eq!(promoted_response.binding_kind, "upstreamAccount");
    assert_eq!(promoted_response.group_name, None);
    assert_eq!(promoted_response.upstream_account_id, Some(account_id));
    assert_eq!(
        promoted_response.upstream_account_name.as_deref(),
        Some("Prompt Cache Promote Owner")
    );
    assert!(promoted_response.has_encrypted_session_owner);
    assert_eq!(
        promoted_response.encrypted_owner_account_id,
        Some(account_id)
    );

    let sticky_account_id: i64 =
        sqlx::query_scalar("SELECT account_id FROM pool_sticky_routes WHERE sticky_key = ?1")
            .bind(prompt_cache_key)
            .fetch_one(&state.pool)
            .await
            .expect("promotion should align sticky route to promoted account");
    assert_eq!(sticky_account_id, account_id);

    let effective_constraint = resolve_prompt_cache_effective_routing_constraint(
        &state.pool,
        Some(prompt_cache_key),
        true,
        true,
    )
    .await
    .expect("resolve effective routing constraint after promotion");
    assert!(effective_constraint.1);
    match effective_constraint.0 {
        Some(PromptCacheConversationBindingConstraint::UpstreamAccount(bound_id)) => {
            assert_eq!(bound_id, account_id);
        }
        other => panic!("expected promoted account binding constraint, got {other:?}"),
    }

    let Json(event_response) = list_prompt_cache_conversation_operation_events(
        State(state.clone()),
        AxumPath(prompt_cache_key.to_string()),
        axum::extract::Query(ListPromptCacheConversationOperationEventsQuery {
            page: Some(1),
            page_size: Some(20),
            info_type: None,
            routing_scope: None,
            routing_model: None,
        }),
    )
    .await
    .expect("list promotion operation events");
    assert!(
        event_response
            .items
            .iter()
            .any(|event| event.action == "groupBindingPromoted" && event.origin == "systemAuto")
    );
}

#[tokio::test]
async fn bulk_prompt_cache_conversation_bindings_bind_to_upstream_account_across_keys() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let group_name = "bulk-prompt-cache-bind-group";
    ensure_test_group_binding(&state.pool, group_name, None).await;
    let account_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Bulk Prompt Cache Binding",
        "sk-bulk-prompt-cache-binding",
        Some(group_name),
        None,
        None,
    )
    .await;
    let prompt_cache_keys = ["bulk-bind-key-1", "bulk-bind-key-2"];

    let payload: BulkPromptCacheConversationBindingsRequest = serde_json::from_value(json!({
        "promptCacheKeys": ["  bulk-bind-key-1  ", "bulk-bind-key-2"],
        "action": "bind",
        "bindingKind": "upstreamAccount",
        "upstreamAccountId": account_id,
    }))
    .expect("deserialize bulk bind payload");
    let Json(response) =
        post_bulk_prompt_cache_conversation_bindings(State(state.clone()), Json(payload))
            .await
            .expect("bulk bind should succeed");
    assert_eq!(response.action, "bind");
    assert_eq!(response.total_requested, prompt_cache_keys.len());
    assert_eq!(response.total_succeeded, prompt_cache_keys.len());
    assert_eq!(response.total_failed, 0);

    for prompt_cache_key in prompt_cache_keys {
        let item = response
            .items
            .iter()
            .find(|candidate| candidate.prompt_cache_key == prompt_cache_key)
            .expect("bulk bind response should include each key");
        assert!(item.ok);
        assert_eq!(item.error, None);
        let binding = item
            .binding
            .as_ref()
            .expect("successful bind should include binding snapshot");
        assert_eq!(binding.binding_kind, "upstreamAccount");
        assert_eq!(binding.upstream_account_id, Some(account_id));
        assert_eq!(
            binding.upstream_account_name.as_deref(),
            Some("Bulk Prompt Cache Binding")
        );

        let binding_row = load_prompt_cache_conversation_binding_row(&state.pool, prompt_cache_key)
            .await
            .expect("bulk bind row should load")
            .expect("bulk bind row should exist");
        assert_eq!(
            binding_row.binding_kind,
            PROMPT_CACHE_BINDING_KIND_UPSTREAM_ACCOUNT
        );
        assert_eq!(binding_row.upstream_account_id, Some(account_id));

        let sticky_account_id: i64 =
            sqlx::query_scalar("SELECT account_id FROM pool_sticky_routes WHERE sticky_key = ?1")
                .bind(prompt_cache_key)
                .fetch_one(&state.pool)
                .await
                .expect("bulk bind should align sticky route");
        assert_eq!(sticky_account_id, account_id);

        let Json(event_response) = list_prompt_cache_conversation_operation_events(
            State(state.clone()),
            AxumPath(prompt_cache_key.to_string()),
            axum::extract::Query(ListPromptCacheConversationOperationEventsQuery {
                page: Some(1),
                page_size: Some(20),
                info_type: None,
                routing_scope: None,
                routing_model: None,
            }),
        )
        .await
        .expect("list bulk bind operation events");
        let binding_event = event_response
            .items
            .iter()
            .find(|event| event.action == "manualBindingUpdated" && event.origin == "dashboardBulk")
            .expect("bulk binding should record its routing transition on the binding event");
        assert!(
            binding_event
                .sticky_transitions
                .iter()
                .any(|transition| transition.model_key.is_none() && transition.after.is_some())
        );
        assert!(
            !event_response
                .items
                .iter()
                .any(|event| event.action == "stickyTargetChanged"
                    && event.origin == "dashboardBulk"),
            "one conversation-level manual operation should not produce a second fallback-only event"
        );
    }
}

#[tokio::test]
async fn bulk_prompt_cache_conversation_bindings_bind_none_clears_only_manual_binding() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    enable_encrypted_session_owner_routing_for_test(&state).await;
    let group_name = "bulk-prompt-cache-bind-none-group";
    ensure_test_group_binding(&state.pool, group_name, None).await;
    let account_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Bulk Prompt Cache Bind None",
        "sk-bulk-prompt-cache-bind-none",
        Some(group_name),
        None,
        None,
    )
    .await;
    let prompt_cache_key = "bulk-bind-none-key";

    let setup_payload: UpdatePromptCacheConversationBindingRequest =
        serde_json::from_value(json!({
            "bindingKind": "upstreamAccount",
            "upstreamAccountId": account_id,
        }))
        .expect("deserialize setup account binding payload");
    let Json(setup_response) = patch_prompt_cache_conversation_binding(
        State(state.clone()),
        AxumPath(prompt_cache_key.to_string()),
        Json(setup_payload),
    )
    .await
    .expect("setup account binding should succeed");
    assert_eq!(setup_response.binding_kind, "upstreamAccount");
    upsert_prompt_cache_encrypted_session_owner(&state.pool, prompt_cache_key, account_id)
        .await
        .expect("seed encrypted session owner");

    let payload: BulkPromptCacheConversationBindingsRequest = serde_json::from_value(json!({
        "promptCacheKeys": [prompt_cache_key],
        "action": "bind",
        "bindingKind": "none",
    }))
    .expect("deserialize bulk bind-none payload");
    let Json(response) =
        post_bulk_prompt_cache_conversation_bindings(State(state.clone()), Json(payload))
            .await
            .expect("bulk bind-none should succeed");
    assert_eq!(response.action, "bind");
    assert_eq!(response.total_requested, 1);
    assert_eq!(response.total_succeeded, 1);
    assert_eq!(response.total_failed, 0);

    let item = response
        .items
        .first()
        .expect("bulk bind-none should return one item");
    assert!(item.ok);
    let binding = item
        .binding
        .as_ref()
        .expect("successful bind-none should include binding snapshot");
    assert_eq!(binding.binding_kind, "none");
    assert!(binding.has_encrypted_session_owner);
    assert_eq!(binding.encrypted_owner_account_id, Some(account_id));
    assert_eq!(binding.upstream_account_id, None);

    let binding_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM prompt_cache_conversation_bindings WHERE prompt_cache_key = ?1",
    )
    .bind(prompt_cache_key)
    .fetch_one(&state.pool)
    .await
    .expect("count binding rows after bulk bind-none");
    assert_eq!(binding_count, 0);

    let sticky_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM pool_sticky_routes WHERE sticky_key = ?1")
            .bind(prompt_cache_key)
            .fetch_one(&state.pool)
            .await
            .expect("count sticky rows after bulk bind-none");
    assert_eq!(sticky_count, 1);

    let owner_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM prompt_cache_encrypted_session_owners WHERE prompt_cache_key = ?1",
    )
    .bind(prompt_cache_key)
    .fetch_one(&state.pool)
    .await
    .expect("count encrypted owner rows after bulk bind-none");
    assert_eq!(owner_count, 1);

    let Json(event_response) = list_prompt_cache_conversation_operation_events(
        State(state.clone()),
        AxumPath(prompt_cache_key.to_string()),
        axum::extract::Query(ListPromptCacheConversationOperationEventsQuery {
            page: Some(1),
            page_size: Some(20),
            info_type: None,
            routing_scope: None,
            routing_model: None,
        }),
    )
    .await
    .expect("list bulk bind-none operation events");
    assert!(
        event_response
            .items
            .iter()
            .any(|event| event.action == "bindingCleared" && event.origin == "dashboardBulk")
    );
    assert!(
        !event_response
            .items
            .iter()
            .any(|event| event.action == "affinityReset" && event.origin == "dashboardBulk")
    );
    assert!(
        !event_response
            .items
            .iter()
            .any(|event| event.action == "stickyTargetCleared" && event.origin == "dashboardBulk")
    );
}

#[tokio::test]
async fn bulk_prompt_cache_conversation_bindings_clear_and_reset_affinity_removes_all_affinity_rows()
 {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    enable_encrypted_session_owner_routing_for_test(&state).await;
    let group_name = "bulk-prompt-cache-clear-group";
    ensure_test_group_binding(&state.pool, group_name, None).await;
    let account_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Bulk Prompt Cache Clear",
        "sk-bulk-prompt-cache-clear",
        Some(group_name),
        None,
        None,
    )
    .await;
    let prompt_cache_key = "bulk-clear-affinity-key";

    let binding_payload: UpdatePromptCacheConversationBindingRequest =
        serde_json::from_value(json!({
            "bindingKind": "upstreamAccount",
            "upstreamAccountId": account_id,
        }))
        .expect("deserialize setup binding payload");
    let Json(setup_response) = patch_prompt_cache_conversation_binding(
        State(state.clone()),
        AxumPath(prompt_cache_key.to_string()),
        Json(binding_payload),
    )
    .await
    .expect("setup account binding should succeed");
    assert_eq!(setup_response.binding_kind, "upstreamAccount");
    upsert_prompt_cache_encrypted_session_owner(&state.pool, prompt_cache_key, account_id)
        .await
        .expect("seed encrypted session owner");
    upsert_sticky_route_for_model_if_current(
        &state.pool,
        prompt_cache_key,
        Some("gpt-5.4-2026-05-01"),
        account_id,
        None,
        &format_utc_iso(Utc::now()),
    )
    .await
    .expect("seed a model-specific sticky route before bulk clear");

    let payload: BulkPromptCacheConversationBindingsRequest = serde_json::from_value(json!({
        "promptCacheKeys": [prompt_cache_key],
        "action": "clearAndResetAffinity",
    }))
    .expect("deserialize bulk clear payload");
    let Json(response) =
        post_bulk_prompt_cache_conversation_bindings(State(state.clone()), Json(payload))
            .await
            .expect("bulk clear should succeed");
    assert_eq!(response.action, "clearAndResetAffinity");
    assert_eq!(response.total_requested, 1);
    assert_eq!(response.total_succeeded, 1);
    assert_eq!(response.total_failed, 0);

    let item = response
        .items
        .first()
        .expect("bulk clear should return one item");
    assert!(item.ok);
    let binding = item
        .binding
        .as_ref()
        .expect("successful clear should include binding snapshot");
    assert_eq!(binding.binding_kind, "none");
    assert!(!binding.has_encrypted_session_owner);
    assert_eq!(binding.upstream_account_id, None);

    let binding_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM prompt_cache_conversation_bindings WHERE prompt_cache_key = ?1",
    )
    .bind(prompt_cache_key)
    .fetch_one(&state.pool)
    .await
    .expect("count binding rows after bulk clear");
    assert_eq!(binding_count, 0);

    let sticky_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM pool_sticky_routes WHERE sticky_key = ?1")
            .bind(prompt_cache_key)
            .fetch_one(&state.pool)
            .await
            .expect("count sticky rows after bulk clear");
    assert_eq!(sticky_count, 0);

    let sticky_model_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM pool_sticky_model_routes WHERE sticky_key = ?1")
            .bind(prompt_cache_key)
            .fetch_one(&state.pool)
            .await
            .expect("count model sticky rows after bulk clear");
    assert_eq!(sticky_model_count, 0);

    let owner_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM prompt_cache_encrypted_session_owners WHERE prompt_cache_key = ?1",
    )
    .bind(prompt_cache_key)
    .fetch_one(&state.pool)
    .await
    .expect("count encrypted owner rows after bulk clear");
    assert_eq!(owner_count, 0);

    let owner_row = load_prompt_cache_encrypted_session_owner_row(&state.pool, prompt_cache_key)
        .await
        .expect("load encrypted owner row after clear");
    assert!(owner_row.is_none());

    let effective_constraint = resolve_prompt_cache_effective_routing_constraint(
        &state.pool,
        Some(prompt_cache_key),
        true,
        true,
    )
    .await
    .expect("resolve routing constraint after bulk clear");
    assert!(effective_constraint.0.is_none());
    assert!(!effective_constraint.1);

    let Json(event_response) = list_prompt_cache_conversation_operation_events(
        State(state.clone()),
        AxumPath(prompt_cache_key.to_string()),
        axum::extract::Query(ListPromptCacheConversationOperationEventsQuery {
            page: Some(1),
            page_size: Some(20),
            info_type: None,
            routing_scope: None,
            routing_model: None,
        }),
    )
    .await
    .expect("list bulk clear operation events");
    let reset_event = event_response
        .items
        .iter()
        .find(|event| event.action == "affinityReset" && event.origin == "dashboardBulk")
        .expect("full reset should emit one dashboard bulk event");
    assert_eq!(
        reset_event
            .routing_scope
            .as_ref()
            .map(|scope| scope.kind.as_str()),
        Some("all")
    );
    assert_eq!(reset_event.sticky_transitions.len(), 2);
    assert!(
        reset_event
            .sticky_transitions
            .iter()
            .any(|transition| transition.model_key.as_deref() == Some("gpt-5.4"))
    );
    assert!(
        !event_response
            .items
            .iter()
            .any(|event| event.action == "stickyTargetCleared" && event.origin == "dashboardBulk"),
        "a full reset records all bucket changes in its single aggregate event"
    );
}

#[tokio::test]
async fn prompt_cache_clear_and_reset_affinity_fences_stale_sticky_revival_and_emits_only_fresh_runtime_sticky_event()
 {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let stale_account_id =
        insert_test_pool_api_key_account(&state, "Prompt Cache Stale Sticky", "upstream-stale")
            .await;
    let rebound_account_id =
        insert_test_pool_api_key_account(&state, "Prompt Cache Fresh Sticky", "upstream-fresh")
            .await;
    let prompt_cache_key = "prompt-cache-affinity-fence-key";
    let stale_invoke_id = "prompt-cache-stale-sticky-invoke";
    let fresh_invoke_id = "prompt-cache-fresh-sticky-invoke";
    let keepalive_invoke_id = "prompt-cache-keepalive-sticky-invoke";
    let seeded_at = format_utc_iso(Utc::now());

    upsert_sticky_route(&state.pool, prompt_cache_key, stale_account_id, &seeded_at)
        .await
        .expect("seed sticky route before clear");
    let stale_generation = load_sticky_affinity_generation(&state.pool, prompt_cache_key)
        .await
        .expect("load affinity generation before clear");
    assert_eq!(stale_generation, 0);

    let payload: BulkPromptCacheConversationBindingsRequest = serde_json::from_value(json!({
        "promptCacheKeys": [prompt_cache_key],
        "action": "clearAndResetAffinity",
    }))
    .expect("deserialize clear payload");
    let Json(response) =
        post_bulk_prompt_cache_conversation_bindings(State(state.clone()), Json(payload))
            .await
            .expect("clear and reset affinity should succeed");
    assert_eq!(response.action, "clearAndResetAffinity");
    assert_eq!(response.total_succeeded, 1);

    let fresh_generation = load_sticky_affinity_generation(&state.pool, prompt_cache_key)
        .await
        .expect("load affinity generation after clear");
    assert_eq!(fresh_generation, stale_generation + 1);
    assert!(
        load_sticky_route(&state.pool, prompt_cache_key)
            .await
            .expect("load sticky route after clear")
            .is_none()
    );

    record_pool_route_success_with_affinity_generation(
        &state.pool,
        stale_account_id,
        Utc::now(),
        Some(prompt_cache_key),
        Some(prompt_cache_key),
        Some(stale_invoke_id),
        Some(stale_generation),
    )
    .await
    .expect("stale success writeback should be ignored");
    assert!(
        load_sticky_route(&state.pool, prompt_cache_key)
            .await
            .expect("load sticky route after stale success")
            .is_none()
    );

    record_pool_route_success_with_affinity_generation(
        &state.pool,
        rebound_account_id,
        Utc::now(),
        Some(prompt_cache_key),
        Some(prompt_cache_key),
        Some(fresh_invoke_id),
        Some(fresh_generation),
    )
    .await
    .expect("fresh success should create sticky route");
    let sticky_row = load_sticky_route(&state.pool, prompt_cache_key)
        .await
        .expect("load sticky route after fresh success")
        .expect("fresh success should rebuild sticky route");
    assert_eq!(sticky_row.account_id, rebound_account_id);
    let keepalive_generation = load_sticky_affinity_generation(&state.pool, prompt_cache_key)
        .await
        .expect("load affinity generation after fresh success");
    assert_eq!(keepalive_generation, fresh_generation + 1);

    record_pool_route_success_with_affinity_generation(
        &state.pool,
        rebound_account_id,
        Utc::now(),
        Some(prompt_cache_key),
        Some(prompt_cache_key),
        Some(keepalive_invoke_id),
        Some(keepalive_generation),
    )
    .await
    .expect("same-account keepalive should refresh without a new event");

    let Json(event_response) = list_prompt_cache_conversation_operation_events(
        State(state.clone()),
        AxumPath(prompt_cache_key.to_string()),
        axum::extract::Query(ListPromptCacheConversationOperationEventsQuery {
            page: Some(1),
            page_size: Some(20),
            info_type: None,
            routing_scope: None,
            routing_model: None,
        }),
    )
    .await
    .expect("list prompt cache operation events after stale and fresh successes");

    assert!(
        !event_response.items.iter().any(|event| {
            event.action == "stickyTargetChanged"
                && event.origin == "systemAuto"
                && event.invoke_id.as_deref() == Some(stale_invoke_id)
        }),
        "stale in-flight success must not resurrect sticky ownership"
    );

    let fresh_events = event_response
        .items
        .iter()
        .filter(|event| event.action == "stickyTargetChanged" && event.origin == "systemAuto")
        .collect::<Vec<_>>();
    assert_eq!(
        fresh_events.len(),
        1,
        "fresh assignment should emit exactly one runtime sticky change event"
    );
    let fresh_event = fresh_events[0];
    assert_eq!(fresh_event.invoke_id.as_deref(), Some(fresh_invoke_id));
    assert_eq!(fresh_event.info_types, vec!["routing".to_string()]);
    assert_eq!(
        fresh_event
            .sticky_before
            .as_ref()
            .map(|sticky| sticky.upstream_account_id),
        None
    );
    assert_eq!(
        fresh_event
            .sticky_after
            .as_ref()
            .map(|sticky| sticky.upstream_account_id),
        Some(rebound_account_id)
    );
    assert!(
        !event_response.items.iter().any(|event| {
            event.action == "stickyTargetChanged"
                && event.origin == "systemAuto"
                && event.invoke_id.as_deref() == Some(keepalive_invoke_id)
        }),
        "same-account keepalive should not emit extra sticky change noise"
    );
}

#[tokio::test]
async fn prompt_cache_sticky_routes_are_isolated_by_normalized_model_key() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let fallback_account_id =
        insert_test_pool_api_key_account(&state, "Prompt Cache Fallback", "model-fallback").await;
    let model_account_id =
        insert_test_pool_api_key_account(&state, "Prompt Cache Model", "model-specific").await;
    let prompt_cache_key = "prompt-cache-model-isolation-key";
    let now_iso = format_utc_iso(Utc::now());

    upsert_sticky_route(&state.pool, prompt_cache_key, fallback_account_id, &now_iso)
        .await
        .expect("seed all-model fallback");
    assert!(
        upsert_sticky_route_for_model_if_current(
            &state.pool,
            prompt_cache_key,
            Some(" GPT-5.4-2026-05-01 "),
            model_account_id,
            None,
            &now_iso,
        )
        .await
        .expect("write normalized model route"),
        "the first model-specific success should materialize its exact bucket"
    );

    let (dated_alias_route, _) =
        load_sticky_route_with_model_generation(&state.pool, prompt_cache_key, Some("gpt-5.4"))
            .await
            .expect("load normalized alias route");
    assert_eq!(
        dated_alias_route.map(|route| route.account_id),
        Some(model_account_id),
        "a dated alias and its base model must share one Sticky bucket"
    );

    let (other_model_route, _) = load_sticky_route_with_model_generation(
        &state.pool,
        prompt_cache_key,
        Some("gpt-5.1-codex-max"),
    )
    .await
    .expect("load fallback route for unrelated model");
    assert_eq!(
        other_model_route.map(|route| route.account_id),
        Some(fallback_account_id),
        "an unrelated model must not inherit another model's exact Sticky route"
    );
}

#[tokio::test]
async fn prompt_cache_manual_binding_is_idempotent_for_unchanged_model_routes() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let account_id =
        insert_test_pool_api_key_account(&state, "Prompt Cache Manual", "manual-model-route").await;
    let prompt_cache_key = "prompt-cache-manual-idempotent-model-route";
    let initial_payload: UpdatePromptCacheConversationBindingRequest =
        serde_json::from_value(json!({
            "bindingKind": "upstreamAccount",
            "upstreamAccountId": account_id,
        }))
        .expect("deserialize manual binding payload");

    let _ = patch_prompt_cache_conversation_binding(
        State(state.clone()),
        AxumPath(prompt_cache_key.to_string()),
        Json(initial_payload),
    )
    .await
    .expect("save initial manual binding");
    let now_iso = format_utc_iso(Utc::now());
    assert!(
        upsert_sticky_route_for_model_if_current(
            &state.pool,
            prompt_cache_key,
            Some("gpt-5.4"),
            account_id,
            None,
            &now_iso,
        )
        .await
        .expect("materialize exact model route")
    );
    let epoch_before = load_sticky_affinity_generation(&state.pool, prompt_cache_key)
        .await
        .expect("load manual binding epoch");
    let (_, model_generation_before) =
        load_sticky_route_with_model_generation(&state.pool, prompt_cache_key, Some("gpt-5.4"))
            .await
            .expect("load exact model generation");

    let repeat_payload: UpdatePromptCacheConversationBindingRequest =
        serde_json::from_value(json!({
            "bindingKind": "upstreamAccount",
            "upstreamAccountId": account_id,
        }))
        .expect("deserialize repeated manual binding payload");

    let _ = patch_prompt_cache_conversation_binding(
        State(state.clone()),
        AxumPath(prompt_cache_key.to_string()),
        Json(repeat_payload),
    )
    .await
    .expect("repeat manual binding without changing routes");

    assert_eq!(
        load_sticky_affinity_generation(&state.pool, prompt_cache_key)
            .await
            .expect("load epoch after idempotent binding"),
        epoch_before,
        "a no-op binding must not fence in-flight requests"
    );
    let (_, model_generation_after) =
        load_sticky_route_with_model_generation(&state.pool, prompt_cache_key, Some("gpt-5.4"))
            .await
            .expect("load model generation after idempotent binding");
    assert_eq!(model_generation_after, model_generation_before);
}

#[tokio::test]
async fn prompt_cache_manual_binding_rolls_back_when_sticky_update_fails() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let account_id =
        insert_test_pool_api_key_account(&state, "Prompt Cache Atomic", "atomic-manual-route")
            .await;
    let prompt_cache_key = "prompt-cache-manual-binding-atomic";
    sqlx::query(
        r#"
        CREATE TRIGGER fail_prompt_cache_manual_sticky_insert
        BEFORE INSERT ON pool_sticky_routes
        WHEN NEW.sticky_key = 'prompt-cache-manual-binding-atomic'
        BEGIN
            SELECT RAISE(ABORT, 'forced sticky write failure');
        END
        "#,
    )
    .execute(&state.pool)
    .await
    .expect("install sticky write failure trigger");
    let payload: UpdatePromptCacheConversationBindingRequest = serde_json::from_value(json!({
        "bindingKind": "upstreamAccount",
        "upstreamAccountId": account_id,
    }))
    .expect("deserialize atomic manual binding payload");

    let error = patch_prompt_cache_conversation_binding(
        State(state.clone()),
        AxumPath(prompt_cache_key.to_string()),
        Json(payload),
    )
    .await
    .expect_err("sticky failure must fail the account binding transaction");
    assert!(matches!(error, ApiError::Internal(_)));
    assert!(
        load_prompt_cache_conversation_binding_row(&state.pool, prompt_cache_key)
            .await
            .expect("load binding after failed transaction")
            .is_none(),
        "the binding row must roll back with the Sticky write"
    );
    let sticky_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM pool_sticky_routes WHERE sticky_key = ?1")
            .bind(prompt_cache_key)
            .fetch_one(&state.pool)
            .await
            .expect("count sticky rows after failed transaction");
    assert_eq!(sticky_count, 0);
}

#[tokio::test]
async fn prompt_cache_routing_events_filter_by_all_or_normalized_model_scope() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let prompt_cache_key = "prompt-cache-routing-event-model-filter";
    for (action, scope) in [
        ("legacyAll", r#"{"kind":"all"}"#),
        (
            "gpt54",
            r#"{"kind":"model","modelKey":"gpt-5.4","requestModel":"gpt-5.4-2026-05-01"}"#,
        ),
        (
            "gpt51",
            r#"{"kind":"model","modelKey":"gpt-5.1-codex-max"}"#,
        ),
    ] {
        sqlx::query(
            r#"
            INSERT INTO prompt_cache_conversation_operation_events (
                prompt_cache_key, action, origin, info_types_json, occurred_at, headline,
                routing_scope_json
            ) VALUES (?1, ?2, 'systemAuto', '["routing"]', datetime('now'), ?2, ?3)
            "#,
        )
        .bind(prompt_cache_key)
        .bind(action)
        .bind(scope)
        .execute(&state.pool)
        .await
        .expect("seed scoped routing event");
    }

    let Json(model_events) = list_prompt_cache_conversation_operation_events(
        State(state.clone()),
        AxumPath(prompt_cache_key.to_string()),
        axum::extract::Query(ListPromptCacheConversationOperationEventsQuery {
            page: Some(1),
            page_size: Some(20),
            info_type: Some("routing".to_string()),
            routing_scope: Some("model".to_string()),
            routing_model: Some(" GPT-5.4-2026-05-01 ".to_string()),
        }),
    )
    .await
    .expect("filter routing events by normalized model");
    assert_eq!(model_events.total, 1);
    assert_eq!(model_events.items[0].action, "gpt54");
    assert_eq!(
        model_events.routing_model_facets,
        vec!["gpt-5.1-codex-max".to_string(), "gpt-5.4".to_string()],
        "facets are derived from all retained events rather than the active filter"
    );

    let Json(all_scope_events) = list_prompt_cache_conversation_operation_events(
        State(state),
        AxumPath(prompt_cache_key.to_string()),
        axum::extract::Query(ListPromptCacheConversationOperationEventsQuery {
            page: Some(1),
            page_size: Some(20),
            info_type: Some("routing".to_string()),
            routing_scope: Some("all".to_string()),
            routing_model: None,
        }),
    )
    .await
    .expect("filter routing events by all-model scope");
    assert_eq!(all_scope_events.total, 1);
    assert_eq!(all_scope_events.items[0].action, "legacyAll");
}

#[tokio::test]
async fn fresh_assignment_persists_selection_audit_for_attempt_and_sticky_event() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let selected_account_id =
        insert_test_pool_api_key_account(&state, "Selected dzw", "upstream-selected").await;
    let excluded_account_id =
        insert_test_pool_api_key_account(&state, "Excluded CIII", "upstream-excluded").await;
    let prompt_cache_key = "prompt-cache-selection-audit-key";
    let invoke_id = "selection-audit-invoke";
    let occurred_at = format_utc_iso(Utc::now());
    let audit = PoolRoutingSelectionAudit {
        selected_account_id,
        selected_account_name: "Selected dzw".to_string(),
        eligible_candidate_count: 1,
        winner_reason_code: "onlyEligibleCandidate".to_string(),
        compared_account_id: None,
        compared_account_name: None,
        selected_score: None,
        compared_score: None,
        handoff_admission: None,
        excluded_candidates: vec![PoolRoutingSelectionAuditExcludedCandidate {
            account_id: excluded_account_id,
            account_name: "Excluded CIII".to_string(),
            reason_code: "modelNotAllowed".to_string(),
        }],
    };
    let attempt_id = sqlx::query(
        r#"
        INSERT INTO pool_upstream_request_attempts (
            attempt_public_id, invoke_id, occurred_at, endpoint, route_mode, sticky_key,
            routing_source, request_model, routing_selection_audit_json, upstream_account_id,
            upstream_route_key, attempt_index, distinct_account_index, same_account_retry_index,
            requester_ip, started_at, finished_at, status, phase, created_at
        ) VALUES (
            ?1, ?2, ?3, '/v1/responses', ?4, ?5, 'freshAssignment', ?6, ?7, ?8,
            'route-selection-audit', 1, 1, 0, '203.0.113.15', ?3, ?3,
            'success', 'completed', ?3
        )
        "#,
    )
    .bind("SELECTAUDIT1")
    .bind(invoke_id)
    .bind(&occurred_at)
    .bind(INVOCATION_ROUTE_MODE_POOL)
    .bind(prompt_cache_key)
    .bind("gpt-5.4")
    .bind(serde_json::to_string(&audit).expect("serialize selection audit"))
    .bind(selected_account_id)
    .execute(&state.pool)
    .await
    .expect("insert attempt with selection audit")
    .last_insert_rowid();
    let generation = load_sticky_affinity_generation(&state.pool, prompt_cache_key)
        .await
        .expect("load empty sticky generation");

    record_pool_route_success_with_affinity_generation_for_attempt(
        &state.pool,
        selected_account_id,
        Utc::now(),
        Some(prompt_cache_key),
        Some(prompt_cache_key),
        None,
        Some(attempt_id),
        Some(generation),
    )
    .await
    .expect("persist fresh sticky success");

    let Json(attempts) =
        fetch_invocation_pool_attempts(State(state.clone()), AxumPath(invoke_id.to_string()))
            .await
            .expect("fetch attempt with selection audit");
    assert_eq!(attempts.len(), 1);
    assert_eq!(
        attempts[0]
            .routing_selection_audit
            .as_ref()
            .map(|value| value.selected_account_name.as_str()),
        Some("Selected dzw")
    );
    assert_eq!(
        attempts[0]
            .routing_selection_audit
            .as_ref()
            .map(|value| value.excluded_candidates[0].reason_code.as_str()),
        Some("modelNotAllowed")
    );

    let Json(events) = list_prompt_cache_conversation_operation_events(
        State(state.clone()),
        AxumPath(prompt_cache_key.to_string()),
        axum::extract::Query(ListPromptCacheConversationOperationEventsQuery {
            page: Some(1),
            page_size: Some(20),
            info_type: None,
            routing_scope: None,
            routing_model: None,
        }),
    )
    .await
    .expect("list sticky event with selection audit");
    let event = events
        .items
        .iter()
        .find(|event| event.action == "stickyTargetChanged")
        .expect("fresh assignment should emit sticky target event");
    assert_eq!(
        event
            .routing_context
            .as_ref()
            .and_then(|context| context.routing_selection_audit.as_ref())
            .map(|value| value.winner_reason_code.as_str()),
        Some("onlyEligibleCandidate")
    );
    assert_eq!(event.invoke_id.as_deref(), Some(invoke_id));
    assert_eq!(
        event
            .routing_scope
            .as_ref()
            .and_then(|scope| scope.request_model.as_deref()),
        None,
        "requestModel duplicates the normalized model key and should be omitted"
    );
}

#[tokio::test]
async fn model_scoped_sticky_clear_cause_does_not_cross_models() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let cleared_account_id =
        insert_test_pool_api_key_account(&state, "Cleared model account", "upstream-cleared").await;
    let replacement_account_id = insert_test_pool_api_key_account(
        &state,
        "Independent model account",
        "upstream-independent",
    )
    .await;
    let prompt_cache_key = "prompt-cache-model-clear-cause-scope";
    let now_iso = format_utc_iso(Utc::now());

    upsert_sticky_route_for_model_if_current(
        &state.pool,
        prompt_cache_key,
        Some("gpt-5.4"),
        cleared_account_id,
        None,
        &now_iso,
    )
    .await
    .expect("seed model-scoped sticky route");
    let (_, gpt54_token) =
        load_sticky_route_with_model_generation(&state.pool, prompt_cache_key, Some("gpt-5.4"))
            .await
            .expect("load gpt-5.4 sticky token");

    let clear_attempt_id = sqlx::query(
        r#"
        INSERT INTO pool_upstream_request_attempts (
            attempt_public_id, invoke_id, occurred_at, endpoint, route_mode, sticky_key,
            routing_source, request_model, upstream_account_id, upstream_route_key,
            attempt_index, distinct_account_index, same_account_retry_index, requester_ip,
            started_at, finished_at, status, phase, created_at
        ) VALUES (
            'CLEAR54', 'clear-gpt-54', ?1, '/v1/responses', ?2, ?3,
            'stickyRoute', 'gpt-5.4', ?4, 'route-clear-54',
            1, 1, 0, '203.0.113.54', ?1, ?1, 'failed', 'completed', ?1
        )
        "#,
    )
    .bind(&now_iso)
    .bind(INVOCATION_ROUTE_MODE_POOL)
    .bind(prompt_cache_key)
    .bind(cleared_account_id)
    .execute(&state.pool)
    .await
    .expect("insert gpt-5.4 clear attempt")
    .last_insert_rowid();

    assert!(
        delete_sticky_route_if_matches_with_cause(
            &state.pool,
            prompt_cache_key,
            cleared_account_id,
            Some(gpt54_token),
            Some(clear_attempt_id),
            Some(429),
            Some("upstreamHttp429"),
            Some(prompt_cache_key),
            &now_iso,
        )
        .await
        .expect("clear gpt-5.4 sticky route"),
        "the matching model bucket should be cleared"
    );

    let gpt54_clear_cause = sqlx::query_as::<_, (Option<String>, Option<i64>)>(
        r#"
        SELECT last_clear_cause_attempt_public_id, last_clear_cause_http_status
        FROM pool_sticky_model_route_generations
        WHERE sticky_key = ?1 AND model_key = 'gpt-5.4'
        "#,
    )
    .bind(prompt_cache_key)
    .fetch_one(&state.pool)
    .await
    .expect("load gpt-5.4 clear cause");
    assert_eq!(gpt54_clear_cause.0.as_deref(), Some("CLEAR54"));
    assert_eq!(gpt54_clear_cause.1, Some(429));

    let replacement_attempt_id = sqlx::query(
        r#"
        INSERT INTO pool_upstream_request_attempts (
            attempt_public_id, invoke_id, occurred_at, endpoint, route_mode, sticky_key,
            routing_source, request_model, upstream_account_id, upstream_route_key,
            attempt_index, distinct_account_index, same_account_retry_index, requester_ip,
            started_at, finished_at, status, phase, created_at
        ) VALUES (
            'SUCCESS51', 'success-gpt-51', ?1, '/v1/responses', ?2, ?3,
            'freshAssignment', 'gpt-5.1-codex-max', ?4, 'route-success-51',
            1, 1, 0, '203.0.113.51', ?1, ?1, 'success', 'completed', ?1
        )
        "#,
    )
    .bind(&now_iso)
    .bind(INVOCATION_ROUTE_MODE_POOL)
    .bind(prompt_cache_key)
    .bind(replacement_account_id)
    .execute(&state.pool)
    .await
    .expect("insert gpt-5.1 success attempt")
    .last_insert_rowid();

    assert!(matches!(
        upsert_runtime_prompt_cache_conversation_sticky_route(
            &state.pool,
            prompt_cache_key,
            Some(prompt_cache_key),
            replacement_account_id,
            &now_iso,
            Some("success-gpt-51"),
            Some(replacement_attempt_id),
            None,
        )
        .await
        .expect("persist independent model sticky route"),
        RuntimeStickyMutation::Changed {
            previous_upstream_account_id: None
        }
    ));

    let Json(events) = list_prompt_cache_conversation_operation_events(
        State(state),
        AxumPath(prompt_cache_key.to_string()),
        axum::extract::Query(ListPromptCacheConversationOperationEventsQuery {
            page: Some(1),
            page_size: Some(20),
            info_type: None,
            routing_scope: None,
            routing_model: None,
        }),
    )
    .await
    .expect("list model-scoped sticky events");
    let replacement_event = events
        .items
        .iter()
        .find(|event| event.invoke_id.as_deref() == Some("success-gpt-51"))
        .expect("gpt-5.1 success event should be present");
    assert_eq!(
        replacement_event
            .routing_context
            .as_ref()
            .map(|context| context.reason_code.as_str()),
        Some("firstSuccessfulAssignment")
    );
    assert_eq!(
        replacement_event
            .routing_context
            .as_ref()
            .and_then(|context| context.causing_attempt_id.as_deref()),
        None,
        "a gpt-5.4 clear must not become the cause of a different model's assignment"
    );
}
