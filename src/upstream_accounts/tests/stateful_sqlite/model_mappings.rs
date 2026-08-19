use super::*;
use crate::tests::seed_pool_routing_api_key;

fn mapping(source_model: &str, target_model: &str, enabled: bool) -> ModelMapping {
    ModelMapping {
        source_model: source_model.to_string(),
        target_model: target_model.to_string(),
        enabled,
    }
}

#[tokio::test]
async fn post_create_sync_warms_empty_model_mapping_cache_entry() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    seed_pool_routing_api_key(&state, "pool-model-cache-key").await;
    let generation_before_create = state
        .pool_routing_runtime_cache
        .lock()
        .await
        .as_ref()
        .expect("routing cache after API-key save")
        .model_routing
        .generation;

    let account_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Cached empty mapping account",
        "sk-cached-empty-mapping",
        None,
        Some("https://cached-empty-mapping.example.com/backend-api/codex"),
    )
    .await;

    let cache = state.pool_routing_runtime_cache.lock().await;
    let model_routing = &cache
        .as_ref()
        .expect("routing cache after account creation")
        .model_routing;
    assert!(model_routing.generation > generation_before_create);
    assert!(
        model_routing.mappings_by_account.contains_key(&account_id),
        "a new account must receive an explicit empty mapping entry"
    );
    drop(cache);
    assert_eq!(
        load_model_mapping_for_account(state.as_ref(), account_id, Some("client-model"))
            .await
            .expect("resolve cached empty mapping"),
        None
    );
}

#[tokio::test]
async fn model_mappings_api_replaces_rows_resets_state_and_refreshes_cache() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let api_key_account_id = insert_api_key_account(&state.pool, "Mapping API key").await;
    let crypto_key = state
        .upstream_accounts
        .crypto_key
        .as_ref()
        .expect("test crypto key");
    let oauth_account_id = insert_syncable_oauth_account(
        &state.pool,
        crypto_key,
        "Mapping OAuth",
        "mapping-oauth@example.com",
        "org_mapping_oauth",
        "user_mapping_oauth",
    )
    .await;

    observe_model_route_seen(&state.pool, api_key_account_id, Some("client-fast"))
        .await
        .expect("seed model route");
    ensure_account_has_unsupported_model_tag(&state.pool, api_key_account_id, "upstream-old")
        .await
        .expect("seed unsupported target tag");
    let initial_cache = refresh_pool_routing_runtime_cache(state.as_ref())
        .await
        .expect("seed runtime cache");

    let Json(detail) = update_upstream_account_model_mappings(
        State(state.clone()),
        HeaderMap::new(),
        AxumPath(api_key_account_id),
        Json(UpdateModelMappingsRequest {
            model_mappings: vec![
                mapping(" client-* ", " upstream-fast ", true),
                mapping("disabled", "unused", false),
            ],
        }),
    )
    .await
    .expect("save API key mappings");
    assert_eq!(detail.model_mappings.len(), 2);
    assert_eq!(detail.model_mappings[0].source_model, "client-*");
    assert_eq!(detail.model_mappings[0].target_model, "upstream-fast");
    assert!(!detail.model_mappings[1].enabled);

    let stored_mappings: String =
        sqlx::query_scalar("SELECT model_mappings_json FROM pool_upstream_accounts WHERE id = ?1")
            .bind(api_key_account_id)
            .fetch_one(&state.pool)
            .await
            .expect("load stored mappings");
    assert_eq!(
        decode_model_mappings_json(Some(&stored_mappings)),
        detail.model_mappings
    );
    let route_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM pool_upstream_account_model_routes WHERE account_id = ?1",
    )
    .bind(api_key_account_id)
    .fetch_one(&state.pool)
    .await
    .expect("count reset model routes");
    assert_eq!(route_count, 0);
    let unsupported_tag_count: i64 = sqlx::query_scalar(
        r#"
        SELECT COUNT(*)
        FROM pool_upstream_account_tags account_tags
        JOIN pool_tags tags ON tags.id = account_tags.tag_id
        WHERE account_tags.account_id = ?1
          AND tags.system_key LIKE 'unsupported_model:%'
        "#,
    )
    .bind(api_key_account_id)
    .fetch_one(&state.pool)
    .await
    .expect("count reset unsupported model tags");
    assert_eq!(unsupported_tag_count, 0);
    assert_eq!(
        load_model_mapping_for_account(state.as_ref(), api_key_account_id, Some("CLIENT-FAST"))
            .await
            .expect("resolve cached mapping")
            .expect("mapping should match")
            .target_model,
        "upstream-fast"
    );
    let cache = state.pool_routing_runtime_cache.lock().await;
    let refreshed_generation = cache
        .as_ref()
        .expect("runtime cache")
        .model_routing
        .generation;
    assert!(refreshed_generation > initial_cache.model_routing.generation);
    drop(cache);

    let Json(oauth_detail) = update_upstream_account_model_mappings(
        State(state.clone()),
        HeaderMap::new(),
        AxumPath(oauth_account_id),
        Json(UpdateModelMappingsRequest {
            model_mappings: vec![mapping("client-oauth", "upstream-oauth", true)],
        }),
    )
    .await
    .expect("save OAuth mappings");
    assert_eq!(
        oauth_detail.model_mappings[0].target_model,
        "upstream-oauth"
    );
    let generation_before_rejected_save = state
        .pool_routing_runtime_cache
        .lock()
        .await
        .as_ref()
        .expect("runtime cache remains installed")
        .model_routing
        .generation;

    let invalid = update_upstream_account_model_mappings(
        State(state.clone()),
        HeaderMap::new(),
        AxumPath(api_key_account_id),
        Json(UpdateModelMappingsRequest {
            model_mappings: vec![
                mapping("CLIENT-*", "one", true),
                mapping("client-*", "two", false),
            ],
        }),
    )
    .await
    .expect_err("duplicate source rules must be rejected");
    assert_eq!(invalid.0, StatusCode::BAD_REQUEST);
    let cache_after_rejected_save = state
        .pool_routing_runtime_cache
        .lock()
        .await
        .as_ref()
        .expect("runtime cache remains installed")
        .model_routing
        .generation;
    assert_eq!(cache_after_rejected_save, generation_before_rejected_save);
    assert_eq!(
        load_model_mapping_for_account(state.as_ref(), api_key_account_id, Some("client-fast"))
            .await
            .expect("resolve retained mapping")
            .expect("mapping should remain")
            .target_model,
        "upstream-fast"
    );
}

