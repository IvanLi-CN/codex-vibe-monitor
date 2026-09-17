#[tokio::test]
pub(crate) async fn generic_sticky_route_success_does_not_emit_prompt_cache_conversation_event() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let account_id =
        insert_test_pool_api_key_account(&state, "Generic Sticky Route", "upstream-generic").await;
    let sticky_key = "generic-sticky-only-key";

    record_pool_route_success_with_affinity_generation(
        &state.pool,
        account_id,
        Utc::now(),
        Some(sticky_key),
        None,
        Some("generic-sticky-invoke"),
        None,
    )
    .await
    .expect("generic sticky success should still upsert the sticky route");

    let sticky_row = load_sticky_route(&state.pool, sticky_key)
        .await
        .expect("load sticky route after generic sticky success")
        .expect("generic sticky success should persist sticky route");
    assert_eq!(sticky_row.account_id, account_id);

    let Json(event_response) = list_prompt_cache_conversation_operation_events(
        State(state.clone()),
        AxumPath(sticky_key.to_string()),
        axum::extract::Query(ListPromptCacheConversationOperationEventsQuery {
            page: Some(1),
            page_size: Some(20),
            info_type: None,
            routing_scope: None,
            routing_model: None,
        }),
    )
    .await
    .expect("list generic sticky operation events");
    assert!(
        event_response.items.is_empty(),
        "generic sticky affinities must not create prompt cache conversation events"
    );
}

#[tokio::test]
pub(crate) async fn bulk_prompt_cache_conversation_bindings_set_fast_mode_rewrite_mode_preserves_binding_kind()
 {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let group_name = "bulk-prompt-cache-fast-mode-group";
    ensure_test_group_binding(&state.pool, group_name, None).await;
    let account_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Bulk Prompt Cache Fast Mode",
        "sk-bulk-prompt-cache-fast-mode",
        Some(group_name),
        None,
        None,
    )
    .await;
    let prompt_cache_key_none = "bulk-fast-mode-none-key";
    let prompt_cache_key_account = "bulk-fast-mode-account-key";

    let account_binding_payload: UpdatePromptCacheConversationBindingRequest =
        serde_json::from_value(json!({
            "bindingKind": "upstreamAccount",
            "upstreamAccountId": account_id,
        }))
        .expect("deserialize setup account binding payload");
    let Json(account_binding_response) = patch_prompt_cache_conversation_binding(
        State(state.clone()),
        AxumPath(prompt_cache_key_account.to_string()),
        Json(account_binding_payload),
    )
    .await
    .expect("setup account binding should succeed");
    assert_eq!(account_binding_response.binding_kind, "upstreamAccount");

    let payload: BulkPromptCacheConversationBindingsRequest = serde_json::from_value(json!({
        "promptCacheKeys": [prompt_cache_key_none, prompt_cache_key_account],
        "action": "setFastModeRewriteMode",
        "fastModeRewriteMode": "fill_missing",
    }))
    .expect("deserialize bulk fast mode payload");
    let Json(response) =
        post_bulk_prompt_cache_conversation_bindings(State(state.clone()), Json(payload))
            .await
            .expect("bulk fast mode update should succeed");
    assert_eq!(response.action, "setFastModeRewriteMode");
    assert_eq!(response.total_requested, 2);
    assert_eq!(response.total_succeeded, 2);
    assert_eq!(response.total_failed, 0);

    let none_item = response
        .items
        .iter()
        .find(|candidate| candidate.prompt_cache_key == prompt_cache_key_none)
        .expect("response should include unbound key");
    assert!(none_item.ok);
    let none_binding = none_item
        .binding
        .as_ref()
        .expect("successful fast mode write should include binding snapshot");
    assert_eq!(none_binding.binding_kind, "none");
    assert_eq!(
        none_binding.fast_mode_rewrite_mode,
        Some(TagFastModeRewriteMode::FillMissing)
    );

    let account_item = response
        .items
        .iter()
        .find(|candidate| candidate.prompt_cache_key == prompt_cache_key_account)
        .expect("response should include bound key");
    assert!(account_item.ok);
    let account_binding = account_item
        .binding
        .as_ref()
        .expect("successful fast mode write should include binding snapshot");
    assert_eq!(account_binding.binding_kind, "upstreamAccount");
    assert_eq!(account_binding.upstream_account_id, Some(account_id));
    assert_eq!(
        account_binding.fast_mode_rewrite_mode,
        Some(TagFastModeRewriteMode::FillMissing)
    );

    assert_bulk_fast_mode_persistence_and_events(
        state,
        prompt_cache_key_none,
        prompt_cache_key_account,
        account_id,
    )
    .await;
}

