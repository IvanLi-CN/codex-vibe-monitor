#[tokio::test]
pub(crate) async fn cache_usage_missing_does_not_claim_an_observation_only_route() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let account_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Cache Observation Only",
        "cache-observation-only-key",
        None,
        Some("https://cache-observation-only.example.com/backend-api/codex"),
    )
    .await;
    let model = "gpt-cache-observation-only";
    enable_cache_hit_protection(&state).await;
    observe_model_route_seen(&state.pool, account_id, Some(model))
        .await
        .expect("seed observation-only route");
    observe_model_route_cache_hit(
        &state.pool,
        account_id,
        Some(model),
        Some(3_840),
        Some(384),
        1,
    )
    .await
    .expect("record a healthy cache observation");

    observe_model_route_cache_hit(&state.pool, account_id, Some(model), None, None, 1)
        .await
        .expect("ignore missing usage for observation-only route");

    let route = load_model_routing_states(&state.pool, account_id)
        .await
        .expect("load observation-only route")
        .into_iter()
        .find(|route| route.model == model)
        .expect("observation-only route is visible");
    assert_eq!(route.state, MODEL_ROUTE_STATE_AVAILABLE);
    assert_eq!(route.priority, MODEL_ROUTE_PRIORITY_NORMAL);
    assert_eq!(route.cache_last_hit_rate_percent, Some(10));
    assert_eq!(route.cache_concurrency_limit, None);
    assert!(route.cache_usage_missing_since.is_none());
    assert!(route.cache_usage_missing_reason.is_none());
    let missing_event_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM pool_upstream_account_events WHERE account_id = ?1 AND model = ?2 AND action = ?3",
    )
    .bind(account_id)
    .bind(model)
    .bind(UPSTREAM_ACCOUNT_ACTION_MODEL_ROUTE_CACHE_OBSERVATION_MISSING)
    .fetch_one(&state.pool)
    .await
    .expect("count observation-only missing usage events");
    assert_eq!(missing_event_count, 0);
}

async fn assert_cache_protection_disabled_cleanup(
    state: &AppState,
    account_id: i64,
    cache_model: &str,
    missing_only_model: &str,
    failure_model: &str,
) {
    let cache_route = cache_hit_route_state(state, account_id, cache_model).await;
    assert_eq!(
        (
            cache_route.0,
            cache_route.1,
            cache_route.2,
            cache_route.3,
            cache_route.4,
            cache_route.5
        ),
        (
            MODEL_ROUTE_STATE_AVAILABLE.to_string(),
            None,
            None,
            0_i64,
            0_i64,
            None,
        )
    );
    let routes = load_model_routing_states(&state.pool, account_id)
        .await
        .expect("load cache routes after disabling protection");
    let cache = routes
        .iter()
        .find(|route| route.model == cache_model)
        .expect("cache route is visible");
    assert!(
        cache.cache_usage_missing_since.is_none() && cache.cache_usage_missing_reason.is_none()
    );
    let missing = routes
        .iter()
        .find(|route| route.model == missing_only_model)
        .expect("missing-only route remains visible");
    assert_eq!(missing.state, MODEL_ROUTE_STATE_AVAILABLE);
    assert_eq!(missing.priority, MODEL_ROUTE_PRIORITY_NORMAL);
    assert_eq!(missing.cache_concurrency_limit, None);
    assert_eq!(missing.cache_recovery_limit, None);
    assert!(
        missing.cache_usage_missing_since.is_none() && missing.cache_usage_missing_reason.is_none()
    );
    let non_cache = sqlx::query_as::<_, (String, String, Option<String>)>("SELECT state, priority, cooldown_until FROM pool_upstream_account_model_routes WHERE account_id = ?1 AND model = ?2").bind(account_id).bind(failure_model).fetch_one(&state.pool).await.expect("load non-cache cooldown after cache settings update");
    assert_eq!(non_cache.0, MODEL_ROUTE_STATE_COOLING_DOWN);
    assert_eq!(non_cache.1, MODEL_ROUTE_PRIORITY_EXCLUDED);
    assert!(non_cache.2.is_some());
    let event = sqlx::query_as::<_, (String, String, String)>("SELECT action, reason_code, model FROM pool_upstream_account_events WHERE account_id = ?1 AND model = ?2 ORDER BY id DESC LIMIT 1").bind(account_id).bind(cache_model).fetch_one(&state.pool).await.expect("load cache settings cleanup event");
    assert_eq!(
        (event.0.as_str(), event.1.as_str(), event.2.as_str()),
        (
            UPSTREAM_ACCOUNT_ACTION_MODEL_ROUTE_RESET,
            "cache_hit_protection_disabled",
            cache_model
        )
    );
}

