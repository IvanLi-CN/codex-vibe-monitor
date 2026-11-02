use super::*;

async fn enable_cache_hit_protection(state: &AppState) {
    sqlx::query(
        "UPDATE pool_routing_settings SET cache_hit_protection_enabled = 1, cache_hit_low_rate_threshold_percent = 10, cache_hit_overflow_mode = 'queue' WHERE id = 1",
    )
    .execute(&state.pool)
    .await
    .expect("enable cache-hit protection");
    refresh_pool_routing_runtime_cache(state)
        .await
        .expect("publish cache-hit protection settings");
}

async fn cache_hit_route_state(
    state: &AppState,
    account_id: i64,
    model: &str,
) -> (
    String,
    Option<i64>,
    Option<i64>,
    i64,
    i64,
    Option<i64>,
    Option<String>,
) {
    sqlx::query_as(
        "SELECT state, cache_concurrency_limit, cache_recovery_limit, cache_low_hit_streak, cache_cooldown_level, cache_last_hit_rate_percent, cooldown_until FROM pool_upstream_account_model_routes WHERE account_id = ?1 AND model = ?2",
    )
    .bind(account_id)
    .bind(model)
    .fetch_one(&state.pool)
    .await
    .expect("load cache-hit route state")
}

#[tokio::test]
async fn cache_hit_protection_observation_respects_sample_boundary_and_threshold() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let account_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Cache Boundary",
        "cache-boundary-key",
        None,
        Some("https://cache-boundary.example.com/backend-api/codex"),
    )
    .await;
    let model = "gpt-cache-boundary";
    enable_cache_hit_protection(&state).await;
    observe_model_route_seen(&state.pool, account_id, Some(model))
        .await
        .expect("seed model route");

    observe_model_route_cache_hit(
        &state.pool,
        account_id,
        Some(model),
        Some(3_839),
        Some(0),
        8,
    )
    .await
    .expect("ignore undersized sample");
    let state_after_small = cache_hit_route_state(&state, account_id, model).await;
    assert_eq!(state_after_small.1, None);
    assert_eq!(state_after_small.5, None);

    observe_model_route_cache_hit(
        &state.pool,
        account_id,
        Some(model),
        Some(3_840),
        Some(384),
        8,
    )
    .await
    .expect("observe threshold-equal sample");
    let state_after_equal = cache_hit_route_state(&state, account_id, model).await;
    assert_eq!(state_after_equal.0, MODEL_ROUTE_STATE_AVAILABLE);
    assert_eq!(state_after_equal.1, None);
    assert_eq!(state_after_equal.5, Some(10));

    observe_model_route_cache_hit(
        &state.pool,
        account_id,
        Some(model),
        Some(3_840),
        Some(383),
        8,
    )
    .await
    .expect("observe low cache-hit sample");
    let state_after_low = cache_hit_route_state(&state, account_id, model).await;
    assert_eq!(state_after_low.0, MODEL_ROUTE_STATE_DEGRADED);
    assert_eq!(state_after_low.1, Some(4));
    assert_eq!(state_after_low.2, Some(8));
    assert_eq!(state_after_low.5, Some(9));
    let visible_route = load_model_routing_states(&state.pool, account_id)
        .await
        .expect("load visible cache-hit route")
        .into_iter()
        .find(|route| route.model == model)
        .expect("cache-hit route is visible");
    assert_eq!(visible_route.cache_concurrency_limit, Some(4));
    assert_eq!(visible_route.cache_recovery_limit, Some(8));
    assert_eq!(visible_route.cache_last_hit_rate_percent, Some(9));
    assert!(!visible_route.probe_required);

    observe_model_route_cache_hit(&state.pool, account_id, Some(model), None, None, 8)
        .await
        .expect("constrain cache-owned route with missing usage");
    let missing_usage_route = load_model_routing_states(&state.pool, account_id)
        .await
        .expect("load missing-usage route")
        .into_iter()
        .find(|route| route.model == model)
        .expect("missing-usage route is visible");
    assert_eq!(missing_usage_route.state, MODEL_ROUTE_STATE_DEGRADED);
    assert_eq!(missing_usage_route.cache_concurrency_limit, Some(1));
    assert!(missing_usage_route.cache_usage_missing_since.is_some());
    assert_eq!(
        missing_usage_route.cache_usage_missing_reason.as_deref(),
        Some("missing_input_tokens")
    );
    assert_eq!(
        model_route_concurrency_limit(&state.pool, account_id, Some(model))
            .await
            .expect("load constrained missing-usage limit"),
        Some(1)
    );

    observe_model_route_cache_hit(&state.pool, account_id, Some(model), None, None, 8)
        .await
        .expect("keep the same missing-usage episode constrained");
    let missing_event_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM pool_upstream_account_events WHERE account_id = ?1 AND action = ?2",
    )
    .bind(account_id)
    .bind(UPSTREAM_ACCOUNT_ACTION_MODEL_ROUTE_CACHE_OBSERVATION_MISSING)
    .fetch_one(&state.pool)
    .await
    .expect("count missing cache-usage events");
    assert_eq!(missing_event_count, 1);

    observe_model_route_cache_hit(
        &state.pool,
        account_id,
        Some(model),
        Some(3_840),
        Some(384),
        1,
    )
    .await
    .expect("clear missing-usage marker with a valid sample");
    let observed_route = load_model_routing_states(&state.pool, account_id)
        .await
        .expect("load route after valid cache observation")
        .into_iter()
        .find(|route| route.model == model)
        .expect("observed route is visible");
    assert!(observed_route.cache_usage_missing_since.is_none());
    assert!(observed_route.cache_usage_missing_reason.is_none());
    assert_eq!(observed_route.cache_concurrency_limit, Some(2));
}

#[tokio::test]
async fn cache_usage_missing_does_not_claim_an_observation_only_route() {
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

#[tokio::test]
async fn disabling_cache_hit_protection_clears_only_cache_owned_route_state() {
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
    let missing_only_before_disable = load_model_routing_states(&state.pool, account_id)
        .await
        .expect("load missing-only route before disabling protection")
        .into_iter()
        .find(|route| route.model == missing_only_model)
        .expect("missing-only route is visible");
    assert_eq!(
        missing_only_before_disable.state,
        MODEL_ROUTE_STATE_DEGRADED
    );
    assert_eq!(
        missing_only_before_disable.priority,
        MODEL_ROUTE_PRIORITY_DEMOTED
    );
    assert!(missing_only_before_disable.last_failure_kind.is_none());
    assert!(
        missing_only_before_disable
            .cache_usage_missing_since
            .is_some()
    );
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
            oauth_request_compression_algorithm: None,
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
            automatic_reserve_fallback_enabled: None,
            reserve_fallback_model_policy: None,
        }),
    )
    .await
    .expect("disable cache-hit protection");
    assert!(!updated.cache_hit_protection.enabled);
    let cache_route = cache_hit_route_state(&state, account_id, cache_model).await;
    assert_eq!(cache_route.0, MODEL_ROUTE_STATE_AVAILABLE);
    assert_eq!(cache_route.1, None);
    assert_eq!(cache_route.2, None);
    assert_eq!(cache_route.3, 0);
    assert_eq!(cache_route.4, 0);
    assert_eq!(cache_route.5, None);
    let cache_route = load_model_routing_states(&state.pool, account_id)
        .await
        .expect("load cache route after disabling protection")
        .into_iter()
        .find(|route| route.model == cache_model)
        .expect("cache route is visible");
    assert!(cache_route.cache_usage_missing_since.is_none());
    assert!(cache_route.cache_usage_missing_reason.is_none());
    let missing_only_route = load_model_routing_states(&state.pool, account_id)
        .await
        .expect("load missing-only route after disabling protection")
        .into_iter()
        .find(|route| route.model == missing_only_model)
        .expect("missing-only route remains visible");
    assert_eq!(missing_only_route.state, MODEL_ROUTE_STATE_AVAILABLE);
    assert_eq!(missing_only_route.priority, MODEL_ROUTE_PRIORITY_NORMAL);
    assert_eq!(missing_only_route.cache_concurrency_limit, None);
    assert_eq!(missing_only_route.cache_recovery_limit, None);
    assert!(missing_only_route.cache_usage_missing_since.is_none());
    assert!(missing_only_route.cache_usage_missing_reason.is_none());
    let non_cache_route = sqlx::query_as::<_, (String, String, Option<String>)>(
        "SELECT state, priority, cooldown_until FROM pool_upstream_account_model_routes WHERE account_id = ?1 AND model = ?2",
    )
    .bind(account_id)
    .bind(failure_model)
    .fetch_one(&state.pool)
    .await
    .expect("load non-cache cooldown after cache settings update");
    assert_eq!(non_cache_route.0, MODEL_ROUTE_STATE_COOLING_DOWN);
    assert_eq!(non_cache_route.1, MODEL_ROUTE_PRIORITY_EXCLUDED);
    assert!(non_cache_route.2.is_some());
    let cleanup_event = sqlx::query_as::<_, (String, String, String)>(
        "SELECT action, reason_code, model FROM pool_upstream_account_events WHERE account_id = ?1 AND model = ?2 ORDER BY id DESC LIMIT 1",
    )
    .bind(account_id)
    .bind(cache_model)
    .fetch_one(&state.pool)
    .await
    .expect("load cache settings cleanup event");
    assert_eq!(cleanup_event.0, UPSTREAM_ACCOUNT_ACTION_MODEL_ROUTE_RESET);
    assert_eq!(cleanup_event.1, "cache_hit_protection_disabled");
    assert_eq!(cleanup_event.2, cache_model);
}

