use super::*;

#[test]
fn pool_route_surfaces_last_upstream_error_when_failover_is_exhausted() {
    run_routing_failover_future_with_large_stack(async move {
        let (upstream_base, _attempts, upstream_handle) =
            spawn_pool_retry_upstream(&[("Bearer upstream-primary", 99)]).await;
        let state = test_state_with_openai_base(
            Url::parse(&upstream_base).expect("valid upstream base url"),
        )
        .await;
        seed_pool_routing_api_key(&state, "pool-live-key").await;
        insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;

        let response = proxy_openai_v1(
            State(state),
            OriginalUri("/v1/responses".parse().expect("valid uri")),
            Method::POST,
            HeaderMap::from_iter([(
                http_header::AUTHORIZATION,
                HeaderValue::from_static("Bearer pool-live-key"),
            )]),
            Body::from(
                r#"{"model":"gpt-5","input":"hello","stickyKey":"sticky-500"}"#
                    .as_bytes()
                    .to_vec(),
            ),
        )
        .await;

        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read failure body");
        let payload: Value = serde_json::from_slice(&body).expect("decode failure payload");
        assert_eq!(
            payload["error"].as_str(),
            Some("pool upstream responded with 500")
        );

        upstream_handle.abort();
    });
}

#[test]
fn pool_route_returns_clear_429_when_only_account_is_rate_limited() {
    run_routing_failover_future_with_large_stack(async move {
        let (upstream_base, attempts, upstream_handle) =
            spawn_pool_rate_limit_responses_upstream(&[("Bearer upstream-primary", 99)]).await;
        let state = test_state_with_openai_base(
            Url::parse(&upstream_base).expect("valid upstream base url"),
        )
        .await;
        seed_pool_routing_api_key(&state, "pool-live-key").await;
        insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;

        let response = proxy_openai_v1(
            State(state),
            OriginalUri("/v1/responses".parse().expect("valid uri")),
            Method::POST,
            HeaderMap::from_iter([(
                http_header::AUTHORIZATION,
                HeaderValue::from_static("Bearer pool-live-key"),
            )]),
            Body::from(
                r#"{"model":"gpt-5","input":"hello","stickyKey":"sticky-429-terminal"}"#
                    .as_bytes()
                    .to_vec(),
            ),
        )
        .await;

        assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read failure body");
        let payload: Value = serde_json::from_slice(&body).expect("decode failure payload");
        assert_eq!(
            payload["error"].as_str(),
            Some(POOL_ALL_ACCOUNTS_RATE_LIMITED_MESSAGE)
        );

        let attempts = attempts.lock().expect("lock attempts");
        assert_eq!(attempts.get("Bearer upstream-primary").copied(), Some(1));

        upstream_handle.abort();
    });
}

#[tokio::test]
async fn pool_route_returns_clear_503_when_all_accounts_are_temporarily_degraded() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    seed_pool_routing_api_key(&state, "pool-live-key").await;
    let primary_id = insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;
    let secondary_id =
        insert_test_pool_api_key_account(&state, "Secondary", "upstream-secondary").await;
    set_test_account_degraded_route_state(
        &state.pool,
        primary_id,
        FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429,
        "test degraded plain 429",
    )
    .await;
    set_test_account_degraded_route_state(
        &state.pool,
        secondary_id,
        FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_5XX,
        "test degraded 5xx",
    )
    .await;

    let response = proxy_openai_v1(
        State(state),
        OriginalUri("/v1/responses".parse().expect("valid uri")),
        Method::POST,
        HeaderMap::from_iter([(
            http_header::AUTHORIZATION,
            HeaderValue::from_static("Bearer pool-live-key"),
        )]),
        Body::from(
            r#"{"model":"gpt-5","input":"hello","stickyKey":"sticky-degraded-only"}"#
                .as_bytes()
                .to_vec(),
        ),
    )
    .await;

    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read failure body");
    let payload: Value = serde_json::from_slice(&body).expect("decode failure payload");
    assert_eq!(
        payload["error"].as_str(),
        Some(POOL_ALL_ACCOUNTS_DEGRADED_MESSAGE)
    );
}