async fn assert_bulk_fast_mode_persistence_and_events(
    state: Arc<AppState>,
    prompt_cache_key_none: &str,
    prompt_cache_key_account: &str,
    account_id: i64,
) {
    let none_row = load_prompt_cache_conversation_binding_row(&state.pool, prompt_cache_key_none)
        .await
        .expect("load none-row after bulk fast mode")
        .expect("none-row should exist after bulk fast mode");
    assert_eq!(none_row.binding_kind, PROMPT_CACHE_BINDING_KIND_NONE);
    assert_eq!(
        none_row.fast_mode_rewrite_mode.as_deref(),
        Some("fill_missing")
    );

    let account_row =
        load_prompt_cache_conversation_binding_row(&state.pool, prompt_cache_key_account)
            .await
            .expect("load account-row after bulk fast mode")
            .expect("account-row should exist after bulk fast mode");
    assert_eq!(
        account_row.binding_kind,
        PROMPT_CACHE_BINDING_KIND_UPSTREAM_ACCOUNT
    );
    assert_eq!(account_row.upstream_account_id, Some(account_id));
    assert_eq!(
        account_row.fast_mode_rewrite_mode.as_deref(),
        Some("fill_missing")
    );

    let Json(none_events) = list_prompt_cache_conversation_operation_events(
        State(state.clone()),
        AxumPath(prompt_cache_key_none.to_string()),
        axum::extract::Query(ListPromptCacheConversationOperationEventsQuery {
            page: Some(1),
            page_size: Some(20),
            info_type: Some("requestRewrite".to_string()),
            routing_scope: None,
            routing_model: None,
        }),
    )
    .await
    .expect("list none-key bulk fast mode events");
    assert!(none_events.items.iter().any(
        |event| event.action == "conversationPolicyUpdated" && event.origin == "dashboardBulk"
    ));

    let Json(account_events) = list_prompt_cache_conversation_operation_events(
        State(state.clone()),
        AxumPath(prompt_cache_key_account.to_string()),
        axum::extract::Query(ListPromptCacheConversationOperationEventsQuery {
            page: Some(1),
            page_size: Some(20),
            info_type: Some("requestRewrite".to_string()),
            routing_scope: None,
            routing_model: None,
        }),
    )
    .await
    .expect("list account-key bulk fast mode events");
    assert!(account_events.items.iter().any(
        |event| event.action == "conversationPolicyUpdated" && event.origin == "dashboardBulk"
    ));
}

#[tokio::test]
pub(crate) async fn prompt_cache_conversation_operation_events_list_filters_by_info_type() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let prompt_cache_key = "prompt-cache-operation-events-key";
    seed_prompt_cache_operation_events(&state, prompt_cache_key).await;

    let Json(all_events) = list_prompt_cache_conversation_operation_events(
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
    .expect("list all prompt cache operation events");
    assert!(all_events.total >= 2);
    let manual_binding_event = all_events
        .items
        .iter()
        .find(|event| event.action == "manualBindingUpdated")
        .expect("manual binding event should be recorded");
    assert!(!manual_binding_event.sticky_transitions.is_empty());
    let policy_event = all_events
        .items
        .iter()
        .find(|event| event.action == "conversationPolicyUpdated")
        .expect("policy event should be recorded");
    assert_eq!(policy_event.origin, "detailDrawer");
    for expected in ["routing", "forwardProxy", "requestRewrite"] {
        assert!(
            policy_event
                .info_types
                .iter()
                .any(|value| value == expected)
        );
    }

    assert_operation_event_filters(state, prompt_cache_key).await;
}