#[tokio::test]
pub(crate) async fn disabling_cache_hit_protection_clears_only_cache_owned_route_state() {
    disabling_cache_hit_protection_clears_only_cache_owned_route_state_impl().await;
}

async fn disabling_cache_hit_protection_clears_only_cache_owned_route_state_impl() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let account_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Cache Settings Cleanup",
        "cache-settings-cleanup-key",
        None,
        Some("https://cache-settings-cleanup.example.com/backend-api/codex"),
    )
    .await;
    let cache_model = "gpt-cache-settings-cleanup";
    let missing_only_model = "gpt-cache-settings-missing-only";
    let failure_model = "gpt-cache-settings-non-cache-failure";
    enable_cache_hit_protection(&state).await;
    observe_model_route_seen(&state.pool, account_id, Some(cache_model))
        .await
        .expect("seed cache model route");
    observe_model_route_cache_hit(
        &state.pool,
        account_id,
        Some(cache_model),
        Some(3_840),
        Some(0),
        2,
    )
    .await
    .expect("create cache-owned protection state");
    observe_model_route_cache_hit(&state.pool, account_id, Some(cache_model), None, None, 1)
        .await
        .expect("mark cache-owned route usage unavailable");
    observe_model_route_seen(&state.pool, account_id, Some(missing_only_model))
        .await
        .expect("seed missing-only cache model route");
    sqlx::query(
        "UPDATE pool_upstream_account_model_routes SET cache_concurrency_limit = 4, cache_recovery_limit = 8 WHERE account_id = ?1 AND model = ?2",
    )
    .bind(account_id)
    .bind(missing_only_model)
    .execute(&state.pool)
    .await
    .expect("seed cache limit without a cache failure");
    observe_model_route_cache_hit(
        &state.pool,
        account_id,
        Some(missing_only_model),
        None,
        None,
        1,
    )
    .await
    .expect("mark missing-only cache route usage unavailable");
    assert_missing_only_route_before_disable(&state, account_id, missing_only_model).await;
    observe_model_route_seen(&state.pool, account_id, Some(failure_model))
        .await
        .expect("seed independent failed model route");
    sqlx::query(
        "UPDATE pool_upstream_account_model_routes SET state = 'cooling_down', priority = 'excluded', last_failure_kind = 'upstream_transport_error', last_failure_message = 'transport failure', cooldown_until = ?3 WHERE account_id = ?1 AND model = ?2",
    )
    .bind(account_id)
    .bind(failure_model)
    .bind((Utc::now() + chrono::Duration::seconds(30)).to_rfc3339())
    .execute(&state.pool)
    .await
    .expect("seed non-cache cooldown");

    let Json(updated) = update_pool_routing_settings(
        State(state.clone()),
        HeaderMap::new(),
        Json(UpdatePoolRoutingSettingsRequest {
            api_key: None,
            maintenance: None,
            request_compression_algorithm: None,
            request_compression_level_preset: None,
            codex_imagegen_rewrite_mode: None,
            available_models: None,
            available_models_mode: None,
            timeouts: None,
            cache_hit_protection: Some(UpdateCacheHitProtectionSettingsRequest {
                enabled: Some(false),
                low_hit_rate_threshold_percent: None,
                overflow_mode: None,
            }),
            priority_handoff_admission_enabled: None,
        }),
    )
    .await
    .expect("disable cache-hit protection");
    assert!(!updated.cache_hit_protection.enabled);
    assert_cache_protection_disabled_cleanup(
        &state,
        account_id,
        cache_model,
        missing_only_model,
        failure_model,
    )
    .await;
}