#[tokio::test]
async fn pool_route_returns_clear_429_when_all_accounts_are_already_in_429_cooldown() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    seed_pool_routing_api_key(&state, "pool-live-key").await;
    let primary_id = insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;
    let secondary_id =
        insert_test_pool_api_key_account(&state, "Secondary", "upstream-secondary").await;
    set_test_account_rate_limited_cooldown(&state.pool, primary_id, 120).await;
    set_test_account_rate_limited_cooldown(&state.pool, secondary_id, 120).await;

    let response = proxy_openai_v1(
        State(state),
        OriginalUri("/v1/responses".parse().expect("valid uri")),
        Method::POST,
        HeaderMap::from_iter([(
            http_header::AUTHORIZATION,
            HeaderValue::from_static("Bearer pool-live-key"),
        )]),
        Body::from(
            r#"{"model":"gpt-5","input":"hello","stickyKey":"sticky-429-cooldown"}"#
                .as_bytes()
                .to_vec(),
        ),
    )
    .await;

    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read failure body");
    let payload: Value = serde_json::from_slice(&body).expect("decode failure payload");
    assert_eq!(
        payload["error"].as_str(),
        Some(POOL_ALL_ACCOUNTS_RATE_LIMITED_MESSAGE)
    );
}

#[tokio::test]
async fn pool_route_ignores_missing_credentials_when_all_routable_accounts_are_rate_limited() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    seed_pool_routing_api_key(&state, "pool-live-key").await;
    let primary_id = insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;
    let missing_credentials_id =
        insert_test_pool_api_key_account(&state, "Missing Credentials", "upstream-missing").await;
    set_test_account_rate_limited_cooldown(&state.pool, primary_id, 120).await;
    clear_test_account_credentials(&state.pool, missing_credentials_id).await;

    let response = proxy_openai_v1(
        State(state),
        OriginalUri("/v1/responses".parse().expect("valid uri")),
        Method::POST,
        HeaderMap::from_iter([(
            http_header::AUTHORIZATION,
            HeaderValue::from_static("Bearer pool-live-key"),
        )]),
        Body::from(
            r#"{"model":"gpt-5","input":"hello","stickyKey":"sticky-429-missing-creds"}"#
                .as_bytes()
                .to_vec(),
        ),
    )
    .await;

    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read failure body");
    let payload: Value = serde_json::from_slice(&body).expect("decode failure payload");
    assert_eq!(
        payload["error"].as_str(),
        Some(POOL_ALL_ACCOUNTS_RATE_LIMITED_MESSAGE)
    );
}

#[tokio::test]
async fn pool_route_stale_sticky_binding_does_not_hide_pool_wide_429() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    seed_pool_routing_api_key(&state, "pool-live-key").await;
    let primary_id = insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;
    let secondary_id =
        insert_test_pool_api_key_account(&state, "Secondary", "upstream-secondary").await;
    set_test_account_status(&state.pool, primary_id, "needs_reauth").await;
    set_test_account_rate_limited_cooldown(&state.pool, secondary_id, 120).await;
    let sticky_seen_at = format_utc_iso(Utc::now());
    upsert_test_sticky_route_at(
        &state.pool,
        "sticky-429-stale-binding",
        primary_id,
        &sticky_seen_at,
    )
    .await;

    let response = proxy_openai_v1(
        State(state),
        OriginalUri("/v1/responses".parse().expect("valid uri")),
        Method::POST,
        HeaderMap::from_iter([(
            http_header::AUTHORIZATION,
            HeaderValue::from_static("Bearer pool-live-key"),
        )]),
        Body::from(
            r#"{"model":"gpt-5","input":"hello","stickyKey":"sticky-429-stale-binding"}"#
                .as_bytes()
                .to_vec(),
        ),
    )
    .await;

    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read failure body");
    let payload: Value = serde_json::from_slice(&body).expect("decode failure payload");
    assert_eq!(
        payload["error"].as_str(),
        Some(POOL_ALL_ACCOUNTS_RATE_LIMITED_MESSAGE)
    );
}

