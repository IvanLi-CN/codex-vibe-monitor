#[tokio::test]
pub(crate) async fn prompt_cache_conversation_proxy_override_bypasses_node_shunt_group_slots() {
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
pub(crate) async fn prompt_cache_conversation_binding_route_accepts_encoded_key() {
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
pub(crate) async fn prompt_cache_group_binding_timeout_response_ignores_account_overrides() {
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

    seed_timeout_policy_layers(&state.pool, group_name, first_account_id).await;

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

    assert_persisted_group_timeout_binding(state, prompt_cache_key).await;
}

async fn seed_timeout_policy_layers(pool: &Pool<Sqlite>, group_name: &str, account_id: i64) {
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
    .execute(pool)
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
    .execute(pool)
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
    .bind(account_id)
    .execute(pool)
    .await
    .expect("seed account timeout override");
}

async fn assert_persisted_group_timeout_binding(state: Arc<AppState>, prompt_cache_key: &str) {
    let Json(get_response) =
        get_prompt_cache_conversation_binding(State(state), AxumPath(prompt_cache_key.to_string()))
            .await
            .expect("get group timeout binding");
    assert_eq!(
        get_response.timeouts.responses_first_byte_timeout_secs,
        Some(31)
    );
    assert_eq!(
        get_response.timeouts.compact_first_byte_timeout_secs,
        Some(22)
    );
    assert_eq!(
        get_response.timeouts.responses_stream_timeout_secs,
        Some(33)
    );
    assert_eq!(get_response.timeouts.compact_stream_timeout_secs, Some(91));
    assert_eq!(
        get_response
            .timeout_field_sources
            .responses_first_byte_timeout_secs,
        "group"
    );
    assert_eq!(
        get_response
            .timeout_field_sources
            .compact_first_byte_timeout_secs,
        "root"
    );
    assert_eq!(
        get_response
            .timeout_field_sources
            .responses_stream_timeout_secs,
        "group"
    );
    assert_eq!(
        get_response
            .timeout_field_sources
            .compact_stream_timeout_secs,
        "conversation"
    );
}

#[tokio::test]
pub(crate) async fn prompt_cache_conversation_binding_reports_encrypted_owner_and_clear_keeps_owner_lock()
 {
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
pub(crate) async fn prompt_cache_conversation_binding_hides_encrypted_owner_when_routing_disabled()
{
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
pub(crate) async fn prompt_cache_group_binding_promotes_to_account_after_encrypted_owner_lock() {
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

    assert_promoted_binding_state(state, prompt_cache_key, account_id).await;
}

async fn assert_promoted_binding_state(
    state: Arc<AppState>,
    prompt_cache_key: &str,
    account_id: i64,
) {
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
pub(crate) async fn bulk_prompt_cache_conversation_bindings_bind_to_upstream_account_across_keys() {
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
        assert_bulk_account_binding(&state, &response, prompt_cache_key, account_id).await;
    }
}

async fn assert_bulk_account_binding(
    state: &Arc<AppState>,
    response: &BulkPromptCacheConversationBindingsResponse,
    prompt_cache_key: &str,
    account_id: i64,
) {
    let item = response
        .items
        .iter()
        .find(|candidate| candidate.prompt_cache_key == prompt_cache_key)
        .expect("bulk bind response should include each key");
    assert!(item.ok);
    assert_eq!(item.error, None);
    let binding = item.binding.as_ref().expect("successful bind snapshot");
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
    .expect("list bulk bind operation events");
    let event = events
        .items
        .iter()
        .find(|event| event.action == "manualBindingUpdated" && event.origin == "dashboardBulk")
        .expect("bulk binding event");
    assert!(
        event
            .sticky_transitions
            .iter()
            .any(|transition| { transition.model_key.is_none() && transition.after.is_some() })
    );
    assert!(
        !events.items.iter().any(|event| {
            event.action == "stickyTargetChanged" && event.origin == "dashboardBulk"
        })
    );
}

#[tokio::test]
pub(crate) async fn bulk_prompt_cache_conversation_bindings_bind_none_clears_only_manual_binding() {
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

    assert_bind_none_preserves_affinity(state, prompt_cache_key).await;
}

async fn assert_bind_none_preserves_affinity(state: Arc<AppState>, prompt_cache_key: &str) {
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
pub(crate) async fn bulk_prompt_cache_conversation_bindings_clear_and_reset_affinity_removes_all_affinity_rows()
 {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let prompt_cache_key = "bulk-clear-affinity-key";
    seed_bulk_clear_affinity(&state, prompt_cache_key).await;

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

    assert_all_affinity_rows_removed(&state.pool, prompt_cache_key).await;

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
            .any(|transition| { transition.model_key.as_deref() == Some("gpt-5.4") })
    );
    assert!(
        !event_response.items.iter().any(|event| {
            event.action == "stickyTargetCleared" && event.origin == "dashboardBulk"
        })
    );
}

async fn seed_bulk_clear_affinity(state: &Arc<AppState>, prompt_cache_key: &str) {
    enable_encrypted_session_owner_routing_for_test(state).await;
    let group_name = "bulk-prompt-cache-clear-group";
    ensure_test_group_binding(&state.pool, group_name, None).await;
    let account_id = insert_test_pool_api_key_account_with_options(
        state,
        "Bulk Prompt Cache Clear",
        "sk-bulk-prompt-cache-clear",
        Some(group_name),
        None,
        None,
    )
    .await;
    let payload = serde_json::from_value(json!({
        "bindingKind": "upstreamAccount", "upstreamAccountId": account_id,
    }))
    .expect("deserialize setup binding payload");
    let Json(response) = patch_prompt_cache_conversation_binding(
        State(state.clone()),
        AxumPath(prompt_cache_key.to_string()),
        Json(payload),
    )
    .await
    .expect("setup account binding should succeed");
    assert_eq!(response.binding_kind, "upstreamAccount");
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
}

async fn assert_all_affinity_rows_removed(pool: &Pool<Sqlite>, prompt_cache_key: &str) {
    let binding_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM prompt_cache_conversation_bindings WHERE prompt_cache_key = ?1",
    )
    .bind(prompt_cache_key)
    .fetch_one(pool)
    .await
    .expect("count binding rows after bulk clear");
    assert_eq!(binding_count, 0);

    let sticky_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM pool_sticky_routes WHERE sticky_key = ?1")
            .bind(prompt_cache_key)
            .fetch_one(pool)
            .await
            .expect("count sticky rows after bulk clear");
    assert_eq!(sticky_count, 0);

    let sticky_model_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM pool_sticky_model_routes WHERE sticky_key = ?1")
            .bind(prompt_cache_key)
            .fetch_one(pool)
            .await
            .expect("count model sticky rows after bulk clear");
    assert_eq!(sticky_model_count, 0);

    let owner_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM prompt_cache_encrypted_session_owners WHERE prompt_cache_key = ?1",
    )
    .bind(prompt_cache_key)
    .fetch_one(pool)
    .await
    .expect("count encrypted owner rows after bulk clear");
    assert_eq!(owner_count, 0);

    let owner_row = load_prompt_cache_encrypted_session_owner_row(pool, prompt_cache_key)
        .await
        .expect("load encrypted owner row after clear");
    assert!(owner_row.is_none());

    let effective_constraint =
        resolve_prompt_cache_effective_routing_constraint(pool, Some(prompt_cache_key), true, true)
            .await
            .expect("resolve routing constraint after bulk clear");
    assert!(effective_constraint.0.is_none());
    assert!(!effective_constraint.1);
}

use super::*;