async fn assert_missing_only_route_before_disable(state: &AppState, account_id: i64, model: &str) {
    let route = load_model_routing_states(&state.pool, account_id)
        .await
        .expect("load missing-only route before disabling protection")
        .into_iter()
        .find(|route| route.model == model)
        .expect("missing-only route is visible");
    assert_eq!(route.state, MODEL_ROUTE_STATE_DEGRADED);
    assert_eq!(route.priority, MODEL_ROUTE_PRIORITY_DEMOTED);
    assert!(route.last_failure_kind.is_none());
    assert!(route.cache_usage_missing_since.is_some());
}

#[tokio::test]
pub(crate) async fn cache_hit_protection_concurrency_halves_then_recovers() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let account_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Cache Concurrency",
        "cache-concurrency-key",
        None,
        Some("https://cache-concurrency.example.com/backend-api/codex"),
    )
    .await;
    let model = "gpt-cache-concurrency";
    enable_cache_hit_protection(&state).await;
    observe_model_route_seen(&state.pool, account_id, Some(model))
        .await
        .expect("seed model route");

    for expected_limit in [4, 2, 1] {
        observe_model_route_cache_hit(
            &state.pool,
            account_id,
            Some(model),
            Some(3_840),
            Some(0),
            8,
        )
        .await
        .expect("apply low cache-hit protection");
        assert_eq!(
            cache_hit_route_state(&state, account_id, model).await.1,
            Some(expected_limit)
        );
    }
    assert_eq!(cache_hit_route_state(&state, account_id, model).await.3, 1);

    for expected_limit in [2, 3, 4, 5, 6, 7] {
        observe_model_route_cache_hit(
            &state.pool,
            account_id,
            Some(model),
            Some(3_840),
            Some(3_840),
            8,
        )
        .await
        .expect("apply healthy cache-hit observation");
        assert_eq!(
            cache_hit_route_state(&state, account_id, model).await.1,
            Some(expected_limit)
        );
    }
    observe_model_route_cache_hit(
        &state.pool,
        account_id,
        Some(model),
        Some(3_840),
        Some(3_840),
        8,
    )
    .await
    .expect("fully recover cache-hit route");
    let recovered = cache_hit_route_state(&state, account_id, model).await;
    assert_eq!(recovered.0, MODEL_ROUTE_STATE_AVAILABLE);
    assert_eq!(recovered.1, None);
    assert_eq!(recovered.2, None);
}

#[tokio::test]
pub(crate) async fn cache_hit_protection_serializes_concurrent_observations() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let account_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Cache Serialized Observations",
        "cache-serialized-observations-key",
        None,
        Some("https://cache-serialized-observations.example.com/backend-api/codex"),
    )
    .await;
    let model = "gpt-cache-serialized-observations";
    enable_cache_hit_protection(&state).await;
    observe_model_route_seen(&state.pool, account_id, Some(model))
        .await
        .expect("seed model route");

    let first = observe_model_route_cache_hit(
        &state.pool,
        account_id,
        Some(model),
        Some(3_840),
        Some(0),
        8,
    );
    let second = observe_model_route_cache_hit(
        &state.pool,
        account_id,
        Some(model),
        Some(3_840),
        Some(0),
        8,
    );
    let (first, second) = tokio::join!(first, second);
    first.expect("first low-hit observation should persist");
    second.expect("second low-hit observation should persist");

    let route = cache_hit_route_state(&state, account_id, model).await;
    assert_eq!(route.1, Some(2));
    assert_eq!(route.2, Some(8));
}