#[tokio::test]
async fn pool_route_missing_credentials_sticky_binding_does_not_hide_pool_wide_429() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    seed_pool_routing_api_key(&state, "pool-live-key").await;
    let primary_id = insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;
    let secondary_id =
        insert_test_pool_api_key_account(&state, "Secondary", "upstream-secondary").await;
    clear_test_account_credentials(&state.pool, primary_id).await;
    set_test_account_rate_limited_cooldown(&state.pool, secondary_id, 120).await;
    let sticky_seen_at = format_utc_iso(Utc::now());
    upsert_test_sticky_route_at(
        &state.pool,
        "sticky-429-missing-creds-binding",
        primary_id,
        &sticky_seen_at,
    )
    .await;

    let response = proxy_openai_v1(
        State(state),
        OriginalUri("/v1/responses".parse().expect("valid uri")),
        Method::POST,
        HeaderMap::from_iter([(
            http_header::AUTHORIZATION,
            HeaderValue::from_static("Bearer pool-live-key"),
        )]),
        Body::from(
            r#"{"model":"gpt-5","input":"hello","stickyKey":"sticky-429-missing-creds-binding"}"#
                .as_bytes()
                .to_vec(),
        ),
    )
    .await;

    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read failure body");
    let payload: Value = serde_json::from_slice(&body).expect("decode failure payload");
    assert_eq!(
        payload["error"].as_str(),
        Some(POOL_ALL_ACCOUNTS_RATE_LIMITED_MESSAGE)
    );
}

#[test]
fn pool_route_keeps_generic_no_candidate_when_other_accounts_are_unavailable_for_other_reasons() {
    run_routing_failover_future_with_large_stack(async move {
        let (upstream_base, attempts, upstream_handle) =
            spawn_pool_rate_limit_responses_upstream(&[("Bearer upstream-primary", 99)]).await;
        let state = test_state_with_openai_base(
            Url::parse(&upstream_base).expect("valid upstream base url"),
        )
        .await;
        seed_pool_routing_api_key(&state, "pool-live-key").await;
        insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;
        let secondary_id =
            insert_test_pool_api_key_account(&state, "Secondary", "upstream-secondary").await;
        set_test_account_status(&state.pool, secondary_id, "needs_reauth").await;

        let response = proxy_openai_v1(
            State(state),
            OriginalUri("/v1/responses".parse().expect("valid uri")),
            Method::POST,
            HeaderMap::from_iter([(
                http_header::AUTHORIZATION,
                HeaderValue::from_static("Bearer pool-live-key"),
            )]),
            Body::from(
                r#"{"model":"gpt-5","input":"hello","stickyKey":"sticky-429-mixed-no-candidate"}"#
                    .as_bytes()
                    .to_vec(),
            ),
        )
        .await;

        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(
            response
                .headers()
                .get(http_header::RETRY_AFTER)
                .and_then(|value| value.to_str().ok()),
            Some("10")
        );
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read failure body");
        let payload: Value = serde_json::from_slice(&body).expect("decode failure payload");
        assert_eq!(
            payload["error"].as_str(),
            Some(POOL_NO_AVAILABLE_ACCOUNT_MESSAGE)
        );

        let attempts = attempts.lock().expect("lock attempts");
        assert_eq!(attempts.get("Bearer upstream-primary").copied(), Some(1));
        assert_eq!(attempts.get("Bearer upstream-secondary").copied(), None);

        upstream_handle.abort();
    });
}

#[tokio::test]
async fn pool_route_waits_for_header_sticky_account_before_first_attempt() {
    let (upstream_base, attempts, upstream_handle) = spawn_pool_retry_upstream(&[]).await;
    let state = test_state_with_openai_base_and_pool_no_available_wait(
        Url::parse(&upstream_base).expect("valid upstream base url"),
        Duration::from_millis(180),
        Duration::from_millis(10),
    )
    .await;
    seed_pool_routing_api_key(&state, "pool-live-key").await;
    let delayed_id = insert_test_pool_api_key_account(&state, "Delayed", "upstream-delayed").await;
    set_test_account_status(&state.pool, delayed_id, "needs_reauth").await;

    let request_state = state.clone();
    let request_task = tokio::spawn(async move {
        proxy_openai_v1(
            State(request_state),
            OriginalUri("/v1/responses".parse().expect("valid uri")),
            Method::POST,
            HeaderMap::from_iter([
                (
                    http_header::AUTHORIZATION,
                    HeaderValue::from_static("Bearer pool-live-key"),
                ),
                (
                    HeaderName::from_static("x-sticky-key"),
                    HeaderValue::from_static("sticky-wait-header"),
                ),
            ]),
            Body::from(r#"{"model":"gpt-5","input":"hello"}"#.as_bytes().to_vec()),
        )
        .await
    });

    let pool = state.pool.clone();
    let release_task = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(40)).await;
        set_test_account_status(&pool, delayed_id, "active").await;
    });

    let response = request_task
        .await
        .expect("header sticky request task should join");
    release_task
        .await
        .expect("delayed account release task should join");

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read proxy response");
    let payload: Value = serde_json::from_slice(&body).expect("decode proxy response");
    assert_eq!(payload["authorization"], "Bearer upstream-delayed");
    assert_eq!(payload["attempt"], 1);
    wait_for_pool_attempt_row_count(&state.pool, 1).await;
    assert_eq!(count_pool_upstream_request_attempts(&state.pool).await, 1);

    let attempts = attempts.lock().expect("lock attempts");
    assert_eq!(attempts.get("Bearer upstream-delayed").copied(), Some(1));

    upstream_handle.abort();
}

