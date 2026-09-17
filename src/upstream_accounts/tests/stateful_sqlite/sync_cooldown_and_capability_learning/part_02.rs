use super::*;

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

    assert_non_cache_probe_recovered(&state, account_id, model).await;
}

async fn assert_non_cache_probe_recovered(state: &AppState, account_id: i64, model: &str) {
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
pub(crate) async fn cache_hit_protection_cooldown_restarts_with_single_probe() {
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
    assert_cache_cooldown_after_low_observations(&state, account_id, model).await;
    assert_cache_cooldown_probe_cycle(&state, account_id, model).await;
}

async fn assert_cache_cooldown_after_low_observations(
    state: &AppState,
    account_id: i64,
    model: &str,
) {
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
    let first_cooldown = cache_hit_route_state(state, account_id, model).await;
    assert_eq!(first_cooldown.0, MODEL_ROUTE_STATE_COOLING_DOWN);
    assert_eq!(first_cooldown.3, 0);
    assert_eq!(first_cooldown.4, 1);
    assert!(first_cooldown.6.is_some());
}

async fn assert_cache_cooldown_probe_cycle(state: &AppState, account_id: i64, model: &str) {
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
        let state_after_low = cache_hit_route_state(state, account_id, model).await;
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
    let second_cooldown = cache_hit_route_state(state, account_id, model).await;
    assert_eq!(second_cooldown.0, MODEL_ROUTE_STATE_COOLING_DOWN);
    assert_eq!(second_cooldown.4, 2);
}

#[tokio::test]
pub(crate) async fn api_key_temporary_http_failure_changes_only_the_exact_model_route() {
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
pub(crate) async fn api_key_transport_failure_preserves_kind_and_changes_only_the_exact_model_route()
 {
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
pub(crate) async fn api_key_pre_attempt_transport_failure_uses_the_exact_request_model() {
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
pub(crate) async fn api_key_explicit_model_429_honors_toggle_and_preserves_reason() {
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
pub(crate) async fn api_key_unattributed_disabled_and_payload_failures_are_evidence_only() {
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
pub(crate) async fn stale_model_failure_does_not_overwrite_newer_success() {
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
pub(crate) async fn later_failed_attempt_does_not_suppress_model_failure() {
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
pub(crate) async fn observing_model_after_retention_window_resets_dynamic_health() {
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
pub(crate) async fn precise_attempt_start_preserves_same_second_model_success() {
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
pub(crate) async fn manual_model_route_reset_fences_in_flight_failure() {
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
pub(crate) async fn manual_model_route_reset_survives_diagnostic_persistence_failure() {
    let _priority_handoff_guard = crate::upstream_accounts::priority_handoff_test_guard().await;
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
pub(crate) async fn api_key_model_route_health_isolated_and_resettable() {
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
pub(crate) async fn concurrent_model_failures_reach_cooldown_threshold_without_lost_updates() {
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
pub(crate) async fn sparse_model_failures_keep_count_until_cooldown_threshold() {
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