async fn seed_prompt_cache_operation_events(state: &Arc<AppState>, prompt_cache_key: &str) {
    let account_id = insert_test_pool_api_key_account_with_options(
        state,
        "Prompt Cache Operation Events",
        "sk-prompt-cache-operation-events",
        Some("prompt-cache-ops-group"),
        None,
        None,
    )
    .await;

    let binding_payload: UpdatePromptCacheConversationBindingRequest =
        serde_json::from_value(json!({
            "bindingKind": "upstreamAccount",
            "upstreamAccountId": account_id,
        }))
        .expect("deserialize binding payload");
    let _ = patch_prompt_cache_conversation_binding(
        State(state.clone()),
        AxumPath(prompt_cache_key.to_string()),
        Json(binding_payload),
    )
    .await
    .expect("save manual binding for operation events");

    let policy_payload: UpdatePromptCacheConversationBindingRequest =
        serde_json::from_value(json!({
            "bindingKind": "upstreamAccount",
            "upstreamAccountId": account_id,
            "allowSwitchUpstream": false,
            "fastModeRewriteMode": "fill_missing",
            "forwardProxyKeys": ["__direct__"],
        }))
        .expect("deserialize policy payload");
    let _ = patch_prompt_cache_conversation_binding(
        State(state.clone()),
        AxumPath(prompt_cache_key.to_string()),
        Json(policy_payload),
    )
    .await
    .expect("save prompt cache policy update");
}

async fn assert_operation_event_filters(state: Arc<AppState>, prompt_cache_key: &str) {
    let Json(request_rewrite_events) = list_prompt_cache_conversation_operation_events(
        State(state.clone()),
        AxumPath(prompt_cache_key.to_string()),
        axum::extract::Query(ListPromptCacheConversationOperationEventsQuery {
            page: Some(1),
            page_size: Some(20),
            info_type: Some("requestRewrite".to_string()),
            routing_scope: None,
            routing_model: None,
        }),
    )
    .await
    .expect("filter prompt cache operation events by request rewrite");
    assert_eq!(request_rewrite_events.total, 1);
    assert_eq!(
        request_rewrite_events.items[0].action,
        "conversationPolicyUpdated"
    );

    let Json(forward_proxy_events) = list_prompt_cache_conversation_operation_events(
        State(state.clone()),
        AxumPath(prompt_cache_key.to_string()),
        axum::extract::Query(ListPromptCacheConversationOperationEventsQuery {
            page: Some(1),
            page_size: Some(20),
            info_type: Some("forwardProxy".to_string()),
            routing_scope: None,
            routing_model: None,
        }),
    )
    .await
    .expect("filter prompt cache operation events by forward proxy");
    assert_eq!(forward_proxy_events.total, 1);
    assert_eq!(
        forward_proxy_events.items[0].action,
        "conversationPolicyUpdated"
    );
}

#[tokio::test]
pub(crate) async fn bulk_prompt_cache_conversation_bindings_reject_invalid_target_without_partial_writes()
 {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let prompt_cache_keys = ["bulk-invalid-target-a", "bulk-invalid-target-b"];
    let payload: BulkPromptCacheConversationBindingsRequest = serde_json::from_value(json!({
        "promptCacheKeys": prompt_cache_keys,
        "action": "bind",
        "bindingKind": "group",
        "groupName": "missing-bulk-target-group",
    }))
    .expect("deserialize invalid bulk bind payload");

    let err = post_bulk_prompt_cache_conversation_bindings(State(state.clone()), Json(payload))
        .await
        .expect_err("invalid target should fail before partial writes");
    assert!(matches!(err, ApiError::BadRequest(_)));

    for prompt_cache_key in prompt_cache_keys {
        let binding_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM prompt_cache_conversation_bindings WHERE prompt_cache_key = ?1",
        )
        .bind(prompt_cache_key)
        .fetch_one(&state.pool)
        .await
        .expect("count bindings after invalid bulk request");
        assert_eq!(binding_count, 0);

        let sticky_count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM pool_sticky_routes WHERE sticky_key = ?1")
                .bind(prompt_cache_key)
                .fetch_one(&state.pool)
                .await
                .expect("count sticky routes after invalid bulk request");
        assert_eq!(sticky_count, 0);
    }
}