#[tokio::test]
async fn cache_hit_protection_concurrency_halves_then_recovers() {
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
async fn cache_hit_protection_serializes_concurrent_observations() {
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
async fn cache_hit_protection_atomically_reserves_single_model_slot() {
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

    release_pool_routing_reservation(&state, "cache-hit-reservation-a");
    release_pool_routing_reservation(&state, "cache-hit-reservation-b");
}

#[tokio::test]
async fn cache_hit_protection_reserves_sticky_fast_path_before_returning_it() {
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
async fn cache_hit_protection_reroutes_a_capped_normal_sticky_route() {
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
async fn model_route_reservation_keeps_resolved_model_when_retry_context_lacks_one() {
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
async fn model_route_reservation_preserves_an_explicit_empty_model() {
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
fn terminal_reservation_key_reuses_the_active_pool_route_key() {
    assert_eq!(
        pool_routing_reservation_key_for_invoke_id("pool-ws-42-turn-3").as_deref(),
        Some("pool-route-42")
    );
}

#[tokio::test]
async fn observing_a_stale_model_route_clears_cache_hit_protection_state() {
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
async fn cache_hit_protection_reroutes_to_another_legal_combination() {
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
async fn model_route_single_probe_recovery_is_atomic_for_non_cache_cooldowns() {
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

    let success_started_at =
        format_naive_precise(Utc::now().with_timezone(&Shanghai).naive_local());
    let successful_attempt = sqlx::query(
        "INSERT INTO pool_upstream_request_attempts (invoke_id, occurred_at, endpoint, route_mode, request_model, upstream_account_id, upstream_route_key, attempt_index, distinct_account_index, same_account_retry_index, started_at, status) VALUES (?1, ?2, '/v1/responses', 'pool', ?3, ?4, 'route', 1, 1, 0, ?5, 'pending')",
    )
    .bind("non-cache-expired-cooldown-success")
    .bind(format_utc_iso(Utc::now()))
    .bind(model)
    .bind(account_id)
    .bind(&success_started_at)
    .execute(&state.pool)
    .await
    .expect("insert successful non-cache probe")
    .last_insert_rowid();
    record_model_route_success_from_attempt(
        &state.pool,
        account_id,
        successful_attempt,
        Some(&success_started_at),
    )
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
    assert!(recovered.1.is_none());
    assert!(recovered.2.is_none());
}

#[tokio::test]
async fn cache_hit_protection_cooldown_restarts_with_single_probe() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let account_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Cache Cooldown",
        "cache-cooldown-key",
        None,
        Some("https://cache-cooldown.example.com/backend-api/codex"),
    )
    .await;
    let model = "gpt-cache-cooldown";
    enable_cache_hit_protection(&state).await;
    observe_model_route_seen(&state.pool, account_id, Some(model))
        .await
        .expect("seed model route");

    for _ in 0..5 {
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
    }
    let first_cooldown = cache_hit_route_state(&state, account_id, model).await;
    assert_eq!(first_cooldown.0, MODEL_ROUTE_STATE_COOLING_DOWN);
    assert_eq!(first_cooldown.3, 0);
    assert_eq!(first_cooldown.4, 1);
    assert!(first_cooldown.6.is_some());

    sqlx::query(
        "UPDATE pool_upstream_account_model_routes SET cooldown_until = ?3 WHERE account_id = ?1 AND model = ?2",
    )
    .bind(account_id)
    .bind(model)
    .bind((Utc::now() - chrono::Duration::seconds(1)).to_rfc3339())
    .execute(&state.pool)
    .await
    .expect("expire cooldown");
    assert_eq!(
        model_route_concurrency_limit(&state.pool, account_id, Some(model))
            .await
            .expect("load expired cooldown probe limit"),
        Some(1)
    );
    assert!(
        load_model_routing_states(&state.pool, account_id)
            .await
            .expect("load expired cache-hit route")
            .into_iter()
            .find(|route| route.model == model)
            .expect("expired cache-hit route is visible")
            .probe_required
    );

    observe_model_route_cache_hit(&state.pool, account_id, Some(model), None, None, 1)
        .await
        .expect("unknown probe observation");
    let missing_usage_route = load_model_routing_states(&state.pool, account_id)
        .await
        .expect("load expired route with missing cache usage")
        .into_iter()
        .find(|route| route.model == model)
        .expect("missing-usage route is visible");
    assert_eq!(missing_usage_route.state, MODEL_ROUTE_STATE_DEGRADED);
    assert_eq!(missing_usage_route.cache_concurrency_limit, Some(1));
    assert!(missing_usage_route.cache_usage_missing_since.is_some());
    assert_eq!(
        missing_usage_route.cache_usage_missing_reason.as_deref(),
        Some("missing_input_tokens")
    );

    for expected_streak in [1, 2] {
        observe_model_route_cache_hit(
            &state.pool,
            account_id,
            Some(model),
            Some(3_840),
            Some(0),
            1,
        )
        .await
        .expect("apply low probe observation");
        let state_after_low = cache_hit_route_state(&state, account_id, model).await;
        assert_eq!(state_after_low.0, MODEL_ROUTE_STATE_DEGRADED);
        assert_eq!(state_after_low.1, Some(1));
        assert_eq!(state_after_low.3, expected_streak);
    }
    observe_model_route_cache_hit(
        &state.pool,
        account_id,
        Some(model),
        Some(3_840),
        Some(0),
        1,
    )
    .await
    .expect("re-enter cache cooldown");
    let second_cooldown = cache_hit_route_state(&state, account_id, model).await;
    assert_eq!(second_cooldown.0, MODEL_ROUTE_STATE_COOLING_DOWN);
    assert_eq!(second_cooldown.4, 2);
}

#[tokio::test]
async fn api_key_temporary_http_failure_changes_only_the_exact_model_route() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let account_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Temporary model scope",
        "temporary-model-scope-key",
        None,
        Some("https://temporary-model-scope.example.com/backend-api/codex"),
    )
    .await;
    upsert_sticky_route(
        &state.pool,
        "sticky-temporary-model-scope",
        account_id,
        &format_utc_iso(Utc::now()),
    )
    .await
    .expect("seed temporary model sticky route");
    let attempt_id = insert_model_failure_attempt(
        &state,
        account_id,
        "temporary-model-502",
        Some("gpt-5.6-terra"),
    )
    .await;

    record_pool_route_http_failure_for_endpoint_with_image_intent_for_attempt(
        &state.pool,
        account_id,
        UPSTREAM_ACCOUNT_KIND_API_KEY_CODEX,
        true,
        Some("sticky-temporary-model-scope"),
        StatusCode::BAD_GATEWAY,
        "pool upstream responded with 502: Upstream access forbidden, please contact administrator",
        Some("temporary-model-502"),
        "/v1/responses",
        ImageIntent::Unknown,
        Some(attempt_id),
        None,
    )
    .await
    .expect("record API key temporary HTTP failure");

    let account = load_upstream_account_row(&state.pool, account_id)
        .await
        .expect("load API key account")
        .expect("API key account exists");
    assert_eq!(account.status, UPSTREAM_ACCOUNT_STATUS_ACTIVE);
    assert!(account.last_route_failure_kind.is_none());
    assert!(account.cooldown_until.is_none());
    assert_eq!(account.consecutive_route_failures, 0);
    assert!(account.temporary_route_failure_streak_started_at.is_none());
    assert_eq!(
        load_sticky_route(&state.pool, "sticky-temporary-model-scope")
            .await
            .expect("load temporary model sticky route")
            .map(|route| route.account_id),
        Some(account_id),
    );

    let route = load_model_routing_states(&state.pool, account_id)
        .await
        .expect("load temporary model route")
        .into_iter()
        .find(|route| route.model == "gpt-5.6-terra")
        .expect("temporary model route exists");
    assert_eq!(route.state, MODEL_ROUTE_STATE_DEGRADED);
    assert_eq!(route.priority, MODEL_ROUTE_PRIORITY_DEMOTED);
    assert_eq!(route.failure_count, 1);
    assert_eq!(
        route.last_failure_kind.as_deref(),
        Some(FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_5XX),
    );

    let event = sqlx::query_as::<_, (String, Option<String>, Option<i64>, Option<String>, Option<String>)>(
        "SELECT action, reason_code, http_status, failure_kind, model FROM pool_upstream_account_events WHERE account_id = ?1 ORDER BY id DESC LIMIT 1",
    )
    .bind(account_id)
    .fetch_one(&state.pool)
    .await
    .expect("load temporary model event");
    assert_eq!(event.0, UPSTREAM_ACCOUNT_ACTION_MODEL_ROUTE_DEGRADED);
    assert_eq!(
        event.1.as_deref(),
        Some(UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_5XX),
    );
    assert_eq!(event.2, Some(502));
    assert_eq!(
        event.3.as_deref(),
        Some(FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_5XX)
    );
    assert_eq!(event.4.as_deref(), Some("gpt-5.6-terra"));
}

#[tokio::test]
async fn api_key_transport_failure_preserves_kind_and_changes_only_the_exact_model_route() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let account_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Temporary transport model scope",
        "temporary-transport-model-key",
        None,
        Some("https://temporary-transport-model.example.com/backend-api/codex"),
    )
    .await;
    let attempt_id = insert_model_failure_attempt(
        &state,
        account_id,
        "temporary-model-stream",
        Some("gpt-5.6-terra"),
    )
    .await;

    record_pool_route_transport_failure_for_attempt_with_kind(
        &state.pool,
        account_id,
        None,
        "upstream stream error before first chunk",
        PROXY_FAILURE_UPSTREAM_STREAM_ERROR,
        Some("temporary-model-stream"),
        Some(attempt_id),
    )
    .await
    .expect("record API key temporary transport failure");

    let account = load_upstream_account_row(&state.pool, account_id)
        .await
        .expect("load API key account")
        .expect("API key account exists");
    assert!(account.last_route_failure_kind.is_none());
    assert!(account.cooldown_until.is_none());
    assert_eq!(account.consecutive_route_failures, 0);

    let route = load_model_routing_states(&state.pool, account_id)
        .await
        .expect("load transport model route")
        .into_iter()
        .find(|route| route.model == "gpt-5.6-terra")
        .expect("transport model route exists");
    assert_eq!(route.state, MODEL_ROUTE_STATE_DEGRADED);
    assert_eq!(route.priority, MODEL_ROUTE_PRIORITY_DEMOTED);
    assert_eq!(
        route.last_failure_kind.as_deref(),
        Some(PROXY_FAILURE_UPSTREAM_STREAM_ERROR),
    );

    let event = sqlx::query_as::<_, (Option<i64>, Option<String>, Option<String>)>(
        "SELECT attempt_id, failure_kind, model FROM pool_upstream_account_events WHERE account_id = ?1 ORDER BY id DESC LIMIT 1",
    )
    .bind(account_id)
    .fetch_one(&state.pool)
    .await
    .expect("load temporary transport model event");
    assert_eq!(event.0, Some(attempt_id));
    assert_eq!(
        event.1.as_deref(),
        Some(PROXY_FAILURE_UPSTREAM_STREAM_ERROR)
    );
    assert_eq!(event.2.as_deref(), Some("gpt-5.6-terra"));
}

#[tokio::test]
async fn api_key_pre_attempt_transport_failure_uses_the_exact_request_model() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let account_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Pre-attempt transport model scope",
        "pre-attempt-transport-model-key",
        None,
        Some("https://pre-attempt-transport-model.example.com/backend-api/codex"),
    )
    .await;

    record_pool_route_transport_failure_for_model(
        &state.pool,
        account_id,
        None,
        "no selectable forward proxy node",
        Some("pre-attempt-transport-model"),
        Some("gpt-5.6-terra"),
    )
    .await
    .expect("record API key pre-attempt transport failure");

    let account = load_upstream_account_row(&state.pool, account_id)
        .await
        .expect("load API key account")
        .expect("API key account exists");
    assert!(account.last_route_failure_kind.is_none());
    assert!(account.cooldown_until.is_none());
    assert_eq!(account.consecutive_route_failures, 0);

    let route = load_model_routing_states(&state.pool, account_id)
        .await
        .expect("load pre-attempt transport model route")
        .into_iter()
        .find(|route| route.model == "gpt-5.6-terra")
        .expect("pre-attempt transport model route exists");
    assert_eq!(route.state, MODEL_ROUTE_STATE_DEGRADED);
    assert_eq!(route.priority, MODEL_ROUTE_PRIORITY_DEMOTED);
    assert_eq!(route.failure_count, 1);
    assert_eq!(
        route.last_failure_kind.as_deref(),
        Some(PROXY_FAILURE_FAILED_CONTACT_UPSTREAM),
    );

    let event = sqlx::query_as::<_, (Option<i64>, Option<String>, Option<String>, Option<String>)>(
        "SELECT attempt_id, reason_code, failure_kind, model FROM pool_upstream_account_events WHERE account_id = ?1 ORDER BY id DESC LIMIT 1",
    )
    .bind(account_id)
    .fetch_one(&state.pool)
    .await
    .expect("load pre-attempt transport model event");
    assert_eq!(event.0, None);
    assert_eq!(
        event.1.as_deref(),
        Some(UPSTREAM_ACCOUNT_ACTION_REASON_TRANSPORT_FAILURE),
    );
    assert_eq!(
        event.2.as_deref(),
        Some(PROXY_FAILURE_FAILED_CONTACT_UPSTREAM),
    );
    assert_eq!(event.3.as_deref(), Some("gpt-5.6-terra"));
}