#[tokio::test]
async fn pool_route_waits_for_recovered_alternate_after_upstream_failure() {
    let (upstream_base, attempts, upstream_handle) = spawn_pool_static_failure_responses_upstream(
        &[("Bearer upstream-primary", StatusCode::INTERNAL_SERVER_ERROR)],
    )
    .await;
    let state = test_state_with_openai_base_and_pool_no_available_wait(
        Url::parse(&upstream_base).expect("valid upstream base url"),
        Duration::from_secs(2),
        Duration::from_millis(10),
    )
    .await;
    seed_pool_routing_api_key(&state, "pool-live-key").await;
    insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;
    let delayed_id = insert_test_pool_api_key_account(&state, "Delayed", "upstream-delayed").await;
    set_test_account_status(&state.pool, delayed_id, "needs_reauth").await;

    let wait_started_rx = crate::proxy::register_pool_no_available_wait_hook(&state);
    let request_state = state.clone();
    let request_task = tokio::spawn(async move {
        proxy_openai_v1(
            State(request_state),
            OriginalUri("/v1/responses".parse().expect("valid uri")),
            Method::POST,
            HeaderMap::from_iter([(
                http_header::AUTHORIZATION,
                HeaderValue::from_static("Bearer pool-live-key"),
            )]),
            Body::from(
                r#"{"model":"gpt-5","input":"hello","stickyKey":"sticky-recover-after-upstream-failure"}"#
                    .as_bytes()
                    .to_vec(),
            ),
        )
        .await
    });

    let wait_started_at = tokio::task::spawn_blocking(move || {
        wait_started_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("request should signal once the bounded wait starts");
        Instant::now()
    })
    .await
    .expect("wait hook worker should join");
    set_test_account_status(&state.pool, delayed_id, "active").await;
    invalidate_pool_routing_runtime_cache(state.as_ref()).await;
    state.pool_routing_availability.publish();

    let response = request_task.await.expect("request task should join");
    let elapsed = wait_started_at.elapsed();

    assert_eq!(response.status(), StatusCode::OK);
    assert!(
        elapsed < Duration::from_millis(900),
        "availability publication should wake the bounded wait before its timeout, elapsed={elapsed:?}"
    );
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read proxy response");
    let payload: Value = serde_json::from_slice(&body).expect("decode proxy response");
    assert_eq!(payload["authorization"], "Bearer upstream-delayed");
    assert_eq!(payload["attempt"], 1);

    wait_for_pool_attempt_row_count(&state.pool, 4).await;
    assert_eq!(count_pool_upstream_request_attempts(&state.pool).await, 4);

    let attempts = attempts.lock().expect("lock attempts");
    assert_eq!(attempts.get("Bearer upstream-primary").copied(), Some(3));
    assert_eq!(attempts.get("Bearer upstream-delayed").copied(), Some(1));

    upstream_handle.abort();
}

