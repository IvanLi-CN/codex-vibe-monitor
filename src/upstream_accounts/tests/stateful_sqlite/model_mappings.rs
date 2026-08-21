use super::*;
use crate::tests::{
    seed_pool_routing_api_key, test_state_with_openai_base_and_pool_no_available_wait,
};

fn mapping(source_model: &str, target_model: &str, enabled: bool) -> ModelMapping {
    ModelMapping {
        source_model: source_model.to_string(),
        target_model: target_model.to_string(),
        enabled,
    }
}

#[tokio::test]
async fn empty_requested_model_uses_an_empty_model_health_key() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let account_id = insert_api_key_account(&state.pool, "Empty model health key").await;

    observe_model_route_seen(&state.pool, account_id, Some(""))
        .await
        .expect("record empty model route");
    let row: (String,) = sqlx::query_as(
        "SELECT model FROM pool_upstream_account_model_routes WHERE account_id = ?1",
    )
    .bind(account_id)
    .fetch_one(&state.pool)
    .await
    .expect("load empty model route");
    assert_eq!(row.0, "");
    assert_eq!(
        model_route_penalty(&state.pool, account_id, Some(""))
            .await
            .expect("load empty model route penalty"),
        ModelRoutePenalty::Normal
    );
}

#[tokio::test]
async fn empty_model_cooldown_expiry_is_visible_to_failover() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let account_id = insert_api_key_account(&state.pool, "Empty model cooldown").await;
    observe_model_route_seen(&state.pool, account_id, Some(""))
        .await
        .expect("record empty model route");
    let future = (Utc::now() + chrono::Duration::seconds(30)).to_rfc3339();
    sqlx::query(
        "UPDATE pool_upstream_account_model_routes SET state = ?1, cooldown_until = ?2 WHERE account_id = ?3 AND model = ?4",
    )
    .bind(MODEL_ROUTE_STATE_COOLING_DOWN)
    .bind(&future)
    .bind(account_id)
    .bind("")
    .execute(&state.pool)
    .await
    .expect("mark empty model route cooling down");

    assert_eq!(
        earliest_model_route_cooldown_expiry(&state.pool, Some(""), &[account_id])
            .await
            .expect("load empty model cooldown expiry"),
        Some(future)
    );
    assert!(
        !model_route_requires_expired_cooldown_probe(&state.pool, account_id, Some(""))
            .await
            .expect("check future empty model cooldown")
    );
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
async fn routing_hot_cache_invalidation_rebuilds_once_with_a_new_generation() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    seed_pool_routing_api_key(&state, "pool-routing-invalidation-key").await;

    let (initial, initial_was_warm) = load_pool_routing_runtime_cache_with_status(state.as_ref())
        .await
        .expect("load seeded routing cache");
    assert!(initial_was_warm);

    invalidate_pool_routing_runtime_cache(state.as_ref()).await;
    assert!(
        state
            .pool_routing_runtime_cache
            .lock()
            .await
            .as_ref()
            .is_some_and(|cache| cache.invalidated),
        "the failure path must invalidate without rebuilding synchronously"
    );

    let (rebuilt, rebuilt_was_warm) = load_pool_routing_runtime_cache_with_status(state.as_ref())
        .await
        .expect("rebuild invalidated routing cache");
    assert!(!rebuilt_was_warm);
    assert!(rebuilt.generation > initial.generation);

    let (hot, _hot_was_warm) = load_pool_routing_runtime_cache_with_status(state.as_ref())
        .await
        .expect("reuse rebuilt routing cache");
    assert!(
        hot.generation >= rebuilt.generation,
        "test-only fixture writes may force an additional snapshot refresh"
    );
    assert!(!hot.invalidated);
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

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn model_mapping_save_wakes_a_waiting_no_candidate_request() {
    let state = test_state_with_openai_base_and_pool_no_available_wait(
        url::Url::parse("https://api.openai.com/").expect("valid upstream base url"),
        std::time::Duration::from_secs(2),
        std::time::Duration::from_millis(100),
    )
    .await;
    let account_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Mapping waiter account",
        "sk-mapping-waiter",
        None,
        Some("https://mapping-waiter.example.com/backend-api/codex"),
    )
    .await;
    sqlx::query(
        r#"
        UPDATE pool_upstream_accounts
        SET policy_available_models_json = '["ordinary-model"]',
            policy_available_models_mode = 'allowlist'
        WHERE id = ?1
        "#,
    )
    .bind(account_id)
    .execute(&state.pool)
    .await
    .expect("seed account model allowlist");
    refresh_pool_routing_runtime_cache(state.as_ref())
        .await
        .expect("refresh account model allowlist");

    let wait_started_rx = crate::proxy::register_pool_no_available_wait_hook(&state);
    let update_state = state.clone();
    let runtime_handle = tokio::runtime::Handle::current();
    let update_task = std::thread::spawn(move || {
        wait_started_rx
            .recv_timeout(std::time::Duration::from_secs(2))
            .expect("request should enter the no-candidate wait");
        runtime_handle.block_on(async move {
            let Json(_) = update_upstream_account_model_mappings(
                State(update_state),
                HeaderMap::new(),
                AxumPath(account_id),
                Json(UpdateModelMappingsRequest {
                    model_mappings: vec![mapping("client-*", "ordinary-model", true)],
                }),
            )
            .await
            .expect("save mapping should wake the waiting request");
        });
    });

    let started = std::time::Instant::now();
    let mut wait_deadline = None;
    let resolution = resolve_pool_account_for_request_with_wait(
        state.as_ref(),
        None,
        Some("client-fast"),
        &[],
        &std::collections::HashSet::new(),
        None,
        true,
        &mut wait_deadline,
        Some(std::time::Instant::now() + std::time::Duration::from_secs(1)),
    )
    .await
    .expect("waiting request should resolve");
    let elapsed = started.elapsed();
    update_task
        .join()
        .expect("mapping update thread should join");

    match resolution {
        PoolAccountResolutionWithWait::Resolution(PoolAccountResolution::Resolved(account)) => {
            assert_eq!(account.account_id, account_id);
        }
        other => panic!("mapped account should resolve after save, got {other:?}"),
    }
    assert!(
        elapsed < std::time::Duration::from_millis(800),
        "mapping save should wake the request before its deadline, elapsed={elapsed:?}"
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
    let mut tag_rule = test_tag_routing_rule();
    tag_rule.available_models = vec!["upstream-special".to_string()];
    let tag = insert_test_tag(&state.pool, "mapped-target-allowlist", &tag_rule)
        .await
        .expect("insert mapped target tag");
    sync_account_tag_links(&state.pool, account_id, &[tag.summary.id])
        .await
        .expect("attach mapped target tag");
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
    let effective_rule = load_effective_routing_rule_for_account(&state.pool, account_id)
        .await
        .expect("load constrained effective rule");
    assert_eq!(
        effective_rule.tag_available_models.as_deref(),
        Some(["upstream-special".to_string()].as_slice())
    );

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

    sqlx::query("UPDATE pool_upstream_accounts SET model_mappings_json = ?1 WHERE id = ?2")
        .bind(
            serde_json::to_string(&vec![mapping(
                "client-*",
                "upstream-not-allowed-by-tag",
                true,
            )])
            .expect("encode disallowed mapping"),
        )
        .bind(account_id)
        .execute(&state.pool)
        .await
        .expect("seed disallowed mapped target");
    refresh_pool_routing_runtime_cache(state.as_ref())
        .await
        .expect("refresh disallowed mapping cache");
    let effective_rule = load_effective_routing_rule_for_account(&state.pool, account_id)
        .await
        .expect("load disallowed effective rule");
    assert_eq!(
        effective_rule.tag_available_models.as_deref(),
        Some(["upstream-special".to_string()].as_slice())
    );
    assert_eq!(
        load_model_mapping_for_account(state.as_ref(), account_id, Some("client-fast"))
            .await
            .expect("load disallowed mapping")
            .expect("disallowed mapping should be cached")
            .target_model,
        "upstream-not-allowed-by-tag"
    );
    assert!(
        !account_accepts_requested_model_or_cached_mapping(
            state.as_ref(),
            account_id,
            Some("client-fast"),
            &effective_rule,
        )
        .await
        .expect("check disallowed mapping"),
        "tag model allowlist must reject the mapped target before candidate selection"
    );
    let binding = PromptCacheConversationBindingConstraint::Group("test-direct-group".to_string());
    let resolution = resolve_pool_account_for_request_with_binding_constraint_and_model(
        state.as_ref(),
        None,
        Some("client-fast"),
        &[],
        &std::collections::HashSet::new(),
        Some(&binding),
    )
    .await
    .expect("bound disallowed mapping resolution result");
    assert!(
        !matches!(resolution, PoolAccountResolution::Resolved(_)),
        "prompt-cache binding must not bypass a mapped target tag allowlist, got {resolution:?}"
    );

    sqlx::query(
        r#"
        UPDATE pool_upstream_accounts
        SET model_mappings_json = '[{"sourceModel":"client-*","targetModel":"upstream-special","enabled":true}]'
        WHERE id = ?1
        "#,
    )
    .bind(account_id)
    .execute(&state.pool)
    .await
    .expect("restore allowed mapped target");
    refresh_pool_routing_runtime_cache(state.as_ref())
        .await
        .expect("refresh allowed mapping cache");
    ensure_account_has_unsupported_model_tag(&state.pool, account_id, "upstream-special")
        .await
        .expect("deny mapped target");
    refresh_pool_routing_runtime_cache(state.as_ref())
        .await
        .expect("publish mapped target system deny");
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
    let resolution = resolve_pool_account_for_request_with_binding_constraint_and_model(
        state.as_ref(),
        None,
        Some("client-fast"),
        &[],
        &std::collections::HashSet::new(),
        Some(&binding),
    )
    .await
    .expect("bound denied mapping resolution result");
    assert!(
        !matches!(resolution, PoolAccountResolution::Resolved(_)),
        "prompt-cache binding must not bypass a mapped target system deny, got {resolution:?}"
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

#[tokio::test]
async fn model_mapping_cache_override_warms_mapping_only_account() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let mapped_account_id = insert_api_key_account(&state.pool, "Mapped warm account").await;
    let source_account_id = insert_api_key_account(&state.pool, "Source warm account").await;
    for (account_id, models) in [
        (mapped_account_id, r#"["different-model"]"#),
        (source_account_id, r#"["finite-0"]"#),
    ] {
        sqlx::query(
            r#"
            UPDATE pool_upstream_accounts
            SET policy_available_models_json = ?2,
                policy_available_models_mode = 'allowlist'
            WHERE id = ?1
            "#,
        )
        .bind(account_id)
        .bind(models)
        .execute(&state.pool)
        .await
        .expect("seed warm account policy");
    }

    let mappings = vec![mapping("finite-0", "mapped-target", true)];
    let cache = build_pool_model_routing_runtime_cache_with_mapping_override(
        &state.pool,
        Some((mapped_account_id, &mappings)),
    )
    .await
    .expect("build mapping override cache");

    assert!(
        cache
            .warmed_model_account_ids
            .get("finite-0")
            .is_some_and(|account_ids| account_ids.contains(&mapped_account_id)),
        "mapping-only account should be present in the warmed source-model index"
    );
    assert!(
        !cache
            .available_models
            .iter()
            .any(|model| model == "mapped-target")
    );
}