#[tokio::test]
async fn api_key_explicit_model_429_honors_toggle_and_preserves_reason() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let account_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Explicit model 429",
        "explicit-model-429-key",
        None,
        Some("https://explicit-model-429.example.com/backend-api/codex"),
    )
    .await;
    let enabled_attempt = insert_model_failure_attempt(
        &state,
        account_id,
        "explicit-model-429-enabled",
        Some("gpt-rate-limited"),
    )
    .await;

    record_pool_route_http_failure_for_endpoint_with_image_intent_for_attempt(
        &state.pool,
        account_id,
        UPSTREAM_ACCOUNT_KIND_API_KEY_CODEX,
        false,
        None,
        StatusCode::TOO_MANY_REQUESTS,
        "rate limit reached for gpt-rate-limited",
        Some("explicit-model-429-enabled"),
        "/v1/responses",
        ImageIntent::Unknown,
        Some(enabled_attempt),
        None,
    )
    .await
    .expect("record enabled explicit model 429");

    let enabled_event = sqlx::query_as::<_, (Option<String>, Option<i64>, Option<String>)>(
        "SELECT reason_code, http_status, model FROM pool_upstream_account_events WHERE account_id = ?1 ORDER BY id DESC LIMIT 1",
    )
    .bind(account_id)
    .fetch_one(&state.pool)
    .await
    .expect("load enabled explicit model 429 event");
    assert_eq!(
        enabled_event.0.as_deref(),
        Some(UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_429_RATE_LIMIT),
    );
    assert_eq!(enabled_event.1, Some(429));
    assert_eq!(enabled_event.2.as_deref(), Some("gpt-rate-limited"));

    sqlx::query(
        "UPDATE pool_upstream_accounts SET policy_status_change_upstream_http_429_rate_limit = 0 WHERE id = ?1",
    )
    .bind(account_id)
    .execute(&state.pool)
    .await
    .expect("disable model 429 status changes");
    let disabled_attempt = insert_model_failure_attempt(
        &state,
        account_id,
        "explicit-model-429-disabled",
        Some("gpt-rate-disabled"),
    )
    .await;
    record_pool_route_http_failure_for_endpoint_with_image_intent_for_attempt(
        &state.pool,
        account_id,
        UPSTREAM_ACCOUNT_KIND_API_KEY_CODEX,
        false,
        None,
        StatusCode::TOO_MANY_REQUESTS,
        "rate limit reached for gpt-rate-disabled",
        Some("explicit-model-429-disabled"),
        "/v1/responses",
        ImageIntent::Unknown,
        Some(disabled_attempt),
        None,
    )
    .await
    .expect("record disabled explicit model 429");

    assert!(
        load_model_routing_states(&state.pool, account_id)
            .await
            .expect("load explicit model 429 routes")
            .iter()
            .all(|route| route.model != "gpt-rate-disabled")
    );
}