#[tokio::test]
pub(crate) async fn prompt_cache_same_account_binding_newer_than_owner_keeps_owner_guard_active() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let owner_account_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Prompt Cache Same Account Owner",
        "sk-prompt-cache-same-account-owner",
        None,
        None,
        None,
    )
    .await;
    let prompt_cache_key = "prompt-cache-same-account-binding-newer-key";

    sqlx::query(
        r#"
        INSERT INTO prompt_cache_encrypted_session_owners (
            prompt_cache_key,
            owner_upstream_account_id,
            first_locked_at,
            last_confirmed_at,
            updated_at
        )
        VALUES (?1, ?2, '2026-06-14 12:00:00', '2026-06-14 12:00:00', '2026-06-14 12:00:00')
        "#,
    )
    .bind(prompt_cache_key)
    .bind(owner_account_id)
    .execute(&state.pool)
    .await
    .expect("seed encrypted owner row");

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
        VALUES (?1, 'upstream_account', NULL, ?2, '2026-06-14 12:00:01', '2026-06-14 12:00:01')
        "#,
    )
    .bind(prompt_cache_key)
    .bind(owner_account_id)
    .execute(&state.pool)
    .await
    .expect("seed same-account binding with newer timestamp");

    let (constraint, owner_auto_guard_active) = resolve_prompt_cache_effective_routing_constraint(
        &state.pool,
        Some(prompt_cache_key),
        true,
        true,
    )
    .await
    .expect("resolve same-account newer binding");

    assert!(owner_auto_guard_active);
    match constraint {
        Some(PromptCacheConversationBindingConstraint::UpstreamAccount(bound_id)) => {
            assert_eq!(bound_id, owner_account_id);
        }
        other => panic!("expected owner account constraint, got {other:?}"),
    }
}

#[tokio::test]
pub(crate) async fn prompt_cache_same_group_binding_overrides_owner_guard() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let group_name = "prompt-cache-same-group-owner-override-group";
    ensure_test_group_binding(&state.pool, group_name, None).await;
    let owner_account_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Prompt Cache Same Group Owner",
        "sk-prompt-cache-same-group-owner",
        Some(group_name),
        None,
        None,
    )
    .await;
    let prompt_cache_key = "prompt-cache-same-group-owner-override-key";

    sqlx::query(
        r#"
        INSERT INTO prompt_cache_encrypted_session_owners (
            prompt_cache_key,
            owner_upstream_account_id,
            first_locked_at,
            last_confirmed_at,
            updated_at
        )
        VALUES (?1, ?2, '2026-06-14 12:00:00', '2026-06-14 12:10:00', '2026-06-14 12:10:00')
        "#,
    )
    .bind(prompt_cache_key)
    .bind(owner_account_id)
    .execute(&state.pool)
    .await
    .expect("seed encrypted owner in target group");

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
        VALUES (?1, 'group', ?2, NULL, '2026-06-14 12:05:00', '2026-06-14 12:05:00')
        "#,
    )
    .bind(prompt_cache_key)
    .bind(group_name)
    .execute(&state.pool)
    .await
    .expect("seed manual same-group override older than reconfirmed owner");

    let (constraint, owner_auto_guard_active) = resolve_prompt_cache_effective_routing_constraint(
        &state.pool,
        Some(prompt_cache_key),
        true,
        true,
    )
    .await
    .expect("resolve same-group manual override after owner reconfirmation");

    assert!(!owner_auto_guard_active);
    match constraint {
        Some(PromptCacheConversationBindingConstraint::Group(bound_group_name)) => {
            assert_eq!(bound_group_name, group_name);
        }
        other => panic!("expected explicit group override, got {other:?}"),
    }
}