#[tokio::test]
pub(crate) async fn cache_hit_protection_atomically_reserves_single_model_slot() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let account_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Cache Reservation",
        "cache-reservation-key",
        None,
        Some("https://cache-reservation.example.com/backend-api/codex"),
    )
    .await;
    let model = "gpt-cache-reservation";
    enable_cache_hit_protection(&state).await;
    observe_model_route_seen(&state.pool, account_id, Some(model))
        .await
        .expect("seed model route");
    observe_model_route_cache_hit(
        &state.pool,
        account_id,
        Some(model),
        Some(3_840),
        Some(0),
        2,
    )
    .await
    .expect("limit the combination to one request");
    assert_eq!(
        model_route_concurrency_limit(&state.pool, account_id, Some(model))
            .await
            .expect("load model concurrency limit"),
        Some(1)
    );
    sqlx::query(
        "UPDATE pool_routing_settings SET cache_hit_overflow_mode = 'reroute' WHERE id = 1",
    )
    .execute(&state.pool)
    .await
    .expect("switch cache-hit overflow mode to reroute");
    refresh_pool_routing_runtime_cache(state.as_ref())
        .await
        .expect("publish cache-hit overflow mode");

    let excluded_ids = Vec::new();
    let excluded_routes = HashSet::new();
    let first = resolve_pool_account_for_request_with_route_requirement_and_image_intent_and_override_and_codex_imagegen_request_and_reservation(
        &state,
        None,
        Some(model),
        &excluded_ids,
        &excluded_routes,
        None,
        None,
        None,
        "/v1/responses",
        crate::ImageIntent::Unknown,
        false,
        Some("cache-hit-reservation-a"),
    );
    let second = resolve_pool_account_for_request_with_route_requirement_and_image_intent_and_override_and_codex_imagegen_request_and_reservation(
        &state,
        None,
        Some(model),
        &excluded_ids,
        &excluded_routes,
        None,
        None,
        None,
        "/v1/responses",
        crate::ImageIntent::Unknown,
        false,
        Some("cache-hit-reservation-b"),
    );
    let (first, second) = tokio::join!(first, second);
    let resolutions = [
        first.expect("first selection should complete"),
        second.expect("second selection should complete"),
    ];
    assert_cache_reservation_outcome(&resolutions);

    release_pool_routing_reservation(&state, "cache-hit-reservation-a");
    release_pool_routing_reservation(&state, "cache-hit-reservation-b");
}

fn assert_cache_reservation_outcome(resolutions: &[PoolAccountResolution]) {
    assert_eq!(
        resolutions
            .iter()
            .filter(|r| matches!(r, PoolAccountResolution::Resolved(_)))
            .count(),
        1
    );
    assert_eq!(
        resolutions
            .iter()
            .filter(|r| matches!(r, PoolAccountResolution::NoCandidate(_)))
            .count(),
        1
    );
    let audit = resolutions
        .iter()
        .find_map(|resolution| match resolution {
            PoolAccountResolution::NoCandidate(audit) => Some(audit),
            _ => None,
        })
        .expect("capacity conflict should retain a no-candidate audit");
    assert_eq!(audit.terminal_reason_code, "modelConcurrencyLimit");
    assert_eq!(audit.candidate_count, 1);
    assert_eq!(audit.eligible_candidate_count, 1);
    assert_eq!(audit.reservation_conflict_count, 1);
    assert_eq!(audit.candidates[0].reason_code, "modelConcurrencyLimit");
}