#[tokio::test]
async fn api_key_unattributed_disabled_and_payload_failures_are_evidence_only() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let account_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Evidence only temporary failures",
        "evidence-only-temporary-key",
        None,
        Some("https://evidence-only-temporary.example.com/backend-api/codex"),
    )
    .await;

    record_pool_route_http_failure(
        &state.pool,
        account_id,
        UPSTREAM_ACCOUNT_KIND_API_KEY_CODEX,
        false,
        None,
        StatusCode::BAD_GATEWAY,
        "pool upstream responded with 502",
        Some("temporary-without-model"),
    )
    .await
    .expect("record unattributed temporary failure");

    let payload_attempt = insert_model_failure_attempt(
        &state,
        account_id,
        "payload-too-large-model",
        Some("gpt-payload"),
    )
    .await;
    record_pool_route_http_failure_for_endpoint_with_image_intent_for_attempt(
        &state.pool,
        account_id,
        UPSTREAM_ACCOUNT_KIND_API_KEY_CODEX,
        false,
        None,
        StatusCode::PAYLOAD_TOO_LARGE,
        "request body is too large",
        Some("payload-too-large-model"),
        "/v1/responses",
        ImageIntent::Unknown,
        Some(payload_attempt),
        None,
    )
    .await
    .expect("record payload failure");

    sqlx::query(
        "UPDATE pool_upstream_accounts SET policy_status_change_upstream_http_5xx = 0 WHERE id = ?1",
    )
    .bind(account_id)
    .execute(&state.pool)
    .await
    .expect("disable upstream 5xx model status changes");
    let disabled_attempt = insert_model_failure_attempt(
        &state,
        account_id,
        "disabled-502-model",
        Some("gpt-disabled"),
    )
    .await;
    record_pool_route_http_failure_for_endpoint_with_image_intent_for_attempt(
        &state.pool,
        account_id,
        UPSTREAM_ACCOUNT_KIND_API_KEY_CODEX,
        false,
        None,
        StatusCode::BAD_GATEWAY,
        "pool upstream responded with 502",
        Some("disabled-502-model"),
        "/v1/responses",
        ImageIntent::Unknown,
        Some(disabled_attempt),
        None,
    )
    .await
    .expect("record disabled model status change");

    let account = load_upstream_account_row(&state.pool, account_id)
        .await
        .expect("load evidence-only account")
        .expect("evidence-only account exists");
    assert!(account.last_route_failure_kind.is_none());
    assert!(account.cooldown_until.is_none());
    assert_eq!(account.consecutive_route_failures, 0);
    assert!(
        load_model_routing_states(&state.pool, account_id)
            .await
            .expect("load evidence-only model routes")
            .is_empty()
    );
}

#[tokio::test]
async fn stale_model_failure_does_not_overwrite_newer_success() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let account_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Stale model health",
        "stale-model-health-key",
        None,
        Some("https://stale-model-health.example.com/backend-api/codex"),
    )
    .await;

    async fn attempt(
        state: &AppState,
        account_id: i64,
        model: &str,
        started_at: &str,
        status: &str,
    ) -> i64 {
        sqlx::query(
            "INSERT INTO pool_upstream_request_attempts (invoke_id, occurred_at, endpoint, route_mode, request_model, upstream_account_id, upstream_route_key, attempt_index, distinct_account_index, same_account_retry_index, started_at, status) VALUES (?1, ?2, '/v1/responses', 'pool', ?3, ?4, 'route', 1, 1, 0, ?5, ?6)",
        )
        .bind(format!("stale-model-{model}-{started_at}"))
        .bind(format_utc_iso(Utc::now()))
        .bind(model)
        .bind(account_id)
        .bind(started_at)
        .bind(status)
        .execute(&state.pool)
        .await
        .expect("insert stale model attempt")
        .last_insert_rowid()
    }

    let now = Utc::now();
    let old_started = format_utc_iso(now - chrono::Duration::seconds(5));
    let newer_started = format_utc_iso(now - chrono::Duration::seconds(1));
    let old_attempt = attempt(&state, account_id, "gpt-stale", &old_started, "failed").await;
    let newer_attempt = attempt(&state, account_id, "gpt-stale", &newer_started, "pending").await;

    record_model_route_success_from_attempt(
        &state.pool,
        account_id,
        newer_attempt,
        Some(&newer_started),
    )
    .await
    .expect("record newer model success");
    record_model_route_failure_from_attempt(
        &state.pool,
        account_id,
        old_attempt,
        StatusCode::BAD_REQUEST,
        Some("model unavailable"),
        Some("model_unavailable"),
    )
    .await
    .expect("ignore stale model failure");

    let route = load_model_routing_states(&state.pool, account_id)
        .await
        .expect("load stale model route")
        .into_iter()
        .find(|route| route.model == "gpt-stale")
        .expect("stale model route exists");
    assert_eq!(route.failure_count, 0);
    assert_eq!(route.state, MODEL_ROUTE_STATE_AVAILABLE);
    assert!(route.last_failure_at.is_none());
}

#[tokio::test]
async fn later_failed_attempt_does_not_suppress_model_failure() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let account_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Later failed model attempt",
        "later-failed-model-attempt-key",
        None,
        Some("https://later-failed-model-attempt.example.com/backend-api/codex"),
    )
    .await;
    let now = Utc::now();
    let local_started_at = |offset_seconds: i64| {
        format_naive_precise(
            (now + ChronoDuration::seconds(offset_seconds))
                .with_timezone(&Shanghai)
                .naive_local(),
        )
    };
    let insert_attempt = |invoke_id: &str, started_at: String, status: &str| {
        sqlx::query(
            "INSERT INTO pool_upstream_request_attempts (invoke_id, occurred_at, endpoint, route_mode, request_model, upstream_account_id, upstream_route_key, attempt_index, distinct_account_index, same_account_retry_index, started_at, status) VALUES (?1, ?2, '/v1/responses', 'pool', 'gpt-overlap', ?3, 'route', 1, 1, 0, ?4, ?5)",
        )
        .bind(invoke_id.to_string())
        .bind(format_utc_iso(Utc::now()))
        .bind(account_id)
        .bind(started_at)
        .bind(status.to_string())
    };
    let success_started = local_started_at(-3);
    let success_attempt = insert_attempt("overlap-success", success_started.clone(), "success")
        .execute(&state.pool)
        .await
        .expect("insert overlap success")
        .last_insert_rowid();
    record_model_route_success_from_attempt(
        &state.pool,
        account_id,
        success_attempt,
        Some(&success_started),
    )
    .await
    .expect("record overlap success");

    let failing_started = local_started_at(-2);
    let failing_attempt = insert_attempt("overlap-failure", failing_started, "failed")
        .execute(&state.pool)
        .await
        .expect("insert overlap failure")
        .last_insert_rowid();
    insert_attempt("overlap-later-failure", local_started_at(-1), "failed")
        .execute(&state.pool)
        .await
        .expect("insert later failed overlap attempt");

    record_model_route_failure_from_attempt(
        &state.pool,
        account_id,
        failing_attempt,
        StatusCode::BAD_REQUEST,
        Some("model unavailable"),
        Some("model_unavailable"),
    )
    .await
    .expect("record unsuppressed overlap failure");

    let route = load_model_routing_states(&state.pool, account_id)
        .await
        .expect("load overlap model route")
        .into_iter()
        .find(|route| route.model == "gpt-overlap")
        .expect("overlap model route exists");
    assert_eq!(route.failure_count, 1);
    assert_eq!(route.state, MODEL_ROUTE_STATE_DEGRADED);
}