#[tokio::test]
async fn model_mapping_routing_bypasses_allowlist_but_respects_system_deny() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let account_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Mapped routing account",
        "sk-mapped-routing-account",
        None,
        Some("https://mapped-routing.example.com/backend-api/codex"),
    )
    .await;
    sqlx::query(
        r#"
        UPDATE pool_upstream_accounts
        SET policy_available_models_json = '["ordinary-model"]',
            policy_available_models_mode = 'allowlist',
            model_mappings_json = '[{"sourceModel":"client-*","targetModel":"upstream-special","enabled":true}]'
        WHERE id = ?1
        "#,
    )
    .bind(account_id)
    .execute(&state.pool)
    .await
    .expect("seed constrained account mapping");
    refresh_pool_routing_runtime_cache(state.as_ref())
        .await
        .expect("refresh routing cache");

    let resolution = resolve_pool_account_for_request_with_binding_constraint_and_model(
        state.as_ref(),
        None,
        Some("client-fast"),
        &[],
        &std::collections::HashSet::new(),
        None,
    )
    .await
    .expect("mapped model should route");
    let PoolAccountResolution::Resolved(resolved) = resolution else {
        panic!("expected mapped account to resolve, got {resolution:?}");
    };
    assert_eq!(resolved.account_id, account_id);

    ensure_account_has_unsupported_model_tag(&state.pool, account_id, "upstream-special")
        .await
        .expect("deny mapped target");
    let resolution = resolve_pool_account_for_request_with_binding_constraint_and_model(
        state.as_ref(),
        None,
        Some("client-fast"),
        &[],
        &std::collections::HashSet::new(),
        None,
    )
    .await
    .expect("resolution result");
    assert!(
        !matches!(resolution, PoolAccountResolution::Resolved(_)),
        "system deny for the mapped target must block routing, got {resolution:?}"
    );
}

#[tokio::test]
async fn model_mapping_does_not_bypass_conversation_model_override() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let account_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Mapped conversation override account",
        "sk-mapped-conversation-override",
        None,
        Some("https://mapped-conversation-override.example.com/backend-api/codex"),
    )
    .await;
    sqlx::query(
        r#"
        UPDATE pool_upstream_accounts
        SET policy_available_models_json = '["ordinary-model"]',
            policy_available_models_mode = 'allowlist',
            model_mappings_json = '[{"sourceModel":"client-*","targetModel":"upstream-special","enabled":true}]'
        WHERE id = ?1
        "#,
    )
    .bind(account_id)
    .execute(&state.pool)
    .await
    .expect("seed constrained account mapping");
    refresh_pool_routing_runtime_cache(state.as_ref())
        .await
        .expect("refresh routing cache");

    let conversation_override = ConversationRoutingOverride {
        available_models: Some(vec!["conversation-only".to_string()]),
        available_models_mode: Some(AvailableModelsMode::Allowlist),
        ..Default::default()
    };
    let resolution =
        resolve_pool_account_for_request_with_route_requirement_and_image_intent_and_override(
            state.as_ref(),
            None,
            Some("client-fast"),
            &[],
            &std::collections::HashSet::new(),
            None,
            None,
            Some(&conversation_override),
            "",
            crate::ImageIntent::Unknown,
        )
        .await
        .expect("conversation-constrained resolution result");
    assert!(
        !matches!(resolution, PoolAccountResolution::Resolved(_)),
        "an account mapping must not broaden an explicit conversation model restriction, got {resolution:?}"
    );
}

#[tokio::test]
async fn model_mapping_cache_warms_only_first_ten_finite_source_models() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    for index in 0..11 {
        let account_id = insert_api_key_account(&state.pool, &format!("Warm {index}")).await;
        sqlx::query(
            r#"
            UPDATE pool_upstream_accounts
            SET policy_available_models_json = ?2,
                policy_available_models_mode = 'allowlist',
                model_mappings_json = ?3
            WHERE id = ?1
            "#,
        )
        .bind(account_id)
        .bind(format!(r#"["finite-{index}"]"#))
        .bind(r#"[{"sourceModel":"client-*","targetModel":"target-only","enabled":true}]"#)
        .execute(&state.pool)
        .await
        .expect("seed finite model policy");
    }

    let cache = build_pool_model_routing_runtime_cache(&state.pool)
        .await
        .expect("build model routing cache");
    assert_eq!(cache.available_models.len(), 11);
    assert_eq!(
        cache.warmed_model_account_ids.len(),
        MAX_WARMED_ROUTING_MODELS
    );
    assert!(cache.warmed_model_account_ids.contains_key("finite-0"));
    assert!(cache.warmed_model_account_ids.contains_key("finite-9"));
    assert!(!cache.warmed_model_account_ids.contains_key("finite-10"));
    assert!(
        !cache
            .available_models
            .iter()
            .any(|model| model == "target-only")
    );
    assert!(!cache.warmed_model_account_ids.contains_key("target-only"));
}