#[tokio::test]
async fn pool_route_existing_sticky_owner_waits_for_recovered_alternate_after_upstream_failure() {
    let (upstream_base, attempts, upstream_handle) = spawn_pool_static_failure_responses_upstream(
        &[("Bearer upstream-primary", StatusCode::INTERNAL_SERVER_ERROR)],
    )
    .await;
    let state = test_state_with_openai_base_and_pool_no_available_wait(
        Url::parse(&upstream_base).expect("valid upstream base url"),
        Duration::from_secs(2),
        Duration::from_millis(10),
    )
    .await;
    seed_pool_routing_api_key(&state, "pool-live-key").await;
    let primary_id = insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;
    let delayed_id = insert_test_pool_api_key_account(&state, "Delayed", "upstream-delayed").await;
    upsert_test_sticky_route_at(
        &state.pool,
        "sticky-existing-owner-wait-recovered",
        primary_id,
        &format_utc_iso(Utc::now()),
    )
    .await;
    set_test_account_generic_route_cooldown(&state.pool, primary_id, 120).await;
    set_test_account_status(&state.pool, delayed_id, "needs_reauth").await;

    let wait_started_rx = crate::proxy::register_pool_no_available_wait_hook(&state);
    let request_state = state.clone();
    let request_task = tokio::spawn(async move {
        proxy_openai_v1(
            State(request_state),
            OriginalUri("/v1/responses".parse().expect("valid uri")),
            Method::POST,
            HeaderMap::from_iter([(
                http_header::AUTHORIZATION,
                HeaderValue::from_static("Bearer pool-live-key"),
            )]),
            Body::from(
                r#"{"model":"gpt-5","input":"hello","stickyKey":"sticky-existing-owner-wait-recovered"}"#
                    .as_bytes()
                    .to_vec(),
            ),
        )
        .await
    });

    let wait_started_at = tokio::task::spawn_blocking(move || {
        wait_started_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("request should signal once the bounded wait starts");
        Instant::now()
    })
    .await
    .expect("wait hook worker should join");
    set_test_account_status(&state.pool, delayed_id, "active").await;
    invalidate_pool_routing_runtime_cache(state.as_ref()).await;
    state.pool_routing_availability.publish();

    let response = request_task.await.expect("request task should join");
    let elapsed = wait_started_at.elapsed();

    assert_eq!(response.status(), StatusCode::OK);
    assert!(
        elapsed < Duration::from_millis(900),
        "availability publication should wake the bounded wait before its timeout, elapsed={elapsed:?}"
    );
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read proxy response");
    let payload: Value = serde_json::from_slice(&body).expect("decode proxy response");
    assert_eq!(payload["authorization"], "Bearer upstream-delayed");
    assert_eq!(payload["attempt"], 1);

    wait_for_pool_attempt_row_count(&state.pool, 4).await;
    assert_eq!(count_pool_upstream_request_attempts(&state.pool).await, 4);

    let attempts = attempts.lock().expect("lock attempts");
    assert_eq!(attempts.get("Bearer upstream-primary").copied(), Some(3));
    assert_eq!(attempts.get("Bearer upstream-delayed").copied(), Some(1));
    drop(attempts);

    let mut route_account_id =
        load_test_sticky_route_account_id(&state.pool, "sticky-existing-owner-wait-recovered")
            .await;
    for _ in 0..20 {
        if route_account_id == Some(delayed_id) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
        route_account_id =
            load_test_sticky_route_account_id(&state.pool, "sticky-existing-owner-wait-recovered")
                .await;
    }
    assert_eq!(route_account_id, Some(delayed_id));

    upstream_handle.abort();
}

#[tokio::test]
async fn pool_route_body_sticky_returns_503_after_wait_timeout() {
    let state = test_state_with_openai_base_and_pool_no_available_wait(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
        Duration::from_millis(60),
        Duration::from_millis(10),
    )
    .await;
    seed_pool_routing_api_key(&state, "pool-live-key").await;
    let blocked_id = insert_test_pool_api_key_account(&state, "Blocked", "upstream-blocked").await;
    set_test_account_status(&state.pool, blocked_id, "needs_reauth").await;

    let started = Instant::now();
    let response = proxy_openai_v1(
        State(state.clone()),
        OriginalUri("/v1/responses".parse().expect("valid uri")),
        Method::POST,
        HeaderMap::from_iter([(
            http_header::AUTHORIZATION,
            HeaderValue::from_static("Bearer pool-live-key"),
        )]),
        Body::from(
            r#"{"model":"gpt-5","input":"hello","stickyKey":"sticky-wait-body-timeout"}"#
                .as_bytes()
                .to_vec(),
        ),
    )
    .await;

    assert!(
        started.elapsed() >= Duration::from_millis(50),
        "request should wait roughly the bounded window before failing"
    );
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        response
            .headers()
            .get(http_header::RETRY_AFTER)
            .and_then(|value| value.to_str().ok()),
        Some("10")
    );
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read failure body");
    let payload: Value = serde_json::from_slice(&body).expect("decode failure payload");
    assert_eq!(
        payload["error"].as_str(),
        Some(POOL_NO_AVAILABLE_ACCOUNT_MESSAGE)
    );
    assert_eq!(count_pool_upstream_request_attempts(&state.pool).await, 0);
}