#[tokio::test]
async fn observing_model_after_retention_window_resets_dynamic_health() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let account_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Expired model observation",
        "expired-model-observation-key",
        None,
        Some("https://expired-model-observation.example.com/backend-api/codex"),
    )
    .await;
    let old = format_utc_iso(Utc::now() - ChronoDuration::days(8));
    let cooldown_until = format_utc_iso(Utc::now() + ChronoDuration::seconds(30));
    sqlx::query(
        "INSERT INTO pool_upstream_account_model_routes (account_id, model, state, priority, consecutive_failures, streak_started_at, changed_at, last_seen_at, last_success_at, last_failure_at, last_failure_kind, last_failure_message, cooldown_until) VALUES (?1, 'gpt-expired-observation', 'cooling_down', 'excluded', 5, ?2, ?2, ?2, ?2, ?2, 'model_unavailable', 'model unavailable', ?3)",
    )
    .bind(account_id)
    .bind(&old)
    .bind(&cooldown_until)
    .execute(&state.pool)
    .await
    .expect("seed expired observation route");

    observe_model_route_seen(&state.pool, account_id, Some("gpt-expired-observation"))
        .await
        .expect("observe expired model route");

    let route = load_model_routing_states(&state.pool, account_id)
        .await
        .expect("load refreshed model route")
        .into_iter()
        .find(|route| route.model == "gpt-expired-observation")
        .expect("refreshed model route exists");
    assert_eq!(route.state, MODEL_ROUTE_STATE_AVAILABLE);
    assert_eq!(route.priority, MODEL_ROUTE_PRIORITY_NORMAL);
    assert_eq!(route.failure_count, 0);
    assert!(route.last_failure_at.is_none());
    assert!(route.last_failure_kind.is_none());
    assert!(route.last_failure_message.is_none());
    assert!(route.cooldown_until.is_none());
}

#[tokio::test]
async fn precise_attempt_start_preserves_same_second_model_success() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let account_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Precise model health",
        "precise-model-health-key",
        None,
        Some("https://precise-model-health.example.com/backend-api/codex"),
    )
    .await;
    let base_epoch = Utc::now().timestamp();
    let failure_at = Utc
        .timestamp_opt(base_epoch, 100_000_000)
        .single()
        .expect("failure timestamp");
    let request_started_at_utc = Utc
        .timestamp_opt(base_epoch, 200_000_000)
        .single()
        .expect("request timestamp");
    let failure_at = format_naive_precise(failure_at.with_timezone(&Shanghai).naive_local());
    let request_started_at = format_naive_precise(
        request_started_at_utc
            .with_timezone(&Shanghai)
            .naive_local(),
    );
    sqlx::query(
        "INSERT INTO pool_upstream_account_model_routes (account_id, model, state, priority, consecutive_failures, streak_started_at, changed_at, last_seen_at, last_failure_at, last_failure_kind, last_failure_message) VALUES (?1, 'gpt-precise', 'degraded', 'demoted', 1, ?2, ?2, ?2, ?2, 'model_unavailable', 'model unavailable')",
    )
    .bind(account_id)
    .bind(&failure_at)
    .execute(&state.pool)
    .await
    .expect("seed precise model route");
    let attempt_id = sqlx::query(
        "INSERT INTO pool_upstream_request_attempts (invoke_id, occurred_at, endpoint, route_mode, request_model, upstream_account_id, upstream_route_key, attempt_index, distinct_account_index, same_account_retry_index, started_at, status) VALUES ('precise-model-health', ?1, '/v1/responses', 'pool', 'gpt-precise', ?2, 'route', 1, 1, 0, ?3, 'failed')",
    )
    .bind(format_utc_iso(Utc::now()))
    .bind(account_id)
    .bind(&request_started_at)
    .execute(&state.pool)
    .await
    .expect("insert precise model attempt")
    .last_insert_rowid();

    record_pool_route_success_for_endpoint_with_image_intent_for_attempt(
        &state.pool,
        account_id,
        request_started_at_utc,
        None,
        None,
        "/v1/responses",
        ImageIntent::Unknown,
        Some(attempt_id),
    )
    .await
    .expect("record same-second model success");

    let route = load_model_routing_states(&state.pool, account_id)
        .await
        .expect("load precise model route")
        .into_iter()
        .find(|route| route.model == "gpt-precise")
        .expect("precise model route exists");
    assert_eq!(route.state, MODEL_ROUTE_STATE_AVAILABLE);
    assert_eq!(route.failure_count, 0);
}

#[tokio::test]
async fn manual_model_route_reset_fences_in_flight_failure() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let account_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Reset fence model health",
        "reset-fence-model-health-key",
        None,
        Some("https://reset-fence-model-health.example.com/backend-api/codex"),
    )
    .await;
    observe_model_route_seen(&state.pool, account_id, Some("gpt-reset-fence"))
        .await
        .expect("observe reset fence model");
    sqlx::query(
        "UPDATE pool_upstream_account_model_routes SET cache_concurrency_limit = 1, cache_usage_missing_since = ?2, cache_usage_missing_reason = 'missing_input_tokens' WHERE account_id = ?1 AND model = 'gpt-reset-fence'",
    )
    .bind(account_id)
    .bind(format_utc_iso(Utc::now()))
    .execute(&state.pool)
    .await
    .expect("seed resettable cache-usage marker");

    let attempt_started_at = format_utc_iso(Utc::now() - chrono::Duration::seconds(2));
    let attempt_id = sqlx::query(
        "INSERT INTO pool_upstream_request_attempts (invoke_id, occurred_at, endpoint, route_mode, request_model, upstream_account_id, upstream_route_key, attempt_index, distinct_account_index, same_account_retry_index, started_at, status) VALUES ('reset-fence-model-health', ?1, '/v1/responses', 'pool', 'gpt-reset-fence', ?2, 'route', 1, 1, 0, ?3, 'failed')",
    )
    .bind(format_utc_iso(Utc::now()))
    .bind(account_id)
    .bind(&attempt_started_at)
    .execute(&state.pool)
    .await
    .expect("insert reset fence model attempt")
    .last_insert_rowid();

    let traffic_before_reset = sqlx::query_as::<_, (Option<String>, Option<String>)>(
        "SELECT last_seen_at, last_success_at FROM pool_upstream_account_model_routes WHERE account_id = ?1 AND model = 'gpt-reset-fence'",
    )
    .bind(account_id)
    .fetch_one(&state.pool)
    .await
    .expect("load reset fence timestamps before reset");

    reset_model_route(&state.pool, account_id, "gpt-reset-fence")
        .await
        .expect("reset model route")
        .expect("reset model route exists");
    record_model_route_failure_from_attempt(
        &state.pool,
        account_id,
        attempt_id,
        StatusCode::BAD_REQUEST,
        Some("model unavailable"),
        Some("model_unavailable"),
    )
    .await
    .expect("record in-flight model failure");

    let route = load_model_routing_states(&state.pool, account_id)
        .await
        .expect("load reset fence route")
        .into_iter()
        .find(|item| item.model == "gpt-reset-fence")
        .expect("reset fence route exists");
    assert_eq!(route.state, MODEL_ROUTE_STATE_AVAILABLE);
    assert_eq!(route.priority, MODEL_ROUTE_PRIORITY_NORMAL);
    assert_eq!(route.failure_count, 0);
    assert!(route.last_failure_at.is_none());
    assert!(route.cache_usage_missing_since.is_none());
    assert!(route.cache_usage_missing_reason.is_none());

    let traffic_after_reset = sqlx::query_as::<_, (Option<String>, Option<String>)>(
        "SELECT last_seen_at, last_success_at FROM pool_upstream_account_model_routes WHERE account_id = ?1 AND model = 'gpt-reset-fence'",
    )
    .bind(account_id)
    .fetch_one(&state.pool)
    .await
    .expect("load reset fence timestamps after reset");
    assert_eq!(traffic_after_reset, traffic_before_reset);
}