#[tokio::test]
pub(crate) async fn prompt_cache_pre_owner_group_binding_keeps_owner_guard_active() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let group_name = "prompt-cache-pre-owner-group-binding";
    ensure_test_group_binding(&state.pool, group_name, None).await;
    let owner_account_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Prompt Cache Pre Owner Group Owner",
        "sk-prompt-cache-pre-owner-group-owner",
        Some(group_name),
        None,
        None,
    )
    .await;
    let prompt_cache_key = "prompt-cache-pre-owner-group-binding-key";

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
        VALUES (?1, 'group', ?2, NULL, '2026-06-14 11:55:00', '2026-06-14 11:55:00')
        "#,
    )
    .bind(prompt_cache_key)
    .bind(group_name)
    .execute(&state.pool)
    .await
    .expect("seed group binding before encrypted owner lock");

    sqlx::query(
        r#"
        INSERT INTO prompt_cache_encrypted_session_owners (
            prompt_cache_key,
            owner_upstream_account_id,
            first_locked_at,
            last_confirmed_at,
            updated_at
        )
        VALUES (?1, ?2, '2026-06-14 12:00:00', '2026-06-14 12:00:00', '2026-06-14 12:00:00')
        "#,
    )
    .bind(prompt_cache_key)
    .bind(owner_account_id)
    .execute(&state.pool)
    .await
    .expect("seed encrypted owner after group binding");

    let (constraint, owner_auto_guard_active) = resolve_prompt_cache_effective_routing_constraint(
        &state.pool,
        Some(prompt_cache_key),
        true,
        true,
    )
    .await
    .expect("resolve pre-owner group binding");

    assert!(owner_auto_guard_active);
    match constraint {
        Some(PromptCacheConversationBindingConstraint::UpstreamAccount(bound_id)) => {
            assert_eq!(bound_id, owner_account_id);
        }
        other => panic!("expected encrypted owner constraint, got {other:?}"),
    }
}

#[tokio::test]
pub(crate) async fn prompt_cache_manual_override_wins_when_binding_and_owner_share_same_second() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let group_name = "prompt-cache-manual-override-same-second-group";
    ensure_test_group_binding(&state.pool, group_name, None).await;
    let owner_account_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Prompt Cache Owner Same Second",
        "sk-prompt-cache-owner-same-second",
        Some("prompt-cache-other-group"),
        None,
        None,
    )
    .await;
    let override_account_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Prompt Cache Override Same Second",
        "sk-prompt-cache-override-same-second",
        Some(group_name),
        None,
        None,
    )
    .await;
    let prompt_cache_key = "prompt-cache-manual-override-same-second-key";

    sqlx::query(
        r#"
        INSERT INTO prompt_cache_encrypted_session_owners (
            prompt_cache_key,
            owner_upstream_account_id,
            first_locked_at,
            last_confirmed_at,
            updated_at
        )
        VALUES (?1, ?2, '2026-06-14 12:00:00', '2026-06-14 12:00:00', '2026-06-14 12:00:00')
        "#,
    )
    .bind(prompt_cache_key)
    .bind(owner_account_id)
    .execute(&state.pool)
    .await
    .expect("seed encrypted owner with fixed-second timestamp");

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
        VALUES (?1, 'upstream_account', NULL, ?2, '2026-06-14 12:00:00', '2026-06-14 12:00:00')
        "#,
    )
    .bind(prompt_cache_key)
    .bind(override_account_id)
    .execute(&state.pool)
    .await
    .expect("seed manual override in the same second");

    let (constraint, owner_auto_guard_active) = resolve_prompt_cache_effective_routing_constraint(
        &state.pool,
        Some(prompt_cache_key),
        true,
        true,
    )
    .await
    .expect("resolve same-second manual override");

    assert!(!owner_auto_guard_active);
    match constraint {
        Some(PromptCacheConversationBindingConstraint::UpstreamAccount(bound_id)) => {
            assert_eq!(bound_id, override_account_id);
        }
        other => panic!("expected explicit upstream account override, got {other:?}"),
    }
}