#[tokio::test]
pub(crate) async fn cache_hit_protection_reserves_sticky_fast_path_before_returning_it() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let account_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Cache Sticky Reservation",
        "cache-sticky-reservation-key",
        None,
        Some("https://cache-sticky-reservation.example.com/backend-api/codex"),
    )
    .await;
    let model = "gpt-cache-sticky-reservation";
    let sticky_key = "cache-hit-sticky-reservation";
    enable_cache_hit_protection(&state).await;
    observe_model_route_seen(&state.pool, account_id, Some(model))
        .await
        .expect("seed model route");
    sqlx::query(
        "UPDATE pool_upstream_account_model_routes SET cache_concurrency_limit = 1, cache_recovery_limit = 2 WHERE account_id = ?1 AND model = ?2",
    )
    .bind(account_id)
    .bind(model)
    .execute(&state.pool)
    .await
    .expect("seed sticky model concurrency limit");
    upsert_sticky_route(
        &state.pool,
        sticky_key,
        account_id,
        &format_utc_iso(Utc::now()),
    )
    .await
    .expect("seed sticky route");

    let excluded_ids = Vec::new();
    let excluded_routes = HashSet::new();
    let first = resolve_pool_account_for_request_with_route_requirement_and_image_intent_and_override_and_codex_imagegen_request_and_reservation(
        &state,
        Some(sticky_key),
        Some(model),
        &excluded_ids,
        &excluded_routes,
        None,
        None,
        None,
        "/v1/responses",
        crate::ImageIntent::Unknown,
        false,
        Some("cache-hit-sticky-reservation-a"),
    );
    let second = resolve_pool_account_for_request_with_route_requirement_and_image_intent_and_override_and_codex_imagegen_request_and_reservation(
        &state,
        Some(sticky_key),
        Some(model),
        &excluded_ids,
        &excluded_routes,
        None,
        None,
        None,
        "/v1/responses",
        crate::ImageIntent::Unknown,
        false,
        Some("cache-hit-sticky-reservation-b"),
    );
    let (first, second) = tokio::join!(first, second);
    let resolutions = [
        first.expect("first sticky selection should complete"),
        second.expect("second sticky selection should complete"),
    ];
    assert_eq!(
        resolutions
            .iter()
            .filter(|resolution| matches!(resolution, PoolAccountResolution::Resolved(_)))
            .count(),
        1
    );
    assert_eq!(
        resolutions
            .iter()
            .filter(|resolution| matches!(resolution, PoolAccountResolution::NoCandidate(_)))
            .count(),
        1
    );

    release_pool_routing_reservation(&state, "cache-hit-sticky-reservation-a");
    release_pool_routing_reservation(&state, "cache-hit-sticky-reservation-b");
}

#[tokio::test]
pub(crate) async fn cache_hit_protection_reroutes_a_capped_normal_sticky_route() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let sticky_account_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Cache Sticky Reroute Source",
        "cache-sticky-reroute-source-key",
        None,
        Some("https://cache-sticky-reroute-source.example.com/backend-api/codex"),
    )
    .await;
    let alternative_account_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Cache Sticky Reroute Alternative",
        "cache-sticky-reroute-alternative-key",
        None,
        Some("https://cache-sticky-reroute-alternative.example.com/backend-api/codex"),
    )
    .await;
    let model = "gpt-cache-sticky-reroute";
    let sticky_key = "cache-hit-sticky-reroute";
    enable_cache_hit_protection(&state).await;
    sqlx::query(
        "UPDATE pool_routing_settings SET cache_hit_overflow_mode = 'reroute' WHERE id = 1",
    )
    .execute(&state.pool)
    .await
    .expect("enable cache-hit reroute mode");
    observe_model_route_seen(&state.pool, sticky_account_id, Some(model))
        .await
        .expect("seed sticky model route");
    sqlx::query(
        "UPDATE pool_upstream_account_model_routes SET cache_concurrency_limit = 1, cache_recovery_limit = 2 WHERE account_id = ?1 AND model = ?2",
    )
    .bind(sticky_account_id)
    .bind(model)
    .execute(&state.pool)
    .await
    .expect("seed sticky model concurrency limit");
    upsert_sticky_route(
        &state.pool,
        sticky_key,
        sticky_account_id,
        &format_utc_iso(Utc::now()),
    )
    .await
    .expect("seed reroutable sticky route");

    let excluded_ids = Vec::new();
    let excluded_routes = HashSet::new();
    let first = resolve_pool_account_for_request_with_route_requirement_and_image_intent_and_override_and_codex_imagegen_request_and_reservation(
        &state,
        Some(sticky_key),
        Some(model),
        &excluded_ids,
        &excluded_routes,
        None,
        None,
        None,
        "/v1/responses",
        crate::ImageIntent::Unknown,
        false,
        Some("cache-hit-sticky-reroute-a"),
    )
    .await
    .expect("reserve normal sticky route");
    let PoolAccountResolution::Resolved(first) = first else {
        panic!("expected sticky route to be selected before its cap is full");
    };
    assert_eq!(first.account_id, sticky_account_id);

    let second = resolve_pool_account_for_request_with_route_requirement_and_image_intent_and_override_and_codex_imagegen_request_and_reservation(
        &state,
        Some(sticky_key),
        Some(model),
        &excluded_ids,
        &excluded_routes,
        None,
        None,
        None,
        "/v1/responses",
        crate::ImageIntent::Unknown,
        false,
        Some("cache-hit-sticky-reroute-b"),
    )
    .await
    .expect("reroute after sticky model cap is full");
    let PoolAccountResolution::Resolved(second) = second else {
        panic!("expected a legal alternative after sticky model cap is full");
    };
    assert_eq!(second.account_id, alternative_account_id);

    release_pool_routing_reservation(&state, "cache-hit-sticky-reroute-a");
    release_pool_routing_reservation(&state, "cache-hit-sticky-reroute-b");
}