#[tokio::test]
#[expect(
    clippy::await_holding_lock,
    reason = "The process-global priority handoff mirror must be isolated from concurrent stateful tests."
)]
async fn manual_model_route_reset_survives_diagnostic_persistence_failure() {
    let _priority_handoff_guard = crate::upstream_accounts::priority_handoff_test_guard();
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let account_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Reset diagnostic failure",
        "reset-diagnostic-failure-key",
        None,
        Some("https://reset-diagnostic-failure.example.com/backend-api/codex"),
    )
    .await;
    let model = "gpt-reset-diagnostic-failure";
    observe_model_route_seen(&state.pool, account_id, Some(model))
        .await
        .expect("observe reset diagnostic model");
    let (decision, permit) = admit_priority_handoff(account_id, Some(model));
    assert!(matches!(
        decision,
        PriorityHandoffAdmissionDecision::Admitted { .. }
    ));
    assert_eq!(
        permit
            .as_ref()
            .and_then(|permit| permit.complete_failure(true)),
        Some(PRIORITY_HANDOFF_FAILURE_COOLDOWN_REASON)
    );
    assert_eq!(
        priority_handoff_admission_snapshot(account_id, Some(model)).0,
        "coolingDown"
    );

    sqlx::query("DROP TABLE pool_upstream_account_events")
        .execute(&state.pool)
        .await
        .expect("drop diagnostic event table");
    let reset = reset_model_route(&state.pool, account_id, model)
        .await
        .expect("reset should ignore diagnostic persistence failure")
        .expect("reset model route exists");
    assert_eq!(reset.state, MODEL_ROUTE_STATE_AVAILABLE);
    assert_eq!(reset.priority, MODEL_ROUTE_PRIORITY_NORMAL);
    assert_eq!(
        priority_handoff_admission_snapshot(account_id, Some(model)),
        ("verifying".to_string(), 0)
    );
    drop(permit);
}

#[tokio::test]
async fn api_key_model_route_health_isolated_and_resettable() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let account_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Model health account",
        "model-health-key",
        None,
        Some("https://model-health.example.com/backend-api/codex"),
    )
    .await;

    async fn attempt(state: &AppState, account_id: i64, model: &str) -> i64 {
        sqlx::query(
            "INSERT INTO pool_upstream_request_attempts (invoke_id, occurred_at, endpoint, route_mode, request_model, upstream_account_id, upstream_route_key, attempt_index, distinct_account_index, same_account_retry_index, status) VALUES (?1, ?2, '/v1/responses', 'pool', ?3, ?4, 'route', 1, 1, 0, 'failed')",
        )
        .bind(format!("model-health-{model}"))
        .bind(format_utc_iso(Utc::now()))
        .bind(model)
        .bind(account_id)
        .execute(&state.pool)
        .await
        .expect("insert model attempt")
        .last_insert_rowid()
    }

    let attempt_a = attempt(&state, account_id, "gpt-5.5").await;
    for _ in 0..5 {
        record_model_route_failure_from_attempt(
            &state.pool,
            account_id,
            attempt_a,
            StatusCode::BAD_REQUEST,
            Some("unsupported model: gpt-5.5"),
            Some("model_unavailable"),
        )
        .await
        .expect("record model-specific failure");
    }
    let states = load_model_routing_states(&state.pool, account_id)
        .await
        .expect("load model routing states");
    assert_eq!(states.len(), 1);
    assert_eq!(states[0].model, "gpt-5.5");
    assert_eq!(states[0].state, MODEL_ROUTE_STATE_COOLING_DOWN);
    assert_eq!(states[0].priority, MODEL_ROUTE_PRIORITY_EXCLUDED);
    assert_eq!(
        model_route_penalty(&state.pool, account_id, Some("gpt-5.5"))
            .await
            .unwrap(),
        ModelRoutePenalty::Excluded
    );

    let attempt_b = attempt(&state, account_id, "gpt-5.4").await;
    record_model_route_success_from_attempt(&state.pool, account_id, attempt_b, None)
        .await
        .expect("record independent model success");
    assert_eq!(
        model_route_penalty(&state.pool, account_id, Some("gpt-5.4"))
            .await
            .unwrap(),
        ModelRoutePenalty::Normal
    );

    reset_model_route(&state.pool, account_id, "gpt-5.5")
        .await
        .expect("reset model route")
        .expect("model route exists");
    let reset = load_model_routing_states(&state.pool, account_id)
        .await
        .expect("load reset state")
        .into_iter()
        .find(|item| item.model == "gpt-5.5")
        .expect("reset model state exists");
    assert_eq!(reset.state, MODEL_ROUTE_STATE_AVAILABLE);
    assert_eq!(reset.priority, MODEL_ROUTE_PRIORITY_NORMAL);
    assert_eq!(reset.failure_count, 0);
    let event_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM pool_upstream_account_events WHERE account_id = ?1 AND model = 'gpt-5.5'",
    )
    .bind(account_id)
    .fetch_one(&state.pool)
    .await
    .expect("count model events");
    assert!(event_count >= 2);
}

#[tokio::test]
async fn concurrent_model_failures_reach_cooldown_threshold_without_lost_updates() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let account_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Concurrent model health",
        "concurrent-model-health-key",
        None,
        Some("https://concurrent-model-health.example.com/backend-api/codex"),
    )
    .await;
    let attempt_id = sqlx::query(
        "INSERT INTO pool_upstream_request_attempts (invoke_id, occurred_at, endpoint, route_mode, request_model, upstream_account_id, upstream_route_key, attempt_index, distinct_account_index, same_account_retry_index, status) VALUES ('concurrent-model-health', ?1, '/v1/responses', 'pool', 'gpt-concurrent', ?2, 'route', 1, 1, 0, 'failed')",
    )
    .bind(format_utc_iso(Utc::now()))
    .bind(account_id)
    .execute(&state.pool)
    .await
    .expect("insert concurrent model attempt")
    .last_insert_rowid();

    let failure = || {
        record_model_route_failure_from_attempt(
            &state.pool,
            account_id,
            attempt_id,
            StatusCode::BAD_REQUEST,
            Some("model unavailable"),
            Some("model_unavailable"),
        )
    };
    let results = tokio::join!(failure(), failure(), failure(), failure(), failure());
    for result in [results.0, results.1, results.2, results.3, results.4] {
        result.expect("record concurrent model failure");
    }

    let route = load_model_routing_states(&state.pool, account_id)
        .await
        .expect("load concurrent route")
        .into_iter()
        .find(|state| state.model == "gpt-concurrent")
        .expect("concurrent route exists");
    assert_eq!(route.failure_count, MODEL_ROUTE_FAILURE_THRESHOLD);
    assert_eq!(route.state, MODEL_ROUTE_STATE_COOLING_DOWN);
    assert_eq!(route.priority, MODEL_ROUTE_PRIORITY_EXCLUDED);
}