#[tokio::test]
pub(crate) async fn prompt_cache_manual_override_wins_even_after_owner_reconfirmation() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let group_name = "prompt-cache-manual-override-owner-newer-group";
    ensure_test_group_binding(&state.pool, group_name, None).await;
    let owner_account_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Prompt Cache Owner Reconfirmed",
        "sk-prompt-cache-owner-reconfirmed",
        Some("prompt-cache-original-owner-group"),
        None,
        None,
    )
    .await;
    let override_account_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Prompt Cache Override Owner Newer",
        "sk-prompt-cache-override-owner-newer",
        Some(group_name),
        None,
        None,
    )
    .await;
    let prompt_cache_key = "prompt-cache-manual-override-owner-newer-key";

    sqlx::query(
        r#"
        INSERT INTO prompt_cache_encrypted_session_owners (
            prompt_cache_key,
            owner_upstream_account_id,
            first_locked_at,
            last_confirmed_at,
            updated_at
        )
        VALUES (?1, ?2, '2026-06-14 12:00:00', '2026-06-14 12:10:00', '2026-06-14 12:10:00')
        "#,
    )
    .bind(prompt_cache_key)
    .bind(owner_account_id)
    .execute(&state.pool)
    .await
    .expect("seed reconfirmed encrypted owner");

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
        VALUES (?1, 'upstream_account', NULL, ?2, '2026-06-14 12:05:00', '2026-06-14 12:05:00')
        "#,
    )
    .bind(prompt_cache_key)
    .bind(override_account_id)
    .execute(&state.pool)
    .await
    .expect("seed manual override older than reconfirmed owner");

    let (constraint, owner_auto_guard_active) = resolve_prompt_cache_effective_routing_constraint(
        &state.pool,
        Some(prompt_cache_key),
        true,
        true,
    )
    .await
    .expect("resolve manual override after owner reconfirmation");

    assert!(!owner_auto_guard_active);
    match constraint {
        Some(PromptCacheConversationBindingConstraint::UpstreamAccount(bound_id)) => {
            assert_eq!(bound_id, override_account_id);
        }
        other => panic!("expected explicit upstream account override, got {other:?}"),
    }
}