#[tokio::test]
pub(crate) async fn model_route_reservation_keeps_resolved_model_when_retry_context_lacks_one() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let account_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Cache Retry Reservation",
        "cache-retry-reservation-key",
        None,
        Some("https://cache-retry-reservation.example.com/backend-api/codex"),
    )
    .await;
    let resolution = resolve_pool_account_for_request(&state, None, &[], &HashSet::new())
        .await
        .expect("resolve account for reservation");
    let PoolAccountResolution::Resolved(account) = resolution else {
        panic!("expected a pool account for reservation");
    };
    assert_eq!(account.account_id, account_id);

    reserve_pool_routing_account_for_model(
        &state,
        "cache-retry-reservation",
        &account,
        Some("gpt-cache-retry-reservation"),
    );
    reserve_pool_routing_account_for_model(&state, "cache-retry-reservation", &account, None);
    assert!(pool_routing_reservation_matches_model(
        &state,
        "cache-retry-reservation",
        account_id,
        Some("gpt-cache-retry-reservation"),
    ));

    release_pool_routing_reservation(&state, "cache-retry-reservation");
}

#[tokio::test]
pub(crate) async fn model_route_reservation_preserves_an_explicit_empty_model() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let account_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Empty Model Reservation",
        "empty-model-reservation-key",
        None,
        Some("https://empty-model-reservation.example.com/backend-api/codex"),
    )
    .await;
    let resolution = resolve_pool_account_for_request(&state, None, &[], &HashSet::new())
        .await
        .expect("resolve account for empty model reservation");
    let PoolAccountResolution::Resolved(account) = resolution else {
        panic!("expected a pool account for empty model reservation");
    };
    assert_eq!(account.account_id, account_id);

    reserve_pool_routing_account_for_model(&state, "empty-model-reservation", &account, Some(""));
    assert_eq!(
        pool_routing_model_reservation_count(&state, account_id, Some("")),
        1
    );
    assert!(pool_routing_reservation_matches_model(
        &state,
        "empty-model-reservation",
        account_id,
        Some(""),
    ));

    release_pool_routing_reservation(&state, "empty-model-reservation");
}

#[test]
pub(crate) fn websocket_terminal_reservation_key_reuses_the_active_pool_route_key() {
    assert_eq!(
        pool_routing_reservation_key_for_invoke_id("pool-ws-42-turn-3").as_deref(),
        Some("pool-route-42")
    );
}