#[tokio::test]
async fn sparse_model_failures_keep_count_until_cooldown_threshold() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let account_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Sparse model health",
        "sparse-model-health-key",
        None,
        Some("https://sparse-model-health.example.com/backend-api/codex"),
    )
    .await;
    let old = format_utc_iso(Utc::now() - ChronoDuration::seconds(45));
    sqlx::query(
        "INSERT INTO pool_upstream_account_model_routes (account_id, model, state, priority, consecutive_failures, streak_started_at, changed_at, last_seen_at, last_failure_at, last_failure_kind, last_failure_message) VALUES (?1, 'gpt-sparse', 'degraded', 'demoted', 4, ?2, ?2, ?3, ?2, 'model_unavailable', 'model unavailable')",
    )
    .bind(account_id)
    .bind(&old)
    .bind(format_utc_iso(Utc::now()))
    .execute(&state.pool)
    .await
    .expect("seed sparse model route");
    let attempt_id = sqlx::query(
        "INSERT INTO pool_upstream_request_attempts (invoke_id, occurred_at, endpoint, route_mode, request_model, upstream_account_id, upstream_route_key, attempt_index, distinct_account_index, same_account_retry_index, status) VALUES ('sparse-model-health', ?1, '/v1/responses', 'pool', 'gpt-sparse', ?2, 'route', 1, 1, 0, 'failed')",
    )
    .bind(format_utc_iso(Utc::now()))
    .bind(account_id)
    .execute(&state.pool)
    .await
    .expect("insert sparse model attempt")
    .last_insert_rowid();

    record_model_route_failure_from_attempt(
        &state.pool,
        account_id,
        attempt_id,
        StatusCode::BAD_REQUEST,
        Some("model unavailable"),
        Some("model_unavailable"),
    )
    .await
    .expect("record sparse model failure");

    let route = load_model_routing_states(&state.pool, account_id)
        .await
        .expect("load sparse model route")
        .into_iter()
        .find(|route| route.model == "gpt-sparse")
        .expect("sparse model route exists");
    assert_eq!(route.failure_count, MODEL_ROUTE_FAILURE_THRESHOLD);
    assert_eq!(route.state, MODEL_ROUTE_STATE_COOLING_DOWN);
    assert_eq!(route.priority, MODEL_ROUTE_PRIORITY_EXCLUDED);
}

#[tokio::test]
async fn expired_model_cooldown_reports_transition_at_expiry() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let account_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Expired model health",
        "expired-model-health-key",
        None,
        Some("https://expired-model-health.example.com/backend-api/codex"),
    )
    .await;
    let cooldown_until = format_utc_iso(Utc::now() - ChronoDuration::seconds(5));
    let changed_at = format_utc_iso(Utc::now() - ChronoDuration::seconds(20));
    sqlx::query(
        "INSERT INTO pool_upstream_account_model_routes (account_id, model, state, priority, consecutive_failures, changed_at, last_seen_at, last_failure_at, cooldown_until) VALUES (?1, 'gpt-expired', 'cooling_down', 'excluded', 5, ?2, ?3, ?2, ?4)",
    )
    .bind(account_id)
    .bind(&changed_at)
    .bind(format_utc_iso(Utc::now()))
    .bind(&cooldown_until)
    .execute(&state.pool)
    .await
    .expect("seed expired model route");

    let route = load_model_routing_states(&state.pool, account_id)
        .await
        .expect("load expired model route")
        .into_iter()
        .find(|route| route.model == "gpt-expired")
        .expect("expired model route exists");
    assert_eq!(route.state, MODEL_ROUTE_STATE_DEGRADED);
    assert_eq!(route.priority, MODEL_ROUTE_PRIORITY_DEMOTED);
    assert_eq!(
        route.changed_at.as_deref().and_then(parse_to_utc_datetime),
        parse_to_utc_datetime(&cooldown_until),
    );
    assert!(route.cooldown_until.is_none());
}

#[tokio::test]
async fn current_quota_route_failure_survives_informational_account_updates() {
    let pool = test_pool().await;
    let account_id = insert_oauth_account(&pool, "Quota exhausted after edit").await;

    record_pool_route_http_failure(
            &pool,
            account_id,
            UPSTREAM_ACCOUNT_KIND_OAUTH_CODEX,
            false,
            Some("sticky-quota-after-edit"),
            StatusCode::TOO_MANY_REQUESTS,
            "oauth_upstream_rejected_request: pool upstream responded with 429: The usage limit has been reached",
            Some("invk_quota_after_edit"),
        )
        .await
        .expect("record wrapped 429 route failure before edit");

    record_account_update_action(
        &pool,
        account_id,
        "account settings were updated after the quota-exhausted failure",
    )
    .await
    .expect("record account update action");

    let row = load_upstream_account_row(&pool, account_id)
        .await
        .expect("load updated row")
        .expect("updated row exists");
    let summary = build_summary_from_row(
        &row,
        None,
        row.last_activity_at.clone(),
        vec![],
        None,
        0,
        Utc::now(),
    );

    assert_eq!(summary.status, UPSTREAM_ACCOUNT_STATUS_ERROR);
    assert_eq!(summary.display_status, UPSTREAM_ACCOUNT_STATUS_ACTIVE);
    assert_eq!(summary.health_status, UPSTREAM_ACCOUNT_HEALTH_STATUS_NORMAL);
    assert_eq!(
        summary.work_status,
        UPSTREAM_ACCOUNT_WORK_STATUS_RATE_LIMITED
    );
    assert_eq!(
        summary.last_action_reason_code.as_deref(),
        Some(UPSTREAM_ACCOUNT_ACTION_REASON_ACCOUNT_UPDATED)
    );
    assert_eq!(
        row.last_route_failure_kind.as_deref(),
        Some(FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED)
    );
}

#[tokio::test]
async fn oauth_summary_exports_missing_refresh_token_flag() {
    let pool = test_pool().await;
    let account_id = insert_oauth_account(&pool, "Manual RT omitted").await;
    sqlx::query("UPDATE pool_upstream_accounts SET has_refresh_token = 0 WHERE id = ?")
        .bind(account_id)
        .execute(&pool)
        .await
        .expect("mark account as missing refresh token");

    let row = load_upstream_account_row(&pool, account_id)
        .await
        .expect("load no refresh token row")
        .expect("no refresh token row exists");
    let summary = build_summary_from_row(
        &row,
        None,
        row.last_activity_at.clone(),
        vec![],
        None,
        0,
        Utc::now(),
    );

    assert!(!summary.has_refresh_token);
}

pub(crate) async fn insert_limit_sample(
    pool: &SqlitePool,
    account_id: i64,
    captured_at: &str,
    plan_type: Option<&str>,
) {
    sqlx::query(
        r#"
            INSERT INTO pool_upstream_account_limit_samples (
                account_id, captured_at, limit_id, limit_name, plan_type,
                primary_used_percent, primary_window_minutes, primary_resets_at,
                secondary_used_percent, secondary_window_minutes, secondary_resets_at,
                credits_has_credits, credits_unlimited, credits_balance
            ) VALUES (
                ?1, ?2, NULL, NULL, ?3,
                NULL, NULL, NULL,
                NULL, NULL, NULL,
                NULL, NULL, NULL
            )
            "#,
    )
    .bind(account_id)
    .bind(captured_at)
    .bind(plan_type)
    .execute(pool)
    .await
    .expect("insert limit sample");
}

pub(crate) async fn insert_limit_sample_with_usage(
    pool: &SqlitePool,
    account_id: i64,
    captured_at: &str,
    primary_used_percent: Option<f64>,
    secondary_used_percent: Option<f64>,
) {
    sqlx::query(
        r#"
            INSERT INTO pool_upstream_account_limit_samples (
                account_id, captured_at, limit_id, limit_name, plan_type,
                primary_used_percent, primary_window_minutes, primary_resets_at,
                secondary_used_percent, secondary_window_minutes, secondary_resets_at,
                credits_has_credits, credits_unlimited, credits_balance
            ) VALUES (
                ?1, ?2, NULL, NULL, 'team',
                ?3, 300, NULL,
                ?4, 10080, NULL,
                NULL, NULL, NULL
            )
            "#,
    )
    .bind(account_id)
    .bind(captured_at)
    .bind(primary_used_percent)
    .bind(secondary_used_percent)
    .execute(pool)
    .await
    .expect("insert limit sample with usage");
}