#[tokio::test]
pub(crate) async fn prompt_cache_group_promotion_ignores_stale_group_after_operator_rebind() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let original_group = "prompt-cache-promotion-original-group";
    let rebound_group = "prompt-cache-promotion-rebound-group";
    ensure_test_group_binding(&state.pool, original_group, None).await;
    ensure_test_group_binding(&state.pool, rebound_group, None).await;

    let stale_success_account_id = insert_test_pool_oauth_account(
        &state,
        "Prompt Cache Stale Promote OAuth",
        "oauth-prompt-cache-stale-promote",
    )
    .await;
    set_test_account_group_name(&state.pool, stale_success_account_id, Some(original_group)).await;
    let rebound_account_id = insert_test_pool_oauth_account(
        &state,
        "Prompt Cache Rebound Group OAuth",
        "oauth-prompt-cache-rebound-group",
    )
    .await;
    set_test_account_group_name(&state.pool, rebound_account_id, Some(rebound_group)).await;
    let prompt_cache_key = "prompt-cache-stale-group-promotion-key";

    let original_group_payload: UpdatePromptCacheConversationBindingRequest =
        serde_json::from_value(json!({
            "bindingKind": "group",
            "groupName": original_group,
        }))
        .expect("deserialize original group payload");
    let _ = patch_prompt_cache_conversation_binding(
        State(state.clone()),
        AxumPath(prompt_cache_key.to_string()),
        Json(original_group_payload),
    )
    .await
    .expect("seed original group binding");

    let rebound_group_payload: UpdatePromptCacheConversationBindingRequest =
        serde_json::from_value(json!({
            "bindingKind": "group",
            "groupName": rebound_group,
        }))
        .expect("deserialize rebound group payload");
    let Json(rebound_response) = patch_prompt_cache_conversation_binding(
        State(state.clone()),
        AxumPath(prompt_cache_key.to_string()),
        Json(rebound_group_payload),
    )
    .await
    .expect("operator rebinds to new group");
    assert_eq!(rebound_response.binding_kind, "group");
    assert_eq!(rebound_response.group_name.as_deref(), Some(rebound_group));

    promote_prompt_cache_group_binding_to_upstream_account(
        &state.pool,
        prompt_cache_key,
        stale_success_account_id,
    )
    .await
    .expect("stale success should not overwrite newer group binding");

    let Json(current_binding) = get_prompt_cache_conversation_binding(
        State(state.clone()),
        AxumPath(prompt_cache_key.to_string()),
    )
    .await
    .expect("load binding after stale promotion attempt");
    assert_eq!(current_binding.binding_kind, "group");
    assert_eq!(current_binding.group_name.as_deref(), Some(rebound_group));
    assert_eq!(current_binding.upstream_account_id, None);

    promote_prompt_cache_group_binding_to_upstream_account(
        &state.pool,
        prompt_cache_key,
        rebound_account_id,
    )
    .await
    .expect("matching rebound group success should still promote");

    let Json(promoted_binding) = get_prompt_cache_conversation_binding(
        State(state.clone()),
        AxumPath(prompt_cache_key.to_string()),
    )
    .await
    .expect("load binding after valid rebound promotion");
    assert_eq!(promoted_binding.binding_kind, "upstreamAccount");
    assert_eq!(
        promoted_binding.upstream_account_id,
        Some(rebound_account_id)
    );
}

#[tokio::test]
pub(crate) async fn prompt_cache_stale_success_cannot_reclaim_owner_after_override_migration() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let original_group = "prompt-cache-owner-original-group";
    let target_group = "prompt-cache-owner-target-group";
    ensure_test_group_binding(&state.pool, original_group, None).await;
    ensure_test_group_binding(&state.pool, target_group, None).await;

    let original_owner_account_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Prompt Cache Original Owner",
        "sk-prompt-cache-original-owner",
        Some(original_group),
        None,
        None,
    )
    .await;
    let target_owner_account_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Prompt Cache Target Owner",
        "sk-prompt-cache-target-owner",
        Some(target_group),
        None,
        None,
    )
    .await;
    let prompt_cache_key = "prompt-cache-owner-migration-guard-key";

    upsert_prompt_cache_encrypted_session_owner(
        &state.pool,
        prompt_cache_key,
        original_owner_account_id,
    )
    .await
    .expect("seed original encrypted owner");

    let target_group_payload: UpdatePromptCacheConversationBindingRequest =
        serde_json::from_value(json!({
            "bindingKind": "group",
            "groupName": target_group,
        }))
        .expect("deserialize target group payload");
    let _ = patch_prompt_cache_conversation_binding(
        State(state.clone()),
        AxumPath(prompt_cache_key.to_string()),
        Json(target_group_payload),
    )
    .await
    .expect("operator rebinds encrypted conversation to target group");

    let migrated = confirm_prompt_cache_encrypted_session_owner_success(
        &state.pool,
        prompt_cache_key,
        target_owner_account_id,
    )
    .await
    .expect("target owner confirmation should succeed");
    assert!(migrated);

    let stale_reclaim = confirm_prompt_cache_encrypted_session_owner_success(
        &state.pool,
        prompt_cache_key,
        original_owner_account_id,
    )
    .await
    .expect("stale original owner confirmation should be evaluated");
    assert!(!stale_reclaim);

    let owner_row = load_prompt_cache_encrypted_session_owner_row(&state.pool, prompt_cache_key)
        .await
        .expect("load encrypted owner row after stale reclaim attempt")
        .expect("encrypted owner row should exist");
    assert_eq!(owner_row.owner_upstream_account_id, target_owner_account_id);
}

use super::*;