#[tokio::test]
pub(crate) async fn observing_a_stale_model_route_clears_cache_hit_protection_state() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let account_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Cache Stale Route",
        "cache-stale-route-key",
        None,
        Some("https://cache-stale-route.example.com/backend-api/codex"),
    )
    .await;
    let model = "gpt-cache-stale-route";
    let stale_at = format_utc_iso(Utc::now() - chrono::Duration::days(8));
    sqlx::query(
        "INSERT INTO pool_upstream_account_model_routes (account_id, model, state, priority, consecutive_failures, changed_at, last_seen_at, cache_concurrency_limit, cache_recovery_limit, cache_low_hit_streak, cache_cooldown_level, cache_last_hit_rate_percent) VALUES (?1, ?2, 'degraded', 'demoted', 2, ?3, ?3, 1, 8, 2, 3, 0)",
    )
    .bind(account_id)
    .bind(model)
    .bind(&stale_at)
    .execute(&state.pool)
    .await
    .expect("seed stale cache-hit route state");

    observe_model_route_seen(&state.pool, account_id, Some(model))
        .await
        .expect("observe stale model route");
    let route = cache_hit_route_state(&state, account_id, model).await;
    assert_eq!(route.0, MODEL_ROUTE_STATE_AVAILABLE);
    assert_eq!(route.1, None);
    assert_eq!(route.2, None);
    assert_eq!(route.3, 0);
    assert_eq!(route.4, 0);
    assert_eq!(route.5, None);
}

#[tokio::test]
pub(crate) async fn cache_hit_protection_reroutes_to_another_legal_combination() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let limited_account_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Cache Reroute Limited",
        "cache-reroute-limited-key",
        None,
        Some("https://cache-reroute-limited.example.com/backend-api/codex"),
    )
    .await;
    let model = "gpt-cache-reroute";
    enable_cache_hit_protection(&state).await;
    observe_model_route_seen(&state.pool, limited_account_id, Some(model))
        .await
        .expect("seed limited model route");
    observe_model_route_cache_hit(
        &state.pool,
        limited_account_id,
        Some(model),
        Some(3_840),
        Some(0),
        2,
    )
    .await
    .expect("limit first account to one request");

    let excluded_ids = Vec::new();
    let excluded_routes = HashSet::new();
    let busy = resolve_pool_account_for_request_with_route_requirement_and_image_intent_and_override_and_codex_imagegen_request_and_reservation(
        &state,
        None,
        Some(model),
        &excluded_ids,
        &excluded_routes,
        None,
        None,
        None,
        "/v1/responses",
        crate::ImageIntent::Unknown,
        false,
        Some("cache-reroute-busy"),
    )
    .await
    .expect("reserve the limited account");
    let PoolAccountResolution::Resolved(busy) = busy else {
        panic!("limited account should be selected while it has capacity");
    };
    assert_eq!(busy.account_id, limited_account_id);

    let alternative_account_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Cache Reroute Alternative",
        "cache-reroute-alternative-key",
        None,
        Some("https://cache-reroute-alternative.example.com/backend-api/codex"),
    )
    .await;
    sqlx::query(
        "UPDATE pool_routing_settings SET cache_hit_overflow_mode = 'reroute' WHERE id = 1",
    )
    .execute(&state.pool)
    .await
    .expect("switch cache-hit overflow mode to reroute");

    let rerouted = resolve_pool_account_for_request_with_route_requirement_and_image_intent_and_override_and_codex_imagegen_request_and_reservation(
        &state,
        None,
        Some(model),
        &excluded_ids,
        &excluded_routes,
        None,
        None,
        None,
        "/v1/responses",
        crate::ImageIntent::Unknown,
        false,
        Some("cache-reroute-alternative"),
    )
    .await
    .expect("reroute selection should complete");
    let PoolAccountResolution::Resolved(rerouted) = rerouted else {
        panic!("an alternate account should remain selectable");
    };
    assert_eq!(rerouted.account_id, alternative_account_id);

    release_pool_routing_reservation(&state, "cache-reroute-busy");
    release_pool_routing_reservation(&state, "cache-reroute-alternative");
}