#[tokio::test]
async fn pool_route_body_sticky_wait_timeout_returns_total_timeout_error_before_first_attempt() {
    let mut config = test_config();
    config.openai_upstream_base_url =
        Url::parse("https://api.openai.com/").expect("valid upstream base url");
    config.pool_upstream_responses_total_timeout = Duration::from_millis(45);
    let state = test_state_from_config_with_pool_no_available_wait(
        config,
        true,
        PoolNoAvailableWaitSettings {
            timeout: Duration::from_millis(220),
            poll_interval: Duration::from_millis(250),
            retry_after_secs: DEFAULT_POOL_NO_AVAILABLE_ACCOUNT_RETRY_AFTER_SECS,
        },
    )
    .await;
    seed_pool_routing_api_key(&state, "pool-live-key").await;
    let blocked_id = insert_test_pool_api_key_account(&state, "Blocked", "upstream-blocked").await;
    set_test_account_status(&state.pool, blocked_id, "needs_reauth").await;

    let started = Instant::now();
    let response = proxy_openai_v1(
        State(state.clone()),
        OriginalUri("/v1/responses".parse().expect("valid uri")),
        Method::POST,
        HeaderMap::from_iter([(
            http_header::AUTHORIZATION,
            HeaderValue::from_static("Bearer pool-live-key"),
        )]),
        Body::from(
            r#"{"model":"gpt-5","input":"hello","stickyKey":"sticky-wait-total-timeout"}"#
                .as_bytes()
                .to_vec(),
        ),
    )
    .await;
    let elapsed = started.elapsed();

    assert!(
        elapsed >= Duration::from_millis(25),
        "request should still wait briefly before failing, elapsed={elapsed:?}"
    );
    assert!(
        elapsed < Duration::from_secs(2),
        "responses total timeout should keep the pre-attempt no-account wait bounded on loaded runners, elapsed={elapsed:?}"
    );
    assert_eq!(response.status(), StatusCode::GATEWAY_TIMEOUT);
    assert_eq!(
        response
            .headers()
            .get(http_header::RETRY_AFTER)
            .and_then(|value| value.to_str().ok()),
        None
    );
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read failure body");
    let payload: Value = serde_json::from_slice(&body).expect("decode failure payload");
    assert_eq!(
        payload["error"].as_str(),
        Some(pool_total_timeout_exhausted_message(Duration::from_millis(45)).as_str())
    );
    assert_eq!(count_pool_upstream_request_attempts(&state.pool).await, 0);
}

#[tokio::test]
async fn resolve_pool_account_for_request_with_wait_respects_external_deadline() {
    let state = test_state_with_openai_base_and_pool_no_available_wait(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
        Duration::from_secs(2),
        Duration::from_secs(1),
    )
    .await;
    let blocked_id = insert_test_pool_api_key_account(&state, "Blocked", "upstream-blocked").await;
    set_test_account_status(&state.pool, blocked_id, "needs_reauth").await;

    let started = Instant::now();
    let mut wait_deadline = None;
    let resolution = resolve_pool_account_for_request_with_wait(
        state.as_ref(),
        None,
        &[],
        &HashSet::new(),
        None,
        true,
        &mut wait_deadline,
        Some(Instant::now() + Duration::from_millis(40)),
    )
    .await
    .expect("helper resolution should succeed");
    let elapsed = started.elapsed();

    assert!(
        elapsed < Duration::from_millis(700),
        "helper should stop on the external deadline instead of sleeping for the full wait window, elapsed={elapsed:?}"
    );
    assert!(
        matches!(
            resolution,
            PoolAccountResolutionWithWait::TotalTimeoutExpired
        ),
        "expected helper to stop on the external deadline, got {resolution:?}"
    );
    assert!(
        wait_deadline.is_some(),
        "bounded waits should record the deadline once they actually start"
    );
}