pub(crate) async fn insert_limit_sample_with_reset_times(
    pool: &SqlitePool,
    account_id: i64,
    captured_at: &str,
    primary_resets_at: Option<&str>,
    secondary_resets_at: Option<&str>,
    primary_used_percent: f64,
    secondary_used_percent: f64,
) {
    sqlx::query(
        r#"
            INSERT INTO pool_upstream_account_limit_samples (
                account_id, captured_at, limit_id, limit_name, plan_type,
                primary_used_percent, primary_window_minutes, primary_resets_at,
                secondary_used_percent, secondary_window_minutes, secondary_resets_at,
                credits_has_credits, credits_unlimited, credits_balance
            ) VALUES (
                ?1, ?2, NULL, NULL, 'team',
                ?3, 300, ?4,
                ?5, 10080, ?6,
                NULL, NULL, NULL
            )
            "#,
    )
    .bind(account_id)
    .bind(captured_at)
    .bind(primary_used_percent)
    .bind(primary_resets_at)
    .bind(secondary_used_percent)
    .bind(secondary_resets_at)
    .execute(pool)
    .await
    .expect("insert limit sample with reset times");
}

pub(crate) async fn seed_route_cooldown(
    pool: &SqlitePool,
    account_id: i64,
    failure_kind: &str,
    cooldown_secs: i64,
) {
    let now = Utc::now();
    let now_iso = format_utc_iso(now);
    let cooldown_until = format_utc_iso(now + ChronoDuration::seconds(cooldown_secs));
    sqlx::query(
        r#"
            UPDATE pool_upstream_accounts
            SET status = ?2,
                last_error = ?3,
                last_error_at = ?4,
                last_route_failure_at = ?4,
                last_route_failure_kind = ?5,
                cooldown_until = ?6,
                consecutive_route_failures = 1,
                temporary_route_failure_streak_started_at = NULL,
                updated_at = ?4
            WHERE id = ?1
            "#,
    )
    .bind(account_id)
    .bind(UPSTREAM_ACCOUNT_STATUS_ACTIVE)
    .bind("seed route cooldown")
    .bind(&now_iso)
    .bind(failure_kind)
    .bind(&cooldown_until)
    .execute(pool)
    .await
    .expect("seed route cooldown");
}

#[test]
fn pool_blocked_failure_kinds_are_not_temporary_route_failures() {
    assert!(!route_failure_kind_is_temporary(Some(
        PROXY_FAILURE_POOL_ROUTING_BLOCKED,
    )));
    assert!(!route_failure_kind_is_temporary(Some(
        PROXY_FAILURE_POOL_ASSIGNED_ACCOUNT_BLOCKED,
    )));
}

pub(crate) async fn seed_hard_unavailable_route_failure(
    pool: &SqlitePool,
    account_id: i64,
    status: &str,
    failure_kind: &str,
    reason_code: &str,
    http_status: Option<i64>,
) {
    let now_iso = format_utc_iso(Utc::now());
    sqlx::query(
        r#"
            UPDATE pool_upstream_accounts
            SET status = ?2,
                last_error = ?3,
                last_error_at = ?4,
                last_route_failure_at = ?4,
                last_route_failure_kind = ?5,
                cooldown_until = NULL,
                consecutive_route_failures = 1,
                temporary_route_failure_streak_started_at = NULL,
                last_action = ?6,
                last_action_source = ?7,
                last_action_reason_code = ?8,
                last_action_reason_message = ?3,
                last_action_http_status = ?9,
                last_action_at = ?4,
                updated_at = ?4
            WHERE id = ?1
            "#,
    )
    .bind(account_id)
    .bind(status)
    .bind("seed hard unavailable")
    .bind(&now_iso)
    .bind(failure_kind)
    .bind(UPSTREAM_ACCOUNT_ACTION_ROUTE_HARD_UNAVAILABLE)
    .bind(UPSTREAM_ACCOUNT_ACTION_SOURCE_CALL)
    .bind(reason_code)
    .bind(http_status)
    .execute(pool)
    .await
    .expect("seed hard unavailable");
}

#[tokio::test]
async fn record_pool_route_success_does_not_clear_newer_route_failure_state() {
    let pool = test_pool().await;
    let account_id = insert_api_key_account(&pool, "Stale Success Guard").await;
    seed_hard_unavailable_route_failure(
        &pool,
        account_id,
        UPSTREAM_ACCOUNT_STATUS_ERROR,
        FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED,
        UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED,
        Some(429),
    )
    .await;

    record_pool_route_success(
        &pool,
        account_id,
        Utc::now() - ChronoDuration::minutes(5),
        Some("sticky-stale-success"),
        Some("invk_stale_success"),
    )
    .await
    .expect("record stale route success");

    let after = load_upstream_account_row(&pool, account_id)
        .await
        .expect("load row after stale success")
        .expect("row exists after stale success");
    assert_eq!(after.status, UPSTREAM_ACCOUNT_STATUS_ERROR);
    assert_eq!(
        after.last_route_failure_kind.as_deref(),
        Some(FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED)
    );
    assert_eq!(
        after.last_action.as_deref(),
        Some(UPSTREAM_ACCOUNT_ACTION_ROUTE_HARD_UNAVAILABLE)
    );
    assert!(
        load_sticky_route(&pool, "sticky-stale-success")
            .await
            .expect("load sticky route after stale success")
            .is_none()
    );
}

#[tokio::test]
async fn model_success_is_recorded_when_account_success_is_stale() {
    let pool = test_pool().await;
    let account_id = insert_api_key_account(&pool, "Stale Account Success Model Recovery").await;
    let request_started_at_utc = Utc::now() - ChronoDuration::seconds(5);
    let request_started_at = format_naive_precise(
        request_started_at_utc
            .with_timezone(&Shanghai)
            .naive_local(),
    );
    let attempt_id = sqlx::query(
        "INSERT INTO pool_upstream_request_attempts (invoke_id, occurred_at, endpoint, route_mode, request_model, upstream_account_id, upstream_route_key, attempt_index, distinct_account_index, same_account_retry_index, started_at, status) VALUES ('stale-account-success-model', ?1, '/v1/responses', 'pool', 'gpt-stale-account-success', ?2, 'route', 1, 1, 0, ?3, 'failed')",
    )
    .bind(format_utc_iso(Utc::now()))
    .bind(account_id)
    .bind(&request_started_at)
    .execute(&pool)
    .await
    .expect("insert stale account success attempt")
    .last_insert_rowid();

    let model_failure_at = format_naive_precise(
        (request_started_at_utc - ChronoDuration::seconds(1))
            .with_timezone(&Shanghai)
            .naive_local(),
    );
    sqlx::query(
        "INSERT INTO pool_upstream_account_model_routes (account_id, model, state, priority, consecutive_failures, streak_started_at, changed_at, last_seen_at, last_failure_at, last_failure_kind, last_failure_message) VALUES (?1, 'gpt-stale-account-success', 'degraded', 'demoted', 1, ?2, ?2, ?3, ?2, 'model_unavailable', 'model unavailable')",
    )
    .bind(account_id)
    .bind(&model_failure_at)
    .bind(format_utc_iso(Utc::now()))
    .execute(&pool)
    .await
    .expect("seed stale account success model failure");
    seed_hard_unavailable_route_failure(
        &pool,
        account_id,
        UPSTREAM_ACCOUNT_STATUS_ERROR,
        FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED,
        UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED,
        Some(429),
    )
    .await;

    record_pool_route_success_for_endpoint_with_image_intent_for_attempt(
        &pool,
        account_id,
        request_started_at_utc,
        None,
        None,
        "/v1/responses",
        ImageIntent::Unknown,
        Some(attempt_id),
    )
    .await
    .expect("record stale account success");

    let route = load_model_routing_states(&pool, account_id)
        .await
        .expect("load stale account success model route")
        .into_iter()
        .find(|route| route.model == "gpt-stale-account-success")
        .expect("stale account success model route exists");
    assert_eq!(route.state, MODEL_ROUTE_STATE_AVAILABLE);
    assert_eq!(route.priority, MODEL_ROUTE_PRIORITY_NORMAL);
    assert_eq!(route.failure_count, 0);
    assert!(route.last_failure_at.is_none());

    let account = load_upstream_account_row(&pool, account_id)
        .await
        .expect("load stale account success account")
        .expect("stale account success account exists");
    assert_eq!(account.status, UPSTREAM_ACCOUNT_STATUS_ERROR);
}