#[tokio::test]
pub(crate) async fn model_route_single_probe_recovery_is_atomic_for_non_cache_cooldowns() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let account_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Single Probe Recovery",
        "single-probe-recovery-key",
        None,
        Some("https://single-probe-recovery.example.com/backend-api/codex"),
    )
    .await;
    let model = "gpt-single-probe-recovery";
    enable_cache_hit_protection(&state).await;
    observe_model_route_seen(&state.pool, account_id, Some(model))
        .await
        .expect("seed model route");
    sqlx::query(
        "UPDATE pool_upstream_account_model_routes SET state = 'cooling_down', priority = 'excluded', cooldown_until = ?3, last_failure_kind = 'http_5xx', last_failure_message = 'upstream unavailable' WHERE account_id = ?1 AND model = ?2",
    )
    .bind(account_id)
    .bind(model)
    .bind((Utc::now() - chrono::Duration::seconds(1)).to_rfc3339())
    .execute(&state.pool)
    .await
    .expect("expire non-cache model cooldown");
    assert_eq!(
        model_route_concurrency_limit(&state.pool, account_id, Some(model))
            .await
            .expect("load non-cache probe limit"),
        Some(1)
    );

    let excluded_ids = Vec::new();
    let excluded_routes = HashSet::new();
    let first = resolve_pool_account_for_request_with_route_requirement_and_image_intent_and_override_and_codex_imagegen_request_and_reservation(
        &state,
        None,
        Some(model),
        &excluded_ids,
        &excluded_routes,
        None,
        None,
        None,
        "/v1/responses",
        crate::ImageIntent::Unknown,
        false,
        Some("single-probe-reservation-a"),
    );
    let second = resolve_pool_account_for_request_with_route_requirement_and_image_intent_and_override_and_codex_imagegen_request_and_reservation(
        &state,
        None,
        Some(model),
        &excluded_ids,
        &excluded_routes,
        None,
        None,
        None,
        "/v1/responses",
        crate::ImageIntent::Unknown,
        false,
        Some("single-probe-reservation-b"),
    );
    let (first, second) = tokio::join!(first, second);
    let resolutions = [
        first.expect("first probe selection should complete"),
        second.expect("second probe selection should complete"),
    ];
    assert_eq!(
        resolutions
            .iter()
            .filter(|resolution| matches!(resolution, PoolAccountResolution::Resolved(_)))
            .count(),
        1
    );
    assert_eq!(
        resolutions
            .iter()
            .filter(|resolution| matches!(resolution, PoolAccountResolution::NoCandidate(_)))
            .count(),
        1
    );

    release_pool_routing_reservation(&state, "single-probe-reservation-a");
    release_pool_routing_reservation(&state, "single-probe-reservation-b");

    assert_single_probe_recovery(&state, account_id, model).await;
}

async fn assert_single_probe_recovery(state: &AppState, account_id: i64, model: &str) {
    let started_at = format_naive_precise(Utc::now().with_timezone(&Shanghai).naive_local());
    let attempt_id = sqlx::query(
        "INSERT INTO pool_upstream_request_attempts (invoke_id, occurred_at, endpoint, route_mode, request_model, upstream_account_id, upstream_route_key, attempt_index, distinct_account_index, same_account_retry_index, started_at, status) VALUES (?1, ?2, '/v1/responses', 'pool', ?3, ?4, 'route', 1, 1, 0, ?5, 'pending')",
    )
    .bind("non-cache-expired-cooldown-success")
    .bind(format_utc_iso(Utc::now()))
    .bind(model)
    .bind(account_id)
    .bind(&started_at)
    .execute(&state.pool)
    .await
    .expect("insert successful non-cache probe")
    .last_insert_rowid();
    record_model_route_success_from_attempt(&state.pool, account_id, attempt_id, Some(&started_at))
        .await
        .expect("recover successful non-cache probe without cache usage");
    let recovered = sqlx::query_as::<_, (String, Option<String>, Option<String>)>(
        "SELECT state, last_failure_kind, cooldown_until FROM pool_upstream_account_model_routes WHERE account_id = ?1 AND model = ?2",
    )
    .bind(account_id)
    .bind(model)
    .fetch_one(&state.pool)
    .await
    .expect("load recovered non-cache model route");
    assert_eq!(recovered.0, MODEL_ROUTE_STATE_AVAILABLE);
    assert_eq!(
        model_route_concurrency_limit(&state.pool, account_id, Some(model))
            .await
            .expect("load recovered model concurrency"),
        None
    );
    assert!(recovered.1.is_none() && recovered.2.is_none());
}

use super::*;