#[tokio::test]
async fn pool_route_selection_task_join_error_keeps_retryable_error_mapping() {
    let task = tokio_util::task::AbortOnDropHandle::new(tokio::spawn(async {
        panic!("synthetic pool route selection panic");
        #[allow(unreachable_code)]
        (Ok(PoolAccountResolutionWithWait::TotalTimeoutExpired), None)
    }));

    let fallback_deadline = Some(Instant::now() + Duration::from_secs(1));
    let (resolution, wait_deadline) =
        await_pool_route_selection_task(task, fallback_deadline).await;
    let err = resolution.expect_err("panicked selection task should surface as an error");
    assert!(err.to_string().contains("pool route selection task failed"));
    assert_eq!(wait_deadline, fallback_deadline);

    let mapped = build_pool_route_selection_failure_error(&err, 0, 0);
    assert_eq!(mapped.status, StatusCode::BAD_GATEWAY);
    assert_eq!(mapped.failure_kind, PROXY_FAILURE_POOL_NO_AVAILABLE_ACCOUNT);
    assert_eq!(mapped.attempt_summary.pool_attempt_count, 0);
    assert!(mapped.message.contains("pool route selection task failed"));
}

#[test]
fn elapsed_no_candidate_next_eligible_at_retries_with_a_bounded_delay() {
    let audit = PoolRoutingNoCandidateAudit {
        terminal_reason_code: "expiredCooldownProbe".to_string(),
        candidate_count: 1,
        eligible_candidate_count: 1,
        reservation_conflict_count: 0,
        next_eligible_at: Some((Utc::now() - chrono::Duration::milliseconds(1)).to_rfc3339()),
        excluded_reason_counts: std::collections::BTreeMap::from([(
            "expiredCooldownProbe".to_string(),
            1,
        )]),
        candidates: Vec::new(),
    };

    assert!(
        no_candidate_next_eligible_delay(&audit)
            .expect("expired next-eligible timestamp should schedule a reselect")
            >= Duration::from_millis(25),
        "a stale timestamp must not create a zero-delay database selection loop"
    );
}

#[tokio::test]
async fn failover_route_selection_task_preserves_the_external_deadline() {
    let state = test_state_with_openai_base_and_pool_no_available_wait(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
        Duration::from_secs(2),
        Duration::from_secs(1),
    )
    .await;
    let blocked_id = insert_test_pool_api_key_account(&state, "Blocked", "upstream-blocked").await;
    set_test_account_status(&state.pool, blocked_id, "needs_reauth").await;

    let started = Instant::now();
    let (resolution, wait_deadline) = resolve_pool_account_for_failover_on_fresh_task(
        state,
        None,
        None,
        Vec::new(),
        HashSet::new(),
        None,
        None,
        None,
        true,
        None,
        Some(Instant::now() + Duration::from_millis(40)),
        "/v1/responses".to_string(),
        crate::ImageIntent::Unknown,
        false,
        "deadline-selection-task".to_string(),
        PoolQuotaRoute::Normal,
    )
    .await;
    let elapsed = started.elapsed();

    assert!(
        matches!(
            resolution.expect("selection task should join successfully"),
            PoolAccountResolutionWithWait::TotalTimeoutExpired
        ),
        "selection task should preserve total-timeout expiry"
    );
    assert!(wait_deadline.is_some());
    assert!(
        elapsed < Duration::from_millis(700),
        "selection task should not outlive the external deadline, elapsed={elapsed:?}"
    );
}

#[tokio::test]
async fn dropping_pool_route_selection_task_wait_aborts_the_inner_task() {
    struct DropSignal(Arc<AtomicBool>);

    impl Drop for DropSignal {
        fn drop(&mut self) {
            self.0.store(true, Ordering::SeqCst);
        }
    }

    let dropped = Arc::new(AtomicBool::new(false));
    let inner_dropped = dropped.clone();
    let (started_tx, started_rx) = tokio::sync::oneshot::channel();
    let task = tokio_util::task::AbortOnDropHandle::new(tokio::spawn(async move {
        let _drop_signal = DropSignal(inner_dropped);
        let _ = started_tx.send(());
        std::future::pending::<()>().await;
        #[allow(unreachable_code)]
        (Ok(PoolAccountResolutionWithWait::TotalTimeoutExpired), None)
    }));
    let outer = tokio::spawn(await_pool_route_selection_task(task, None));
    started_rx
        .await
        .expect("inner selection task should start before the outer wait is aborted");

    outer.abort();
    let join_error = outer
        .await
        .expect_err("aborting the outer wait should cancel it");
    assert!(join_error.is_cancelled());
    tokio::time::timeout(Duration::from_secs(1), async {
        while !dropped.load(Ordering::SeqCst) {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("inner route selection task should be aborted and dropped");
}
